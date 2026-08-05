//! Diagnostic trouble code reading.
//!
//! Reading DTCs before and after a session is how the user finds out whether an
//! effect upset the vehicle's lamp monitoring, so this is a first-class feature
//! rather than a diagnostic afterthought.

/// A stored diagnostic trouble code.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Dtc {
    /// 3-byte code as reported by the ECU.
    pub code: u32,
    /// BMW's conventional hexadecimal rendering, for example `0x8040B8`.
    pub code_hex: String,
    /// Raw status mask byte.
    pub status: u8,
    /// Whether the fault is present right now.
    pub confirmed: bool,
    /// Whether the fault was seen during the current ignition cycle.
    pub pending: bool,
    /// Whether the warning indicator was requested.
    pub warning_indicator: bool,
}

impl Dtc {
    fn new(code: u32, status: u8) -> Self {
        Self {
            code,
            code_hex: format!("0x{code:06X}"),
            status,
            // Bit assignments per ISO 14229 DTC status mask.
            confirmed: status & 0x08 != 0,
            pending: status & 0x04 != 0,
            warning_indicator: status & 0x80 != 0,
        }
    }
}

/// Status mask selecting every DTC the ECU knows about.
pub const STATUS_MASK_ALL: u8 = 0xFF;
/// Status mask selecting confirmed faults only.
pub const STATUS_MASK_CONFIRMED: u8 = 0x08;

/// Parses a `ReadDTCInformation` sub-function 0x02 response body.
///
/// The response is `59 02 <availability_mask> [dtc_hi dtc_mid dtc_lo status]*`.
/// The caller passes everything after the `59 02`.
pub fn parse_dtc_report(body: &[u8]) -> Vec<Dtc> {
    // First byte is the status availability mask, not part of any record.
    let records = body.get(1..).unwrap_or_default();
    records
        .chunks_exact(4)
        .map(|chunk| {
            let code = u32::from_be_bytes([0, chunk[0], chunk[1], chunk[2]]);
            Dtc::new(code, chunk[3])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_records() {
        // availability mask, then two 4-byte records
        let body = [0xFF, 0x80, 0x40, 0xB8, 0x08, 0x9C, 0xBC, 0x00, 0x04];
        let dtcs = parse_dtc_report(&body);

        assert_eq!(dtcs.len(), 2);
        assert_eq!(dtcs[0].code_hex, "0x8040B8");
        assert!(dtcs[0].confirmed);
        assert!(!dtcs[0].pending);
        assert_eq!(dtcs[1].code_hex, "0x9CBC00");
        assert!(dtcs[1].pending);
        assert!(!dtcs[1].confirmed);
    }

    #[test]
    fn empty_report_yields_no_codes() {
        assert!(parse_dtc_report(&[0xFF]).is_empty());
        assert!(parse_dtc_report(&[]).is_empty());
    }

    #[test]
    fn trailing_partial_record_is_ignored() {
        // A truncated record must not panic or produce a bogus code.
        let body = [0xFF, 0x80, 0x40, 0xB8, 0x08, 0x9C, 0xBC];
        assert_eq!(parse_dtc_report(&body).len(), 1);
    }

    #[test]
    fn warning_indicator_bit_is_decoded() {
        let dtcs = parse_dtc_report(&[0xFF, 0x00, 0x00, 0x01, 0x88]);
        assert!(dtcs[0].warning_indicator);
        assert!(dtcs[0].confirmed);
    }
}
