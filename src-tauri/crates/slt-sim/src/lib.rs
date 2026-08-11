//! A mock vehicle: HSFZ and DoIP servers in front of simulated ECUs.
//!
//! This exists so the whole stack, including the real framing and UDS client,
//! can be exercised end to end without a car. Anything that only works against
//! real hardware is a code path nobody tests until it matters.

pub mod ecu;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use slt_transport::{doip, hsfz, Protocol};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

pub use ecu::{Conditions, LampState, SimulatedEcu, SIM_LAMP_DID};

/// The simulated vehicle's ECUs, keyed by low byte of the diagnostic address.
pub type EcuMap = Arc<Mutex<HashMap<u8, SimulatedEcu>>>;

/// Builds a default simulated F-series vehicle.
pub fn default_vehicle() -> EcuMap {
    let mut map = HashMap::new();
    for (address, label) in [
        (0x10u8, "FEM_GW / ZGW"),
        (0x40, "FEM_BODY / BDC"),
        (0x41, "TMS left"),
        (0x42, "TMS right"),
        (0x43, "LHM left"),
        (0x44, "LHM right"),
        (0x72, "REM"),
        (0x60, "KOMBI"),
    ] {
        map.insert(address, SimulatedEcu::new(address, label));
    }
    Arc::new(Mutex::new(map))
}

/// A running simulator.
pub struct Simulator {
    pub protocol: Protocol,
    pub address: SocketAddr,
    pub ecus: EcuMap,
    task: JoinHandle<()>,
}

impl Drop for Simulator {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Simulator {
    /// Starts a simulator bound to loopback.
    ///
    /// Passing port 0 lets the OS choose, which keeps concurrent tests from
    /// colliding; the chosen port is reported back in [`Simulator::address`].
    pub async fn start(protocol: Protocol, port: u16) -> std::io::Result<Self> {
        Self::start_with(protocol, port, default_vehicle()).await
    }

    /// Starts a simulator over a caller-supplied set of ECUs, so a test can
    /// pre-seed faults or conditions.
    pub async fn start_with(
        protocol: Protocol,
        port: u16,
        ecus: EcuMap,
    ) -> std::io::Result<Self> {
        let listener =
            TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)).await?;
        let address = listener.local_addr()?;
        let served = Arc::clone(&ecus);

        let task = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, peer)) => {
                        tracing::debug!(%peer, "simulator accepted a connection");
                        let ecus = Arc::clone(&served);
                        tokio::spawn(async move {
                            let result = match protocol {
                                Protocol::Hsfz => serve_hsfz(stream, ecus).await,
                                Protocol::DoIp => serve_doip(stream, ecus).await,
                                Protocol::IsoTp => {
                                    Err(std::io::Error::new(
                                        std::io::ErrorKind::Unsupported,
                                        "simulator has no ISO-TP listener yet",
                                    ))
                                }
                            };
                            if let Err(e) = result {
                                tracing::debug!(error = %e, "simulator session ended");
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "simulator accept failed");
                        return;
                    }
                }
            }
        });

        tracing::info!(%address, protocol = protocol.as_str(), "simulator listening");
        Ok(Self {
            protocol,
            address,
            ecus,
            task,
        })
    }

    pub fn ip(&self) -> IpAddr {
        self.address.ip()
    }

    pub fn port(&self) -> u16 {
        self.address.port()
    }
}

/// Serves one HSFZ client.
async fn serve_hsfz(mut stream: TcpStream, ecus: EcuMap) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 2048];

    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..n]);

        while let Ok(Some((frame, consumed))) = hsfz::decode_frame(&buffer) {
            buffer.drain(..consumed);
            if frame.control != hsfz::ControlWord::Diagnostic {
                continue;
            }
            let Some((src, dst)) = frame.addresses else {
                continue;
            };

            // Real gateways acknowledge before the ECU answers, and a client
            // that cannot cope with that will fail here rather than in a car.
            let ack = hsfz::Frame {
                control: hsfz::ControlWord::Acknowledge,
                addresses: Some((dst, src)),
                payload: Vec::new(),
            };
            stream.write_all(&ack.encode()).await?;

            let responses = {
                let mut guard = ecus.lock().await;
                match guard.get_mut(&dst) {
                    Some(ecu) => ecu.handle_sequence(&frame.payload),
                    None => {
                        // No such module: the gateway reports a bad destination.
                        let error = hsfz::Frame {
                            control: hsfz::ControlWord::IncorrectDestinationAddress,
                            addresses: None,
                            payload: Vec::new(),
                        };
                        stream.write_all(&error.encode()).await?;
                        stream.flush().await?;
                        continue;
                    }
                }
            };

            for response in responses {
                // An empty response models a suppressed positive response.
                if response.is_empty() {
                    continue;
                }
                let reply = hsfz::Frame::diagnostic(dst, src, response);
                stream.write_all(&reply.encode()).await?;
            }
            stream.flush().await?;
        }
    }
}

/// Serves one DoIP client, including the routing activation handshake.
async fn serve_doip(mut stream: TcpStream, ecus: EcuMap) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 2048];
    let mut routing_active = false;

    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..n]);

        while let Ok(Some((message, consumed))) = doip::decode_message(&buffer) {
            buffer.drain(..consumed);

            match message.payload_type {
                doip::PayloadType::RoutingActivationRequest => {
                    let tester = u16::from_be_bytes([
                        message.payload.first().copied().unwrap_or(0),
                        message.payload.get(1).copied().unwrap_or(0),
                    ]);
                    let mut payload = Vec::new();
                    payload.extend_from_slice(&tester.to_be_bytes());
                    payload.extend_from_slice(&0x0010u16.to_be_bytes());
                    payload.push(0x10); // routing successfully activated
                    payload.extend_from_slice(&[0x00; 4]);
                    let response =
                        doip::Message::new(doip::PayloadType::RoutingActivationResponse, payload);
                    stream.write_all(&response.encode()).await?;
                    stream.flush().await?;
                    routing_active = true;
                }
                doip::PayloadType::DiagnosticMessage => {
                    // ISO 13400 forbids diagnostics before activation, and the
                    // client must not be allowed to get away with skipping it.
                    if !routing_active {
                        let nack =
                            doip::Message::new(doip::PayloadType::GenericNack, vec![0x02]);
                        stream.write_all(&nack.encode()).await?;
                        stream.flush().await?;
                        continue;
                    }
                    let Ok((source, target, uds)) = message.as_diagnostic() else {
                        continue;
                    };

                    let ack_payload = {
                        let mut p = Vec::new();
                        p.extend_from_slice(&target.to_be_bytes());
                        p.extend_from_slice(&source.to_be_bytes());
                        p.push(0x00); // ack code
                        p
                    };
                    let ack = doip::Message::new(
                        doip::PayloadType::DiagnosticMessageAck,
                        ack_payload,
                    );
                    stream.write_all(&ack.encode()).await?;

                    let responses = {
                        let mut guard = ecus.lock().await;
                        match guard.get_mut(&(target as u8)) {
                            Some(ecu) => ecu.handle_sequence(uds),
                            None => {
                                let mut p = Vec::new();
                                p.extend_from_slice(&target.to_be_bytes());
                                p.extend_from_slice(&source.to_be_bytes());
                                p.push(0x03); // unknown target address
                                let nack = doip::Message::new(
                                    doip::PayloadType::DiagnosticMessageNack,
                                    p,
                                );
                                stream.write_all(&nack.encode()).await?;
                                stream.flush().await?;
                                continue;
                            }
                        }
                    };

                    for response in responses {
                        if response.is_empty() {
                            continue;
                        }
                        let reply = doip::Message::diagnostic(target, source, &response);
                        stream.write_all(&reply.encode()).await?;
                    }
                    stream.flush().await?;
                }
                doip::PayloadType::AliveCheckResponse => {}
                other => {
                    tracing::debug!(?other, "simulator ignoring message");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slt_transport::{ecu as ecu_addr, Connection};
    use std::time::Duration;

    const TIMEOUT: Duration = Duration::from_secs(2);

    /// Exercises the real HSFZ client against the simulator, which validates the
    /// framing, the acknowledgement handling and the UDS layer together.
    #[tokio::test]
    async fn hsfz_client_reads_vin_through_real_framing() {
        let sim = Simulator::start(Protocol::Hsfz, 0).await.unwrap();
        let connection = Connection::open(Protocol::Hsfz, sim.ip(), Some(sim.port()), TIMEOUT)
            .await
            .unwrap();
        let client = slt_uds::UdsClient::new(connection);

        let vin = client
            .read_string(ecu_addr::FEM_GW, slt_uds::did::VIN)
            .await
            .unwrap();
        assert_eq!(vin, "WBA8E9G51GNT12345");
    }

    #[tokio::test]
    async fn doip_client_reads_vin_after_routing_activation() {
        let sim = Simulator::start(Protocol::DoIp, 0).await.unwrap();
        let connection = Connection::open(Protocol::DoIp, sim.ip(), Some(sim.port()), TIMEOUT)
            .await
            .unwrap();
        let client = slt_uds::UdsClient::new(connection);

        let vin = client
            .read_string(ecu_addr::FEM_BODY, slt_uds::did::VIN)
            .await
            .unwrap();
        assert_eq!(vin, "WBA8E9G51GNT12345");
    }

    #[tokio::test]
    async fn absent_ecu_surfaces_a_gateway_rejection() {
        let sim = Simulator::start(Protocol::Hsfz, 0).await.unwrap();
        let connection = Connection::open(Protocol::Hsfz, sim.ip(), Some(sim.port()), TIMEOUT)
            .await
            .unwrap();
        let client = slt_uds::UdsClient::new(connection);

        // 0x55 is not a module the simulated vehicle has.
        assert!(!client.probe(slt_transport::EcuAddress::new(0x55)).await);
    }

    #[tokio::test]
    async fn scan_finds_the_seeded_modules() {
        let sim = Simulator::start(Protocol::Hsfz, 0).await.unwrap();
        let connection = Connection::open(Protocol::Hsfz, sim.ip(), Some(sim.port()), TIMEOUT)
            .await
            .unwrap();
        let client = slt_uds::UdsClient::new(connection);

        let results = client.scan_lighting_ecus().await;
        assert!(results.iter().all(|r| r.present));
        assert!(results
            .iter()
            .any(|r| r.address == 0x40 && r.serial.is_some()));
    }

    #[tokio::test]
    async fn response_pending_is_handled_transparently() {
        let ecus = default_vehicle();
        {
            let mut guard = ecus.lock().await;
            guard.get_mut(&0x40).unwrap().pending_responses = 3;
        }
        let sim = Simulator::start_with(Protocol::Hsfz, 0, ecus).await.unwrap();
        let connection = Connection::open(Protocol::Hsfz, sim.ip(), Some(sim.port()), TIMEOUT)
            .await
            .unwrap();
        let client = slt_uds::UdsClient::new(connection);

        // Every request is preceded by three 0x78 frames; the client must keep
        // reading past them rather than treating them as a failure.
        let vin = client
            .read_string(ecu_addr::FEM_BODY, slt_uds::did::VIN)
            .await
            .unwrap();
        assert_eq!(vin, "WBA8E9G51GNT12345");
    }

    #[tokio::test]
    async fn actuation_and_release_move_simulated_lamp_state() {
        let sim = Simulator::start(Protocol::Hsfz, 0).await.unwrap();
        let connection = Connection::open(Protocol::Hsfz, sim.ip(), Some(sim.port()), TIMEOUT)
            .await
            .unwrap();
        let client = slt_uds::UdsClient::new(connection);

        client
            .start_session(ecu_addr::FEM_BODY, slt_uds::Session::Extended)
            .await
            .unwrap();
        client
            .io_control(
                ecu_addr::FEM_BODY,
                SIM_LAMP_DID,
                slt_uds::IoControl::ShortTermAdjustment,
                &[0x30, 100],
            )
            .await
            .unwrap();

        {
            let mut guard = sim.ecus.lock().await;
            let fem = guard.get_mut(&0x40).unwrap();
            assert_eq!(fem.lamp_state(0x30).level, 100);
        }

        client
            .release_io(ecu_addr::FEM_BODY, SIM_LAMP_DID)
            .await
            .unwrap();

        {
            let mut guard = sim.ecus.lock().await;
            let fem = guard.get_mut(&0x40).unwrap();
            assert_eq!(fem.controlled_count(), 0);
        }
    }

    #[tokio::test]
    async fn guard_blocks_a_programming_session_before_it_reaches_the_wire() {
        let sim = Simulator::start(Protocol::Hsfz, 0).await.unwrap();
        let connection = Connection::open(Protocol::Hsfz, sim.ip(), Some(sim.port()), TIMEOUT)
            .await
            .unwrap();
        let client = slt_uds::UdsClient::new(connection);

        let err = client
            .start_session(ecu_addr::FEM_BODY, slt_uds::Session::Programming)
            .await
            .unwrap_err();
        assert!(matches!(err, slt_uds::UdsError::Blocked(_)));
    }

    #[tokio::test]
    async fn dtcs_round_trip_through_the_client() {
        let ecus = default_vehicle();
        {
            let mut guard = ecus.lock().await;
            guard.get_mut(&0x40).unwrap().add_dtc(0x8040B8, 0x08);
        }
        let sim = Simulator::start_with(Protocol::Hsfz, 0, ecus).await.unwrap();
        let connection = Connection::open(Protocol::Hsfz, sim.ip(), Some(sim.port()), TIMEOUT)
            .await
            .unwrap();
        let client = slt_uds::UdsClient::new(connection);

        let dtcs = client
            .read_dtcs(ecu_addr::FEM_BODY, slt_uds::dtc::STATUS_MASK_ALL)
            .await
            .unwrap();
        assert_eq!(dtcs.len(), 1);
        assert_eq!(dtcs[0].code_hex, "0x8040B8");

        client.clear_dtcs(ecu_addr::FEM_BODY).await.unwrap();
        assert!(client
            .read_dtcs(ecu_addr::FEM_BODY, slt_uds::dtc::STATUS_MASK_ALL)
            .await
            .unwrap()
            .is_empty());
    }
}
