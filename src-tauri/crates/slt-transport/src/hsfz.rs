//! HSFZ (Hochgeschwindigkeits-Fahrzeug-Zugang), BMW's proprietary diagnostic
//! transport for F-series vehicles over ENET.
//!
//! Frame layout: a 4-byte big-endian length, a 2-byte control word, then a body
//! whose length is exactly the length field. The body layout depends on the
//! control word, which is why decoding cannot assume a fixed offset for the
//! address bytes.
//!
//! See `docs/protocol-research.md` section 1.1.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::error::{HsfzRejection, Result, TransportError};

/// Standard HSFZ diagnostic port.
pub const DIAGNOSTIC_PORT: u16 = 6801;
/// Standard HSFZ control/discovery port.
pub const CONTROL_PORT: u16 = 6811;
/// Conventional tester source address.
pub const TESTER_ADDRESS: u8 = 0xF4;

/// Guard against a corrupt length field causing an enormous allocation.
const MAX_BODY_LEN: usize = 64 * 1024;

/// HSFZ control words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ControlWord {
    /// Diagnostic request or response, carrying a UDS payload.
    Diagnostic = 0x01,
    /// Gateway acknowledgement of a transfer. Echoed before the real response.
    Acknowledge = 0x02,
    /// Terminal 15 (ignition) status.
    Terminal15 = 0x10,
    /// Vehicle identification data.
    VehicleIdent = 0x11,
    /// Keepalive probe.
    AliveCheck = 0x12,
    /// Status data inquiry.
    StatusInquiry = 0x13,
    IncorrectTesterAddress = 0x40,
    IncorrectControlWord = 0x41,
    IncorrectFormat = 0x42,
    IncorrectDestinationAddress = 0x43,
    MessageTooLarge = 0x44,
    DiagAppNotReady = 0x45,
    OutOfMemory = 0xFF,
}

impl ControlWord {
    pub fn from_u16(value: u16) -> Option<Self> {
        Some(match value {
            0x01 => Self::Diagnostic,
            0x02 => Self::Acknowledge,
            0x10 => Self::Terminal15,
            0x11 => Self::VehicleIdent,
            0x12 => Self::AliveCheck,
            0x13 => Self::StatusInquiry,
            0x40 => Self::IncorrectTesterAddress,
            0x41 => Self::IncorrectControlWord,
            0x42 => Self::IncorrectFormat,
            0x43 => Self::IncorrectDestinationAddress,
            0x44 => Self::MessageTooLarge,
            0x45 => Self::DiagAppNotReady,
            0xFF => Self::OutOfMemory,
            _ => return None,
        })
    }

    /// Whether the body of a frame with this control word begins with
    /// source and target address bytes.
    ///
    /// Note the alive-check special case: it only carries addresses when the
    /// body is exactly the two address bytes.
    fn has_addresses(self, body_len: usize) -> bool {
        match self {
            Self::Diagnostic | Self::Acknowledge => true,
            Self::AliveCheck => body_len == 2,
            _ => false,
        }
    }

    /// Maps an error control word onto a typed rejection.
    fn as_rejection(self, body: &[u8]) -> Option<HsfzRejection> {
        Some(match self {
            Self::IncorrectTesterAddress => HsfzRejection::IncorrectTesterAddress {
                expected: body.first().copied().unwrap_or(0),
                received: body.get(1).copied().unwrap_or(0),
            },
            Self::IncorrectControlWord => HsfzRejection::IncorrectControlWord,
            Self::IncorrectFormat => HsfzRejection::IncorrectFormat,
            Self::IncorrectDestinationAddress => HsfzRejection::IncorrectDestinationAddress,
            Self::MessageTooLarge => HsfzRejection::MessageTooLarge,
            Self::DiagAppNotReady => HsfzRejection::DiagAppNotReady,
            Self::OutOfMemory => HsfzRejection::OutOfMemory,
            _ => return None,
        })
    }
}

/// A decoded HSFZ frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub control: ControlWord,
    /// Present only for control words that carry addressing.
    pub addresses: Option<(u8, u8)>,
    /// Body bytes after any address prefix. For diagnostic frames this is the
    /// UDS payload.
    pub payload: Vec<u8>,
}

impl Frame {
    /// Builds a diagnostic request frame.
    pub fn diagnostic(src: u8, dst: u8, payload: Vec<u8>) -> Self {
        Self {
            control: ControlWord::Diagnostic,
            addresses: Some((src, dst)),
            payload,
        }
    }

    /// Serializes to the wire format.
    pub fn encode(&self) -> Vec<u8> {
        let addr_len = if self.addresses.is_some() { 2 } else { 0 };
        let body_len = addr_len + self.payload.len();

        let mut out = Vec::with_capacity(6 + body_len);
        out.extend_from_slice(&(body_len as u32).to_be_bytes());
        out.extend_from_slice(&(self.control as u16).to_be_bytes());
        if let Some((src, dst)) = self.addresses {
            out.push(src);
            out.push(dst);
        }
        out.extend_from_slice(&self.payload);
        out
    }

    /// Parses a frame body given its control word.
    fn decode_body(control: ControlWord, body: Vec<u8>) -> Result<Self> {
        if control.has_addresses(body.len()) {
            if body.len() < 2 {
                return Err(TransportError::MalformedFrame(format!(
                    "control word {:#04X} requires 2 address bytes but body is {} bytes",
                    control as u16,
                    body.len()
                )));
            }
            let src = body[0];
            let dst = body[1];
            Ok(Self {
                control,
                addresses: Some((src, dst)),
                payload: body[2..].to_vec(),
            })
        } else {
            Ok(Self {
                control,
                addresses: None,
                payload: body,
            })
        }
    }

    /// Returns the typed rejection if this frame is an error response.
    pub fn rejection(&self) -> Option<HsfzRejection> {
        self.control.as_rejection(&self.payload)
    }
}

/// Decodes a frame from a byte slice, returning it alongside the number of
/// bytes consumed. Returns `Ok(None)` when more data is needed.
///
/// Exposed separately from the connection so the framing logic is unit
/// testable without a socket, and reusable by the simulator.
pub fn decode_frame(buf: &[u8]) -> Result<Option<(Frame, usize)>> {
    if buf.len() < 6 {
        return Ok(None);
    }
    let body_len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if body_len > MAX_BODY_LEN {
        return Err(TransportError::FrameTooLarge {
            actual: body_len,
            max: MAX_BODY_LEN,
        });
    }
    let raw_control = u16::from_be_bytes([buf[4], buf[5]]);
    let total = 6 + body_len;
    if buf.len() < total {
        return Ok(None);
    }
    let control = ControlWord::from_u16(raw_control).ok_or_else(|| {
        TransportError::MalformedFrame(format!("unknown control word {raw_control:#06X}"))
    })?;
    let frame = Frame::decode_body(control, buf[6..total].to_vec())?;
    Ok(Some((frame, total)))
}

/// A live HSFZ connection to a vehicle gateway.
pub struct HsfzConnection {
    stream: TcpStream,
    tester: u8,
    /// Leftover bytes from a previous read. TCP may coalesce or split frames.
    buffer: Vec<u8>,
    timeout: Duration,
}

impl HsfzConnection {
    /// Opens a connection. HSFZ needs no routing activation, so the socket is
    /// immediately usable.
    pub async fn connect(ip: IpAddr, port: u16, timeout: Duration) -> Result<Self> {
        let addr = SocketAddr::new(ip, port);
        let stream = tokio::time::timeout(timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| TransportError::Timeout(timeout))??;
        // Diagnostic traffic is small and latency-sensitive; Nagle would batch
        // our requests and add tens of milliseconds to effect steps.
        stream.set_nodelay(true)?;
        Ok(Self {
            stream,
            tester: TESTER_ADDRESS,
            buffer: Vec::with_capacity(1024),
            timeout,
        })
    }

    pub fn tester_address(&self) -> u8 {
        self.tester
    }

    pub fn set_tester_address(&mut self, address: u8) {
        self.tester = address;
    }

    /// Sends a UDS payload to `target` and returns the UDS response payload.
    ///
    /// Transparently absorbs the gateway acknowledgement frame that precedes
    /// every real response, and surfaces error control words as typed errors.
    pub async fn request(&mut self, target: u8, payload: &[u8]) -> Result<Vec<u8>> {
        let frame = Frame::diagnostic(self.tester, target, payload.to_vec());
        self.stream.write_all(&frame.encode()).await?;
        self.stream.flush().await?;

        loop {
            let frame = self.read_frame().await?;

            if let Some(rejection) = frame.rejection() {
                return Err(TransportError::GatewayRejected(rejection));
            }

            match frame.control {
                // The gateway echoes an ack before the ECU's real answer.
                ControlWord::Acknowledge => continue,
                ControlWord::AliveCheck => {
                    self.send_alive_response(target).await?;
                    continue;
                }
                ControlWord::Diagnostic => {
                    // On the response the addresses are swapped: the ECU is now
                    // the source. A mismatch means we picked up traffic for a
                    // different request.
                    if let Some((src, _dst)) = frame.addresses {
                        if src != target {
                            tracing::warn!(
                                expected = format!("0x{target:02X}"),
                                actual = format!("0x{src:02X}"),
                                "discarding response from unexpected ECU"
                            );
                            continue;
                        }
                    }
                    return Ok(frame.payload);
                }
                other => {
                    tracing::debug!(control = ?other, "ignoring non-diagnostic frame");
                    continue;
                }
            }
        }
    }

    async fn send_alive_response(&mut self, _target: u8) -> Result<()> {
        let frame = Frame {
            control: ControlWord::AliveCheck,
            addresses: Some((self.tester, 0x00)),
            payload: Vec::new(),
        };
        self.stream.write_all(&frame.encode()).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Reads one complete frame, buffering partial reads.
    async fn read_frame(&mut self) -> Result<Frame> {
        loop {
            if let Some((frame, consumed)) = decode_frame(&self.buffer)? {
                self.buffer.drain(..consumed);
                return Ok(frame);
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
    fn diagnostic_frame_round_trips() {
        let frame = Frame::diagnostic(0xF4, 0x40, vec![0x22, 0xF1, 0x90]);
        let bytes = frame.encode();

        // length = 3 byte payload + 2 address bytes
        assert_eq!(&bytes[0..4], &[0x00, 0x00, 0x00, 0x05]);
        assert_eq!(&bytes[4..6], &[0x00, 0x01]);
        assert_eq!(bytes[6], 0xF4);
        assert_eq!(bytes[7], 0x40);
        assert_eq!(&bytes[8..], &[0x22, 0xF1, 0x90]);
        // Total on the wire is payload + 8.
        assert_eq!(bytes.len(), 3 + 8);

        let (decoded, consumed) = decode_frame(&bytes).unwrap().unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(decoded, frame);
    }

    #[test]
    fn partial_frame_requests_more_data() {
        let bytes = Frame::diagnostic(0xF4, 0x40, vec![0x22, 0xF1, 0x90]).encode();
        for split in 0..bytes.len() {
            assert!(
                decode_frame(&bytes[..split]).unwrap().is_none(),
                "expected incomplete at {split} bytes"
            );
        }
        assert!(decode_frame(&bytes).unwrap().is_some());
    }

    #[test]
    fn coalesced_frames_decode_one_at_a_time() {
        let mut stream = Frame::diagnostic(0xF4, 0x40, vec![0x3E, 0x80]).encode();
        stream.extend(Frame::diagnostic(0x40, 0xF4, vec![0x7E, 0x00]).encode());

        let (first, consumed) = decode_frame(&stream).unwrap().unwrap();
        assert_eq!(first.payload, vec![0x3E, 0x80]);

        let (second, _) = decode_frame(&stream[consumed..]).unwrap().unwrap();
        assert_eq!(second.payload, vec![0x7E, 0x00]);
        assert_eq!(second.addresses, Some((0x40, 0xF4)));
    }

    #[test]
    fn acknowledge_frame_carries_addresses() {
        let ack = Frame {
            control: ControlWord::Acknowledge,
            addresses: Some((0x40, 0xF4)),
            payload: vec![],
        };
        let (decoded, _) = decode_frame(&ack.encode()).unwrap().unwrap();
        assert_eq!(decoded.control, ControlWord::Acknowledge);
        assert_eq!(decoded.addresses, Some((0x40, 0xF4)));
    }

    #[test]
    fn alive_check_without_addresses_is_accepted() {
        // A zero-length alive check carries no addresses.
        let bytes = [0x00, 0x00, 0x00, 0x00, 0x00, 0x12];
        let (decoded, _) = decode_frame(&bytes).unwrap().unwrap();
        assert_eq!(decoded.control, ControlWord::AliveCheck);
        assert_eq!(decoded.addresses, None);
    }

    #[test]
    fn alive_check_with_two_bytes_carries_addresses() {
        let bytes = [0x00, 0x00, 0x00, 0x02, 0x00, 0x12, 0xF4, 0x10];
        let (decoded, _) = decode_frame(&bytes).unwrap().unwrap();
        assert_eq!(decoded.addresses, Some((0xF4, 0x10)));
    }

    #[test]
    fn error_control_word_maps_to_rejection() {
        let bytes = [0x00, 0x00, 0x00, 0x00, 0x00, 0x43];
        let (decoded, _) = decode_frame(&bytes).unwrap().unwrap();
        assert_eq!(
            decoded.rejection(),
            Some(HsfzRejection::IncorrectDestinationAddress)
        );
    }

    #[test]
    fn incorrect_tester_address_reports_both_addresses() {
        let bytes = [0x00, 0x00, 0x00, 0x02, 0x00, 0x40, 0xF4, 0xEF];
        let (decoded, _) = decode_frame(&bytes).unwrap().unwrap();
        assert_eq!(
            decoded.rejection(),
            Some(HsfzRejection::IncorrectTesterAddress {
                expected: 0xF4,
                received: 0xEF
            })
        );
    }

    #[test]
    fn unknown_control_word_is_rejected() {
        let bytes = [0x00, 0x00, 0x00, 0x00, 0x99, 0x99];
        assert!(matches!(
            decode_frame(&bytes),
            Err(TransportError::MalformedFrame(_))
        ));
    }

    #[test]
    fn oversized_length_is_rejected_before_allocating() {
        let bytes = [0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x01];
        assert!(matches!(
            decode_frame(&bytes),
            Err(TransportError::FrameTooLarge { .. })
        ));
    }

    #[test]
    fn vehicle_ident_frame_has_no_addresses() {
        let vin = b"WBA8E9G51GNT12345";
        let mut bytes = (vin.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(&0x0011u16.to_be_bytes());
        bytes.extend_from_slice(vin);

        let (decoded, _) = decode_frame(&bytes).unwrap().unwrap();
        assert_eq!(decoded.control, ControlWord::VehicleIdent);
        assert_eq!(decoded.addresses, None);
        assert_eq!(decoded.payload, vin.to_vec());
    }
}
