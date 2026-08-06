//! The IPC surface exposed to the frontend.
//!
//! Every command returns `Result<T, String>` because errors are shown directly
//! to the user: a typed error would only be stringified in the UI anyway, and
//! the messages from the lower layers are already written to be read by a person.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use slt_catalog::{Catalog, Lamp};
use slt_engine::{Effect, EngineStatus, Preflight};
use slt_transport::{DiscoveredVehicle, EcuAddress, Protocol};
use slt_uds::{dtc::STATUS_MASK_ALL, Dtc, EcuScanResult, Session as UdsSession, VehicleInfo};
use tauri::{Emitter, State};

use crate::state::{self, AppState};

/// How long to listen for vehicle announcements.
const DISCOVERY_TIMEOUT: Duration = Duration::from_millis(2500);

/// Connection state reported to the UI.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStatus {
    pub connected: bool,
    pub protocol: Option<String>,
    pub host: Option<String>,
    pub catalog_id: Option<String>,
    pub catalog_verified: bool,
    pub engine_ready: bool,
    pub simulator_running: bool,
}

/// A lamp entry for the UI.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LampInfo {
    pub id: u8,
    pub id_hex: String,
    pub code: String,
    pub name: String,
    pub group: String,
    pub safety_critical: bool,
    pub featured: bool,
}

impl From<&'static Lamp> for LampInfo {
    fn from(lamp: &'static Lamp) -> Self {
        Self {
            id: lamp.id,
            id_hex: format!("0x{:02X}", lamp.id),
            code: lamp.code.to_string(),
            name: lamp.name.to_string(),
            group: lamp.group.label().to_string(),
            safety_critical: lamp.safety_critical,
            featured: lamp.featured,
        }
    }
}

/// A catalog file available to load.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSummary {
    pub path: String,
    pub chassis_id: String,
    pub name: String,
    pub transport: String,
    pub action_count: usize,
    pub verified: bool,
    pub notes: String,
}

// -- Connection ------------------------------------------------------------

#[tauri::command]
pub async fn discover_vehicles() -> Result<Vec<DiscoveredVehicle>, String> {
    slt_transport::discover(DISCOVERY_TIMEOUT)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn connect_vehicle(
    state: State<'_, AppState>,
    protocol: Protocol,
    host: String,
    port: Option<u16>,
) -> Result<VehicleInfo, String> {
    let client = state::connect(protocol, &host, port).await?;
    let info = client.read_vehicle_info().await;

    let mut session = state.session.lock().await;
    session.client = Some(Arc::new(client));
    session.protocol = Some(protocol);
    session.host = Some(host);

    // The engine only exists once a catalog is loaded, so a failure here is
    // expected and not worth surfacing as a connection failure.
    if session.catalog.is_some() {
        if let Err(e) = session.build_engine() {
            tracing::warn!(error = %e, "connected but could not build the engine");
        }
    }

    Ok(info)
}

#[tauri::command]
pub async fn disconnect_vehicle(state: State<'_, AppState>) -> Result<(), String> {
    let mut session = state.session.lock().await;
    session.disconnect().await;
    Ok(())
}

#[tauri::command]
pub async fn connection_status(state: State<'_, AppState>) -> Result<ConnectionStatus, String> {
    let session = state.session.lock().await;
    Ok(ConnectionStatus {
        connected: session.client.is_some(),
        protocol: session.protocol.map(|p| p.as_str().to_string()),
        host: session.host.clone(),
        catalog_id: session.catalog.as_ref().map(|c| c.chassis.id.clone()),
        catalog_verified: session
            .catalog
            .as_ref()
            .is_some_and(|c| c.fully_verified()),
        engine_ready: session.engine.is_some(),
        simulator_running: session.simulator.is_some(),
    })
}

// -- Read-only diagnostics -----------------------------------------------

#[tauri::command]
pub async fn read_vehicle_info(state: State<'_, AppState>) -> Result<VehicleInfo, String> {
    let client = {
        let session = state.session.lock().await;
        session.require_client()?
    };
    Ok(client.read_vehicle_info().await)
}

#[tauri::command]
pub async fn scan_ecus(state: State<'_, AppState>) -> Result<Vec<EcuScanResult>, String> {
    let client = {
        let session = state.session.lock().await;
        session.require_client()?
    };
    Ok(client.scan_lighting_ecus().await)
}

#[tauri::command]
pub async fn read_dtcs(
    state: State<'_, AppState>,
    ecu: Option<u16>,
) -> Result<Vec<Dtc>, String> {
    let client = {
        let session = state.session.lock().await;
        session.require_client()?
    };
    let address = EcuAddress::new(ecu.unwrap_or(slt_transport::ecu::FEM_BODY.0));
    client
        .read_dtcs(address, STATUS_MASK_ALL)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn clear_dtcs(state: State<'_, AppState>, ecu: Option<u16>) -> Result<(), String> {
    let client = {
        let session = state.session.lock().await;
        session.require_client()?
    };
    let address = EcuAddress::new(ecu.unwrap_or(slt_transport::ecu::FEM_BODY.0));
    client.clear_dtcs(address).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn read_short_circuit_counters(
    state: State<'_, AppState>,
) -> Result<HashMap<String, u8>, String> {
    let session = state.session.lock().await;
    session.require_client()?;
    let counters = state::read_counters(&session).await;
    // Keys are stringified because JSON object keys must be strings, and hex is
    // what the user sees everywhere else.
    Ok(counters
        .into_iter()
        .map(|(lamp, count)| (format!("0x{lamp:02X}"), count))
        .collect())
}

/// Runs the preflight safety check and hands the result to the engine.
#[tauri::command]
pub async fn run_safety_preflight(state: State<'_, AppState>) -> Result<Preflight, String> {
    let session = state.session.lock().await;
    let client = session.require_client()?;
    let catalog_verified = session
        .catalog
        .as_ref()
        .is_some_and(|c| c.fully_verified());

    let dtcs = client
        .read_dtcs(slt_transport::ecu::FEM_BODY, STATUS_MASK_ALL)
        .await
        .unwrap_or_default();
    let counters = state::read_counters(&session).await;

    // Evaluated on a throwaway supervisor, then handed to the engine's own.
    let mut supervisor = slt_engine::SafetySupervisor::new(Duration::from_millis(40));
    let preflight = supervisor.evaluate_preflight(dtcs, counters, catalog_verified);

    if let Some(engine) = &session.engine {
        engine
            .set_preflight(preflight.clone())
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(preflight)
}

// -- Catalog ---------------------------------------------------------------

#[tauri::command]
pub async fn list_catalogs(state: State<'_, AppState>) -> Result<Vec<CatalogSummary>, String> {
    let dir = {
        let guard = state.catalog_dir.lock().await;
        guard.clone()
    };
    let Some(dir) = dir else {
        return Ok(Vec::new());
    };

    let mut summaries = Vec::new();
    let entries = std::fs::read_dir(&dir)
        .map_err(|e| format!("could not read {}: {e}", dir.display()))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        match Catalog::load(&path) {
            Ok(catalog) => summaries.push(CatalogSummary {
                path: path.display().to_string(),
                chassis_id: catalog.chassis.id.clone(),
                name: catalog.chassis.name.clone(),
                transport: match catalog.chassis.transport {
                    Protocol::Hsfz => "HSFZ".into(),
                    Protocol::DoIp => "DoIP".into(),
                },
                action_count: catalog.actions.len(),
                verified: catalog.fully_verified(),
                notes: catalog.chassis.notes.clone(),
            }),
            // A broken catalog should not hide the working ones.
            Err(e) => tracing::warn!(path = %path.display(), error = %e, "skipping catalog"),
        }
    }
    summaries.sort_by(|a, b| a.chassis_id.cmp(&b.chassis_id));
    Ok(summaries)
}

#[tauri::command]
pub async fn load_catalog(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let catalog = Catalog::load(std::path::Path::new(&path)).map_err(|e| e.to_string())?;
    let id = catalog.chassis.id.clone();

    let mut session = state.session.lock().await;
    session.catalog = Some(Arc::new(catalog));
    if session.client.is_some() {
        session.build_engine()?;
    }
    Ok(id)
}

#[tauri::command]
pub async fn active_catalog(state: State<'_, AppState>) -> Result<Option<Catalog>, String> {
    let session = state.session.lock().await;
    Ok(session.catalog.as_ref().map(|c| (**c).clone()))
}

#[tauri::command]
pub async fn list_lamps(state: State<'_, AppState>) -> Result<Vec<LampInfo>, String> {
    let session = state.session.lock().await;
    // Without a catalog the full enumeration is still useful for browsing.
    let lamps = match &session.catalog {
        Some(catalog) => catalog.lamps(),
        None => slt_catalog::lamp::ALL.iter().collect(),
    };
    Ok(lamps.into_iter().map(LampInfo::from).collect())
}

// -- Effects ---------------------------------------------------------------

#[tauri::command]
pub fn list_effects() -> Vec<Effect> {
    slt_engine::effect::presets()
}

#[tauri::command]
pub async fn set_lamp(
    state: State<'_, AppState>,
    lamp: u8,
    level: u8,
) -> Result<(), String> {
    let session = state.session.lock().await;
    session
        .require_engine()?
        .set_lamp(lamp, level)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn release_all(state: State<'_, AppState>) -> Result<(), String> {
    let session = state.session.lock().await;
    session
        .require_engine()?
        .release_all()
        .await
        .map_err(|e| e.to_string())
}

/// The user-facing emergency stop.
#[tauri::command]
pub async fn panic_stop(state: State<'_, AppState>) -> Result<(), String> {
    let session = state.session.lock().await;
    session
        .require_engine()?
        .panic_stop()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_effect(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    effect: Effect,
) -> Result<(), String> {
    let session = state.session.lock().await;
    let engine = session.require_engine()?;

    // Bridge engine events onto the Tauri event bus for this run.
    let mut events = engine.subscribe();
    let handle = app.clone();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            if handle.emit("engine-event", &event).is_err() {
                break;
            }
        }
    });

    engine.start(effect).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn stop_effect(state: State<'_, AppState>) -> Result<(), String> {
    let session = state.session.lock().await;
    session
        .require_engine()?
        .stop()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn engine_status(state: State<'_, AppState>) -> Result<EngineStatus, String> {
    let session = state.session.lock().await;
    match session.engine.as_ref() {
        Some(engine) => engine.status().await.map_err(|e| e.to_string()),
        None => Ok(EngineStatus::default()),
    }
}

#[tauri::command]
pub async fn submit_beat(state: State<'_, AppState>, bpm: f32) -> Result<(), String> {
    let session = state.session.lock().await;
    // Beats arrive continuously from the audio analyser, so a missing engine is
    // an expected transient rather than an error worth reporting.
    if let Some(engine) = session.engine.as_ref() {
        engine.submit_beat(bpm).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn set_beat_sync(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    let session = state.session.lock().await;
    session
        .require_engine()?
        .set_beat_sync(enabled)
        .await
        .map_err(|e| e.to_string())
}

// -- Simulator -------------------------------------------------------------

#[tauri::command]
pub async fn start_simulator(
    state: State<'_, AppState>,
    protocol: Protocol,
) -> Result<String, String> {
    let simulator = slt_sim::Simulator::start(protocol, 0)
        .await
        .map_err(|e| format!("could not start the simulator: {e}"))?;
    let address = format!("{}:{}", simulator.ip(), simulator.port());

    let mut session = state.session.lock().await;
    session.simulator = Some(simulator);
    Ok(address)
}

#[tauri::command]
pub async fn stop_simulator(state: State<'_, AppState>) -> Result<(), String> {
    let mut session = state.session.lock().await;
    session.simulator = None;
    Ok(())
}

#[tauri::command]
pub async fn simulator_status(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let session = state.session.lock().await;
    Ok(session
        .simulator
        .as_ref()
        .map(|s| format!("{}:{}", s.ip(), s.port())))
}

/// Re-exported so the frontend's generated types line up with the Rust side.
pub use slt_uds::Session as DiagnosticSession;

/// Kept for the manual console: opens the session an action needs.
#[allow(dead_code)]
pub(crate) async fn ensure_session(
    client: &slt_uds::UdsClient,
    ecu: EcuAddress,
) -> Result<(), String> {
    client
        .start_session(ecu, UdsSession::Extended)
        .await
        .map_err(|e| e.to_string())
}
