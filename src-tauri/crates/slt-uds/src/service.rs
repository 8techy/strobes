//! UDS service identifiers, control parameters and negative response codes.
//!
//! Constants are named after the EDIABAS jobs they back where that helps, since
//! `docs/protocol-research.md` section 4 documents the mapping BMW itself uses.

/// UDS service identifiers.
pub mod sid {
    /// DiagnosticSessionControl.
    pub const DIAGNOSTIC_SESSION_CONTROL: u8 = 0x10;
    /// ECUReset.
    pub const ECU_RESET: u8 = 0x11;
    /// ClearDiagnosticInformation.
    pub const CLEAR_DIAGNOSTIC_INFORMATION: u8 = 0x14;
    /// ReadDTCInformation.
    pub const READ_DTC_INFORMATION: u8 = 0x19;
    /// ReadDataByIdentifier. EDIABAS `STATUS_LESEN`.
    pub const READ_DATA_BY_IDENTIFIER: u8 = 0x22;
    /// DynamicallyDefineDataIdentifier. EDIABAS `STATUS_BLOCK_LESEN`.
    pub const DYNAMICALLY_DEFINE_DATA_IDENTIFIER: u8 = 0x2C;
    /// WriteDataByIdentifier. EDIABAS `STEUERN`. Blocked by the safety guard.
    pub const WRITE_DATA_BY_IDENTIFIER: u8 = 0x2E;
    /// SecurityAccess. Required by some non-BMW body modules before actuation.
    pub const SECURITY_ACCESS: u8 = 0x27;
    /// InputOutputControlByIdentifier. EDIABAS `STEUERN_IO`. The actuation path.
    pub const IO_CONTROL_BY_IDENTIFIER: u8 = 0x2F;
    /// RoutineControl. EDIABAS `STEUERN_ROUTINE`.
    pub const ROUTINE_CONTROL: u8 = 0x31;
    /// TesterPresent.
    pub const TESTER_PRESENT: u8 = 0x3E;
    /// ControlDTCSetting.
    pub const CONTROL_DTC_SETTING: u8 = 0x85;

    /// A positive response echoes the request SID with this bit set.
    pub const POSITIVE_RESPONSE_OFFSET: u8 = 0x40;
    /// A negative response begins with this byte.
    pub const NEGATIVE_RESPONSE: u8 = 0x7F;
}

/// Diagnostic session types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Session {
    /// Normal operation.
    Default = 0x01,
    /// Precursor to flashing. Strobes refuses to enter this.
    Programming = 0x02,
    /// Required for actuation. Reverts automatically on timeout.
    Extended = 0x03,
}

impl Session {
    pub fn as_byte(self) -> u8 {
        self as u8
    }
}

/// `InputOutputControlByIdentifier` control parameters.
///
/// BMW's own mnemonics from `Anleitung_STATUS_STEUERN_UDS` are given because
/// they are what appears in Tool32 and in captured traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IoControl {
    /// `RCTECU`. Hands control back to the ECU. This is how effects are undone.
    ReturnControlToEcu = 0x00,
    /// `RTD`. Reset to the ECU's default state.
    ResetToDefault = 0x01,
    /// `FCS`. Hold the present state.
    FreezeCurrentState = 0x02,
    /// `STA`. Apply a value. The default when no parameter is given.
    ShortTermAdjustment = 0x03,
}

impl IoControl {
    pub fn as_byte(self) -> u8 {
        self as u8
    }

    /// Parses BMW's mnemonic spelling, as used in Tool32 job arguments.
    pub fn from_mnemonic(mnemonic: &str) -> Option<Self> {
        Some(match mnemonic.to_ascii_uppercase().as_str() {
            "RCTECU" => Self::ReturnControlToEcu,
            "RTD" => Self::ResetToDefault,
            "FCS" => Self::FreezeCurrentState,
            "STA" => Self::ShortTermAdjustment,
            _ => return None,
        })
    }
}

/// `RoutineControl` sub-functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoutineControl {
    /// `STR`.
    Start = 0x01,
    /// `STPR`.
    Stop = 0x02,
    /// `RRR`.
    RequestResults = 0x03,
}

impl RoutineControl {
    pub fn as_byte(self) -> u8 {
        self as u8
    }

    pub fn from_mnemonic(mnemonic: &str) -> Option<Self> {
        Some(match mnemonic.to_ascii_uppercase().as_str() {
            "STR" => Self::Start,
            "STPR" => Self::Stop,
            "RRR" => Self::RequestResults,
            _ => return None,
        })
    }
}

/// Well-known data identifiers.
pub mod did {
    /// Vehicle identification number.
    pub const VIN: u16 = 0xF190;
    /// ECU serial number.
    pub const ECU_SERIAL: u16 = 0xF18C;
    /// Active diagnostic session.
    pub const ACTIVE_SESSION: u16 = 0xF186;
    /// ECU manufacturing date.
    pub const MANUFACTURING_DATE: u16 = 0xF18B;
    /// ECU software version (BMW SVK).
    pub const SVK: u16 = 0xF101;
    /// ECU hardware number.
    pub const HARDWARE_NUMBER: u16 = 0xF191;
}

/// ISO 14229 negative response codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Nrc(pub u8);

impl Nrc {
    pub const GENERAL_REJECT: Self = Self(0x10);
    pub const SERVICE_NOT_SUPPORTED: Self = Self(0x11);
    pub const SUB_FUNCTION_NOT_SUPPORTED: Self = Self(0x12);
    pub const INCORRECT_MESSAGE_LENGTH: Self = Self(0x13);
    pub const CONDITIONS_NOT_CORRECT: Self = Self(0x22);
    pub const REQUEST_SEQUENCE_ERROR: Self = Self(0x24);
    pub const REQUEST_OUT_OF_RANGE: Self = Self(0x31);
    pub const SECURITY_ACCESS_DENIED: Self = Self(0x33);
    pub const INVALID_KEY: Self = Self(0x35);
    pub const RESPONSE_PENDING: Self = Self(0x78);
    pub const SUB_FUNCTION_NOT_SUPPORTED_IN_SESSION: Self = Self(0x7E);
    pub const SERVICE_NOT_SUPPORTED_IN_SESSION: Self = Self(0x7F);

    /// Whether the ECU is asking us to wait rather than reporting a failure.
    pub fn is_response_pending(self) -> bool {
        self == Self::RESPONSE_PENDING
    }

    /// Whether this code means "the identifier does not exist here", which is
    /// the expected answer during a discovery sweep rather than an error.
    pub fn is_not_found(self) -> bool {
        self == Self::REQUEST_OUT_OF_RANGE
    }

    /// A short technical name.
    pub fn name(self) -> &'static str {
        match self.0 {
            0x10 => "generalReject",
            0x11 => "serviceNotSupported",
            0x12 => "subFunctionNotSupported",
            0x13 => "incorrectMessageLengthOrInvalidFormat",
            0x14 => "responseTooLong",
            0x21 => "busyRepeatRequest",
            0x22 => "conditionsNotCorrect",
            0x24 => "requestSequenceError",
            0x25 => "noResponseFromSubnetComponent",
            0x26 => "failurePreventsExecutionOfRequestedAction",
            0x31 => "requestOutOfRange",
            0x33 => "securityAccessDenied",
            0x35 => "invalidKey",
            0x36 => "exceedNumberOfAttempts",
            0x37 => "requiredTimeDelayNotExpired",
            0x70 => "uploadDownloadNotAccepted",
            0x71 => "transferDataSuspended",
            0x72 => "generalProgrammingFailure",
            0x73 => "wrongBlockSequenceCounter",
            0x78 => "requestCorrectlyReceived-ResponsePending",
            0x7E => "subFunctionNotSupportedInActiveSession",
            0x7F => "serviceNotSupportedInActiveSession",
            0x81 => "rpmTooHigh",
            0x82 => "rpmTooLow",
            0x83 => "engineIsRunning",
            0x84 => "engineIsNotRunning",
            0x85 => "engineRunTimeTooLow",
            0x87 => "temperatureTooLow",
            0x88 => "vehicleSpeedTooHigh",
            0x89 => "vehicleSpeedTooLow",
            0x92 => "voltageTooHigh",
            0x93 => "voltageTooLow",
            _ => "unknown",
        }
    }

    /// A plain-language explanation aimed at the user rather than an engineer.
    ///
    /// The UI shows these directly, so they name the likely cause instead of
    /// restating the code.
    pub fn explanation(self) -> &'static str {
        match self.0 {
            0x11 => "This module does not support that operation. The catalog may name the wrong ECU.",
            0x12 => "The module rejected the control parameter for this identifier.",
            0x13 => "The request was the wrong length. The catalog's parameter encoding is probably wrong.",
            0x22 => "Conditions are not right. Check that the ignition is on.",
            0x24 => "Operations arrived out of order, for example a stop before a start.",
            0x31 => "This module has no such identifier.",
            0x33 => "The module requires a security unlock that Strobes does not perform.",
            0x78 => "The module needs more time and will answer shortly.",
            0x7E | 0x7F => "An extended diagnostic session is required for this operation.",
            0x83 => "The body module refused because the engine is running.",
            0x88 => "The body module refused because the vehicle reported motion.",
            0x92 => "Supply voltage is too high.",
            0x93 => "Supply voltage is too low. Connect a battery charger before running effects.",
            _ => "The module rejected the request.",
        }
    }
}

impl std::fmt::Display for Nrc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{:02X} {}", self.0, self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_control_bytes_match_iso_14229() {
        assert_eq!(IoControl::ReturnControlToEcu.as_byte(), 0x00);
        assert_eq!(IoControl::ResetToDefault.as_byte(), 0x01);
        assert_eq!(IoControl::FreezeCurrentState.as_byte(), 0x02);
        assert_eq!(IoControl::ShortTermAdjustment.as_byte(), 0x03);
    }

    #[test]
    fn bmw_mnemonics_map_to_control_parameters() {
        assert_eq!(
            IoControl::from_mnemonic("STA"),
            Some(IoControl::ShortTermAdjustment)
        );
        assert_eq!(
            IoControl::from_mnemonic("rctecu"),
            Some(IoControl::ReturnControlToEcu)
        );
        assert_eq!(IoControl::from_mnemonic("nonsense"), None);
    }

    #[test]
    fn routine_mnemonics_map_to_sub_functions() {
        assert_eq!(RoutineControl::from_mnemonic("STR"), Some(RoutineControl::Start));
        assert_eq!(RoutineControl::from_mnemonic("STPR"), Some(RoutineControl::Stop));
        assert_eq!(
            RoutineControl::from_mnemonic("RRR"),
            Some(RoutineControl::RequestResults)
        );
    }

    #[test]
    fn response_pending_is_not_treated_as_failure() {
        assert!(Nrc::RESPONSE_PENDING.is_response_pending());
        assert!(!Nrc::CONDITIONS_NOT_CORRECT.is_response_pending());
    }

    #[test]
    fn request_out_of_range_marks_a_missing_identifier() {
        assert!(Nrc::REQUEST_OUT_OF_RANGE.is_not_found());
        assert!(!Nrc::SECURITY_ACCESS_DENIED.is_not_found());
    }

    #[test]
    fn every_documented_nrc_has_an_explanation() {
        for code in [0x11, 0x12, 0x13, 0x22, 0x31, 0x33, 0x78, 0x7F, 0x93] {
            let nrc = Nrc(code);
            assert_ne!(nrc.name(), "unknown", "code 0x{code:02X} needs a name");
            assert!(!nrc.explanation().is_empty());
        }
    }
}
