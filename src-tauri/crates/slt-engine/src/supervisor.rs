//! The safety supervisor.
//!
//! Sits between effect playback and the UDS client, and owns three jobs that the
//! research phase identified as mattering more than anything else in this
//! application:
//!
//! 1. **Preflight.** Refuse to run at all if the car is already reporting lamp
//!    faults, because strobing an output whose short-circuit counter is already
//!    counting down risks permanently disabling that driver.
//! 2. **Rate limiting.** Enforce a per-lamp minimum dwell so rapid switching
//!    does not look like a fault to the module's cold/warm lamp monitoring.
//! 3. **Release tracking.** Remember every output currently held, so a stop, a
//!    panic, a disconnect or a dropped connection can hand all of them back.
//!
//! See `docs/protocol-research.md` section 7.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use slt_uds::Dtc;

/// BMW's per-lamp short-circuit shutdown limit. At this value the output is
/// permanently disabled and the module needs replacing.
pub const SHORT_CIRCUIT_LIMIT: u8 = 50;

/// Supervisor rejections.
#[derive(Debug, thiserror::Error)]
pub enum SafetyError {
    #[error("preflight has not been run; call run_preflight before starting an effect")]
    PreflightRequired,

    #[error("preflight failed: {0}")]
    PreflightFailed(String),

    #[error(
        "lamp 0x{lamp:02X} changed {elapsed_ms} ms ago but needs {required_ms} ms between changes"
    )]
    DwellTooShort {
        lamp: u8,
        elapsed_ms: u64,
        required_ms: u64,
    },

    #[error("action '{0}' has an unverified identifier; enable research mode to send it")]
    UnverifiedAction(String),

    #[error("effect '{effect}' drives a legally-regulated signalling device; confirm before running")]
    SafetyCriticalNotConfirmed { effect: String },
}

/// The outcome of a preflight check.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Preflight {
    /// Whether it is safe to run effects.
    pub passed: bool,
    /// Faults stored before the session started, for after-the-fact comparison.
    pub dtcs_before: Vec<Dtc>,
    /// Lamp outputs whose short-circuit counter is non-zero.
    pub degraded_lamps: Vec<DegradedLamp>,
    /// Blocking problems.
    pub blockers: Vec<String>,
    /// Non-blocking observations.
    pub warnings: Vec<String>,
    /// Whether the catalog's identifiers have been verified on a real car.
    pub catalog_verified: bool,
}

/// A lamp output with a non-zero short-circuit counter.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DegradedLamp {
    pub lamp: u8,
    pub code: String,
    pub name: String,
    pub counter: u8,
    /// Whether the output is already locked out.
    pub locked_out: bool,
}

/// Enforces the safety rules.
pub struct SafetySupervisor {
    /// Allows transmitting unverified catalog actions. Off by default.
    research_mode: bool,
    /// Whether the user accepted driving signalling devices.
    safety_critical_confirmed: bool,
    /// Lamps currently held by the tester, so they can all be released.
    held: HashSet<u8>,
    /// When each lamp last changed, for dwell enforcement.
    last_change: HashMap<u8, Instant>,
    /// Minimum time between changes to the same lamp.
    min_dwell: Duration,
    preflight: Option<Preflight>,
}

impl SafetySupervisor {
    pub fn new(min_dwell: Duration) -> Self {
        Self {
            research_mode: false,
            safety_critical_confirmed: false,
            held: HashSet::new(),
            last_change: HashMap::new(),
            min_dwell,
            preflight: None,
        }
    }

    pub fn min_dwell(&self) -> Duration {
        self.min_dwell
    }

    pub fn set_min_dwell(&mut self, dwell: Duration) {
        self.min_dwell = dwell;
    }

    pub fn research_mode(&self) -> bool {
        self.research_mode
    }

    pub fn set_research_mode(&mut self, enabled: bool) {
        if enabled {
            tracing::warn!("research mode enabled: unverified catalog actions may be transmitted");
        }
        self.research_mode = enabled;
    }

    pub fn set_safety_critical_confirmed(&mut self, confirmed: bool) {
        self.safety_critical_confirmed = confirmed;
    }

    /// Builds a preflight report from readings taken off the car.
    ///
    /// Pure so the decision logic is testable without a vehicle; the caller does
    /// the I/O and hands the results in.
    pub fn evaluate_preflight(
        &mut self,
        dtcs: Vec<Dtc>,
        counters: HashMap<u8, u8>,
        catalog_verified: bool,
    ) -> Preflight {
        let mut blockers = Vec::new();
        let mut warnings = Vec::new();

        let mut degraded_lamps: Vec<DegradedLamp> = counters
            .iter()
            .filter(|(_, &count)| count > 0)
            .map(|(&lamp, &counter)| {
                let entry = slt_catalog::lamp::by_id(lamp);
                DegradedLamp {
                    lamp,
                    code: entry.map_or_else(|| format!("0x{lamp:02X}"), |l| l.code.to_string()),
                    name: entry.map_or("unknown output", |l| l.name).to_string(),
                    counter,
                    locked_out: counter >= SHORT_CIRCUIT_LIMIT,
                }
            })
            .collect();
        degraded_lamps.sort_by_key(|d| d.lamp);

        for lamp in degraded_lamps.iter() {
            if lamp.locked_out {
                blockers.push(format!(
                    "{} ({}) is locked out after {} detected short circuits. Repair the fault before running effects.",
                    lamp.name, lamp.code, lamp.counter
                ));
            } else {
                warnings.push(format!(
                    "{} ({}) has {} of {} short circuits recorded. Effects will not touch it.",
                    lamp.name, lamp.code, lamp.counter, SHORT_CIRCUIT_LIMIT
                ));
            }
        }

        let confirmed: Vec<&Dtc> = dtcs.iter().filter(|d| d.confirmed).collect();
        if !confirmed.is_empty() {
            warnings.push(format!(
                "{} confirmed fault code(s) were already stored before connecting. They are recorded so you can tell them apart from anything new.",
                confirmed.len()
            ));
        }

        if !catalog_verified {
            warnings.push(
                "This catalog contains unverified identifiers. Effects stay disabled until they are confirmed on your car, or research mode is enabled.".to_string(),
            );
        }

        let preflight = Preflight {
            passed: blockers.is_empty(),
            dtcs_before: dtcs,
            degraded_lamps,
            blockers,
            warnings,
            catalog_verified,
        };
        self.preflight = Some(preflight.clone());
        preflight
    }

    pub fn preflight(&self) -> Option<&Preflight> {
        self.preflight.as_ref()
    }

    /// Accepts a preflight report evaluated elsewhere.
    ///
    /// The engine runs on its own task, so the report has to be handed across a
    /// channel rather than produced in place.
    pub fn adopt_preflight(&mut self, preflight: Preflight) {
        self.preflight = Some(preflight);
    }

    /// Lamps the preflight found already degraded, which effects must avoid.
    pub fn degraded_lamp_ids(&self) -> HashSet<u8> {
        self.preflight
            .as_ref()
            .map(|p| p.degraded_lamps.iter().map(|d| d.lamp).collect())
            .unwrap_or_default()
    }

    /// Checks whether an effect may start.
    pub fn authorize_effect(
        &self,
        effect: &crate::effect::Effect,
        catalog_verified: bool,
    ) -> Result<(), SafetyError> {
        let Some(preflight) = &self.preflight else {
            return Err(SafetyError::PreflightRequired);
        };
        if !preflight.passed {
            return Err(SafetyError::PreflightFailed(preflight.blockers.join("; ")));
        }
        if !catalog_verified && !self.research_mode {
            return Err(SafetyError::UnverifiedAction(effect.id.clone()));
        }
        if effect.touches_safety_critical_lamps() && !self.safety_critical_confirmed {
            return Err(SafetyError::SafetyCriticalNotConfirmed {
                effect: effect.id.clone(),
            });
        }
        Ok(())
    }

    /// Checks whether a single action may be transmitted.
    pub fn authorize_action(&self, action: &slt_catalog::Action) -> Result<(), SafetyError> {
        if !action.verified && !self.research_mode {
            return Err(SafetyError::UnverifiedAction(action.id.clone()));
        }
        Ok(())
    }

    /// Enforces the per-lamp dwell time.
    ///
    /// Returns how long to wait rather than erroring, so the scheduler can
    /// simply delay instead of dropping a step and leaving a lamp stuck on.
    pub fn dwell_remaining(&self, lamp: u8) -> Option<Duration> {
        let last = self.last_change.get(&lamp)?;
        let elapsed = last.elapsed();
        (elapsed < self.min_dwell).then(|| self.min_dwell - elapsed)
    }

    /// Rejects a change that arrives too soon. Used by the manual lamp console,
    /// where refusing is more informative than silently delaying.
    pub fn check_dwell(&self, lamp: u8) -> Result<(), SafetyError> {
        if let Some(remaining) = self.dwell_remaining(lamp) {
            let elapsed = self.min_dwell.saturating_sub(remaining);
            return Err(SafetyError::DwellTooShort {
                lamp,
                elapsed_ms: elapsed.as_millis() as u64,
                required_ms: self.min_dwell.as_millis() as u64,
            });
        }
        Ok(())
    }

    /// Records that a lamp was commanded, updating held state and dwell timing.
    pub fn record_change(&mut self, lamp: u8, level: u8) {
        self.last_change.insert(lamp, Instant::now());
        if level > 0 {
            self.held.insert(lamp);
        } else {
            // A lamp commanded to zero is still under tester control until it is
            // explicitly released, so it stays in the held set.
            self.held.insert(lamp);
        }
    }

    /// Records that outputs were handed back to the ECU.
    pub fn record_release(&mut self, lamps: &[u8]) {
        for lamp in lamps {
            self.held.remove(lamp);
        }
    }

    pub fn record_release_all(&mut self) {
        self.held.clear();
    }

    /// Outputs currently held by the tester.
    pub fn held_lamps(&self) -> Vec<u8> {
        let mut lamps: Vec<u8> = self.held.iter().copied().collect();
        lamps.sort_unstable();
        lamps
    }

    pub fn holds_anything(&self) -> bool {
        !self.held.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect;

    fn dtc(code: u32, status: u8) -> Dtc {
        // Round-trips through the parser so status decoding stays consistent.
        let body = [
            0xFF,
            (code >> 16) as u8,
            (code >> 8) as u8,
            code as u8,
            status,
        ];
        slt_uds::dtc::parse_dtc_report(&body).remove(0)
    }

    fn supervisor() -> SafetySupervisor {
        SafetySupervisor::new(Duration::from_millis(40))
    }

    #[test]
    fn clean_car_passes_preflight() {
        let mut sup = supervisor();
        let report = sup.evaluate_preflight(vec![], HashMap::new(), true);
        assert!(report.passed);
        assert!(report.blockers.is_empty());
        assert!(report.degraded_lamps.is_empty());
    }

    #[test]
    fn locked_out_lamp_blocks_preflight() {
        let mut sup = supervisor();
        let counters = HashMap::from([(0x03u8, SHORT_CIRCUIT_LIMIT)]);
        let report = sup.evaluate_preflight(vec![], counters, true);

        assert!(!report.passed);
        assert_eq!(report.degraded_lamps.len(), 1);
        assert!(report.degraded_lamps[0].locked_out);
        // The message names the lamp so the user knows what to repair.
        assert!(report.blockers[0].contains("DRL, left"));
    }

    #[test]
    fn partially_degraded_lamp_warns_but_does_not_block() {
        let mut sup = supervisor();
        let counters = HashMap::from([(0x03u8, 12)]);
        let report = sup.evaluate_preflight(vec![], counters, true);

        assert!(report.passed);
        assert_eq!(report.degraded_lamps.len(), 1);
        assert!(!report.degraded_lamps[0].locked_out);
        assert!(!report.warnings.is_empty());
        assert!(sup.degraded_lamp_ids().contains(&0x03));
    }

    #[test]
    fn zero_counters_are_not_reported_as_degraded() {
        let mut sup = supervisor();
        let counters = HashMap::from([(0x03u8, 0), (0x04u8, 0)]);
        let report = sup.evaluate_preflight(vec![], counters, true);
        assert!(report.degraded_lamps.is_empty());
        assert!(report.passed);
    }

    #[test]
    fn pre_existing_faults_are_recorded_as_a_warning() {
        let mut sup = supervisor();
        let report = sup.evaluate_preflight(vec![dtc(0x8040B8, 0x08)], HashMap::new(), true);
        assert!(report.passed);
        assert_eq!(report.dtcs_before.len(), 1);
        assert!(report.warnings.iter().any(|w| w.contains("confirmed fault")));
    }

    #[test]
    fn effects_require_preflight_first() {
        let sup = supervisor();
        let effect = effect::presets().remove(0);
        assert!(matches!(
            sup.authorize_effect(&effect, true),
            Err(SafetyError::PreflightRequired)
        ));
    }

    #[test]
    fn effects_are_blocked_when_preflight_failed() {
        let mut sup = supervisor();
        sup.evaluate_preflight(vec![], HashMap::from([(0x03u8, SHORT_CIRCUIT_LIMIT)]), true);
        let effect = effect::presets().remove(0);
        assert!(matches!(
            sup.authorize_effect(&effect, true),
            Err(SafetyError::PreflightFailed(_))
        ));
    }

    #[test]
    fn unverified_catalog_blocks_effects_outside_research_mode() {
        let mut sup = supervisor();
        sup.evaluate_preflight(vec![], HashMap::new(), false);
        let effect = effect::presets().remove(0);

        assert!(matches!(
            sup.authorize_effect(&effect, false),
            Err(SafetyError::UnverifiedAction(_))
        ));

        sup.set_research_mode(true);
        assert!(sup.authorize_effect(&effect, false).is_ok());
    }

    #[test]
    fn safety_critical_effect_needs_confirmation() {
        let mut sup = supervisor();
        sup.evaluate_preflight(vec![], HashMap::new(), true);

        let indicators = effect::Effect {
            id: "indicators".into(),
            name: "Indicators".into(),
            description: String::new(),
            // Front left turn signal.
            steps: vec![effect::Step {
                commands: vec![effect::LampCommand::on(0x0D)],
                duration_ms: 200,
            }],
            looping: true,
            timing: effect::Timing::Fixed,
        };

        assert!(matches!(
            sup.authorize_effect(&indicators, true),
            Err(SafetyError::SafetyCriticalNotConfirmed { .. })
        ));

        sup.set_safety_critical_confirmed(true);
        assert!(sup.authorize_effect(&indicators, true).is_ok());
    }

    #[test]
    fn dwell_blocks_an_immediate_second_change() {
        let mut sup = supervisor();
        assert!(sup.check_dwell(0x30).is_ok());

        sup.record_change(0x30, 100);
        assert!(matches!(
            sup.check_dwell(0x30),
            Err(SafetyError::DwellTooShort { lamp: 0x30, .. })
        ));
        // A different lamp is unaffected.
        assert!(sup.check_dwell(0x31).is_ok());
    }

    #[test]
    fn dwell_reports_remaining_time() {
        let mut sup = supervisor();
        sup.record_change(0x30, 100);
        let remaining = sup.dwell_remaining(0x30).expect("should still be waiting");
        assert!(remaining <= Duration::from_millis(40));
        assert!(sup.dwell_remaining(0x31).is_none());
    }

    #[test]
    fn dwell_clears_after_the_interval() {
        let mut sup = SafetySupervisor::new(Duration::from_millis(1));
        sup.record_change(0x30, 100);
        std::thread::sleep(Duration::from_millis(5));
        assert!(sup.check_dwell(0x30).is_ok());
        assert!(sup.dwell_remaining(0x30).is_none());
    }

    #[test]
    fn held_lamps_track_actuation_and_release() {
        let mut sup = supervisor();
        sup.record_change(0x30, 100);
        sup.record_change(0x31, 100);
        assert_eq!(sup.held_lamps(), vec![0x30, 0x31]);
        assert!(sup.holds_anything());

        sup.record_release(&[0x30]);
        assert_eq!(sup.held_lamps(), vec![0x31]);

        sup.record_release_all();
        assert!(!sup.holds_anything());
    }

    #[test]
    fn a_lamp_commanded_off_is_still_held() {
        // Commanding zero brightness is not the same as handing the output back,
        // so it must remain in the release list.
        let mut sup = supervisor();
        sup.record_change(0x30, 0);
        assert_eq!(sup.held_lamps(), vec![0x30]);
    }
}
