//! UDS (ISO 14229) client for BMW vehicles, with a hard safety guard on the
//! services it is willing to transmit.
//!
//! The client owns the connection behind a mutex so a background keepalive task
//! can share it with foreground requests. Keeping the session alive matters:
//! when it lapses the ECU reverts every actuation, which is the safety net that
//! makes this application non-destructive, but it also means an effect stops
//! mid-show if we stop sending TesterPresent.

pub mod dtc;
pub mod service;

use std::sync::Arc;
use std::time::Duration;

use slt_transport::{Connection, EcuAddress, Protocol, TransportError};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

pub use dtc::Dtc;
pub use service::{did, sid, IoControl, Nrc, RoutineControl, Session};

/// Errors from the UDS layer.
#[derive(Debug, thiserror::Error)]
pub enum UdsError {
    #[error(transparent)]
    Transport(#[from] TransportError),

    /// The ECU returned `7F <sid> <nrc>`.
    #[error("ECU {ecu} rejected service 0x{sid:02X}: {nrc}. {}", nrc.explanation())]
    NegativeResponse {
        ecu: EcuAddress,
        sid: u8,
        nrc: Nrc,
    },

    #[error("response too short: expected at least {expected} bytes, got {actual}")]
    ShortResponse { expected: usize, actual: usize },

    #[error("expected a response to service 0x{expected:02X} but got 0x{actual:02X}")]
    ServiceMismatch { expected: u8, actual: u8 },

    #[error("ECU kept answering responsePending after {0} attempts")]
    TooManyPendingResponses(u32),

    /// The safety guard refused to transmit.
    #[error("blocked by safety guard: {0}")]
    Blocked(String),
}

pub type Result<T> = std::result::Result<T, UdsError>;

/// How many consecutive `responsePending` answers to tolerate.
const MAX_PENDING_RESPONSES: u32 = 30;

/// Default per-request timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(2000);

/// TesterPresent interval. The ISO 14229 S3 session timeout is 5 s, so 2 s
/// leaves room for a couple of missed messages before the session lapses.
pub const KEEPALIVE_INTERVAL: Duration = Duration::from_millis(2000);

/// Refuses to transmit requests that could damage a vehicle or persist changes.
///
/// This is enforced at the lowest layer that still understands UDS semantics, so
/// no catalog entry, effect definition or UI path can bypass it.
#[derive(Debug, Clone)]
pub struct SafetyGuard {
    /// Allow `WriteDataByIdentifier`. Off, and there is no UI to turn it on.
    allow_persistent_writes: bool,
    /// Allow requests to the engine control module.
    allow_powertrain: bool,
}

impl Default for SafetyGuard {
    fn default() -> Self {
        Self {
            allow_persistent_writes: false,
            allow_powertrain: false,
        }
    }
}

impl SafetyGuard {
    /// Checks a request before it goes on the wire.
    pub fn check(&self, ecu: EcuAddress, payload: &[u8]) -> Result<()> {
        let Some(&sid) = payload.first() else {
            return Err(UdsError::Blocked("empty request".into()));
        };

        if (ecu == slt_transport::ecu::DME || ecu == slt_transport::zn8::ECM_REQUEST)
            && !self.allow_powertrain
        {
            return Err(UdsError::Blocked(format!(
                "refusing to address the engine control module at {ecu}"
            )));
        }

        match sid {
            // Entering programming session is the first step of flashing. There
            // is no legitimate reason for a light show application to do it, and
            // an interrupted flash bricks the module.
            sid::DIAGNOSTIC_SESSION_CONTROL
                if payload.get(1) == Some(&(Session::Programming as u8)) =>
            {
                Err(UdsError::Blocked(
                    "refusing to enter programming session, which is the precursor to flashing"
                        .into(),
                ))
            }
            // Writes persist across ignition cycles, so a mistake is not
            // self-healing the way an actuation is.
            sid::WRITE_DATA_BY_IDENTIFIER if !self.allow_persistent_writes => {
                Err(UdsError::Blocked(
                    "refusing WriteDataByIdentifier: Strobes never makes persistent changes"
                        .into(),
                ))
            }
            sid::ECU_RESET => Err(UdsError::Blocked(
                "refusing ECUReset: resetting a body controller mid-drive is unsafe".into(),
            )),
            _ => Ok(()),
        }
    }
}

/// Description of the connected vehicle.
#[derive(Debug, Clone, serde::Serialize)]
pub struct VehicleInfo {
    pub vin: Option<String>,
    pub protocol: String,
    pub gateway_serial: Option<String>,
}

/// One ECU found during a scan.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EcuScanResult {
    pub address: u16,
    pub address_hex: String,
    pub label: String,
    pub present: bool,
    pub serial: Option<String>,
    /// Why a probe failed, when it did. Shown in the UI so a missing module is
    /// distinguishable from a connection problem.
    pub note: Option<String>,
}

/// A UDS client bound to one vehicle connection.
pub struct UdsClient {
    connection: Arc<Mutex<Connection>>,
    /// Cached at construction: the wire protocol cannot change for a live
    /// connection, and reading it should not require taking the lock.
    protocol: Protocol,
    guard: SafetyGuard,
    keepalive: Option<KeepaliveHandle>,
}

/// Owns the background TesterPresent task and stops it on drop.
struct KeepaliveHandle {
    handle: JoinHandle<()>,
}

impl Drop for KeepaliveHandle {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

impl UdsClient {
    pub fn new(connection: Connection) -> Self {
        let protocol = connection.protocol();
        Self {
            connection: Arc::new(Mutex::new(connection)),
            protocol,
            guard: SafetyGuard::default(),
            keepalive: None,
        }
    }

    pub fn protocol(&self) -> Protocol {
        self.protocol
    }

    /// Sends a raw UDS request and returns the response body with the service
    /// echo byte removed.
    ///
    /// Handles `responsePending` transparently, since an ECU answering 0x78 is
    /// asking for time rather than reporting failure.
    pub async fn request(&self, ecu: EcuAddress, payload: &[u8]) -> Result<Vec<u8>> {
        self.guard.check(ecu, payload)?;
        let expected_sid = payload[0];

        let mut connection = self.connection.lock().await;
        let mut response = connection.request(ecu, payload).await?;

        let mut pending_count = 0;
        loop {
            match interpret(ecu, expected_sid, &response) {
                Ok(body) => return Ok(body.to_vec()),
                Err(UdsError::NegativeResponse { nrc, .. }) if nrc.is_response_pending() => {
                    pending_count += 1;
                    if pending_count > MAX_PENDING_RESPONSES {
                        return Err(UdsError::TooManyPendingResponses(pending_count));
                    }
                    tracing::trace!(%ecu, pending_count, "ECU asked for more time");
                    response = connection.receive(ecu).await?;
                }
                Err(other) => return Err(other),
            }
        }
    }

    /// Requests a diagnostic session.
    pub async fn start_session(&self, ecu: EcuAddress, session: Session) -> Result<()> {
        self.request(ecu, &[sid::DIAGNOSTIC_SESSION_CONTROL, session.as_byte()])
            .await?;
        Ok(())
    }

    /// Sends a single TesterPresent with the suppress-response bit set.
    pub async fn tester_present(&self, ecu: EcuAddress) -> Result<()> {
        // 0x80 suppresses the positive response, halving traffic. The ECU does
        // not reply at all, so this is fire and forget.
        let payload = [sid::TESTER_PRESENT, 0x80];
        self.guard.check(ecu, &payload)?;
        let mut connection = self.connection.lock().await;
        match connection.request(ecu, &payload).await {
            Ok(_) => Ok(()),
            // A timeout is the expected outcome of a suppressed response.
            Err(TransportError::Timeout(_)) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Starts a background task that keeps `ecu`'s session alive.
    ///
    /// Without this the session lapses after roughly 5 seconds and the ECU drops
    /// every active actuation, so any effect longer than that needs it running.
    pub fn start_keepalive(&mut self, ecu: EcuAddress) {
        self.stop_keepalive();
        let connection = Arc::clone(&self.connection);
        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(KEEPALIVE_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let mut conn = connection.lock().await;
                let payload = [sid::TESTER_PRESENT, 0x80];
                if let Err(e) = conn.request(ecu, &payload).await {
                    if !matches!(e, TransportError::Timeout(_)) {
                        tracing::warn!(error = %e, %ecu, "keepalive failed");
                    }
                }
            }
        });
        self.keepalive = Some(KeepaliveHandle { handle });
    }

    pub fn stop_keepalive(&mut self) {
        self.keepalive = None;
    }

    pub fn keepalive_running(&self) -> bool {
        self.keepalive.is_some()
    }

    /// Reads one data identifier.
    pub async fn read_data_by_identifier(&self, ecu: EcuAddress, id: u16) -> Result<Vec<u8>> {
        let [hi, lo] = id.to_be_bytes();
        let body = self
            .request(ecu, &[sid::READ_DATA_BY_IDENTIFIER, hi, lo])
            .await?;
        // The response echoes the identifier before the data.
        if body.len() < 2 {
            return Err(UdsError::ShortResponse {
                expected: 2,
                actual: body.len(),
            });
        }
        Ok(body[2..].to_vec())
    }

    /// Reads a data identifier and decodes it as ASCII, trimming padding.
    pub async fn read_string(&self, ecu: EcuAddress, id: u16) -> Result<String> {
        let data = self.read_data_by_identifier(ecu, id).await?;
        Ok(String::from_utf8_lossy(&data)
            .trim_matches(|c: char| c == '\0' || c.is_whitespace())
            .to_string())
    }

    /// Actuates an output via `InputOutputControlByIdentifier`.
    pub async fn io_control(
        &self,
        ecu: EcuAddress,
        id: u16,
        control: IoControl,
        values: &[u8],
    ) -> Result<Vec<u8>> {
        let [hi, lo] = id.to_be_bytes();
        let mut payload = Vec::with_capacity(4 + values.len());
        payload.extend_from_slice(&[sid::IO_CONTROL_BY_IDENTIFIER, hi, lo, control.as_byte()]);
        payload.extend_from_slice(values);
        self.request(ecu, &payload).await
    }

    /// Hands an output back to the ECU, undoing an actuation.
    pub async fn release_io(&self, ecu: EcuAddress, id: u16) -> Result<()> {
        self.io_control(ecu, id, IoControl::ReturnControlToEcu, &[])
            .await?;
        Ok(())
    }

    /// Invokes `RoutineControl`.
    pub async fn routine_control(
        &self,
        ecu: EcuAddress,
        id: u16,
        control: RoutineControl,
        values: &[u8],
    ) -> Result<Vec<u8>> {
        let [hi, lo] = id.to_be_bytes();
        let mut payload = Vec::with_capacity(4 + values.len());
        payload.extend_from_slice(&[sid::ROUTINE_CONTROL, control.as_byte(), hi, lo]);
        payload.extend_from_slice(values);
        self.request(ecu, &payload).await
    }

    /// Reads stored trouble codes.
    pub async fn read_dtcs(&self, ecu: EcuAddress, status_mask: u8) -> Result<Vec<Dtc>> {
        // sub-function 0x02 is reportDTCByStatusMask
        let body = self
            .request(ecu, &[sid::READ_DTC_INFORMATION, 0x02, status_mask])
            .await?;
        // Strip the echoed sub-function byte.
        Ok(dtc::parse_dtc_report(body.get(1..).unwrap_or_default()))
    }

    /// Clears stored trouble codes.
    pub async fn clear_dtcs(&self, ecu: EcuAddress) -> Result<()> {
        // 0xFFFFFF is the "all groups" group-of-DTC value.
        self.request(
            ecu,
            &[sid::CLEAR_DIAGNOSTIC_INFORMATION, 0xFF, 0xFF, 0xFF],
        )
        .await?;
        Ok(())
    }

    /// Probes whether an ECU answers at all.
    ///
    /// Uses a read of the active session identifier, which every UDS module
    /// implements and which changes nothing.
    pub async fn probe(&self, ecu: EcuAddress) -> bool {
        match self.read_data_by_identifier(ecu, did::ACTIVE_SESSION).await {
            Ok(_) => true,
            // A negative response still proves something is listening.
            Err(UdsError::NegativeResponse { .. }) => true,
            Err(_) => false,
        }
    }

    /// Probes every known lighting-related module for the active protocol.
    pub async fn scan_lighting_ecus(&self) -> Vec<EcuScanResult> {
        let targets: &[(EcuAddress, &str)] = match self.protocol {
            Protocol::IsoTp => slt_transport::zn8::LIGHTING_SCAN,
            _ => slt_transport::ecu::LIGHTING_SCAN,
        };
        let mut results = Vec::new();
        for (address, label) in targets {
            let present = self.probe(*address).await;
            let serial = if present {
                self.read_string(*address, did::ECU_SERIAL).await.ok()
            } else {
                None
            };
            results.push(EcuScanResult {
                address: address.0,
                address_hex: address.to_string(),
                label: (*label).to_string(),
                present,
                serial,
                note: (!present).then(|| "no response".to_string()),
            });
        }
        results
    }

    /// Reads the VIN from the gateway / body controller (BMW) or BCM (ZN8).
    pub async fn read_vehicle_info(&self) -> VehicleInfo {
        let protocol = self.protocol.as_str().to_string();

        let candidates: &[EcuAddress] = match self.protocol {
            Protocol::IsoTp => &[slt_transport::zn8::BCM_REQUEST, slt_transport::zn8::ECM_REQUEST],
            _ => &[slt_transport::ecu::FEM_GW, slt_transport::ecu::FEM_BODY],
        };

        let mut vin = None;
        for &ecu in candidates {
            if let Ok(value) = self.read_string(ecu, did::VIN).await {
                if !value.is_empty() {
                    vin = Some(value);
                    break;
                }
            }
        }

        let serial_ecu = match self.protocol {
            Protocol::IsoTp => slt_transport::zn8::BCM_REQUEST,
            _ => slt_transport::ecu::FEM_GW,
        };
        let gateway_serial = self
            .read_string(serial_ecu, did::ECU_SERIAL)
            .await
            .ok()
            .filter(|s| !s.is_empty());

        VehicleInfo {
            vin,
            protocol,
            gateway_serial,
        }
    }
}

/// Splits a UDS response into a positive body or a typed negative response.
fn interpret<'a>(ecu: EcuAddress, expected_sid: u8, response: &'a [u8]) -> Result<&'a [u8]> {
    let Some(&first) = response.first() else {
        return Err(UdsError::ShortResponse {
            expected: 1,
            actual: 0,
        });
    };

    if first == sid::NEGATIVE_RESPONSE {
        let nrc = response.get(2).copied().unwrap_or(0);
        return Err(UdsError::NegativeResponse {
            ecu,
            sid: response.get(1).copied().unwrap_or(expected_sid),
            nrc: Nrc(nrc),
        });
    }

    let expected_echo = expected_sid + sid::POSITIVE_RESPONSE_OFFSET;
    if first != expected_echo {
        return Err(UdsError::ServiceMismatch {
            expected: expected_echo,
            actual: first,
        });
    }

    Ok(&response[1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEM: EcuAddress = slt_transport::ecu::FEM_BODY;

    #[tokio::test]
    async fn isotp_loopback_client_reads_vin_and_scans_bcm() {
        use std::time::Duration;
        use slt_transport::{Connection, Protocol};

        let connection = Connection::open(
            Protocol::IsoTp,
            "loopback",
            None,
            Duration::from_millis(500),
        )
        .await
        .unwrap();
        let client = UdsClient::new(connection);
        let info = client.read_vehicle_info().await;
        assert_eq!(info.protocol, "ISO-TP");
        assert!(info.vin.as_deref().unwrap_or("").starts_with("JF1ZN"));

        let scan = client.scan_lighting_ecus().await;
        let bcm = scan
            .iter()
            .find(|r| r.address == slt_transport::zn8::BCM_REQUEST.0)
            .expect("BCM in scan");
        assert!(bcm.present);
    }

    #[test]
    fn positive_response_strips_the_service_echo() {
        // 0x22 request produces a 0x62 response.
        let body = interpret(FEM, 0x22, &[0x62, 0xF1, 0x90, b'W', b'B', b'A']).unwrap();
        assert_eq!(body, &[0xF1, 0x90, b'W', b'B', b'A']);
    }

    #[test]
    fn negative_response_is_typed() {
        let err = interpret(FEM, 0x2F, &[0x7F, 0x2F, 0x31]).unwrap_err();
        match err {
            UdsError::NegativeResponse { sid, nrc, .. } => {
                assert_eq!(sid, 0x2F);
                assert_eq!(nrc, Nrc::REQUEST_OUT_OF_RANGE);
            }
            other => panic!("expected a negative response, got {other:?}"),
        }
    }

    #[test]
    fn wrong_service_echo_is_detected() {
        let err = interpret(FEM, 0x22, &[0x71, 0x01]).unwrap_err();
        assert!(matches!(err, UdsError::ServiceMismatch { .. }));
    }

    #[test]
    fn empty_response_is_rejected() {
        assert!(matches!(
            interpret(FEM, 0x22, &[]),
            Err(UdsError::ShortResponse { .. })
        ));
    }

    #[test]
    fn guard_blocks_programming_session() {
        let guard = SafetyGuard::default();
        let err = guard.check(FEM, &[0x10, 0x02]).unwrap_err();
        assert!(matches!(err, UdsError::Blocked(_)));
        // The extended session we actually need is allowed.
        assert!(guard.check(FEM, &[0x10, 0x03]).is_ok());
    }

    #[test]
    fn guard_blocks_persistent_writes() {
        let guard = SafetyGuard::default();
        assert!(guard.check(FEM, &[0x2E, 0x30, 0x61, 0x01]).is_err());
    }

    #[test]
    fn guard_blocks_ecu_reset() {
        let guard = SafetyGuard::default();
        assert!(guard.check(FEM, &[0x11, 0x01]).is_err());
    }

    #[test]
    fn guard_blocks_requests_to_the_engine_module() {
        let guard = SafetyGuard::default();
        let err = guard
            .check(slt_transport::ecu::DME, &[0x22, 0xF1, 0x90])
            .unwrap_err();
        assert!(matches!(err, UdsError::Blocked(_)));
    }

    #[test]
    fn guard_allows_actuation_and_reads() {
        let guard = SafetyGuard::default();
        assert!(guard.check(FEM, &[0x2F, 0xD0, 0x00, 0x03, 0x01]).is_ok());
        assert!(guard.check(FEM, &[0x31, 0x01, 0xA0, 0x01]).is_ok());
        assert!(guard.check(FEM, &[0x22, 0xF1, 0x90]).is_ok());
        assert!(guard.check(FEM, &[0x19, 0x02, 0xFF]).is_ok());
    }

    #[test]
    fn guard_rejects_an_empty_request() {
        assert!(SafetyGuard::default().check(FEM, &[]).is_err());
    }

    #[test]
    fn keepalive_interval_is_inside_the_session_timeout() {
        // The ISO 14229 S3 timeout is 5 s. Anything at or above that would let
        // the session lapse mid-effect.
        assert!(KEEPALIVE_INTERVAL < Duration::from_secs(5));
    }
}
