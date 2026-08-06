//! The effect engine: a millisecond-accurate scheduler in front of the safety
//! supervisor and the UDS client.
//!
//! Timing lives here rather than in the frontend because effect steps can be as
//! short as 20 ms, and JavaScript timer jitter of 5-15 ms would be a visible
//! fraction of a step. Step deadlines are computed from a fixed origin so error
//! cannot accumulate across a long show.

pub mod actuator;
pub mod effect;
pub mod supervisor;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;

pub use actuator::{Actuator, ActuatorError};
pub use effect::{Effect, LampCommand, Step, Timing};
pub use supervisor::{Preflight, SafetyError, SafetySupervisor};

/// Engine failures.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Safety(#[from] SafetyError),

    #[error(transparent)]
    Actuator(#[from] ActuatorError),

    #[error("effect is invalid: {0}")]
    InvalidEffect(String),

    #[error("the engine task is no longer running")]
    EngineStopped,
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// Events the engine emits for the UI to render.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EngineEvent {
    /// An effect started.
    Started { effect_id: String },
    /// An effect advanced to a step.
    Step {
        effect_id: String,
        index: usize,
        commands: Vec<LampCommand>,
    },
    /// An effect stopped, with the reason.
    Stopped { effect_id: String, reason: String },
    /// All outputs were handed back to the ECU.
    Released { lamps: Vec<u8> },
    /// Something went wrong. The engine has already stopped and released.
    Error { message: String },
    /// Periodic status for the UI.
    Status(EngineStatus),
}

/// A snapshot of what the engine is doing.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub running: bool,
    pub effect_id: Option<String>,
    pub step_index: usize,
    pub step_count: usize,
    pub held_lamps: Vec<u8>,
    /// Measured beats per minute, when beat sync is active.
    pub bpm: Option<f32>,
    pub beat_sync: bool,
    /// Effective per-lamp dwell floor in milliseconds.
    pub min_dwell_ms: u64,
    pub research_mode: bool,
}

/// Commands accepted by the engine task.
enum Command {
    Start(Box<Effect>, oneshot::Sender<Result<()>>),
    Stop(oneshot::Sender<Result<()>>),
    Beat(f32),
    SetBeatSync(bool),
    SetLamp {
        lamp: u8,
        level: u8,
        reply: oneshot::Sender<Result<()>>,
    },
    ReleaseAll(oneshot::Sender<Result<()>>),
    Panic(oneshot::Sender<Result<()>>),
    Status(oneshot::Sender<EngineStatus>),
    SetResearchMode(bool),
    SetSafetyCriticalConfirmed(bool),
    SetPreflight(Box<Preflight>),
}

/// Handle to the running engine.
pub struct Engine {
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<EngineEvent>,
    task: JoinHandle<()>,
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Engine {
    /// Spawns the engine task.
    pub fn spawn(actuator: Actuator, supervisor: SafetySupervisor) -> Self {
        let (command_tx, command_rx) = mpsc::channel(64);
        let (event_tx, _) = broadcast::channel(256);

        let runner = Runner {
            actuator,
            supervisor,
            events: event_tx.clone(),
            playback: None,
            beat_sync: false,
            bpm: None,
            pending_beat: false,
        };
        let task = tokio::spawn(runner.run(command_rx));

        Self {
            commands: command_tx,
            events: event_tx,
            task,
        }
    }

    /// Subscribes to engine events.
    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.events.subscribe()
    }

    async fn send(&self, command: Command) -> Result<()> {
        self.commands
            .send(command)
            .await
            .map_err(|_| EngineError::EngineStopped)
    }

    async fn request<T>(
        &self,
        make: impl FnOnce(oneshot::Sender<T>) -> Command,
    ) -> Result<T> {
        let (tx, rx) = oneshot::channel();
        self.send(make(tx)).await?;
        rx.await.map_err(|_| EngineError::EngineStopped)
    }

    /// Starts an effect, replacing whatever was running.
    pub async fn start(&self, effect: Effect) -> Result<()> {
        effect
            .validate()
            .map_err(EngineError::InvalidEffect)?;
        self.request(|reply| Command::Start(Box::new(effect), reply))
            .await?
    }

    /// Stops playback and releases every held output.
    pub async fn stop(&self) -> Result<()> {
        self.request(Command::Stop).await?
    }

    /// Stops immediately and hands everything back, ignoring dwell limits.
    ///
    /// This is the user-facing panic control, so it must never be blocked by a
    /// rate limit or a preflight state.
    pub async fn panic_stop(&self) -> Result<()> {
        self.request(Command::Panic).await?
    }

    /// Sets one lamp directly, for the manual console.
    pub async fn set_lamp(&self, lamp: u8, level: u8) -> Result<()> {
        self.request(|reply| Command::SetLamp { lamp, level, reply })
            .await?
    }

    /// Hands every held output back to the ECU.
    pub async fn release_all(&self) -> Result<()> {
        self.request(Command::ReleaseAll).await?
    }

    /// Feeds in a detected beat. Ignored unless an effect uses beat timing.
    pub async fn submit_beat(&self, bpm: f32) -> Result<()> {
        self.send(Command::Beat(bpm)).await
    }

    pub async fn set_beat_sync(&self, enabled: bool) -> Result<()> {
        self.send(Command::SetBeatSync(enabled)).await
    }

    pub async fn set_research_mode(&self, enabled: bool) -> Result<()> {
        self.send(Command::SetResearchMode(enabled)).await
    }

    pub async fn set_safety_critical_confirmed(&self, confirmed: bool) -> Result<()> {
        self.send(Command::SetSafetyCriticalConfirmed(confirmed)).await
    }

    /// Hands a completed preflight report to the supervisor.
    pub async fn set_preflight(&self, preflight: Preflight) -> Result<()> {
        self.send(Command::SetPreflight(Box::new(preflight))).await
    }

    pub async fn status(&self) -> Result<EngineStatus> {
        self.request(Command::Status).await
    }
}

/// Playback state for the effect currently running.
struct Playback {
    effect: Effect,
    step_index: usize,
    /// Fixed origin for deadline computation, so timing cannot drift.
    origin: tokio::time::Instant,
    /// Cumulative offset of the current step from `origin`.
    elapsed_ms: u64,
}

impl Playback {
    fn new(effect: Effect) -> Self {
        Self {
            effect,
            step_index: 0,
            origin: tokio::time::Instant::now(),
            elapsed_ms: 0,
        }
    }

    fn current_step(&self) -> Option<&Step> {
        self.effect.steps.get(self.step_index)
    }

    /// When the current step should end.
    ///
    /// Derived from the origin plus accumulated offsets rather than "now plus
    /// duration", so a slow round trip does not push every later step late.
    fn deadline(&self) -> tokio::time::Instant {
        let duration = self
            .current_step()
            .map(|s| s.duration_ms)
            .unwrap_or_default();
        self.origin + Duration::from_millis(self.elapsed_ms + duration)
    }

    /// Advances to the next step, returning false when a non-looping effect ends.
    fn advance(&mut self) -> bool {
        let duration = self
            .current_step()
            .map(|s| s.duration_ms)
            .unwrap_or_default();
        self.elapsed_ms += duration;
        self.step_index += 1;

        if self.step_index >= self.effect.steps.len() {
            if !self.effect.looping {
                return false;
            }
            self.step_index = 0;
        }
        true
    }
}

/// The engine task.
struct Runner {
    actuator: Actuator,
    supervisor: SafetySupervisor,
    events: broadcast::Sender<EngineEvent>,
    playback: Option<Playback>,
    beat_sync: bool,
    bpm: Option<f32>,
    /// Set when a beat arrives while a beat-timed effect is waiting.
    pending_beat: bool,
}

impl Runner {
    async fn run(mut self, mut commands: mpsc::Receiver<Command>) {
        loop {
            // Only arm a timer when a fixed-timing effect is playing. A
            // beat-timed effect waits for a beat instead, and an idle engine
            // waits indefinitely.
            let sleep_until = match &self.playback {
                Some(playback) if playback.effect.timing == Timing::Fixed => {
                    Some(playback.deadline())
                }
                _ => None,
            };

            tokio::select! {
                command = commands.recv() => {
                    match command {
                        Some(command) => {
                            if self.handle(command).await {
                                return;
                            }
                        }
                        // All handles dropped: release before exiting so the car
                        // is never left with outputs held.
                        None => {
                            let _ = self.release_all().await;
                            return;
                        }
                    }
                }
                _ = async {
                    match sleep_until {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        // Never completes, so select waits on the other branch.
                        None => std::future::pending().await,
                    }
                } => {
                    self.tick().await;
                }
            }
        }
    }

    /// Handles a command. Returns true when the task should exit.
    async fn handle(&mut self, command: Command) -> bool {
        match command {
            Command::Start(effect, reply) => {
                let result = self.start(*effect).await;
                let _ = reply.send(result);
            }
            Command::Stop(reply) => {
                let result = self.stop("stopped by user").await;
                let _ = reply.send(result);
            }
            Command::Panic(reply) => {
                let result = self.panic_stop().await;
                let _ = reply.send(result);
            }
            Command::SetLamp { lamp, level, reply } => {
                let result = self.set_lamp(lamp, level).await;
                let _ = reply.send(result);
            }
            Command::ReleaseAll(reply) => {
                let result = self.release_all().await;
                let _ = reply.send(result);
            }
            Command::Beat(bpm) => {
                self.bpm = Some(bpm);
                if self
                    .playback
                    .as_ref()
                    .is_some_and(|p| p.effect.timing == Timing::PerBeat)
                {
                    self.pending_beat = true;
                    self.tick().await;
                }
            }
            Command::SetBeatSync(enabled) => {
                self.beat_sync = enabled;
                if !enabled {
                    self.bpm = None;
                }
            }
            Command::SetResearchMode(enabled) => self.supervisor.set_research_mode(enabled),
            Command::SetSafetyCriticalConfirmed(confirmed) => {
                self.supervisor.set_safety_critical_confirmed(confirmed)
            }
            Command::SetPreflight(preflight) => {
                self.supervisor.adopt_preflight(*preflight);
            }
            Command::Status(reply) => {
                let _ = reply.send(self.status());
            }
        }
        false
    }

    fn status(&self) -> EngineStatus {
        EngineStatus {
            running: self.playback.is_some(),
            effect_id: self.playback.as_ref().map(|p| p.effect.id.clone()),
            step_index: self.playback.as_ref().map_or(0, |p| p.step_index),
            step_count: self.playback.as_ref().map_or(0, |p| p.effect.steps.len()),
            held_lamps: self.supervisor.held_lamps(),
            bpm: self.bpm,
            beat_sync: self.beat_sync,
            min_dwell_ms: self.supervisor.min_dwell().as_millis() as u64,
            research_mode: self.supervisor.research_mode(),
        }
    }

    async fn start(&mut self, effect: Effect) -> Result<()> {
        self.supervisor
            .authorize_effect(&effect, self.actuator.catalog_verified())?;

        // Refuse to drive an output the car has already flagged, rather than
        // adding to a short-circuit counter that is already counting down.
        let degraded = self.supervisor.degraded_lamp_ids();
        let touched: Vec<u8> = effect
            .lamps()
            .into_iter()
            .filter(|l| degraded.contains(l))
            .collect();
        if !touched.is_empty() {
            let names: Vec<String> = touched
                .iter()
                .map(|&id| {
                    slt_catalog::lamp::by_id(id)
                        .map_or_else(|| format!("0x{id:02X}"), |l| l.name.to_string())
                })
                .collect();
            return Err(EngineError::InvalidEffect(format!(
                "this effect drives {}, which the car has already recorded faults for",
                names.join(", ")
            )));
        }

        // Leaving the previous effect's outputs held would strand lamps on.
        if self.playback.is_some() {
            let _ = self.release_all().await;
        }

        let effect_id = effect.id.clone();
        self.playback = Some(Playback::new(effect));
        self.emit(EngineEvent::Started {
            effect_id: effect_id.clone(),
        });

        // Apply step zero immediately: waiting for the first deadline would show
        // a blank frame for the length of the step.
        self.apply_current_step().await?;
        Ok(())
    }

    async fn tick(&mut self) {
        let Some(playback) = &mut self.playback else {
            return;
        };

        if playback.effect.timing == Timing::PerBeat {
            if !self.pending_beat {
                return;
            }
            self.pending_beat = false;
        }

        let Some(playback) = &mut self.playback else {
            return;
        };
        if !playback.advance() {
            let _ = self.stop("effect finished").await;
            return;
        }

        if let Err(e) = self.apply_current_step().await {
            tracing::error!(error = %e, "failed to apply step, stopping");
            self.emit(EngineEvent::Error {
                message: e.to_string(),
            });
            let _ = self.stop("error while applying step").await;
        }
    }

    /// Sends the current step's lamp commands.
    async fn apply_current_step(&mut self) -> Result<()> {
        let Some(playback) = &self.playback else {
            return Ok(());
        };
        let Some(step) = playback.current_step() else {
            return Ok(());
        };
        let commands = step.commands.clone();
        let effect_id = playback.effect.id.clone();
        let index = playback.step_index;

        for command in &commands {
            // Honour the dwell floor by waiting rather than skipping: skipping
            // would leave the lamp in the previous step's state.
            if let Some(remaining) = self.supervisor.dwell_remaining(command.lamp) {
                tokio::time::sleep(remaining).await;
            }
            self.actuator.set_lamp(command.lamp, command.level).await?;
            self.supervisor.record_change(command.lamp, command.level);
        }

        self.emit(EngineEvent::Step {
            effect_id,
            index,
            commands,
        });
        Ok(())
    }

    async fn stop(&mut self, reason: &str) -> Result<()> {
        let effect_id = self
            .playback
            .as_ref()
            .map(|p| p.effect.id.clone())
            .unwrap_or_default();
        self.playback = None;
        self.pending_beat = false;

        let result = self.release_all().await;
        self.emit(EngineEvent::Stopped {
            effect_id,
            reason: reason.to_string(),
        });
        result
    }

    /// Releases everything, ignoring every gate.
    ///
    /// Deliberately bypasses dwell limits and preflight state: if the user hits
    /// panic, or the connection is dropping, handing the outputs back is the only
    /// thing that matters.
    async fn panic_stop(&mut self) -> Result<()> {
        tracing::warn!("panic stop requested");
        self.playback = None;
        self.pending_beat = false;
        let result = self.actuator.release_all().await;
        self.supervisor.record_release_all();
        self.emit(EngineEvent::Released { lamps: Vec::new() });
        result.map_err(Into::into)
    }

    async fn set_lamp(&mut self, lamp: u8, level: u8) -> Result<()> {
        if self.playback.is_some() {
            return Err(EngineError::InvalidEffect(
                "stop the running effect before setting lamps by hand".into(),
            ));
        }
        if self.supervisor.degraded_lamp_ids().contains(&lamp) {
            return Err(EngineError::InvalidEffect(format!(
                "the car has recorded faults for lamp 0x{lamp:02X}; Strobes will not drive it"
            )));
        }
        self.supervisor.check_dwell(lamp)?;
        self.actuator.set_lamp(lamp, level).await?;
        self.supervisor.record_change(lamp, level);
        Ok(())
    }

    async fn release_all(&mut self) -> Result<()> {
        let held = self.supervisor.held_lamps();
        if held.is_empty() {
            return Ok(());
        }
        let result = self.actuator.release_all().await;
        self.supervisor.record_release_all();
        self.emit(EngineEvent::Released { lamps: held });
        result.map_err(Into::into)
    }

    fn emit(&self, event: EngineEvent) {
        // A send error just means nobody is listening yet, which is fine.
        let _ = self.events.send(event);
    }
}

/// Shared, cloneable holder so the Tauri layer can swap the engine on reconnect.
pub type SharedEngine = Arc<Mutex<Option<Engine>>>;

/// Convenience alias for preflight counter readings.
pub type CounterMap = HashMap<u8, u8>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadlines_do_not_drift_across_steps() {
        let effect = Effect {
            id: "t".into(),
            name: "t".into(),
            description: String::new(),
            steps: vec![
                Step {
                    commands: vec![LampCommand::on(0x30)],
                    duration_ms: 100,
                },
                Step {
                    commands: vec![LampCommand::off(0x30)],
                    duration_ms: 150,
                },
            ],
            looping: true,
            timing: Timing::Fixed,
        };

        let mut playback = Playback::new(effect);
        let origin = playback.origin;

        assert_eq!(playback.deadline() - origin, Duration::from_millis(100));
        assert!(playback.advance());
        // Second deadline is 100 + 150 from the origin, not 150 from now.
        assert_eq!(playback.deadline() - origin, Duration::from_millis(250));
        assert!(playback.advance());
        // Looping back keeps accumulating from the same origin.
        assert_eq!(playback.deadline() - origin, Duration::from_millis(350));
        assert_eq!(playback.step_index, 0);
    }

    #[test]
    fn non_looping_effect_reports_completion() {
        let effect = Effect {
            id: "once".into(),
            name: "once".into(),
            description: String::new(),
            steps: vec![Step {
                commands: vec![LampCommand::on(0x30)],
                duration_ms: 50,
            }],
            looping: false,
            timing: Timing::Fixed,
        };

        let mut playback = Playback::new(effect);
        assert!(!playback.advance());
    }

    #[test]
    fn looping_effect_wraps_to_the_first_step() {
        let effect = Effect {
            id: "loop".into(),
            name: "loop".into(),
            description: String::new(),
            steps: vec![
                Step {
                    commands: vec![LampCommand::on(0x30)],
                    duration_ms: 50,
                },
                Step {
                    commands: vec![LampCommand::off(0x30)],
                    duration_ms: 50,
                },
            ],
            looping: true,
            timing: Timing::Fixed,
        };

        let mut playback = Playback::new(effect);
        assert!(playback.advance());
        assert_eq!(playback.step_index, 1);
        assert!(playback.advance());
        assert_eq!(playback.step_index, 0);
    }
}
