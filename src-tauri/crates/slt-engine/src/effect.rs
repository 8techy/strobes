//! Effect definitions and the built-in preset library.
//!
//! An effect is a list of steps; each step sets a group of lamps to a level and
//! holds for a duration. Rendering an effect produces a flat timeline of lamp
//! commands, which the scheduler then walks. Keeping the model this simple means
//! effects are data, so users can author them without touching Rust.

/// One lamp at one brightness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LampCommand {
    /// `LAMPNR` from the BMW enumeration.
    pub lamp: u8,
    /// Brightness 0-100. Modules that only support on/off treat >0 as on.
    pub level: u8,
}

impl LampCommand {
    pub const fn on(lamp: u8) -> Self {
        Self { lamp, level: 100 }
    }

    pub const fn off(lamp: u8) -> Self {
        Self { lamp, level: 0 }
    }

    pub const fn at(lamp: u8, level: u8) -> Self {
        Self { lamp, level }
    }
}

/// One step of an effect.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Step {
    /// Lamp states to apply at the start of this step.
    pub commands: Vec<LampCommand>,
    /// How long to hold before the next step.
    pub duration_ms: u64,
}

/// How an effect advances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Timing {
    /// Use each step's own duration.
    #[default]
    Fixed,
    /// Advance one step per detected beat, ignoring step durations.
    ///
    /// Beats arrive from the frontend's audio analysis, so the scheduler holds
    /// the current step until one lands.
    PerBeat,
}

/// A named light show.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Effect {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub steps: Vec<Step>,
    /// Whether to restart from the beginning after the final step.
    #[serde(default = "default_true")]
    pub looping: bool,
    #[serde(default)]
    pub timing: Timing,
}

fn default_true() -> bool {
    true
}

impl Effect {
    /// Total duration of one pass, ignoring looping.
    pub fn duration_ms(&self) -> u64 {
        self.steps.iter().map(|s| s.duration_ms).sum()
    }

    /// The shortest step, which decides whether the car can render this effect.
    pub fn shortest_step_ms(&self) -> u64 {
        self.steps
            .iter()
            .map(|s| s.duration_ms)
            .min()
            .unwrap_or(u64::MAX)
    }

    /// Every lamp this effect touches.
    pub fn lamps(&self) -> Vec<u8> {
        let mut lamps: Vec<u8> = self
            .steps
            .iter()
            .flat_map(|s| s.commands.iter().map(|c| c.lamp))
            .collect();
        lamps.sort_unstable();
        lamps.dedup();
        lamps
    }

    /// Whether this effect drives any legally-regulated signalling device.
    pub fn touches_safety_critical_lamps(&self) -> bool {
        self.lamps().iter().any(|&id| {
            slt_catalog::lamp::by_id(id).is_some_and(|lamp| lamp.safety_critical)
        })
    }

    /// Rejects an effect that references lamps BMW does not define.
    pub fn validate(&self) -> Result<(), String> {
        if self.steps.is_empty() {
            return Err(format!("effect '{}' has no steps", self.id));
        }
        for (index, step) in self.steps.iter().enumerate() {
            if step.duration_ms == 0 {
                return Err(format!(
                    "step {} of effect '{}' has zero duration",
                    index + 1,
                    self.id
                ));
            }
            for command in &step.commands {
                let known = slt_catalog::lamp::by_id(command.lamp).is_some()
                    || command.lamp == slt_catalog::lamp::ALL_LAMPS;
                if !known {
                    return Err(format!(
                        "step {} of effect '{}' references undefined lamp 0x{:02X}",
                        index + 1,
                        self.id,
                        command.lamp
                    ));
                }
                if command.level > 100 {
                    return Err(format!(
                        "step {} of effect '{}' has level {} above 100",
                        index + 1,
                        self.id,
                        command.level
                    ));
                }
            }
        }
        Ok(())
    }
}

// Lamp shorthands, so the preset table stays readable.
const TFL_L: u8 = 0x03;
const TFL_R: u8 = 0x04;
const SML_L: u8 = 0x05;
const SML_R: u8 = 0x06;
const POL_L: u8 = 0x09;
const POL_R: u8 = 0x0A;
const NSW_L: u8 = 0x0B;
const NSW_R: u8 = 0x0C;
const SL_L: u8 = 0x14;
const SL_R: u8 = 0x15;
const SL2_L: u8 = 0x16;
const SL2_R: u8 = 0x17;
const RING_L: u8 = 0x30;
const RING_R: u8 = 0x31;
const DESIGN_L: u8 = 0x34;
const DESIGN_R: u8 = 0x35;

/// The built-in effect library.
///
/// Every preset here uses only decorative outputs. Nothing in the shipped set
/// drives indicators or brake lights, so a user cannot accidentally imitate an
/// emergency vehicle by picking a preset.
pub fn presets() -> Vec<Effect> {
    vec![
        Effect {
            id: "wig-wag".into(),
            name: "Wig-Wag".into(),
            description: "Left and right headlight rings alternate.".into(),
            looping: true,
            timing: Timing::Fixed,
            steps: vec![
                Step {
                    commands: vec![LampCommand::on(RING_L), LampCommand::off(RING_R)],
                    duration_ms: 200,
                },
                Step {
                    commands: vec![LampCommand::off(RING_L), LampCommand::on(RING_R)],
                    duration_ms: 200,
                },
            ],
        },
        Effect {
            id: "dual-flash".into(),
            name: "Dual Flash".into(),
            description: "Both sides flash twice, then rest.".into(),
            looping: true,
            timing: Timing::Fixed,
            steps: vec![
                Step {
                    commands: vec![LampCommand::on(RING_L), LampCommand::on(RING_R)],
                    duration_ms: 80,
                },
                Step {
                    commands: vec![LampCommand::off(RING_L), LampCommand::off(RING_R)],
                    duration_ms: 80,
                },
                Step {
                    commands: vec![LampCommand::on(RING_L), LampCommand::on(RING_R)],
                    duration_ms: 80,
                },
                Step {
                    commands: vec![LampCommand::off(RING_L), LampCommand::off(RING_R)],
                    duration_ms: 400,
                },
            ],
        },
        Effect {
            id: "drl-alternating".into(),
            name: "DRL Alternating".into(),
            description: "Daytime running lights trade places.".into(),
            looping: true,
            timing: Timing::Fixed,
            steps: vec![
                Step {
                    commands: vec![LampCommand::on(TFL_L), LampCommand::off(TFL_R)],
                    duration_ms: 250,
                },
                Step {
                    commands: vec![LampCommand::off(TFL_L), LampCommand::on(TFL_R)],
                    duration_ms: 250,
                },
            ],
        },
        Effect {
            id: "sweep".into(),
            name: "Outside In".into(),
            description: "Light sweeps from the outer markers to the centre.".into(),
            looping: true,
            timing: Timing::Fixed,
            steps: vec![
                Step {
                    commands: vec![LampCommand::on(SML_L), LampCommand::on(SML_R)],
                    duration_ms: 120,
                },
                Step {
                    commands: vec![
                        LampCommand::off(SML_L),
                        LampCommand::off(SML_R),
                        LampCommand::on(DESIGN_L),
                        LampCommand::on(DESIGN_R),
                    ],
                    duration_ms: 120,
                },
                Step {
                    commands: vec![
                        LampCommand::off(DESIGN_L),
                        LampCommand::off(DESIGN_R),
                        LampCommand::on(RING_L),
                        LampCommand::on(RING_R),
                    ],
                    duration_ms: 120,
                },
                Step {
                    commands: vec![LampCommand::off(RING_L), LampCommand::off(RING_R)],
                    duration_ms: 120,
                },
            ],
        },
        Effect {
            id: "breathe".into(),
            name: "Breathe".into(),
            description: "Headlight rings fade up and down.".into(),
            looping: true,
            timing: Timing::Fixed,
            steps: (0..16)
                .map(|i| {
                    // Triangle wave: 0 up to 100 and back.
                    let level = if i < 8 { i * 14 } else { (15 - i) * 14 };
                    Step {
                        commands: vec![
                            LampCommand::at(RING_L, level.min(100) as u8),
                            LampCommand::at(RING_R, level.min(100) as u8),
                        ],
                        duration_ms: 90,
                    }
                })
                .collect(),
        },
        Effect {
            id: "beat-pulse".into(),
            name: "Beat Pulse".into(),
            description: "Rings pulse on every detected beat.".into(),
            looping: true,
            timing: Timing::PerBeat,
            steps: vec![
                Step {
                    commands: vec![LampCommand::on(RING_L), LampCommand::on(RING_R)],
                    duration_ms: 100,
                },
                Step {
                    commands: vec![LampCommand::off(RING_L), LampCommand::off(RING_R)],
                    duration_ms: 100,
                },
            ],
        },
        Effect {
            id: "beat-bounce".into(),
            name: "Beat Bounce".into(),
            description: "Alternates side to side, one move per beat.".into(),
            looping: true,
            timing: Timing::PerBeat,
            steps: vec![
                Step {
                    commands: vec![
                        LampCommand::on(RING_L),
                        LampCommand::on(POL_L),
                        LampCommand::off(RING_R),
                        LampCommand::off(POL_R),
                    ],
                    duration_ms: 150,
                },
                Step {
                    commands: vec![
                        LampCommand::off(RING_L),
                        LampCommand::off(POL_L),
                        LampCommand::on(RING_R),
                        LampCommand::on(POL_R),
                    ],
                    duration_ms: 150,
                },
            ],
        },
        Effect {
            id: "jdm-drift".into(),
            name: "JDM Drift".into(),
            description: "Classic street-drift strobe: snappy left/right rings, beams and fogs, with matching rear tails.".into(),
            looping: true,
            timing: Timing::Fixed,
            steps: {
                // Outer = low beam, rings/TFL = angel eyes, fog = bumper, SL = rear bars.
                const left_on: &[u8] = &[NSW_L, RING_L, TFL_L, 0x01, 0x07, SL_L, SL2_L];
                const right_on: &[u8] = &[NSW_R, RING_R, TFL_R, 0x02, 0x08, SL_R, SL2_R];
                let side = |on: &[u8], off: &[u8]| -> Vec<LampCommand> {
                    on.iter()
                        .map(|&l| LampCommand::on(l))
                        .chain(off.iter().map(|&l| LampCommand::off(l)))
                        .collect()
                };
                let both = |level: u8| -> Vec<LampCommand> {
                    left_on
                        .iter()
                        .chain(right_on.iter())
                        .map(|&l| LampCommand::at(l, level))
                        .collect()
                };
                vec![
                    Step {
                        commands: side(left_on, right_on),
                        duration_ms: 70,
                    },
                    Step {
                        commands: side(right_on, left_on),
                        duration_ms: 70,
                    },
                    Step {
                        commands: side(left_on, right_on),
                        duration_ms: 70,
                    },
                    Step {
                        commands: side(right_on, left_on),
                        duration_ms: 70,
                    },
                    Step {
                        commands: both(100),
                        duration_ms: 55,
                    },
                    Step {
                        commands: both(0),
                        duration_ms: 55,
                    },
                    Step {
                        commands: both(100),
                        duration_ms: 55,
                    },
                    Step {
                        commands: both(0),
                        duration_ms: 220,
                    },
                ]
            },
        },
        Effect {
            id: "fog-strobe".into(),
            name: "Fog Strobe".into(),
            description: "Rapid alternating fog lights. Needs a fast connection.".into(),
            looping: true,
            timing: Timing::Fixed,
            steps: vec![
                Step {
                    commands: vec![LampCommand::on(NSW_L), LampCommand::off(NSW_R)],
                    duration_ms: 60,
                },
                Step {
                    commands: vec![LampCommand::off(NSW_L), LampCommand::on(NSW_R)],
                    duration_ms: 60,
                },
            ],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_validates() {
        for effect in presets() {
            effect.validate().unwrap_or_else(|e| panic!("{e}"));
        }
    }

    #[test]
    fn no_preset_drives_a_safety_critical_lamp() {
        // Shipping a preset that flashes indicators or brake lights would let a
        // user imitate an emergency vehicle without ever making a choice.
        for effect in presets() {
            assert!(
                !effect.touches_safety_critical_lamps(),
                "preset '{}' drives a signalling device",
                effect.id
            );
        }
    }

    #[test]
    fn preset_ids_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for effect in presets() {
            assert!(seen.insert(effect.id.clone()), "duplicate id {}", effect.id);
        }
    }

    #[test]
    fn duration_sums_the_steps() {
        let effect = presets().into_iter().find(|e| e.id == "wig-wag").unwrap();
        assert_eq!(effect.duration_ms(), 400);
        assert_eq!(effect.shortest_step_ms(), 200);
    }

    #[test]
    fn lamp_list_is_sorted_and_deduplicated() {
        let effect = presets().into_iter().find(|e| e.id == "wig-wag").unwrap();
        assert_eq!(effect.lamps(), vec![RING_L, RING_R]);
    }

    #[test]
    fn breathe_ramps_up_and_back_down_within_range() {
        let effect = presets().into_iter().find(|e| e.id == "breathe").unwrap();
        let levels: Vec<u8> = effect
            .steps
            .iter()
            .map(|s| s.commands[0].level)
            .collect();
        assert_eq!(levels.first(), Some(&0));
        assert!(levels.iter().all(|&l| l <= 100));
        // It must come back down, or the lights would stay on at the end.
        assert!(levels.last() < levels.iter().max());
    }

    #[test]
    fn effect_with_no_steps_is_rejected() {
        let effect = Effect {
            id: "empty".into(),
            name: "Empty".into(),
            description: String::new(),
            steps: vec![],
            looping: true,
            timing: Timing::Fixed,
        };
        assert!(effect.validate().is_err());
    }

    #[test]
    fn zero_duration_step_is_rejected() {
        let effect = Effect {
            id: "instant".into(),
            name: "Instant".into(),
            description: String::new(),
            steps: vec![Step {
                commands: vec![LampCommand::on(RING_L)],
                duration_ms: 0,
            }],
            looping: true,
            timing: Timing::Fixed,
        };
        assert!(effect.validate().is_err());
    }

    #[test]
    fn undefined_lamp_is_rejected() {
        let effect = Effect {
            id: "bogus".into(),
            name: "Bogus".into(),
            description: String::new(),
            steps: vec![Step {
                // 0x0F is a gap in BMW's enumeration.
                commands: vec![LampCommand::on(0x0F)],
                duration_ms: 100,
            }],
            looping: true,
            timing: Timing::Fixed,
        };
        assert!(effect.validate().is_err());
    }

    #[test]
    fn level_above_one_hundred_is_rejected() {
        let effect = Effect {
            id: "toobright".into(),
            name: "Too Bright".into(),
            description: String::new(),
            steps: vec![Step {
                commands: vec![LampCommand::at(RING_L, 200)],
                duration_ms: 100,
            }],
            looping: true,
            timing: Timing::Fixed,
        };
        assert!(effect.validate().is_err());
    }

    #[test]
    fn safety_critical_detection_works() {
        let effect = Effect {
            id: "indicators".into(),
            name: "Indicators".into(),
            description: String::new(),
            // 0x0D is the front left turn signal.
            steps: vec![Step {
                commands: vec![LampCommand::on(0x0D)],
                duration_ms: 100,
            }],
            looping: true,
            timing: Timing::Fixed,
        };
        assert!(effect.touches_safety_critical_lamps());
    }

    #[test]
    fn effects_serialize_round_trip() {
        // Effects travel over IPC and get saved to disk, so this must hold.
        for effect in presets() {
            let json = serde_json::to_string(&effect).unwrap();
            let parsed: Effect = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, effect);
        }
    }
}
