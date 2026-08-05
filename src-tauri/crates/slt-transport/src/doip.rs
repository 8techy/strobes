//! DoIP (Diagnostics over IP, ISO 13400), used by BMW G-series vehicles.
//!
//! Unlike HSFZ this is an open standard with 2-byte logical addressing, and it
//! requires a routing activation handshake before any diagnostic message will
//! be accepted.
//!
//! See `docs/protocol-research.md` section 1.2.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::error::{DoIpNackCode, Result, RoutingActivationDenial, TransportError};

/// Standard DoIP port, used for both TCP and UDP.
pub const PORT: u16 = 13400;
/// Conventional tester logical address.
pub const TESTER_ADDRESS: u16 = 0x0E80;

const PROTOCOL_VERSION: u8 = 0x02;
const MAX_PAYLOAD_LEN: usize = 64 * 1024;
/// Routing activation type 0 (default). Type 1 is regulatory diagnostics.
const ACTIVATION_TYPE_DEFAULT: u8 = 0x00;
const ROUTING_ACTIVATION_SUCCESS: u8 = 0x10;

/// DoIP payload types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum PayloadType {
    GenericNack = 0x0000,
    VehicleIdentRequest = 0x0001,
    VehicleIdentRequestByEid = 0x0002,
    VehicleIdentRequestByVin = 0x0003,
    VehicleAnnouncement = 0x0004,
    RoutingActivationRequest = 0x0005,
    RoutingActivationResponse = 0x0006,
    AliveCheckRequest = 0x0007,
    AliveCheckResponse = 0x0008,
    DiagnosticMessage = 0x8001,
    DiagnosticMessageAck = 0x8002,
    DiagnosticMessageNack = 0x8003,
}

impl PayloadType {
    pub fn from_u16(value: u16) -> Option<Self> {
        Some(match value {
            0x0000 => Self::GenericNack,
            0x0001 => Self::VehicleIdentRequest,
            0x0002 => Self::VehicleIdentRequestByEid,
            0x0003 => Self::VehicleIdentRequestByVin,
            0x0004 => Self::VehicleAnnouncement,
            0x0005 => Self::RoutingActivationRequest,
            0x0006 => Self::RoutingActivationResponse,
            0x0007 => Self::AliveCheckRequest,
            0x0008 => Self::AliveCheckResponse,
            0x8001 => Self::DiagnosticMessage,
            0x8002 => Self::DiagnosticMessageAck,
            0x8003 => Self::DiagnosticMessageNack,
            _ => return None,
        })
    }
}

/// A decoded DoIP message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub payload_type: PayloadType,
    pub payload: Vec<u8>,
}

impl Message {
    pub fn new(payload_type: PayloadType, payload: Vec<u8>) -> Self {
        Self {
            payload_type,
            payload,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + self.payload.len());
        out.push(PROTOCOL_VERSION);
        out.push(!PROTOCOL_VERSION);
        out.extend_from_slice(&(self.payload_type as u16).to_be_bytes());
        out.extend_from_slice(&(self.payload.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.payload);
        out
    }

    /// Builds a diagnostic message carrying a UDS payload.
    pub fn diagnostic(source: u16, target: u16, uds: &[u8]) -> Self {
        let mut payload = Vec::with_capacity(4 + uds.len());
        payload.extend_from_slice(&source.to_be_bytes());
        payload.extend_from_slice(&target.to_be_bytes());
        payload.extend_from_slice(uds);
        Self::new(PayloadType::DiagnosticMessage, payload)
    }

    /// Builds a routing activation request.
    pub fn routing_activation(source: u16) -> Self {
        let mut payload = Vec::with_capacity(7);
        payload.extend_from_slice(&source.to_be_bytes());
        payload.push(ACTIVATION_TYPE_DEFAULT);
        // 4 reserved bytes, ISO 13400 requires them to be zero.
        payload.extend_from_slice(&[0x00; 4]);
        Self::new(PayloadType::RoutingActivationRequest, payload)
    }

    pub fn vehicle_ident_request() -> Self {
        Self::new(PayloadType::VehicleIdentRequest, Vec::new())
    }

    /// Splits a diagnostic message payload into source, target and UDS bytes.
    pub fn as_diagnostic(&self) -> Result<(u16, u16, &[u8])> {
        if self.payload.len() < 4 {
            return Err(TransportError::MalformedFrame(format!(
                "diagnostic message needs 4 address bytes, got {}",
                self.payload.len()
            )));
        }
        let source = u16::from_be_bytes([self.payload[0], self.payload[1]]);
        let target = u16::from_be_bytes([self.payload[2], self.payload[3]]);
        Ok((source, target, &self.payload[4..]))
    }
}

/// Decodes a message, returning it with the bytes consumed. `Ok(None)` means
/// more data is needed.
pub fn decode_message(buf: &[u8]) -> Result<Option<(Message, usize)>> {
    if buf.len() < 8 {
        return Ok(None);
    }
    let version = buf[0];
    let inverse = buf[1];
    if inverse != !version {
        return Err(TransportError::MalformedFrame(format!(
            "protocol version {version:#04X} does not match its inverse {inverse:#04X}"
        )));
    }
    let raw_type = u16::from_be_bytes([buf[2], buf[3]]);
    let payload_len = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
    if payload_len > MAX_PAYLOAD_LEN {
        return Err(TransportError::FrameTooLarge {
            actual: payload_len,
            max: MAX_PAYLOAD_LEN,
        });
    }
    let total = 8 + payload_len;
    if buf.len() < total {
        return Ok(None);
    }
    let payload_type = PayloadType::from_u16(raw_type).ok_or_else(|| {
        TransportError::MalformedFrame(format!("unknown DoIP payload type {raw_type:#06X}"))
    })?;
    Ok(Some((
        Message::new(payload_type, buf[8..total].to_vec()),
        total,
    )))
}

/// Vehicle identification data from a DoIP announcement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VehicleAnnouncement {
    pub vin: String,
    pub logical_address: u16,
    pub eid: [u8; 6],
    pub gid: [u8; 6],
    pub further_action: u8,
}

impl VehicleAnnouncement {
    /// Parses a 32- or 33-byte announcement payload.
    pub fn parse(payload: &[u8]) -> Result<Self> {
        if payload.len() < 32 {
            return Err(TransportError::MalformedFrame(format!(
                "vehicle announcement needs at least 32 bytes, got {}",
                payload.len()
            )));
        }
        let vin = String::from_utf8_lossy(&payload[0..17])
            .trim_matches(|c: char| c == '\0' || c.is_whitespace())
            .to_string();
        let logical_address = u16::from_be_bytes([payload[17], payload[18]]);
        let mut eid = [0u8; 6];
        eid.copy_from_slice(&payload[19..25]);
        let mut gid = [0u8; 6];
        gid.copy_from_slice(&payload[25..31]);
        Ok(Self {
            vin,
            logical_address,
            eid,
            gid,
            further_action: payload[31],
        })
    }
}

/// A live DoIP connection with routing already activated.
pub struct DoIpConnection {
    stream: TcpStream,
    tester: u16,
    buffer: Vec<u8>,
    timeout: Duration,
}

impl DoIpConnection {
    /// Connects and performs routing activation, which ISO 13400 requires
    /// before any diagnostic message is accepted.
    pub async fn connect(ip: IpAddr, port: u16, timeout: Duration) -> Result<Self> {
        let addr = SocketAddr::new(ip, port);
        let stream = tokio::time::timeout(timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| TransportError::Timeout(timeout))??;
        stream.set_nodelay(true)?;

        let mut conn = Self {
            stream,
            tester: TESTER_ADDRESS,
            buffer: Vec::with_capacity(1024),
            timeout,
        };
        conn.activate_routing().await?;
        Ok(conn)
    }

    pub fn tester_address(&self) -> u16 {
        self.tester
    }

    async fn activate_routing(&mut self) -> Result<()> {
        let request = Message::routing_activation(self.tester);
        self.stream.write_all(&request.encode()).await?;
        self.stream.flush().await?;

        loop {
            let message = self.read_message().await?;
            match message.payload_type {
                PayloadType::RoutingActivationResponse => {
                    // [tester_addr(2)][entity_addr(2)][response_code(1)][reserved(4)]
                    let code = message.payload.get(4).copied().ok_or_else(|| {
                        TransportError::MalformedFrame(
                            "routing activation response is missing its response code".into(),
                        )
                    })?;
                    if code == ROUTING_ACTIVATION_SUCCESS {
                        tracing::info!("DoIP routing activated");
                        return Ok(());
                    }
                    return Err(TransportError::RoutingActivationDenied(
                        RoutingActivationDenial::from_code(code),
                    ));
                }
                PayloadType::GenericNack => {
                    let code = message.payload.first().copied().unwrap_or(0);
                    return Err(TransportError::DoIpNack(DoIpNackCode::from_code(code)));
                }
                other => {
                    tracing::debug!(?other, "ignoring message while awaiting routing activation");
                }
            }
        }
    }

    /// Sends a UDS payload to `target` and returns the UDS response.
    pub async fn request(&mut self, target: u16, payload: &[u8]) -> Result<Vec<u8>> {
        self.send(target, payload).await?;
        self.receive(target).await
    }

    /// Sends a diagnostic message without waiting for a response.
    pub async fn send(&mut self, target: u16, payload: &[u8]) -> Result<()> {
        let message = Message::diagnostic(self.tester, target, payload);
        self.stream.write_all(&message.encode()).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Waits for the next diagnostic response from `target` without sending.
    ///
    /// Needed because UDS `responsePending` (NRC 0x78) requires reading further
    /// responses for a request that was already transmitted.
    pub async fn receive(&mut self, target: u16) -> Result<Vec<u8>> {
        loop {
            let message = self.read_message().await?;
            match message.payload_type {
                // Positive ack precedes the real response.
                PayloadType::DiagnosticMessageAck => continue,
                PayloadType::DiagnosticMessageNack => {
                    // [source(2)][target(2)][nack_code(1)]
                    let code = message.payload.get(4).copied().unwrap_or(0);
                    return Err(TransportError::DoIpNack(DoIpNackCode::from_code(code)));
                }
                PayloadType::GenericNack => {
                    let code = message.payload.first().copied().unwrap_or(0);
                    return Err(TransportError::DoIpNack(DoIpNackCode::from_code(code)));
                }
                PayloadType::AliveCheckRequest => {
                    let response = Message::new(
                        PayloadType::AliveCheckResponse,
                        self.tester.to_be_bytes().to_vec(),
                    );
                    self.stream.write_all(&response.encode()).await?;
                    self.stream.flush().await?;
                    continue;
                }
                PayloadType::DiagnosticMessage => {
                    let (source, _target, uds) = message.as_diagnostic()?;
                    if source != target {
                        tracing::warn!(
                            expected = format!("0x{target:04X}"),
                            actual = format!("0x{source:04X}"),
                            "discarding response from unexpected ECU"
                        );
                        continue;
                    }
                    return Ok(uds.to_vec());
                }
                other => {
                    tracing::debug!(?other, "ignoring unexpected message");
                }
            }
        }
    }

    async fn read_message(&mut self) -> Result<Message> {
        loop {
            if let Some((message, consumed)) = decode_message(&self.buffer)? {
                self.buffer.drain(..consumed);
                return Ok(message);
            }
            let mut chunk = [0u8; 2048];
            let n = tokio::time::timeout(self.timeout, self.stream.read(&mut chunk))
                .await
                .map_err(|_| TransportError::Timeout(self.timeout))??;
            if n == 0 {
                return Err(TransportError::Closed);
            }
            self.buffer.extend_from_slice(&chunk[..n]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_message_round_trips() {
        let message = Message::diagnostic(0x0E80, 0x0040, &[0x22, 0xF1, 0x90]);
        let bytes = message.encode();

        assert_eq!(bytes[0], 0x02);
        assert_eq!(bytes[1], 0xFD);
        assert_eq!(&bytes[2..4], &[0x80, 0x01]);
        // payload = 4 address bytes + 3 UDS bytes
        assert_eq!(&bytes[4..8], &[0x00, 0x00, 0x00, 0x07]);
        assert_eq!(bytes.len(), 8 + 7);

        let (decoded, consumed) = decode_message(&bytes).unwrap().unwrap();
        assert_eq!(consumed, bytes.len());
        let (source, target, uds) = decoded.as_diagnostic().unwrap();
        assert_eq!(source, 0x0E80);
        assert_eq!(target, 0x0040);
        assert_eq!(uds, &[0x22, 0xF1, 0x90]);
    }

    #[test]
    fn inverse_version_byte_is_validated() {
        let mut bytes = Message::vehicle_ident_request().encode();
        bytes[1] = 0x00;
        assert!(matches!(
            decode_message(&bytes),
            Err(TransportError::MalformedFrame(_))
        ));
    }

    #[test]
    fn routing_activation_request_is_seven_bytes() {
        let message = Message::routing_activation(0x0E80);
        assert_eq!(message.payload.len(), 7);
        assert_eq!(&message.payload[0..2], &[0x0E, 0x80]);
        assert_eq!(message.payload[2], 0x00);
        assert_eq!(&message.payload[3..7], &[0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn partial_message_requests_more_data() {
        let bytes = Message::diagnostic(0x0E80, 0x0040, &[0x3E, 0x80]).encode();
        for split in 0..bytes.len() {
            assert!(decode_message(&bytes[..split]).unwrap().is_none());
        }
        assert!(decode_message(&bytes).unwrap().is_some());
    }

    #[test]
    fn coalesced_messages_decode_one_at_a_time() {
        let mut stream = Message::new(PayloadType::DiagnosticMessageAck, vec![0, 0, 0, 0, 0]).encode();
        stream.extend(Message::diagnostic(0x0040, 0x0E80, &[0x62, 0xF1, 0x90]).encode());

        let (first, consumed) = decode_message(&stream).unwrap().unwrap();
        assert_eq!(first.payload_type, PayloadType::DiagnosticMessageAck);

        let (second, _) = decode_message(&stream[consumed..]).unwrap().unwrap();
        let (_, _, uds) = second.as_diagnostic().unwrap();
        assert_eq!(uds, &[0x62, 0xF1, 0x90]);
    }

    #[test]
    fn vehicle_announcement_parses() {
        let mut payload = Vec::new();
        payload.extend_from_slice(b"WBA8E9G51GNT12345");
        payload.extend_from_slice(&0x0010u16.to_be_bytes());
        payload.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01]);
        payload.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x02]);
        payload.push(0x00);

        let announcement = VehicleAnnouncement::parse(&payload).unwrap();
        assert_eq!(announcement.vin, "WBA8E9G51GNT12345");
        assert_eq!(announcement.logical_address, 0x0010);
        assert_eq!(announcement.further_action, 0x00);
    }

    #[test]
    fn short_vehicle_announcement_is_rejected() {
        assert!(VehicleAnnouncement::parse(&[0u8; 10]).is_err());
    }

    #[test]
    fn unknown_payload_type_is_rejected() {
        let mut bytes = vec![0x02, 0xFD];
        bytes.extend_from_slice(&0x9999u16.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        assert!(matches!(
            decode_message(&bytes),
            Err(TransportError::MalformedFrame(_))
        ));
    }

    #[test]
    fn routing_denial_codes_map_to_reasons() {
        assert_eq!(
            RoutingActivationDenial::from_code(0x01),
            RoutingActivationDenial::AllSocketsRegistered
        );
        assert_eq!(
            RoutingActivationDenial::from_code(0x06),
            RoutingActivationDenial::UnsupportedActivationType
        );
    }
}
