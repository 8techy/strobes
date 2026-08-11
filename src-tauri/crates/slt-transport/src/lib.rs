//! Diagnostic transports for vehicle connections.
//!
//! BMW ENET uses two wire protocols for the same UDS payloads: HSFZ on F-series
//! and DoIP on G-series. ZN8 / ZC8 (GR86 / BRZ) research adds ISO-TP framing
//! over CAN; the SocketCAN backend is not wired yet. [`Connection`] hides the
//! difference so higher layers only deal in UDS bytes and an ECU address.
//!
//! Protocol details live with the BMW ENET crates; ZN8 uses ISO-TP over CAN.

pub mod can;
pub mod discovery;
pub mod doip;
pub mod error;
pub mod hsfz;
pub mod isotp;

use std::net::IpAddr;
use std::time::Duration;

pub use can::{CanFrame, IsoTpConnection, LoopbackBus, MockEcu};
pub use discovery::{discover, DiscoveredVehicle};
pub use error::{Result, TransportError};

/// Which wire protocol a vehicle speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    /// BMW proprietary, F-series. TCP 6801.
    Hsfz,
    /// ISO 13400, G-series. TCP 13400.
    DoIp,
    /// ISO 15765-2 over CAN. ZN8 / ZC8 research; use host `loopback` without hardware.
    IsoTp,
}

impl Protocol {
    pub fn default_port(self) -> u16 {
        match self {
            Self::Hsfz => hsfz::DIAGNOSTIC_PORT,
            Self::DoIp => doip::PORT,
            // CAN interfaces are named, not TCP-ported. Zero signals "unused".
            Self::IsoTp => 0,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hsfz => "HSFZ",
            Self::DoIp => "DoIP",
            Self::IsoTp => "ISO-TP",
        }
    }
}

/// An ECU diagnostic address.
///
/// HSFZ addresses are one byte; DoIP uses two. Storing a `u16` throughout lets
/// the catalog use a single representation, with the HSFZ path narrowing to the
/// low byte on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct EcuAddress(pub u16);

impl EcuAddress {
    pub const fn new(address: u16) -> Self {
        Self(address)
    }

    /// The single-byte form used by HSFZ.
    pub fn as_hsfz(self) -> u8 {
        self.0 as u8
    }

    /// The two-byte logical form used by DoIP.
    pub fn as_doip(self) -> u16 {
        self.0
    }
}

impl std::fmt::Display for EcuAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0 <= 0xFF {
            write!(f, "0x{:02X}", self.0)
        } else {
            write!(f, "0x{:04X}", self.0)
        }
    }
}

/// A connection to a vehicle, independent of wire protocol.
///
/// Enum dispatch rather than a trait object keeps the call free of allocation
/// and virtual dispatch on the hot path, which matters because the effect
/// scheduler drives this at up to ~50 requests per second.
pub enum Connection {
    Hsfz(hsfz::HsfzConnection),
    DoIp(doip::DoIpConnection),
    IsoTp(can::IsoTpConnection),
}

impl Connection {
    /// Opens a connection using the given protocol.
    ///
    /// `host` is an IP address for HSFZ/DoIP, or a CAN endpoint name for ISO-TP
    /// (`loopback` for the in-process mock; SocketCAN interface names later).
    pub async fn open(
        protocol: Protocol,
        host: &str,
        port: Option<u16>,
        timeout: Duration,
    ) -> Result<Self> {
        Ok(match protocol {
            Protocol::Hsfz | Protocol::DoIp => {
                let ip: IpAddr = host.parse().map_err(|_| {
                    TransportError::MalformedFrame(format!(
                        "'{host}' is not a valid IP address for {}",
                        protocol.as_str()
                    ))
                })?;
                let port = port.unwrap_or_else(|| protocol.default_port());
                match protocol {
                    Protocol::Hsfz => {
                        Self::Hsfz(hsfz::HsfzConnection::connect(ip, port, timeout).await?)
                    }
                    Protocol::DoIp => {
                        Self::DoIp(doip::DoIpConnection::connect(ip, port, timeout).await?)
                    }
                    Protocol::IsoTp => unreachable!(),
                }
            }
            Protocol::IsoTp => {
                let endpoint = host.trim().to_ascii_lowercase();
                if endpoint == "loopback" || endpoint == "sim" || endpoint.is_empty() {
                    Self::IsoTp(can::IsoTpConnection::connect_loopback(timeout).await?)
                } else {
                    return Err(TransportError::IsoTpNotImplemented(
                        "only the 'loopback' ISO-TP endpoint is available so far; SocketCAN comes next",
                    ));
                }
            }
        })
    }

    /// Convenience for tests that already have an [`IpAddr`].
    pub async fn open_ip(
        protocol: Protocol,
        ip: IpAddr,
        port: Option<u16>,
        timeout: Duration,
    ) -> Result<Self> {
        Self::open(protocol, &ip.to_string(), port, timeout).await
    }

    pub fn protocol(&self) -> Protocol {
        match self {
            Self::Hsfz(_) => Protocol::Hsfz,
            Self::DoIp(_) => Protocol::DoIp,
            Self::IsoTp(_) => Protocol::IsoTp,
        }
    }

    /// Sends a UDS request to `target` and returns the raw UDS response.
    ///
    /// Protocol acknowledgements and keepalive probes are handled internally,
    /// so the caller always receives an actual UDS response or an error.
    pub async fn request(&mut self, target: EcuAddress, payload: &[u8]) -> Result<Vec<u8>> {
        match self {
            Self::Hsfz(c) => c.request(target.as_hsfz(), payload).await,
            Self::DoIp(c) => c.request(target.as_doip(), payload).await,
            Self::IsoTp(c) => c.request(target.0, payload).await,
        }
    }

    /// Waits for a further response to a request already sent.
    ///
    /// Used by the UDS layer when an ECU answers `responsePending`.
    pub async fn receive(&mut self, target: EcuAddress) -> Result<Vec<u8>> {
        match self {
            Self::Hsfz(c) => c.receive(target.as_hsfz()).await,
            Self::DoIp(c) => c.receive(target.as_doip()).await,
            Self::IsoTp(c) => c.receive(target.0).await,
        }
    }
}

/// Well-known BMW ECU diagnostic addresses.
///
/// Community-sourced; see `docs/protocol-research.md` section 2.
pub mod ecu {
    use super::EcuAddress;

    /// Gateway. Always present, so it makes a good connectivity probe.
    pub const FEM_GW: EcuAddress = EcuAddress::new(0x10);
    /// Front Electronics Module / Body Domain Controller. Owns exterior lighting.
    pub const FEM_BODY: EcuAddress = EcuAddress::new(0x40);
    /// LED headlight secondary module, left.
    pub const TMS_LEFT: EcuAddress = EcuAddress::new(0x41);
    /// LED headlight secondary module, right.
    pub const TMS_RIGHT: EcuAddress = EcuAddress::new(0x42);
    /// LED headlight main module, left.
    pub const LHM_LEFT: EcuAddress = EcuAddress::new(0x43);
    /// LED headlight main module, right.
    pub const LHM_RIGHT: EcuAddress = EcuAddress::new(0x44);
    /// Rear Electronics Module. Owns rear lighting.
    pub const REM: EcuAddress = EcuAddress::new(0x72);
    /// Instrument cluster.
    pub const KOMBI: EcuAddress = EcuAddress::new(0x60);
    /// Engine control. Never a valid actuation target for this application.
    pub const DME: EcuAddress = EcuAddress::new(0x12);

    /// Modules probed when scanning for lighting capability, with labels.
    pub const LIGHTING_SCAN: &[(EcuAddress, &str)] = &[
        (FEM_GW, "FEM_GW / ZGW (gateway)"),
        (FEM_BODY, "FEM_BODY / BDC (body, exterior lighting)"),
        (TMS_LEFT, "TMS left (headlight secondary)"),
        (TMS_RIGHT, "TMS right (headlight secondary)"),
        (LHM_LEFT, "LHM left (headlight main)"),
        (LHM_RIGHT, "LHM right (headlight main)"),
        (REM, "REM (rear electronics)"),
        (KOMBI, "KOMBI (instrument cluster)"),
    ];
}

/// Hypothesized ZN8 / ZC8 (GR86 / BRZ) diagnostic addresses.
///
/// These are community-reported ISO-TP CAN identifiers, not confirmed in this
/// repository. Treat as research leads until confirmed on a real vehicle.
pub mod zn8 {
    use super::EcuAddress;

    /// Powertrain / ECM request ID. Never an actuation target.
    pub const ECM_REQUEST: EcuAddress = EcuAddress::new(0x7E0);
    /// Powertrain / ECM response ID (informational; transport pairs this).
    pub const ECM_RESPONSE: u16 = 0x7E8;
    /// Body Control Module request ID. Primary lighting candidate.
    pub const BCM_REQUEST: EcuAddress = EcuAddress::new(0x7E1);
    /// Body Control Module response ID.
    pub const BCM_RESPONSE: u16 = 0x7E9;

    /// Modules to probe once an ISO-TP backend exists.
    pub const LIGHTING_SCAN: &[(EcuAddress, &str)] = &[
        (BCM_REQUEST, "BCM (body control, exterior lighting)"),
        (ECM_REQUEST, "ECM (powertrain — probe only, never actuate)"),
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecu_address_narrows_for_hsfz() {
        assert_eq!(ecu::FEM_BODY.as_hsfz(), 0x40);
        assert_eq!(ecu::FEM_BODY.as_doip(), 0x0040);
    }

    #[test]
    fn ecu_address_display_matches_width() {
        assert_eq!(ecu::FEM_BODY.to_string(), "0x40");
        assert_eq!(EcuAddress::new(0x0E80).to_string(), "0x0E80");
    }

    #[test]
    fn protocols_use_standard_ports() {
        assert_eq!(Protocol::Hsfz.default_port(), 6801);
        assert_eq!(Protocol::DoIp.default_port(), 13400);
        assert_eq!(Protocol::IsoTp.default_port(), 0);
    }

    #[test]
    fn dme_is_not_in_the_lighting_scan_list() {
        assert!(!ecu::LIGHTING_SCAN.iter().any(|(a, _)| *a == ecu::DME));
    }

    #[test]
    fn zn8_bcm_is_distinct_from_ecm() {
        assert_ne!(zn8::BCM_REQUEST, zn8::ECM_REQUEST);
        assert_eq!(zn8::BCM_REQUEST.0, 0x7E1);
    }

    #[tokio::test]
    async fn isotp_loopback_endpoint_connects() {
        let conn = Connection::open(
            Protocol::IsoTp,
            "loopback",
            None,
            Duration::from_millis(200),
        )
        .await
        .expect("loopback should connect");
        assert_eq!(conn.protocol(), Protocol::IsoTp);
    }

    #[tokio::test]
    async fn isotp_unknown_interface_is_rejected() {
        let result = Connection::open(
            Protocol::IsoTp,
            "can0",
            None,
            Duration::from_millis(10),
        )
        .await;
        assert!(matches!(
            result,
            Err(TransportError::IsoTpNotImplemented(_))
        ));
    }
}
