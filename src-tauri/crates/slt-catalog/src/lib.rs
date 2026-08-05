//! Chassis catalogs: the per-vehicle data that turns "switch on the left
//! headlight ring" into concrete UDS bytes.
//!
//! The schema mirrors BMW's own `SG_Funktionen` table (see
//! `docs/protocol-research.md` section 4.1) because that table is what actually
//! defines the mapping, including the scaling and byte-order fields that a naive
//! "just send this byte" design would get wrong.
//!
//! Strobelight ships **no** BMW-derived identifiers. Catalog files are authored
//! by users from software they license. Entries whose identifiers have not been
//! verified on a real vehicle are marked `verified = false` and the engine
//! refuses to transmit them unless research mode is explicitly enabled.

pub mod lamp;

use std::collections::HashMap;
use std::path::Path;

use slt_transport::{EcuAddress, Protocol};
use slt_uds::{IoControl, RoutineControl};

pub use lamp::{Lamp, LampGroup};

/// Catalog errors.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("could not read catalog {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("could not parse catalog {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },

    #[error("unsupported schema version {found}, this build understands {supported}")]
    UnsupportedSchema { found: u32, supported: u32 },

    #[error("action '{action}' references ECU '{ecu}', which the catalog does not define")]
    UnknownEcu { action: String, ecu: String },

    #[error("action '{action}' uses service 0x{service:02X}; only 0x2F and 0x31 are supported")]
    UnsupportedService { action: String, service: u8 },

    #[error("action '{0}' has no parameters, so there is nothing to actuate")]
    NoParameters(String),

    #[error("duplicate action id '{0}'")]
    DuplicateAction(String),

    #[error("parameter '{param}' of action '{action}' has div = 0, which cannot be applied")]
    ZeroDivisor { action: String, param: String },

    #[error("value {value} for parameter '{param}' is outside the allowed range {min}..={max}")]
    ValueOutOfRange {
        param: String,
        value: f64,
        min: f64,
        max: f64,
    },

    #[error("action '{action}' expects a value for parameter '{param}'")]
    MissingParameter { action: String, param: String },

    #[error("no action named '{0}' in this catalog")]
    UnknownAction(String),
}

pub type Result<T> = std::result::Result<T, CatalogError>;

/// Schema version this build understands.
pub const SCHEMA_VERSION: u32 = 1;

/// Parameter width, matching `SG_Funktionen`'s `DATENTYP` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DataType {
    #[default]
    Char,
    Int,
    Long,
}

impl DataType {
    pub fn width(self) -> usize {
        match self {
            Self::Char => 1,
            Self::Int => 2,
            Self::Long => 4,
        }
    }

    /// The inclusive unsigned range this width can hold.
    fn raw_bounds(self) -> (i64, i64) {
        match self {
            Self::Char => (0, 0xFF),
            Self::Int => (0, 0xFFFF),
            Self::Long => (0, 0xFFFF_FFFF),
        }
    }
}

/// Byte order, matching `SG_Funktionen`'s `L/H` column where `h` and `-` mean
/// high byte first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ByteOrder {
    #[default]
    High,
    Low,
}

/// One request parameter.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Param {
    pub name: String,
    #[serde(default)]
    pub datatype: DataType,
    #[serde(default)]
    pub byte_order: ByteOrder,
    /// Scaling, applied as `physical = raw * mul / div + add`. Encoding inverts it.
    #[serde(default = "one")]
    pub mul: f64,
    #[serde(default = "one")]
    pub div: f64,
    #[serde(default)]
    pub add: f64,
    /// Inclusive bounds on the physical value.
    pub min: Option<f64>,
    pub max: Option<f64>,
    #[serde(default)]
    pub info: String,
}

fn one() -> f64 {
    1.0
}

impl Param {
    /// Converts a physical value to raw bytes.
    ///
    /// Inverts the `SG_Funktionen` scaling: the table describes how to turn a
    /// raw ECU value into a physical one, so encoding runs it backwards.
    pub fn encode(&self, action_id: &str, value: f64) -> Result<Vec<u8>> {
        if let (Some(min), Some(max)) = (self.min, self.max) {
            if value < min || value > max {
                return Err(CatalogError::ValueOutOfRange {
                    param: self.name.clone(),
                    value,
                    min,
                    max,
                });
            }
        }
        if self.div == 0.0 {
            return Err(CatalogError::ZeroDivisor {
                action: action_id.to_string(),
                param: self.name.clone(),
            });
        }
        if self.mul == 0.0 {
            return Err(CatalogError::ZeroDivisor {
                action: action_id.to_string(),
                param: self.name.clone(),
            });
        }

        let raw = (value - self.add) * self.div / self.mul;
        let (lo, hi) = self.datatype.raw_bounds();
        let clamped = (raw.round() as i64).clamp(lo, hi) as u64;

        let bytes = clamped.to_be_bytes();
        let width = self.datatype.width();
        // Take the least significant `width` bytes, then orient them.
        let mut out = bytes[8 - width..].to_vec();
        if self.byte_order == ByteOrder::Low {
            out.reverse();
        }
        Ok(out)
    }

    /// Converts raw bytes back to a physical value, for decoding responses.
    pub fn decode(&self, bytes: &[u8]) -> f64 {
        let mut ordered = bytes.to_vec();
        if self.byte_order == ByteOrder::Low {
            ordered.reverse();
        }
        let raw = ordered
            .iter()
            .fold(0u64, |acc, &b| (acc << 8) | u64::from(b));
        raw as f64 * self.mul / self.div + self.add
    }
}

/// An ECU the catalog knows about.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct EcuDef {
    pub name: String,
    /// Diagnostic address. TOML integers accept `0x40` notation.
    pub address: u16,
    #[serde(default)]
    pub info: String,
}

impl EcuDef {
    pub fn address(&self) -> EcuAddress {
        EcuAddress::new(self.address)
    }
}

/// A UDS operation template.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Action {
    pub id: String,
    /// Name of an ECU defined in the same catalog.
    pub ecu: String,
    /// `0x2F` (InputOutputControlByIdentifier) or `0x31` (RoutineControl).
    pub service: u8,
    /// DID for `0x2F`, RID for `0x31`.
    pub identifier: u16,
    /// Session required before this action works. Almost always `0x03`.
    #[serde(default = "extended_session")]
    pub session: u8,
    /// Control parameter that applies a value. `STA` (0x03) or `STR` (0x01).
    #[serde(default = "default_actuate")]
    pub control_actuate: u8,
    /// Control parameter that reverts. `RCTECU` (0x00) or `STPR` (0x02).
    #[serde(default = "default_release")]
    pub control_release: u8,
    /// Minimum time this output must hold a state, protecting lamp monitoring.
    #[serde(default = "default_dwell")]
    pub min_dwell_ms: u64,
    /// Whether the identifier has been confirmed on a real vehicle.
    ///
    /// False means the value is a placeholder. The engine refuses to transmit
    /// unverified actions outside research mode.
    #[serde(default)]
    pub verified: bool,
    #[serde(default, rename = "param")]
    pub params: Vec<Param>,
    #[serde(default)]
    pub info: String,
}

fn extended_session() -> u8 {
    0x03
}
fn default_actuate() -> u8 {
    IoControl::ShortTermAdjustment as u8
}
fn default_release() -> u8 {
    IoControl::ReturnControlToEcu as u8
}
fn default_dwell() -> u64 {
    40
}

impl Action {
    /// Builds the UDS request that applies `values`.
    pub fn encode_actuate(&self, values: &HashMap<String, f64>) -> Result<Vec<u8>> {
        let mut payload = self.header(self.control_actuate);
        for param in &self.params {
            let value = values.get(&param.name).copied().ok_or_else(|| {
                CatalogError::MissingParameter {
                    action: self.id.clone(),
                    param: param.name.clone(),
                }
            })?;
            payload.extend(param.encode(&self.id, value)?);
        }
        Ok(payload)
    }

    /// Builds the UDS request that hands the output back to the ECU.
    ///
    /// Release carries no parameters: `ReturnControlToECU` and `StopRoutine` both
    /// take only the identifier.
    pub fn encode_release(&self) -> Vec<u8> {
        self.header(self.control_release)
    }

    /// Builds the service, identifier and control-parameter prefix.
    ///
    /// The byte order differs between the two services: `0x2F` puts the
    /// identifier before the control parameter, `0x31` puts the sub-function
    /// first. Getting this backwards yields `incorrectMessageLength`.
    fn header(&self, control: u8) -> Vec<u8> {
        let [hi, lo] = self.identifier.to_be_bytes();
        match self.service {
            slt_uds::sid::ROUTINE_CONTROL => vec![self.service, control, hi, lo],
            _ => vec![self.service, hi, lo, control],
        }
    }

    pub fn io_control_actuate(&self) -> Option<IoControl> {
        match self.control_actuate {
            0x00 => Some(IoControl::ReturnControlToEcu),
            0x01 => Some(IoControl::ResetToDefault),
            0x02 => Some(IoControl::FreezeCurrentState),
            0x03 => Some(IoControl::ShortTermAdjustment),
            _ => None,
        }
    }

    pub fn routine_control_actuate(&self) -> Option<RoutineControl> {
        match self.control_actuate {
            0x01 => Some(RoutineControl::Start),
            0x02 => Some(RoutineControl::Stop),
            0x03 => Some(RoutineControl::RequestResults),
            _ => None,
        }
    }
}

/// Chassis metadata.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Chassis {
    pub id: String,
    pub name: String,
    /// Wire protocol this chassis generation uses.
    pub transport: Protocol,
    #[serde(default)]
    pub notes: String,
}

/// Which lamps a given chassis actually has.
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct LampAvailability {
    /// Lamp codes present on this chassis. Empty means "assume all".
    #[serde(default)]
    pub available: Vec<String>,
}

/// A parsed chassis catalog.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Catalog {
    #[serde(default = "default_schema")]
    pub schema_version: u32,
    pub chassis: Chassis,
    #[serde(default, rename = "ecu")]
    pub ecus: Vec<EcuDef>,
    #[serde(default, rename = "action")]
    pub actions: Vec<Action>,
    #[serde(default)]
    pub lamps: LampAvailability,
}

fn default_schema() -> u32 {
    SCHEMA_VERSION
}

impl Catalog {
    /// Parses and validates a catalog from TOML text.
    pub fn from_toml(path: &str, text: &str) -> Result<Self> {
        let catalog: Catalog = toml::from_str(text).map_err(|source| CatalogError::Parse {
            path: path.to_string(),
            source,
        })?;
        catalog.validate()?;
        Ok(catalog)
    }

    /// Loads and validates a catalog from disk.
    pub fn load(path: &Path) -> Result<Self> {
        let display = path.display().to_string();
        let text = std::fs::read_to_string(path).map_err(|source| CatalogError::Read {
            path: display.clone(),
            source,
        })?;
        Self::from_toml(&display, &text)
    }

    /// Rejects internally inconsistent catalogs.
    ///
    /// Runs at load time so a malformed file fails immediately with a clear
    /// message rather than producing a confusing `incorrectMessageLength` from
    /// the car later.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(CatalogError::UnsupportedSchema {
                found: self.schema_version,
                supported: SCHEMA_VERSION,
            });
        }

        let mut seen = std::collections::HashSet::new();
        for action in &self.actions {
            if !seen.insert(&action.id) {
                return Err(CatalogError::DuplicateAction(action.id.clone()));
            }
            if !self.ecus.iter().any(|e| e.name == action.ecu) {
                return Err(CatalogError::UnknownEcu {
                    action: action.id.clone(),
                    ecu: action.ecu.clone(),
                });
            }
            if !matches!(
                action.service,
                slt_uds::sid::IO_CONTROL_BY_IDENTIFIER | slt_uds::sid::ROUTINE_CONTROL
            ) {
                return Err(CatalogError::UnsupportedService {
                    action: action.id.clone(),
                    service: action.service,
                });
            }
            if action.params.is_empty() {
                return Err(CatalogError::NoParameters(action.id.clone()));
            }
            for param in &action.params {
                if param.div == 0.0 || param.mul == 0.0 {
                    return Err(CatalogError::ZeroDivisor {
                        action: action.id.clone(),
                        param: param.name.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    pub fn action(&self, id: &str) -> Result<&Action> {
        self.actions
            .iter()
            .find(|a| a.id == id)
            .ok_or_else(|| CatalogError::UnknownAction(id.to_string()))
    }

    pub fn ecu(&self, name: &str) -> Option<&EcuDef> {
        self.ecus.iter().find(|e| e.name == name)
    }

    /// Resolves the ECU address an action targets.
    pub fn action_address(&self, action: &Action) -> Result<EcuAddress> {
        self.ecu(&action.ecu)
            .map(EcuDef::address)
            .ok_or_else(|| CatalogError::UnknownEcu {
                action: action.id.clone(),
                ecu: action.ecu.clone(),
            })
    }

    /// Whether every action carries a verified identifier.
    pub fn fully_verified(&self) -> bool {
        !self.actions.is_empty() && self.actions.iter().all(|a| a.verified)
    }

    /// Lamps this chassis supports, in enumeration order.
    ///
    /// An empty availability list means the catalog author has not narrowed it
    /// down, so every known lamp is offered.
    pub fn lamps(&self) -> Vec<&'static Lamp> {
        if self.lamps.available.is_empty() {
            return lamp::ALL.iter().collect();
        }
        self.lamps
            .available
            .iter()
            .filter_map(|code| lamp::by_code(code))
            .collect()
    }

    /// The longest `min_dwell_ms` of any action, which is the fastest step the
    /// scheduler can honour for this chassis.
    pub fn min_step_ms(&self) -> u64 {
        self.actions
            .iter()
            .map(|a| a.min_dwell_ms)
            .max()
            .unwrap_or(default_dwell())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
schema_version = 1

[chassis]
id = "F3x"
name = "BMW F30"
transport = "hsfz"

[[ecu]]
name = "FEM_BODY"
address = 0x40

[[action]]
id = "fem.lamp.set"
ecu = "FEM_BODY"
service = 0x2F
identifier = 0xD000
verified = true
min_dwell_ms = 40

  [[action.param]]
  name = "lamp"
  datatype = "char"

  [[action.param]]
  name = "level"
  datatype = "char"
  min = 0.0
  max = 100.0
"#;

    fn sample() -> Catalog {
        Catalog::from_toml("sample.toml", SAMPLE).unwrap()
    }

    #[test]
    fn parses_a_valid_catalog() {
        let catalog = sample();
        assert_eq!(catalog.chassis.id, "F3x");
        assert_eq!(catalog.chassis.transport, Protocol::Hsfz);
        assert_eq!(catalog.ecus[0].address(), EcuAddress::new(0x40));
        assert!(catalog.fully_verified());
    }

    #[test]
    fn io_control_request_puts_identifier_before_control_byte() {
        let catalog = sample();
        let action = catalog.action("fem.lamp.set").unwrap();

        let mut values = HashMap::new();
        values.insert("lamp".to_string(), 0x30.into());
        values.insert("level".to_string(), 100.0);

        let payload = action.encode_actuate(&values).unwrap();
        // 2F D0 00 03 <lamp> <level>
        assert_eq!(payload, vec![0x2F, 0xD0, 0x00, 0x03, 0x30, 100]);
    }

    #[test]
    fn release_request_uses_return_control_to_ecu() {
        let catalog = sample();
        let action = catalog.action("fem.lamp.set").unwrap();
        assert_eq!(action.encode_release(), vec![0x2F, 0xD0, 0x00, 0x00]);
    }

    #[test]
    fn routine_control_puts_sub_function_first() {
        let text = r#"
schema_version = 1
[chassis]
id = "G2x"
name = "BMW G20"
transport = "doip"
[[ecu]]
name = "BDC"
address = 0x40
[[action]]
id = "bdc.routine"
ecu = "BDC"
service = 0x31
identifier = 0xA001
control_actuate = 0x01
control_release = 0x02
  [[action.param]]
  name = "lamp"
"#;
        let catalog = Catalog::from_toml("g.toml", text).unwrap();
        let action = catalog.action("bdc.routine").unwrap();

        let mut values = HashMap::new();
        values.insert("lamp".to_string(), 3.0);

        // 31 01 A0 01 03
        assert_eq!(
            action.encode_actuate(&values).unwrap(),
            vec![0x31, 0x01, 0xA0, 0x01, 0x03]
        );
        assert_eq!(action.encode_release(), vec![0x31, 0x02, 0xA0, 0x01]);
    }

    #[test]
    fn scaling_is_inverted_when_encoding() {
        // physical = raw * 0.75 + 1, so raw = (physical - 1) / 0.75
        let param = Param {
            name: "temp".into(),
            datatype: DataType::Char,
            byte_order: ByteOrder::High,
            mul: 0.75,
            div: 1.0,
            add: 1.0,
            min: None,
            max: None,
            info: String::new(),
        };
        // The worked example from BMW's specification: raw 32 reads as 25.
        assert_eq!(param.decode(&[32]), 25.0);
        assert_eq!(param.encode("a", 25.0).unwrap(), vec![32]);
    }

    #[test]
    fn encoding_and_decoding_round_trip() {
        let param = Param {
            name: "rpm".into(),
            datatype: DataType::Int,
            byte_order: ByteOrder::High,
            mul: 1.0,
            div: 1.0,
            add: 0.0,
            min: Some(0.0),
            max: Some(8000.0),
            info: String::new(),
        };
        let encoded = param.encode("a", 2500.0).unwrap();
        assert_eq!(encoded, vec![0x09, 0xC4]);
        assert_eq!(param.decode(&encoded), 2500.0);
    }

    #[test]
    fn low_byte_order_reverses_the_bytes() {
        let param = Param {
            name: "v".into(),
            datatype: DataType::Int,
            byte_order: ByteOrder::Low,
            mul: 1.0,
            div: 1.0,
            add: 0.0,
            min: None,
            max: None,
            info: String::new(),
        };
        assert_eq!(param.encode("a", 2500.0).unwrap(), vec![0xC4, 0x09]);
        assert_eq!(param.decode(&[0xC4, 0x09]), 2500.0);
    }

    #[test]
    fn out_of_range_values_are_rejected() {
        let catalog = sample();
        let action = catalog.action("fem.lamp.set").unwrap();
        let mut values = HashMap::new();
        values.insert("lamp".to_string(), 0x30.into());
        values.insert("level".to_string(), 500.0);

        assert!(matches!(
            action.encode_actuate(&values),
            Err(CatalogError::ValueOutOfRange { .. })
        ));
    }

    #[test]
    fn missing_parameter_is_reported_by_name() {
        let catalog = sample();
        let action = catalog.action("fem.lamp.set").unwrap();
        let values = HashMap::new();

        match action.encode_actuate(&values) {
            Err(CatalogError::MissingParameter { param, .. }) => assert_eq!(param, "lamp"),
            other => panic!("expected a missing parameter error, got {other:?}"),
        }
    }

    #[test]
    fn unknown_ecu_reference_fails_validation() {
        let text = SAMPLE.replace(r#"ecu = "FEM_BODY""#, r#"ecu = "NOPE""#);
        // The [[ecu]] name line is also replaced, so re-break it deliberately.
        let text = text.replacen(r#"name = "NOPE""#, r#"name = "FEM_BODY""#, 1);
        assert!(matches!(
            Catalog::from_toml("bad.toml", &text),
            Err(CatalogError::UnknownEcu { .. })
        ));
    }

    #[test]
    fn unsupported_service_fails_validation() {
        let text = SAMPLE.replace("service = 0x2F", "service = 0x2E");
        assert!(matches!(
            Catalog::from_toml("bad.toml", &text),
            Err(CatalogError::UnsupportedService { service: 0x2E, .. })
        ));
    }

    #[test]
    fn future_schema_version_is_refused() {
        let text = SAMPLE.replace("schema_version = 1", "schema_version = 99");
        assert!(matches!(
            Catalog::from_toml("bad.toml", &text),
            Err(CatalogError::UnsupportedSchema { found: 99, .. })
        ));
    }

    #[test]
    fn action_without_parameters_fails_validation() {
        let text = r#"
schema_version = 1
[chassis]
id = "F3x"
name = "BMW F30"
transport = "hsfz"
[[ecu]]
name = "FEM_BODY"
address = 0x40
[[action]]
id = "empty"
ecu = "FEM_BODY"
service = 0x2F
identifier = 0xD000
"#;
        assert!(matches!(
            Catalog::from_toml("bad.toml", text),
            Err(CatalogError::NoParameters(_))
        ));
    }

    #[test]
    fn duplicate_action_ids_fail_validation() {
        let doubled = format!(
            "{SAMPLE}\n[[action]]\nid = \"fem.lamp.set\"\necu = \"FEM_BODY\"\nservice = 0x2F\nidentifier = 0xD001\n\n  [[action.param]]\n  name = \"lamp\"\n"
        );
        assert!(matches!(
            Catalog::from_toml("bad.toml", &doubled),
            Err(CatalogError::DuplicateAction(_))
        ));
    }

    #[test]
    fn unverified_catalog_is_flagged() {
        let text = SAMPLE.replace("verified = true", "verified = false");
        let catalog = Catalog::from_toml("unverified.toml", &text).unwrap();
        assert!(!catalog.fully_verified());
    }

    #[test]
    fn min_step_follows_the_slowest_action() {
        let catalog = sample();
        assert_eq!(catalog.min_step_ms(), 40);
    }

    #[test]
    fn empty_availability_offers_every_lamp() {
        let catalog = sample();
        assert_eq!(catalog.lamps().len(), lamp::ALL.len());
    }

    #[test]
    fn availability_list_narrows_the_lamp_set() {
        let text = format!("{SAMPLE}\n[lamps]\navailable = [\"TFL_L\", \"TFL_R\"]\n");
        let catalog = Catalog::from_toml("narrow.toml", &text).unwrap();
        let lamps = catalog.lamps();
        assert_eq!(lamps.len(), 2);
        assert_eq!(lamps[0].code, "TFL_L");
    }
}
