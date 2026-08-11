//! ISO 15765-2 (ISO-TP) framing over CAN.
//!
//! This module is the first building block for Toyota GR86 (ZN8) / Subaru BRZ
//! (ZC8) support. It encodes and decodes diagnostic payloads into CAN frames;
//! it does **not** open a SocketCAN socket by itself — see the `can` module.
//!
//! Frame types (PCI high nibble):
//! - `0x0` Single Frame (SF)
//! - `0x1` First Frame (FF)
//! - `0x2` Consecutive Frame (CF)
//! - `0x3` Flow Control (FC)

use crate::error::{Result, TransportError};

/// Classic CAN data-field length used by ISO-TP on 500 kbit/s diagnostic buses.
pub const CAN_DL: usize = 8;

/// Maximum UDS payload that fits in one Single Frame (PCI + up to 7 data bytes).
pub const SF_MAX_DATA: usize = 7;

/// Data bytes carried in a First Frame after the two-byte PCI.
pub const FF_DATA: usize = 6;

/// Data bytes carried in each Consecutive Frame after the one-byte PCI.
pub const CF_DATA: usize = 7;

/// ISO-TP Protocol Control Information kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PciType {
    Single = 0x0,
    First = 0x1,
    Consecutive = 0x2,
    FlowControl = 0x3,
}

impl PciType {
    fn from_nibble(nibble: u8) -> Option<Self> {
        match nibble & 0x0F {
            0x0 => Some(Self::Single),
            0x1 => Some(Self::First),
            0x2 => Some(Self::Consecutive),
            0x3 => Some(Self::FlowControl),
            _ => None,
        }
    }
}

/// Flow-control flag values (ISO 15765-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FlowStatus {
    ContinueToSend = 0x0,
    Wait = 0x1,
    Overflow = 0x2,
}

impl FlowStatus {
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte & 0x0F {
            0x0 => Some(Self::ContinueToSend),
            0x1 => Some(Self::Wait),
            0x2 => Some(Self::Overflow),
            _ => None,
        }
    }
}

/// A decoded ISO-TP frame sitting inside an 8-byte CAN data field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IsoTpFrame {
    Single {
        data: Vec<u8>,
    },
    First {
        total_len: usize,
        data: Vec<u8>,
    },
    Consecutive {
        sequence: u8,
        data: Vec<u8>,
    },
    FlowControl {
        status: FlowStatus,
        block_size: u8,
        /// Separation time minimum, encoded per ISO-TP (0x00–0x7F = ms,
        /// 0xF1–0xF9 = 100 µs steps). Stored raw so callers can interpret.
        st_min: u8,
    },
}

/// Encodes a UDS payload into one or more ISO-TP CAN frames (SF or FF+CF).
///
/// Does not wait for flow control: the caller inserts CF pacing after receiving
/// an FC frame. Returns the First Frame (or Single Frame) first, then any
/// Consecutive Frames that would follow a ContinueToSend with block_size 0.
pub fn encode_payload(payload: &[u8]) -> Result<Vec<[u8; CAN_DL]>> {
    if payload.is_empty() {
        return Err(TransportError::MalformedFrame(
            "ISO-TP payload must not be empty".into(),
        ));
    }
    if payload.len() > 4095 {
        return Err(TransportError::FrameTooLarge {
            actual: payload.len(),
            max: 4095,
        });
    }

    if payload.len() <= SF_MAX_DATA {
        return Ok(vec![encode_single(payload)]);
    }

    let mut frames = Vec::new();
    frames.push(encode_first(payload));

    let mut offset = FF_DATA;
    let mut sequence = 1u8;
    while offset < payload.len() {
        let end = (offset + CF_DATA).min(payload.len());
        frames.push(encode_consecutive(sequence, &payload[offset..end]));
        offset = end;
        sequence = (sequence + 1) & 0x0F;
    }
    Ok(frames)
}

/// Builds a Single Frame.
pub fn encode_single(data: &[u8]) -> [u8; CAN_DL] {
    debug_assert!(data.len() <= SF_MAX_DATA);
    let mut frame = [0u8; CAN_DL];
    frame[0] = data.len() as u8; // PCI: type 0, length in low nibble
    frame[1..1 + data.len()].copy_from_slice(data);
    frame
}

/// Builds a First Frame for a multi-frame transfer.
pub fn encode_first(payload: &[u8]) -> [u8; CAN_DL] {
    debug_assert!(payload.len() > SF_MAX_DATA && payload.len() <= 4095);
    let mut frame = [0u8; CAN_DL];
    let len = payload.len();
    frame[0] = 0x10 | (((len >> 8) as u8) & 0x0F);
    frame[1] = (len & 0xFF) as u8;
    frame[2..8].copy_from_slice(&payload[..FF_DATA]);
    frame
}

/// Builds a Consecutive Frame.
pub fn encode_consecutive(sequence: u8, data: &[u8]) -> [u8; CAN_DL] {
    debug_assert!(data.len() <= CF_DATA);
    let mut frame = [0u8; CAN_DL];
    frame[0] = 0x20 | (sequence & 0x0F);
    frame[1..1 + data.len()].copy_from_slice(data);
    frame
}

/// Builds a Flow Control frame.
pub fn encode_flow_control(status: FlowStatus, block_size: u8, st_min: u8) -> [u8; CAN_DL] {
    let mut frame = [0u8; CAN_DL];
    frame[0] = 0x30 | (status as u8);
    frame[1] = block_size;
    frame[2] = st_min;
    frame
}

/// Decodes one CAN data field into an ISO-TP frame.
///
/// Accepts frames shorter than 8 bytes (some controllers omit padding) by
/// treating missing trailing bytes as zero padding.
pub fn decode_frame(raw: &[u8]) -> Result<IsoTpFrame> {
    if raw.is_empty() {
        return Err(TransportError::MalformedFrame(
            "empty CAN frame".into(),
        ));
    }

    let mut data = [0u8; CAN_DL];
    let copy_len = raw.len().min(CAN_DL);
    data[..copy_len].copy_from_slice(&raw[..copy_len]);

    let pci = data[0];
    let kind = PciType::from_nibble(pci >> 4).ok_or_else(|| {
        TransportError::MalformedFrame(format!("unknown ISO-TP PCI type 0x{pci:02X}"))
    })?;

    match kind {
        PciType::Single => {
            let len = (pci & 0x0F) as usize;
            if len == 0 || len > SF_MAX_DATA {
                return Err(TransportError::MalformedFrame(format!(
                    "invalid Single Frame length {len}"
                )));
            }
            if copy_len < 1 + len {
                return Err(TransportError::MalformedFrame(format!(
                    "Single Frame truncated: need {len} data bytes"
                )));
            }
            Ok(IsoTpFrame::Single {
                data: data[1..1 + len].to_vec(),
            })
        }
        PciType::First => {
            if copy_len < 2 {
                return Err(TransportError::MalformedFrame(
                    "First Frame missing length byte".into(),
                ));
            }
            let total_len = (((pci & 0x0F) as usize) << 8) | data[1] as usize;
            if total_len <= SF_MAX_DATA {
                return Err(TransportError::MalformedFrame(format!(
                    "First Frame length {total_len} should use a Single Frame"
                )));
            }
            Ok(IsoTpFrame::First {
                total_len,
                data: data[2..8].to_vec(),
            })
        }
        PciType::Consecutive => {
            let sequence = pci & 0x0F;
            Ok(IsoTpFrame::Consecutive {
                sequence,
                data: data[1..8].to_vec(),
            })
        }
        PciType::FlowControl => {
            let status = FlowStatus::from_byte(pci).ok_or_else(|| {
                TransportError::MalformedFrame(format!("invalid flow status 0x{pci:02X}"))
            })?;
            Ok(IsoTpFrame::FlowControl {
                status,
                block_size: data[1],
                st_min: data[2],
            })
        }
    }
}

/// Reassembles a multi-frame transfer from a First Frame plus Consecutive Frames.
///
/// `consecutive` must be in order. Sequence numbers wrap at 0x0F and start at 1
/// after the First Frame.
pub fn reassemble(first: &IsoTpFrame, consecutive: &[IsoTpFrame]) -> Result<Vec<u8>> {
    let IsoTpFrame::First { total_len, data } = first else {
        return Err(TransportError::MalformedFrame(
            "reassemble expects a First Frame".into(),
        ));
    };

    let mut out = Vec::with_capacity(*total_len);
    out.extend_from_slice(data);

    let mut expected_seq = 1u8;
    for frame in consecutive {
        let IsoTpFrame::Consecutive { sequence, data } = frame else {
            return Err(TransportError::MalformedFrame(
                "expected Consecutive Frame during reassembly".into(),
            ));
        };
        if *sequence != expected_seq {
            return Err(TransportError::MalformedFrame(format!(
                "ISO-TP sequence error: expected {expected_seq}, got {sequence}"
            )));
        }
        let remaining = total_len.saturating_sub(out.len());
        let take = remaining.min(data.len()).min(CF_DATA);
        out.extend_from_slice(&data[..take]);
        expected_seq = (expected_seq + 1) & 0x0F;
        if out.len() >= *total_len {
            break;
        }
    }

    if out.len() < *total_len {
        return Err(TransportError::MalformedFrame(format!(
            "incomplete ISO-TP transfer: got {} of {total_len} bytes",
            out.len()
        )));
    }
    out.truncate(*total_len);
    Ok(out)
}

/// Separates a complete payload from a stream of already-decoded frames.
///
/// Convenience for tests and for a future socket reader: feed frames until
/// `Some(payload)` is returned.
#[derive(Debug, Default)]
pub struct ReassemblyBuffer {
    expected_len: Option<usize>,
    expected_seq: u8,
    buffer: Vec<u8>,
}

impl ReassemblyBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pushes one frame. Returns `Ok(Some(payload))` when a message is complete.
    pub fn push(&mut self, frame: IsoTpFrame) -> Result<Option<Vec<u8>>> {
        match frame {
            IsoTpFrame::Single { data } => {
                self.reset();
                Ok(Some(data))
            }
            IsoTpFrame::First { total_len, data } => {
                self.expected_len = Some(total_len);
                self.expected_seq = 1;
                self.buffer = data;
                Ok(None)
            }
            IsoTpFrame::Consecutive { sequence, data } => {
                let Some(total_len) = self.expected_len else {
                    return Err(TransportError::MalformedFrame(
                        "Consecutive Frame without a First Frame".into(),
                    ));
                };
                if sequence != self.expected_seq {
                    return Err(TransportError::MalformedFrame(format!(
                        "ISO-TP sequence error: expected {}, got {sequence}",
                        self.expected_seq
                    )));
                }
                let remaining = total_len.saturating_sub(self.buffer.len());
                let take = remaining.min(data.len()).min(CF_DATA);
                self.buffer.extend_from_slice(&data[..take]);
                self.expected_seq = (self.expected_seq + 1) & 0x0F;
                if self.buffer.len() >= total_len {
                    self.buffer.truncate(total_len);
                    let out = std::mem::take(&mut self.buffer);
                    self.reset();
                    Ok(Some(out))
                } else {
                    Ok(None)
                }
            }
            IsoTpFrame::FlowControl { .. } => Ok(None),
        }
    }

    fn reset(&mut self) {
        self.expected_len = None;
        self.expected_seq = 0;
        self.buffer.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_frame_round_trip() {
        let payload = [0x22, 0xF1, 0x90];
        let frames = encode_payload(&payload).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0][0], 0x03);
        assert_eq!(&frames[0][1..4], &payload);

        match decode_frame(&frames[0]).unwrap() {
            IsoTpFrame::Single { data } => assert_eq!(data, payload),
            other => panic!("expected Single, got {other:?}"),
        }
    }

    #[test]
    fn multi_frame_encode_uses_ff_then_cf() {
        // 8 bytes forces FF+CF (SF max is 7).
        let payload: Vec<u8> = (0u8..20).collect();
        let frames = encode_payload(&payload).unwrap();
        assert!(frames.len() >= 3);
        assert_eq!(frames[0][0] & 0xF0, 0x10);
        assert_eq!(frames[1][0] & 0xF0, 0x20);
        assert_eq!(frames[1][0] & 0x0F, 0x01);
    }

    #[test]
    fn reassembly_recovers_multi_frame_payload() {
        let payload: Vec<u8> = (0u8..20).collect();
        let frames = encode_payload(&payload).unwrap();
        let first = decode_frame(&frames[0]).unwrap();
        let consecutive: Vec<_> = frames[1..]
            .iter()
            .map(|f| decode_frame(f).unwrap())
            .collect();
        let rebuilt = reassemble(&first, &consecutive).unwrap();
        assert_eq!(rebuilt, payload);
    }

    #[test]
    fn reassembly_buffer_handles_stream() {
        let payload: Vec<u8> = (1u8..30).collect();
        let frames = encode_payload(&payload).unwrap();
        let mut buf = ReassemblyBuffer::new();
        let mut done = None;
        for raw in frames {
            if let Some(out) = buf.push(decode_frame(&raw).unwrap()).unwrap() {
                done = Some(out);
            }
        }
        assert_eq!(done.unwrap(), payload);
    }

    #[test]
    fn flow_control_encode_decode() {
        let raw = encode_flow_control(FlowStatus::ContinueToSend, 0, 0x0A);
        match decode_frame(&raw).unwrap() {
            IsoTpFrame::FlowControl {
                status,
                block_size,
                st_min,
            } => {
                assert_eq!(status, FlowStatus::ContinueToSend);
                assert_eq!(block_size, 0);
                assert_eq!(st_min, 0x0A);
            }
            other => panic!("expected FC, got {other:?}"),
        }
    }

    #[test]
    fn empty_payload_is_rejected() {
        assert!(encode_payload(&[]).is_err());
    }

    #[test]
    fn sequence_error_is_reported() {
        let first = IsoTpFrame::First {
            total_len: 10,
            data: vec![0; 6],
        };
        let bad = [IsoTpFrame::Consecutive {
            sequence: 2, // should be 1
            data: vec![0; 7],
        }];
        assert!(reassemble(&first, &bad).is_err());
    }
}
