//! Application state shared across IPC calls.
//!
//! Holds the live connection, the loaded catalog and the engine. All three can
//! be absent, which is the normal state before the user connects, so the IPC
//! layer consistently reports "not connected" rather than panicking.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use slt_catalog::Catalog;
use slt_engine::{Actuator, Engine, SafetySupervisor};
use slt_transport::{Connection, Protocol};
use slt_uds::UdsClient;
use tokio::sync::Mutex;

/// Timeout for individual diagnostic requests.
pub const REQUEST_TIMEOUT: Duration = Duration::from_millis(2000);

/// Everything about the current session.
#[derive(Default)]
pub struct Session {
    pub client: Option<Arc<UdsClient>>,
    pub catalog: Option<Arc<Catalog>>,
    pub engine: Option<Engine>,
    pub protocol: Option<Protocol>,
    pub host: Option<String>,
    /// Simulator, when the user started one from the UI.
    pub simulator: Option<slt_sim::Simulator>,
}

/// Managed Tauri state.
pub struct AppState {
    pub session: Mutex<Session>,
    /// Directory catalogs are loaded from.
    pub catalog_dir: Mutex<Option<PathBuf>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            session: Mutex::new(Session::default()),
            catalog_dir: Mutex::new(default_catalog_dir()),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Locates the bundled catalog directory.
///
/// Walks up from the executable and the working directory because the layout
/// differs between `tauri dev` (running from the repo) and an installed bundle.
fn default_catalog_dir() -> Option<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(Path::to_path_buf);
        while let Some(current) = dir {
            candidates.push(current.join("catalog"));
            dir = current.parent().map(Path::to_path_buf);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut dir = Some(cwd);
        while let Some(current) = dir {
            candidates.push(current.join("catalog"));
            dir = current.parent().map(Path::to_path_buf);
        }
    }

    candidates.into_iter().find(|path| path.is_dir())
}

use std::path::Path;

impl Session {
    /// Tears down the session, releasing any held outputs first.
    ///
    /// Order matters: the engine must release before the connection closes, or
    /// the car is left holding outputs until the session times out.
    pub async fn disconnect(&mut self) {
        if let Some(engine) = &self.engine {
            if let Err(e) = engine.panic_stop().await {
                tracing::warn!(error = %e, "could not release outputs while disconnecting");
            }
        }
        self.engine = None;
        self.client = None;
        self.protocol = None;
        self.host = None;
    }

    /// Builds the engine for the current client and catalog.
    pub fn build_engine(&mut self) -> Result<(), String> {
        let client = self
            .client
            .clone()
            .ok_or_else(|| "not connected to a vehicle".to_string())?;
        let catalog = self
            .catalog
            .clone()
            .ok_or_else(|| "no catalog loaded".to_string())?;

        let actuator = Actuator::new(client, catalog).map_err(|e| e.to_string())?;
        let dwell = Duration::from_millis(actuator.min_dwell_ms());
        let supervisor = SafetySupervisor::new(dwell);
        self.engine = Some(Engine::spawn(actuator, supervisor));
        Ok(())
    }

    pub fn require_client(&self) -> Result<Arc<UdsClient>, String> {
        self.client
            .clone()
            .ok_or_else(|| "Not connected to a vehicle.".to_string())
    }

    pub fn require_engine(&self) -> Result<&Engine, String> {
        self.engine.as_ref().ok_or_else(|| {
            "The engine is not running. Connect to a vehicle and load a catalog first.".to_string()
        })
    }

    pub fn require_catalog(&self) -> Result<Arc<Catalog>, String> {
        self.catalog
            .clone()
            .ok_or_else(|| "No catalog loaded.".to_string())
    }
}

/// Opens a connection and wraps it in a UDS client.
pub async fn connect(
    protocol: Protocol,
    host: &str,
    port: Option<u16>,
) -> Result<UdsClient, String> {
    let connection = Connection::open(protocol, host, port, REQUEST_TIMEOUT)
        .await
        .map_err(|e| e.to_string())?;
    Ok(UdsClient::new(connection))
}

/// Reads short-circuit counters, returning an empty map when unsupported.
pub async fn read_counters(session: &Session) -> HashMap<u8, u8> {
    let (Some(client), Some(catalog)) = (&session.client, &session.catalog) else {
        return HashMap::new();
    };
    match Actuator::new(Arc::clone(client), Arc::clone(catalog)) {
        Ok(actuator) => actuator.read_short_circuit_counters().await,
        Err(_) => HashMap::new(),
    }
}
