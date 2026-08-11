//! Abstract CAN frames and an in-process loopback bus.
//!
//! ISO-TP rides on top of this. Production vehicles need a SocketCAN (or other
//! adapter) backend; [`LoopbackBus`] lets the ISO-TP client and ZN8 catalog be
//! exercised without hardware via the `"loopback"` endpoint.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, Notify};

use crate::error::{Result, TransportError};
use crate::isotp::{self, FlowStatus, IsoTpFrame, ReassemblyBuffer};

/// One CAN data frame (11-bit id, up to 8 data bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanFrame {
    pub id: u16,
    pub data: Vec<u8>,
}

impl CanFrame {
    pub fn new(id: u16, data: impl Into<Vec<u8>>) -> Self {
        let data = data.into();
        debug_assert!(data.len() <= 8);
        Self { id, data }
    }
}

/// Default OBD-style response id: request + 8 (e.g. 0x7E1 → 0x7E9).
pub fn default_response_id(request_id: u16) -> u16 {
    request_id.wrapping_add(8)
}

/// Shared CAN endpoint used by [`super::Connection::IsoTp`].
#[derive(Clone)]
pub enum CanEndpoint {
    Loopback(Arc<LoopbackBus>),
}

impl CanEndpoint {
    pub async fn send(&self, frame: CanFrame) -> Result<()> {
        match self {
            Self::Loopback(bus) => bus.send(frame).await,
        }
    }

    pub async fn recv_any(&self, ids: &[u16], timeout: Duration) -> Result<CanFrame> {
        match self {
            Self::Loopback(bus) => bus.recv_any(ids, timeout).await,
        }
    }
}

/// In-memory bus: every send is visible to every receiver.
pub struct LoopbackBus {
    inner: Mutex<LoopbackInner>,
    notify: Notify,
}

struct LoopbackInner {
    queue: VecDeque<CanFrame>,
    /// ISO-TP aware mock ECUs keyed by request id.
    ecus: HashMap<u16, MockEcu>,
}

impl LoopbackBus {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(LoopbackInner {
                queue: VecDeque::new(),
                ecus: HashMap::new(),
            }),
            notify: Notify::new(),
        })
    }

    /// Installs a mock ECU that speaks ISO-TP on `request_id`.
    pub async fn add_ecu(&self, request_id: u16, ecu: MockEcu) {
        let mut guard = self.inner.lock().await;
        guard.ecus.insert(request_id, ecu);
    }

    async fn send(&self, frame: CanFrame) -> Result<()> {
        let replies = {
            let mut guard = self.inner.lock().await;
            if let Some(ecu) = guard.ecus.get_mut(&frame.id) {
                ecu.handle_can_frame(&frame)
            } else {
                Vec::new()
            }
        };

        {
            let mut guard = self.inner.lock().await;
            // Do not echo the request onto the RX queue; only responses matter
            // to the ISO-TP client. (A real bus would see both, but filtering
            // by response id already ignores the request.)
            for reply in replies {
                guard.queue.push_back(reply);
            }
        }
        self.notify.notify_waiters();
        Ok(())
    }

    async fn recv_any(&self, ids: &[u16], timeout: Duration) -> Result<CanFrame> {
        let deadline = Instant::now() + timeout;
        loop {
            {
                let mut guard = self.inner.lock().await;
                if let Some(index) = guard
                    .queue
                    .iter()
                    .position(|frame| ids.contains(&frame.id))
                {
                    return Ok(guard.queue.remove(index).expect("index present"));
                }
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(TransportError::Timeout(timeout));
            }

            tokio::select! {
                _ = self.notify.notified() => {}
                _ = tokio::time::sleep(remaining) => {
                    return Err(TransportError::Timeout(timeout));
                }
            }
        }
    }
}

/// A tiny ISO-TP UDS ECU for loopback development.
pub struct MockEcu {
    response_id: u16,
    vin: String,
    /// When set, IO control of this DID toggles lamp state in memory.
    lamp_did: u16,
    lamp_level: u8,
    /// Reassembly of multi-frame requests from the tester.
    rx: ReassemblyBuffer,
    /// Pending consecutive frames for a multi-frame positive response.
    pending_tx: VecDeque<[u8; 8]>,
}

impl MockEcu {
    pub fn zn8_bcm() -> Self {
        Self {
            response_id: default_response_id(0x7E1),
            vin: "JF1ZN8A0X00000001".into(),
            lamp_did: 0xFFFF,
            lamp_level: 0,
            rx: ReassemblyBuffer::new(),
            pending_tx: VecDeque::new(),
        }
    }

    pub fn zn8_ecm() -> Self {
        Self {
            response_id: default_response_id(0x7E0),
            vin: "JF1ZN8A0X00000001".into(),
            lamp_did: 0x0000,
            lamp_level: 0,
            rx: ReassemblyBuffer::new(),
            pending_tx: VecDeque::new(),
        }
    }

    fn handle_can_frame(&mut self, frame: &CanFrame) -> Vec<CanFrame> {
        let Ok(decoded) = isotp::decode_frame(&frame.data) else {
            return Vec::new();
        };

        // Flow control from the tester: release any queued consecutive frames.
        if let IsoTpFrame::FlowControl { status, .. } = &decoded {
            if *status == FlowStatus::ContinueToSend {
                let mut out = Vec::new();
                while let Some(raw) = self.pending_tx.pop_front() {
                    out.push(CanFrame::new(self.response_id, raw.to_vec()));
                }
                return out;
            }
            return Vec::new();
        }

        let is_first = matches!(decoded, IsoTpFrame::First { .. });
        match self.rx.push(decoded) {
            Ok(Some(payload)) => {
                let response = self.handle_uds(&payload);
                self.frames_for_payload(&response)
            }
            Ok(None) if is_first => {
                let fc = isotp::encode_flow_control(FlowStatus::ContinueToSend, 0, 0);
                vec![CanFrame::new(self.response_id, fc.to_vec())]
            }
            _ => Vec::new(),
        }
    }

    fn handle_uds(&mut self, payload: &[u8]) -> Vec<u8> {
        match payload.first().copied() {
            Some(0x10) => {
                // DiagnosticSessionControl positive echo
                let session = payload.get(1).copied().unwrap_or(0x01);
                vec![0x50, session]
            }
            Some(0x22) if payload.len() >= 3 => {
                let did = u16::from_be_bytes([payload[1], payload[2]]);
                match did {
                    0xF190 => {
                        let mut out = vec![0x62, 0xF1, 0x90];
                        out.extend(self.vin.as_bytes());
                        out
                    }
                    0xF186 => vec![0x62, 0xF1, 0x86, 0x03], // active session
                    0xF18C => {
                        let mut out = vec![0x62, 0xF1, 0x8C];
                        out.extend(b"ZN8-BCM-SIM");
                        out
                    }
                    _ => vec![0x7F, 0x22, 0x31],
                }
            }
            Some(0x2F) if payload.len() >= 4 => {
                let did = u16::from_be_bytes([payload[1], payload[2]]);
                let control = payload[3];
                if did != self.lamp_did {
                    return vec![0x7F, 0x2F, 0x31];
                }
                match control {
                    0x03 => {
                        // ShortTermAdjustment: lamp, level
                        if let (Some(lamp), Some(level)) = (payload.get(4), payload.get(5)) {
                            let _ = lamp;
                            self.lamp_level = *level;
                        }
                        vec![0x6F, payload[1], payload[2], control]
                    }
                    0x00 => {
                        self.lamp_level = 0;
                        vec![0x6F, payload[1], payload[2], control]
                    }
                    _ => vec![0x7F, 0x2F, 0x12],
                }
            }
            Some(0x3E) => {
                // TesterPresent — if suppress bit set, no response expected;
                // still answer for simplicity when not suppressed.
                if payload.get(1).copied().unwrap_or(0) & 0x80 != 0 {
                    Vec::new()
                } else {
                    vec![0x7E, 0x00]
                }
            }
            Some(sid) => vec![0x7F, sid, 0x11], // serviceNotSupported
            None => Vec::new(),
        }
    }

    fn frames_for_payload(&mut self, payload: &[u8]) -> Vec<CanFrame> {
        if payload.is_empty() {
            return Vec::new();
        }
        let Ok(frames) = isotp::encode_payload(payload) else {
            return Vec::new();
        };
        if frames.len() == 1 {
            return vec![CanFrame::new(self.response_id, frames[0].to_vec())];
        }
        // Multi-frame: send First Frame now; stash CFs until flow control.
        self.pending_tx.clear();
        for raw in frames.iter().skip(1) {
            self.pending_tx.push_back(*raw);
        }
        vec![CanFrame::new(self.response_id, frames[0].to_vec())]
    }

    #[cfg(test)]
    pub fn lamp_level(&self) -> u8 {
        self.lamp_level
    }
}

/// ISO-TP client bound to one CAN endpoint.
pub struct IsoTpConnection {
    endpoint: CanEndpoint,
    timeout: Duration,
    /// Last response payload, for `receive` after responsePending (unused on loopback).
    pending: Option<Vec<u8>>,
}

impl IsoTpConnection {
    pub fn new(endpoint: CanEndpoint, timeout: Duration) -> Self {
        Self {
            endpoint,
            timeout,
            pending: None,
        }
    }

    /// Opens the in-process loopback bus with a mock ZN8 BCM.
    pub async fn connect_loopback(timeout: Duration) -> Result<Self> {
        let bus = LoopbackBus::new();
        bus.add_ecu(0x7E1, MockEcu::zn8_bcm()).await;
        bus.add_ecu(0x7E0, MockEcu::zn8_ecm()).await;
        Ok(Self::new(CanEndpoint::Loopback(bus), timeout))
    }

    pub async fn request(&mut self, target: u16, payload: &[u8]) -> Result<Vec<u8>> {
        let response_id = default_response_id(target);
        let frames = isotp::encode_payload(payload)?;

        // Send first (or only) frame.
        self.endpoint
            .send(CanFrame::new(target, frames[0].to_vec()))
            .await?;

        if frames.len() > 1 {
            // Wait for flow control, then send consecutive frames.
            let fc = self.wait_frame(response_id).await?;
            match isotp::decode_frame(&fc.data)? {
                IsoTpFrame::FlowControl {
                    status: FlowStatus::ContinueToSend,
                    ..
                } => {}
                IsoTpFrame::FlowControl {
                    status: FlowStatus::Overflow,
                    ..
                } => {
                    return Err(TransportError::MalformedFrame(
                        "peer reported ISO-TP buffer overflow".into(),
                    ));
                }
                IsoTpFrame::FlowControl {
                    status: FlowStatus::Wait,
                    ..
                } => {
                    return Err(TransportError::MalformedFrame(
                        "peer asked to wait; Wait pacing not implemented yet".into(),
                    ));
                }
                other => {
                    return Err(TransportError::MalformedFrame(format!(
                        "expected flow control, got {other:?}"
                    )));
                }
            }
            for raw in frames.iter().skip(1) {
                self.endpoint
                    .send(CanFrame::new(target, raw.to_vec()))
                    .await?;
            }
        }

        self.receive_payload(response_id).await
    }

    pub async fn receive(&mut self, target: u16) -> Result<Vec<u8>> {
        if let Some(payload) = self.pending.take() {
            return Ok(payload);
        }
        self.receive_payload(default_response_id(target)).await
    }

    async fn receive_payload(&mut self, response_id: u16) -> Result<Vec<u8>> {
        let mut reassembly = ReassemblyBuffer::new();
        loop {
            let frame = self.wait_frame(response_id).await?;
            let decoded = isotp::decode_frame(&frame.data)?;

            if let IsoTpFrame::First { .. } = &decoded {
                // Acknowledge multi-frame responses.
                let fc = isotp::encode_flow_control(FlowStatus::ContinueToSend, 0, 0);
                // Response FC is sent TO the ECU on the request id... actually
                // flow control goes back on the receiver's address, which for
                // the tester is the request id used as TX. Send on the request
                // id derived from response_id - 8.
                let request_id = response_id.wrapping_sub(8);
                self.endpoint
                    .send(CanFrame::new(request_id, fc.to_vec()))
                    .await?;
            }

            if let Some(payload) = reassembly.push(decoded)? {
                return Ok(payload);
            }
        }
    }

    async fn wait_frame(&self, id: u16) -> Result<CanFrame> {
        self.endpoint.recv_any(&[id], self.timeout).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn loopback_isotp_reads_vin() {
        let mut conn = IsoTpConnection::connect_loopback(Duration::from_millis(200))
            .await
            .unwrap();
        let response = conn
            .request(0x7E1, &[0x22, 0xF1, 0x90])
            .await
            .unwrap();
        assert_eq!(response[0], 0x62);
        assert_eq!(&response[3..], b"JF1ZN8A0X00000001");
    }

    #[tokio::test]
    async fn loopback_isotp_actuates_placeholder_lamp_did() {
        let mut conn = IsoTpConnection::connect_loopback(Duration::from_millis(200))
            .await
            .unwrap();
        let response = conn
            .request(0x7E1, &[0x2F, 0xFF, 0xFF, 0x03, 0x80, 100])
            .await
            .unwrap();
        assert_eq!(response, vec![0x6F, 0xFF, 0xFF, 0x03]);
    }
}
