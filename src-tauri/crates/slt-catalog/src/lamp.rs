//! The BMW lamp index enumeration.
//!
//! Transcribed from the `LAMPNRTEXTE` / `TAB_AUSGANG_LEUCHTEN` table published
//! by the BMW community. The same numbering is shared across `FEM_20`, `BDC`,
//! `BDC_G05` and `BDC_G11`, which is what lets one table serve both F and G
//! series. See `docs/protocol-research.md` section 3.
//!
//! This is an index of *outputs*, not a set of commands: knowing that
//! `TMS_LEUCHTRING_L` is `0x30` says nothing about how to switch it on. That
//! part lives in the chassis catalog.

/// Where on the car a lamp sits, used to group the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LampGroup {
    Headlight,
    Indicator,
    Fog,
    Rear,
    Interior,
    Indicator2,
}

impl LampGroup {
    pub fn label(self) -> &'static str {
        match self {
            Self::Headlight => "Headlights",
            Self::Indicator => "Turn signals",
            Self::Fog => "Fog lights",
            Self::Rear => "Rear lights",
            Self::Interior => "Interior and accent",
            Self::Indicator2 => "Dashboard indicators",
        }
    }
}

/// A single addressable lamp output.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Lamp {
    /// The `LAMPNR` value passed to lighting jobs.
    pub id: u8,
    /// BMW's code, for example `TMS_LEUCHTRING_L`.
    pub code: &'static str,
    /// Plain-English name shown in the UI.
    pub name: &'static str,
    pub group: LampGroup,
    /// Whether this output is a legally-required signalling device.
    ///
    /// Effects touching these are gated behind an extra confirmation, because
    /// misusing brake lights or indicators is both dangerous and illegal.
    pub safety_critical: bool,
    /// Whether to offer this lamp prominently. Decorative outputs make the best
    /// light show material and carry the least risk.
    pub featured: bool,
}

macro_rules! lamps {
    ($(($id:expr, $code:literal, $name:literal, $group:ident, $critical:literal, $featured:literal)),* $(,)?) => {
        pub const ALL: &[Lamp] = &[
            $(Lamp {
                id: $id,
                code: $code,
                name: $name,
                group: LampGroup::$group,
                safety_critical: $critical,
                featured: $featured,
            }),*
        ];
    };
}

lamps![
    (0x01, "AL_L", "Low beam, left", Headlight, false, false),
    (0x02, "AL_R", "Low beam, right", Headlight, false, false),
    (0x03, "TFL_L", "Daytime running light, left", Headlight, false, true),
    (0x04, "TFL_R", "Daytime running light, right", Headlight, false, true),
    (0x05, "SML_L", "Side marker, left", Headlight, false, true),
    (0x06, "SML_R", "Side marker, right", Headlight, false, true),
    (0x07, "FL_L", "High beam, left", Headlight, false, false),
    (0x08, "FL_R", "High beam, right", Headlight, false, false),
    (0x09, "POL_L", "Position light, left", Headlight, false, true),
    (0x0A, "POL_R", "Position light, right", Headlight, false, true),
    (0x0B, "NSW_L", "Fog light, left", Fog, false, true),
    (0x0C, "NSW_R", "Fog light, right", Fog, false, true),
    (0x0D, "FRA_V_L", "Front turn signal, left", Indicator, true, false),
    (0x0E, "FRA_V_R", "Front turn signal, right", Indicator, true, false),
    (0x10, "FRA_Z_L", "Side repeater, left", Indicator, true, false),
    (0x11, "FRA_Z_R", "Side repeater, right", Indicator, true, false),
    (0x12, "BIX_L", "Bi-xenon solenoid, left", Headlight, false, false),
    (0x13, "BIX_R", "Bi-xenon solenoid, right", Headlight, false, false),
    (0x14, "SL_L", "Tail light, left", Rear, false, true),
    (0x15, "SL_R", "Tail light, right", Rear, false, true),
    (0x16, "SL_2_L", "Tail light 2, left", Rear, false, true),
    (0x17, "SL_2_R", "Tail light 2, right", Rear, false, true),
    (0x18, "BL_L", "Brake light, left", Rear, true, false),
    (0x19, "BL_R", "Brake light, right", Rear, true, false),
    (0x1A, "BFD_L", "Brake force display, left", Rear, true, false),
    (0x1B, "BFD_R", "Brake force display, right", Rear, true, false),
    (0x1C, "NSL_L", "Rear fog light, left", Rear, false, false),
    (0x1D, "NSL_R", "Rear fog light, right", Rear, false, false),
    (0x1E, "RFS_L", "Reverse light, left", Rear, true, false),
    (0x1F, "RFS_R", "Reverse light, right", Rear, true, false),
    (0x20, "FRA_H_L", "Rear turn signal, left", Indicator, true, false),
    (0x21, "FRA_H_R", "Rear turn signal, right", Indicator, true, false),
    (0x22, "KZL", "License plate light", Rear, false, true),
    (0x23, "BL_M", "Centre brake light", Rear, true, false),
    (0x26, "WBL_LED", "Hazard warning indicator", Indicator2, true, false),
    (0x27, "LCI_0", "Interior channel 0", Interior, false, true),
    (0x28, "LCI_1", "Interior channel 1", Interior, false, true),
    (0x29, "LCI_2", "Interior channel 2", Interior, false, true),
    (0x2A, "LCI_3", "Interior channel 3", Interior, false, true),
    (0x2B, "LCI_4", "Interior channel 4", Interior, false, true),
    (0x2C, "LCI_5", "Interior channel 5", Interior, false, true),
    (0x2D, "LCI_6", "Interior channel 6", Interior, false, true),
    (0x2E, "LCI_7", "Interior channel 7", Interior, false, true),
    (0x2F, "LCI_8", "Interior channel 8", Interior, false, true),
    (0x30, "TMS_LEUCHTRING_L", "Headlight ring, left", Headlight, false, true),
    (0x31, "TMS_LEUCHTRING_R", "Headlight ring, right", Headlight, false, true),
    (0x32, "TMS_SML_L", "Headlight side marker, left", Headlight, false, true),
    (0x33, "TMS_SML_R", "Headlight side marker, right", Headlight, false, true),
    (0x34, "TMS_DESIGN_L", "Headlight accent, left", Headlight, false, true),
    (0x35, "TMS_DESIGN_R", "Headlight accent, right", Headlight, false, true),
    (0x36, "LC_R0", "Rear light carpet 0", Rear, false, true),
    (0x37, "LC_R1", "Rear light carpet 1", Rear, false, true),
    (0x38, "LC_R2", "Rear light carpet 2", Rear, false, true),
    (0x39, "LC_R3", "Rear light carpet 3", Rear, false, true),
    (0x40, "PDC_LED", "Park distance control indicator", Indicator2, false, false),
    (0x41, "HDC_LED", "Hill descent indicator", Indicator2, false, false),
    (0x42, "SPECIAL_FUNCTION_1_LED", "Special function indicator", Indicator2, false, false),
    (0x43, "MSA_LED", "Auto start/stop indicator", Indicator2, false, false),
    (0x44, "SST_LED", "Sport setting indicator", Indicator2, false, false),
];

/// Applies an operation to every lamp the module supports.
pub const ALL_LAMPS: u8 = 0xFE;
/// Marks an undefined lamp slot.
pub const INVALID: u8 = 0xFF;

/// Looks up a lamp by its `LAMPNR`.
pub fn by_id(id: u8) -> Option<&'static Lamp> {
    ALL.iter().find(|lamp| lamp.id == id)
}

/// Looks up a lamp by BMW's code, case-insensitively.
pub fn by_code(code: &str) -> Option<&'static Lamp> {
    ALL.iter()
        .find(|lamp| lamp.code.eq_ignore_ascii_case(code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_lamps_resolve_by_id() {
        assert_eq!(by_id(0x01).unwrap().code, "AL_L");
        assert_eq!(by_id(0x30).unwrap().code, "TMS_LEUCHTRING_L");
        assert_eq!(by_id(0x44).unwrap().code, "SST_LED");
    }

    #[test]
    fn gaps_in_the_enumeration_are_absent() {
        // 0x0F, 0x24, 0x25 and 0x3A-0x3F are not defined by BMW.
        for id in [0x0F, 0x24, 0x25, 0x3A, 0x3F] {
            assert!(by_id(id).is_none(), "0x{id:02X} should not be defined");
        }
    }

    #[test]
    fn reserved_values_are_not_real_lamps() {
        assert!(by_id(ALL_LAMPS).is_none());
        assert!(by_id(INVALID).is_none());
    }

    #[test]
    fn codes_resolve_case_insensitively() {
        assert_eq!(by_code("tms_leuchtring_l").unwrap().id, 0x30);
        assert_eq!(by_code("TFL_L").unwrap().id, 0x03);
        assert!(by_code("not_a_lamp").is_none());
    }

    #[test]
    fn ids_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for lamp in ALL {
            assert!(seen.insert(lamp.id), "duplicate id 0x{:02X}", lamp.id);
        }
    }

    #[test]
    fn codes_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for lamp in ALL {
            assert!(seen.insert(lamp.code), "duplicate code {}", lamp.code);
        }
    }

    #[test]
    fn signalling_devices_are_marked_safety_critical() {
        // Turn signals, brake lights and reverse lights are legally regulated.
        for code in ["FRA_V_L", "BL_L", "BL_M", "RFS_L", "FRA_H_R", "WBL_LED"] {
            assert!(
                by_code(code).unwrap().safety_critical,
                "{code} must be safety critical"
            );
        }
    }

    #[test]
    fn safety_critical_lamps_are_never_featured() {
        // Featured lamps are offered up front, so none may be a signalling device.
        for lamp in ALL {
            assert!(
                !(lamp.featured && lamp.safety_critical),
                "{} cannot be both featured and safety critical",
                lamp.code
            );
        }
    }

    #[test]
    fn decorative_outputs_are_featured() {
        for code in ["TFL_L", "TMS_LEUCHTRING_L", "TMS_DESIGN_R", "POL_L"] {
            assert!(by_code(code).unwrap().featured, "{code} should be featured");
        }
    }
}
