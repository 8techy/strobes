//! A simulated BMW body controller.
//!
//! Models enough UDS behaviour to exercise the real client: session state,
//! session timeout, per-lamp actuation state, DTC storage and short-circuit
//! counters. It deliberately reproduces awkward real-world behaviours such as
//! `responsePending` and `conditionsNotCorrect` so those code paths get
//! exercised somewhere other than a customer's car.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use slt_uds::{sid, Nrc};

/// The DID the simulator treats as the lamp control identifier.
///
/// Arbitrary: real vehicles use a value only BMW's SGBD files know, so this
/// exists purely so tests and the seeded catalog agree on something.
pub const SIM_LAMP_DID: u16 = 0xD0FF;

/// ISO 14229 S3 session timeout.
const SESSION_TIMEOUT: Duration = Duration::from_secs(5);

/// Simulated per-lamp state.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LampState {
    /// Brightness 0-100, as commanded.
    pub level: u8,
    /// Whether the tester currently holds this output.
    pub controlled: bool,
}

/// Vehicle-level conditions the simulator can be told to report.
#[derive(Debug, Clone, Copy)]
pub struct Conditions {
    pub ignition_on: bool,
    pub engine_running: bool,
    pub speed_kph: u16,
    /// Reported as `voltageTooLow` when below 11.0 V.
    pub voltage: f32,
}

impl Default for Conditions {
    fn default() -> Self {
        Self {
            ignition_on: true,
            engine_running: false,
            speed_kph: 0,
            voltage: 12.6,
        }
    }
}

/// A simulated ECU.
pub struct SimulatedEcu {
    pub address: u8,
    pub label: String,
    pub vin: String,
    pub serial: String,
    session: u8,
    session_started: Option<Instant>,
    lamps: HashMap<u8, LampState>,
    /// Per-lamp short-circuit counters. BMW's limit is 50.
    short_circuit_counters: HashMap<u8, u8>,
    dtcs: Vec<(u32, u8)>,
    pub conditions: Conditions,
    /// Emit this many `responsePending` frames before each real response, to
    /// exercise the client's 0x78 handling.
    pub pending_responses: u32,
}

impl SimulatedEcu {
    pub fn new(address: u8, label: impl Into<String>) -> Self {
        Self {
            address,
            label: label.into(),
            vin: "WBA8E9G51GNT12345".to_string(),
            serial: format!("SIM{address:02X}0001"),
            session: 0x01,
            session_started: None,
            lamps: HashMap::new(),
            short_circuit_counters: HashMap::new(),
            dtcs: Vec::new(),
            conditions: Conditions::default(),
            pending_responses: 0,
        }
    }

    /// Seeds a stored fault, so DTC reading has something to return.
    pub fn add_dtc(&mut self, code: u32, status: u8) {
        self.dtcs.push((code, status));
    }

    /// Sets a lamp's short-circuit counter, for testing the preflight refusal.
    pub fn set_short_circuit_counter(&mut self, lamp: u8, count: u8) {
        self.short_circuit_counters.insert(lamp, count);
    }

    pub fn lamp_state(&self, lamp: u8) -> LampState {
        self.lamps.get(&lamp).copied().unwrap_or_default()
    }

    /// How many outputs the tester currently holds.
    pub fn controlled_count(&self) -> usize {
        self.lamps.values().filter(|s| s.controlled).count()
    }

    /// Whether the extended session is currently active, accounting for timeout.
    ///
    /// This models the real safety net: when TesterPresent stops arriving the
    /// session lapses and every actuation is dropped.
    pub fn session_active(&mut self) -> bool {
        if self.session == 0x01 {
            return false;
        }
        match self.session_started {
            Some(started) if started.elapsed() < SESSION_TIMEOUT => true,
            Some(_) => {
                tracing::debug!(address = self.address, "session timed out, releasing outputs");
                self.session = 0x01;
                self.session_started = None;
                for state in self.lamps.values_mut() {
                    state.controlled = false;
                    state.level = 0;
                }
                false
            }
            None => false,
        }
    }

    /// Handles a request, returning every frame the ECU would send back.
    ///
    /// A real module answering `responsePending` sends the 0x78 frames and then
    /// the final response, all unprompted, for a single request. Modelling that
    /// as one call returning several frames is what makes the client's retry loop
    /// testable; returning only the 0x78 would leave the client waiting forever.
    pub fn handle_sequence(&mut self, request: &[u8]) -> Vec<Vec<u8>> {
        let mut frames: Vec<Vec<u8>> = (0..self.pending_responses)
            .map(|_| negative(request.first().copied().unwrap_or(0), Nrc::RESPONSE_PENDING))
            .collect();
        frames.push(self.handle(request));
        frames
    }

    /// Handles a UDS request, returning the final response bytes.
    pub fn handle(&mut self, request: &[u8]) -> Vec<u8> {
        let Some(&service) = request.first() else {
            return negative(0x00, Nrc::INCORRECT_MESSAGE_LENGTH);
        };

        match service {
            sid::DIAGNOSTIC_SESSION_CONTROL => self.session_control(request),
            sid::TESTER_PRESENT => self.tester_present(request),
            sid::READ_DATA_BY_IDENTIFIER => self.read_data(request),
            sid::IO_CONTROL_BY_IDENTIFIER => self.io_control(request),
            sid::ROUTINE_CONTROL => self.routine_control(request),
            sid::READ_DTC_INFORMATION => self.read_dtc(request),
            sid::CLEAR_DIAGNOSTIC_INFORMATION => {
                self.dtcs.clear();
                vec![sid::CLEAR_DIAGNOSTIC_INFORMATION + sid::POSITIVE_RESPONSE_OFFSET]
            }
            // A real module rejects flashing-related services outright unless
            // it has been unlocked, which is what we want to see in tests.
            sid::WRITE_DATA_BY_IDENTIFIER => negative(service, Nrc::SECURITY_ACCESS_DENIED),
            _ => negative(service, Nrc::SERVICE_NOT_SUPPORTED),
        }
    }

    fn session_control(&mut self, request: &[u8]) -> Vec<u8> {
        let Some(&requested) = request.get(1) else {
            return negative(request[0], Nrc::INCORRECT_MESSAGE_LENGTH);
        };
        if requested == 0x02 {
            // Programming session needs a security unlock on a real module.
            return negative(request[0], Nrc::SECURITY_ACCESS_DENIED);
        }
        if requested == 0x03 && !self.conditions.ignition_on {
            return negative(request[0], Nrc::CONDITIONS_NOT_CORRECT);
        }
        self.session = requested;
        self.session_started = Some(Instant::now());
        vec![
            sid::DIAGNOSTIC_SESSION_CONTROL + sid::POSITIVE_RESPONSE_OFFSET,
            requested,
            // P2 and P2* timing parameters, as a real module reports.
            0x00,
            0x32,
            0x01,
            0xF4,
        ]
    }

    fn tester_present(&mut self, request: &[u8]) -> Vec<u8> {
        if self.session != 0x01 {
            self.session_started = Some(Instant::now());
        }
        // Suppress-positive-response bit means answer nothing at all.
        if request.get(1) == Some(&0x80) {
            return Vec::new();
        }
        vec![sid::TESTER_PRESENT + sid::POSITIVE_RESPONSE_OFFSET, 0x00]
    }

    fn read_data(&mut self, request: &[u8]) -> Vec<u8> {
        if request.len() < 3 {
            return negative(request[0], Nrc::INCORRECT_MESSAGE_LENGTH);
        }
        let id = u16::from_be_bytes([request[1], request[2]]);
        let mut response = vec![
            sid::READ_DATA_BY_IDENTIFIER + sid::POSITIVE_RESPONSE_OFFSET,
            request[1],
            request[2],
        ];

        match id {
            slt_uds::did::VIN => response.extend_from_slice(self.vin.as_bytes()),
            slt_uds::did::ECU_SERIAL => response.extend_from_slice(self.serial.as_bytes()),
            slt_uds::did::ACTIVE_SESSION => {
                let active = if self.session_active() { self.session } else { 0x01 };
                response.push(active);
            }
            SIM_LAMP_DID => {
                // Mirrors the real convention that a controllable DID is also
                // readable: report the commanded level of every held lamp.
                for (lamp, state) in &self.lamps {
                    response.push(*lamp);
                    response.push(state.level);
                }
            }
            _ => return negative(request[0], Nrc::REQUEST_OUT_OF_RANGE),
        }
        response
    }

    fn io_control(&mut self, request: &[u8]) -> Vec<u8> {
        if request.len() < 4 {
            return negative(request[0], Nrc::INCORRECT_MESSAGE_LENGTH);
        }
        let id = u16::from_be_bytes([request[1], request[2]]);
        let control = request[3];

        if id != SIM_LAMP_DID {
            return negative(request[0], Nrc::REQUEST_OUT_OF_RANGE);
        }
        if !self.session_active() {
            return negative(request[0], Nrc::SERVICE_NOT_SUPPORTED_IN_SESSION);
        }
        if let Some(nrc) = self.condition_failure() {
            return negative(request[0], nrc);
        }

        match control {
            // ReturnControlToECU
            0x00 => {
                if let Some(&lamp) = request.get(4) {
                    self.release_lamp(lamp);
                } else {
                    // No lamp given means release everything, which is what the
                    // panic path relies on.
                    for state in self.lamps.values_mut() {
                        state.controlled = false;
                        state.level = 0;
                    }
                }
            }
            // ShortTermAdjustment
            0x03 => {
                let Some(&lamp) = request.get(4) else {
                    return negative(request[0], Nrc::INCORRECT_MESSAGE_LENGTH);
                };
                let level = request.get(5).copied().unwrap_or(100);
                if slt_catalog::lamp::by_id(lamp).is_none() && lamp != slt_catalog::lamp::ALL_LAMPS
                {
                    return negative(request[0], Nrc::REQUEST_OUT_OF_RANGE);
                }
                if lamp == slt_catalog::lamp::ALL_LAMPS {
                    for entry in slt_catalog::lamp::ALL {
                        self.lamps.insert(
                            entry.id,
                            LampState {
                                level,
                                controlled: true,
                            },
                        );
                    }
                } else {
                    self.lamps.insert(
                        lamp,
                        LampState {
                            level,
                            controlled: true,
                        },
                    );
                }
            }
            0x01 | 0x02 => {}
            _ => return negative(request[0], Nrc::SUB_FUNCTION_NOT_SUPPORTED),
        }

        vec![
            sid::IO_CONTROL_BY_IDENTIFIER + sid::POSITIVE_RESPONSE_OFFSET,
            request[1],
            request[2],
            control,
        ]
    }

    fn release_lamp(&mut self, lamp: u8) {
        if lamp == slt_catalog::lamp::ALL_LAMPS {
            for state in self.lamps.values_mut() {
                state.controlled = false;
                state.level = 0;
            }
        } else if let Some(state) = self.lamps.get_mut(&lamp) {
            state.controlled = false;
            state.level = 0;
        }
    }

    fn routine_control(&mut self, request: &[u8]) -> Vec<u8> {
        if request.len() < 4 {
            return negative(request[0], Nrc::INCORRECT_MESSAGE_LENGTH);
        }
        if !self.session_active() {
            return negative(request[0], Nrc::SERVICE_NOT_SUPPORTED_IN_SESSION);
        }
        vec![
            sid::ROUTINE_CONTROL + sid::POSITIVE_RESPONSE_OFFSET,
            request[1],
            request[2],
            request[3],
        ]
    }

    fn read_dtc(&mut self, request: &[u8]) -> Vec<u8> {
        if request.get(1) != Some(&0x02) {
            return negative(request[0], Nrc::SUB_FUNCTION_NOT_SUPPORTED);
        }
        let mask = request.get(2).copied().unwrap_or(0xFF);
        let mut response = vec![
            sid::READ_DTC_INFORMATION + sid::POSITIVE_RESPONSE_OFFSET,
            0x02,
            0xFF,
        ];
        for (code, status) in &self.dtcs {
            if status & mask == 0 {
                continue;
            }
            let bytes = code.to_be_bytes();
            response.extend_from_slice(&bytes[1..4]);
            response.push(*status);
        }
        response
    }

    /// Maps simulated vehicle conditions onto the NRC a real module would send.
    fn condition_failure(&self) -> Option<Nrc> {
        if !self.conditions.ignition_on {
            return Some(Nrc::CONDITIONS_NOT_CORRECT);
        }
        if self.conditions.engine_running {
            return Some(Nrc(0x83));
        }
        if self.conditions.speed_kph > 0 {
            return Some(Nrc(0x88));
        }
        if self.conditions.voltage < 11.0 {
            return Some(Nrc(0x93));
        }
        None
    }
}

fn negative(service: u8, nrc: Nrc) -> Vec<u8> {
    vec![sid::NEGATIVE_RESPONSE, service, nrc.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fem() -> SimulatedEcu {
        SimulatedEcu::new(0x40, "FEM_BODY")
    }

    #[test]
    fn reads_vin() {
        let mut ecu = fem();
        let response = ecu.handle(&[0x22, 0xF1, 0x90]);
        assert_eq!(response[0], 0x62);
        assert_eq!(&response[3..], b"WBA8E9G51GNT12345");
    }

    #[test]
    fn unknown_identifier_reports_out_of_range() {
        let mut ecu = fem();
        assert_eq!(ecu.handle(&[0x22, 0x12, 0x34]), vec![0x7F, 0x22, 0x31]);
    }

    #[test]
    fn actuation_requires_an_extended_session() {
        let mut ecu = fem();
        let denied = ecu.handle(&[0x2F, 0xD0, 0xFF, 0x03, 0x30, 100]);
        assert_eq!(denied, vec![0x7F, 0x2F, 0x7F]);

        ecu.handle(&[0x10, 0x03]);
        let allowed = ecu.handle(&[0x2F, 0xD0, 0xFF, 0x03, 0x30, 100]);
        assert_eq!(allowed[0], 0x6F);
        assert_eq!(ecu.lamp_state(0x30).level, 100);
        assert!(ecu.lamp_state(0x30).controlled);
    }

    #[test]
    fn return_control_to_ecu_releases_a_lamp() {
        let mut ecu = fem();
        ecu.handle(&[0x10, 0x03]);
        ecu.handle(&[0x2F, 0xD0, 0xFF, 0x03, 0x30, 100]);
        assert_eq!(ecu.controlled_count(), 1);

        ecu.handle(&[0x2F, 0xD0, 0xFF, 0x00, 0x30]);
        assert_eq!(ecu.controlled_count(), 0);
        assert!(!ecu.lamp_state(0x30).controlled);
    }

    #[test]
    fn release_without_a_lamp_releases_everything() {
        let mut ecu = fem();
        ecu.handle(&[0x10, 0x03]);
        ecu.handle(&[0x2F, 0xD0, 0xFF, 0x03, 0x30, 100]);
        ecu.handle(&[0x2F, 0xD0, 0xFF, 0x03, 0x31, 100]);
        assert_eq!(ecu.controlled_count(), 2);

        ecu.handle(&[0x2F, 0xD0, 0xFF, 0x00]);
        assert_eq!(ecu.controlled_count(), 0);
    }

    #[test]
    fn all_lamps_index_actuates_every_output() {
        let mut ecu = fem();
        ecu.handle(&[0x10, 0x03]);
        ecu.handle(&[0x2F, 0xD0, 0xFF, 0x03, slt_catalog::lamp::ALL_LAMPS, 50]);
        assert_eq!(ecu.controlled_count(), slt_catalog::lamp::ALL.len());
    }

    #[test]
    fn undefined_lamp_index_is_rejected() {
        let mut ecu = fem();
        ecu.handle(&[0x10, 0x03]);
        // 0x0F is a gap in BMW's enumeration.
        assert_eq!(
            ecu.handle(&[0x2F, 0xD0, 0xFF, 0x03, 0x0F, 100]),
            vec![0x7F, 0x2F, 0x31]
        );
    }

    #[test]
    fn programming_session_is_refused() {
        let mut ecu = fem();
        assert_eq!(ecu.handle(&[0x10, 0x02]), vec![0x7F, 0x10, 0x33]);
    }

    #[test]
    fn persistent_writes_are_refused() {
        let mut ecu = fem();
        ecu.handle(&[0x10, 0x03]);
        assert_eq!(ecu.handle(&[0x2E, 0x30, 0x61, 0x01]), vec![0x7F, 0x2E, 0x33]);
    }

    #[test]
    fn ignition_off_blocks_the_extended_session() {
        let mut ecu = fem();
        ecu.conditions.ignition_on = false;
        assert_eq!(ecu.handle(&[0x10, 0x03]), vec![0x7F, 0x10, 0x22]);
    }

    #[test]
    fn a_running_engine_blocks_actuation() {
        let mut ecu = fem();
        ecu.handle(&[0x10, 0x03]);
        ecu.conditions.engine_running = true;
        assert_eq!(
            ecu.handle(&[0x2F, 0xD0, 0xFF, 0x03, 0x30, 100]),
            vec![0x7F, 0x2F, 0x83]
        );
    }

    #[test]
    fn movement_blocks_actuation() {
        let mut ecu = fem();
        ecu.handle(&[0x10, 0x03]);
        ecu.conditions.speed_kph = 30;
        assert_eq!(
            ecu.handle(&[0x2F, 0xD0, 0xFF, 0x03, 0x30, 100]),
            vec![0x7F, 0x2F, 0x88]
        );
    }

    #[test]
    fn low_voltage_blocks_actuation() {
        let mut ecu = fem();
        ecu.handle(&[0x10, 0x03]);
        ecu.conditions.voltage = 10.2;
        assert_eq!(
            ecu.handle(&[0x2F, 0xD0, 0xFF, 0x03, 0x30, 100]),
            vec![0x7F, 0x2F, 0x93]
        );
    }

    #[test]
    fn suppressed_tester_present_gets_no_reply() {
        let mut ecu = fem();
        assert!(ecu.handle(&[0x3E, 0x80]).is_empty());
        assert_eq!(ecu.handle(&[0x3E, 0x00]), vec![0x7E, 0x00]);
    }

    #[test]
    fn configured_pending_responses_precede_the_real_answer() {
        let mut ecu = fem();
        ecu.pending_responses = 2;

        let frames = ecu.handle_sequence(&[0x22, 0xF1, 0x90]);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0], vec![0x7F, 0x22, 0x78]);
        assert_eq!(frames[1], vec![0x7F, 0x22, 0x78]);
        assert_eq!(frames[2][0], 0x62);
    }

    #[test]
    fn without_pending_configured_one_frame_comes_back() {
        let mut ecu = fem();
        assert_eq!(ecu.handle_sequence(&[0x22, 0xF1, 0x90]).len(), 1);
    }

    #[test]
    fn dtcs_are_reported_and_cleared() {
        let mut ecu = fem();
        ecu.add_dtc(0x8040B8, 0x08);
        let response = ecu.handle(&[0x19, 0x02, 0xFF]);
        assert_eq!(&response[3..], &[0x80, 0x40, 0xB8, 0x08]);

        ecu.handle(&[0x14, 0xFF, 0xFF, 0xFF]);
        assert_eq!(ecu.handle(&[0x19, 0x02, 0xFF]).len(), 3);
    }

    #[test]
    fn dtc_status_mask_filters_the_report() {
        let mut ecu = fem();
        ecu.add_dtc(0x8040B8, 0x08); // confirmed
        ecu.add_dtc(0x9CBC00, 0x04); // pending only
        let confirmed = ecu.handle(&[0x19, 0x02, 0x08]);
        assert_eq!(&confirmed[3..], &[0x80, 0x40, 0xB8, 0x08]);
    }
}
