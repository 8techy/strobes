//! Strobes application library.
//!
//! The Tauri shell is deliberately thin: it owns process lifetime and the IPC
//! surface, and delegates everything else to the `slt-*` crates so the domain
//! logic stays testable without a GUI.

pub mod ipc;
pub mod state;

use tracing_subscriber::EnvFilter;

/// Builds and runs the desktop application.
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("STROBELIGHT_LOG")
                .unwrap_or_else(|_| EnvFilter::new("info,slt_engine=debug")),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppState::new())
        .invoke_handler(tauri::generate_handler![
            ipc::discover_vehicles,
            ipc::connect_vehicle,
            ipc::disconnect_vehicle,
            ipc::connection_status,
            ipc::read_vehicle_info,
            ipc::scan_ecus,
            ipc::read_dtcs,
            ipc::clear_dtcs,
            ipc::read_short_circuit_counters,
            ipc::run_safety_preflight,
            ipc::list_catalogs,
            ipc::load_catalog,
            ipc::active_catalog,
            ipc::list_lamps,
            ipc::list_effects,
            ipc::set_lamp,
            ipc::release_all,
            ipc::panic_stop,
            ipc::start_effect,
            ipc::stop_effect,
            ipc::engine_status,
            ipc::submit_beat,
            ipc::set_beat_sync,
            ipc::start_simulator,
            ipc::stop_simulator,
            ipc::simulator_status,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Strobes");
}
