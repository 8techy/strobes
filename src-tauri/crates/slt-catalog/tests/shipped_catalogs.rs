//! Validates the catalogs shipped in the repository.
//!
//! These files are data, so nothing else would catch a typo in them until a user
//! tried to load one.

use std::path::{Path, PathBuf};

use slt_catalog::Catalog;

fn catalog_dir() -> PathBuf {
    // From crates/slt-catalog up to the repository root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("catalog/chassis")
}

fn load_all() -> Vec<(String, Catalog)> {
    let dir = catalog_dir();
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", dir.display()));

    let mut catalogs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let catalog = Catalog::load(&path)
            .unwrap_or_else(|e| panic!("{name} failed to load: {e}"));
        catalogs.push((name, catalog));
    }
    assert!(!catalogs.is_empty(), "no catalogs found in {}", dir.display());
    catalogs
}

#[test]
fn every_shipped_catalog_parses_and_validates() {
    for (name, catalog) in load_all() {
        assert!(!catalog.chassis.id.is_empty(), "{name} has no chassis id");
        assert!(!catalog.ecus.is_empty(), "{name} defines no ECUs");
    }
}

#[test]
fn every_catalog_defines_the_lamp_action() {
    // Without `lamp.set` the engine cannot start, so a catalog missing it would
    // load successfully and then fail confusingly at runtime.
    for (name, catalog) in load_all() {
        catalog
            .action(slt_engine_lamp_action())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

/// Mirrors `slt_engine::actuator::LAMP_ACTION` without depending on the engine,
/// which would be a circular dependency.
fn slt_engine_lamp_action() -> &'static str {
    "lamp.set"
}

#[test]
fn lamp_action_has_a_lamp_parameter() {
    for (name, catalog) in load_all() {
        let action = catalog.action("lamp.set").unwrap();
        assert!(
            action.params.iter().any(|p| p.name == "lamp"),
            "{name}: lamp.set needs a 'lamp' parameter"
        );
    }
}

#[test]
fn only_the_simulator_catalog_is_marked_verified() {
    // Shipping a real chassis catalog as verified would let the engine transmit
    // placeholder identifiers to somebody's car.
    for (name, catalog) in load_all() {
        if catalog.chassis.id == "SIM" {
            assert!(
                catalog.fully_verified(),
                "{name}: the simulator catalog should be verified so the app is usable without a car"
            );
        } else {
            assert!(
                !catalog.fully_verified(),
                "{name}: real chassis catalogs must ship unverified until confirmed on a vehicle"
            );
        }
    }
}

#[test]
fn placeholder_identifiers_are_never_marked_verified() {
    for (name, catalog) in load_all() {
        for action in &catalog.actions {
            if action.identifier == 0x0000 {
                assert!(
                    !action.verified,
                    "{name}: action '{}' has a placeholder identifier but claims to be verified",
                    action.id
                );
            }
        }
    }
}

#[test]
fn declared_lamp_codes_all_exist() {
    // A typo here would silently drop a lamp from the UI.
    for (name, catalog) in load_all() {
        for code in &catalog.lamps.available {
            assert!(
                slt_catalog::lamp::by_code(code).is_some(),
                "{name}: '{code}' is not a known lamp code"
            );
        }
        assert_eq!(
            catalog.lamps().len(),
            catalog.lamps.available.len().max(
                if catalog.lamps.available.is_empty() {
                    slt_catalog::lamp::ALL.len()
                } else {
                    0
                }
            ),
            "{name}: some declared lamp codes did not resolve"
        );
    }
}

#[test]
fn transport_matches_the_chassis_generation() {
    // F-series speaks HSFZ and G-series speaks DoIP; getting this backwards means
    // the app connects to the wrong port and reports a confusing timeout.
    for (name, catalog) in load_all() {
        match catalog.chassis.id.as_str() {
            id if id.starts_with('F') => assert_eq!(
                catalog.chassis.transport,
                slt_transport::Protocol::Hsfz,
                "{name}: F-series must use HSFZ"
            ),
            id if id.starts_with('G') || id.starts_with('U') => assert_eq!(
                catalog.chassis.transport,
                slt_transport::Protocol::DoIp,
                "{name}: G-series must use DoIP"
            ),
            _ => {}
        }
    }
}

#[test]
fn dwell_times_respect_the_lin_bus_floor() {
    // The body controller reaches the headlight modules over LIN, which updates
    // on the order of tens of milliseconds. Anything below 20 ms cannot render.
    for (name, catalog) in load_all() {
        for action in &catalog.actions {
            assert!(
                action.min_dwell_ms >= 20,
                "{name}: action '{}' has min_dwell_ms below the 20 ms the car can render",
                action.id
            );
        }
    }
}
