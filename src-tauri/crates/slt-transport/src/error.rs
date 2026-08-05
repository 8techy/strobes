use std::io;

/// Errors surfaced by the transport layer.
///
/// Deliberately distinguishes protocol-level rejections (which carry diagnostic
/// meaning the UI should explain) from plain I/O failures.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),

    #[error("connection closed by peer")]
    Closed,

    #[error("timed out after {0:?}")]
    Timeout(std::time::Duration),

    #[error("frame is malformed: {0}")]
    MalformedFrame(String),

    #[error("frame payload of {actual} bytes exceeds the {max} byte limit")]
    FrameTooLarge { actual: usize, max: usize },

    /// The HSFZ gateway rejected the request with a diagnostic control word.
    #[error("gateway rejected request: {0}")]
    GatewayRejected(HsfzRejection),

    /// DoIP routing activation was denied. Per ISO 13400 the socket must be closed.
    #[error("routing activation denied: {0}")]
    RoutingActivationDenied(RoutingActivationDenial),

    #[error("DoIP negative acknowledge: {0}")]
    DoIpNack(DoIpNackCode),

    #[error("no vehicle responded to discovery on {0}")]
    NoVehicleFound(String),

    #[error("response arrived from ECU 0x{actual:04X} but 0x{expected:04X} was addressed")]
    UnexpectedResponder { expected: u16, actual: u16 },
}

/// HSFZ control words that indicate the gateway refused the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HsfzRejection {
    IncorrectTesterAddress { expected: u8, received: u8 },
    IncorrectControlWord,
    IncorrectFormat,
    IncorrectDestinationAddress,
    MessageTooLarge,
    DiagAppNotReady,
    OutOfMemory,
}

impl std::fmt::Display for HsfzRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IncorrectTesterAddress { expected, received } => write!(
                f,
                "incorrect tester address (gateway expected 0x{expected:02X}, got 0x{received:02X})"
            ),
            Self::IncorrectControlWord => f.write_str("incorrect control word"),
            Self::IncorrectFormat => f.write_str("incorrect format"),
            Self::IncorrectDestinationAddress => {
                f.write_str("incorrect destination address (no such ECU on this vehicle)")
            }
            Self::MessageTooLarge => f.write_str("message too large"),
            Self::DiagAppNotReady => {
                f.write_str("diagnostic application not ready (another tester may be connected)")
            }
            Self::OutOfMemory => f.write_str("gateway out of memory"),
        }
    }
}

/// ISO 13400 routing activation response codes that represent a denial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingActivationDenial {
    UnknownSourceAddress,
    AllSocketsRegistered,
    SourceAddressMismatch,
    SourceAddressAlreadyRegistered,
    MissingAuthentication,
    RejectedConfirmation,
    UnsupportedActivationType,
    Other(u8),
}

impl RoutingActivationDenial {
    pub fn from_code(code: u8) -> Self {
        match code {
            0x00 => Self::UnknownSourceAddress,
            0x01 => Self::AllSocketsRegistered,
            0x02 => Self::SourceAddressMismatch,
            0x03 => Self::SourceAddressAlreadyRegistered,
            0x04 => Self::MissingAuthentication,
            0x05 => Self::RejectedConfirmation,
            0x06 => Self::UnsupportedActivationType,
            other => Self::Other(other),
        }
    }
}

impl std::fmt::Display for RoutingActivationDenial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSourceAddress => f.write_str("unknown source address"),
            Self::AllSocketsRegistered => {
                f.write_str("all diagnostic sockets in use (another tester is connected)")
            }
            Self::SourceAddressMismatch => f.write_str("source address mismatch"),
            Self::SourceAddressAlreadyRegistered => f.write_str("source address already registered"),
            Self::MissingAuthentication => f.write_str("missing authentication"),
            Self::RejectedConfirmation => f.write_str("rejected confirmation"),
            Self::UnsupportedActivationType => f.write_str("unsupported activation type"),
            Self::Other(c) => write!(f, "unspecified denial (code 0x{c:02X})"),
        }
    }
}

/// ISO 13400 diagnostic message negative acknowledge codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoIpNackCode {
    InvalidSourceAddress,
    UnknownTargetAddress,
    DiagnosticMessageTooLarge,
    OutOfMemory,
    TargetUnreachable,
    UnknownNetwork,
    TransportProtocolError,
    Other(u8),
}

impl DoIpNackCode {
    pub fn from_code(code: u8) -> Self {
        match code {
            0x02 => Self::InvalidSourceAddress,
            0x03 => Self::UnknownTargetAddress,
            0x04 => Self::DiagnosticMessageTooLarge,
            0x05 => Self::OutOfMemory,
            0x06 => Self::TargetUnreachable,
            0x07 => Self::UnknownNetwork,
            0x08 => Self::TransportProtocolError,
            other => Self::Other(other),
        }
    }
}

impl std::fmt::Display for DoIpNackCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSourceAddress => f.write_str("invalid source address"),
            Self::UnknownTargetAddress => {
                f.write_str("unknown target address (no such ECU on this vehicle)")
            }
            Self::DiagnosticMessageTooLarge => f.write_str("diagnostic message too large"),
            Self::OutOfMemory => f.write_str("entity out of memory"),
            Self::TargetUnreachable => f.write_str("target unreachable"),
            Self::UnknownNetwork => f.write_str("unknown network"),
            Self::TransportProtocolError => f.write_str("transport protocol error"),
            Self::Other(c) => write!(f, "unspecified nack (code 0x{c:02X})"),
        }
    }
}

pub type Result<T> = std::result::Result<T, TransportError>;
