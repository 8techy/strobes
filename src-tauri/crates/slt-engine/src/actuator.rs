//! Turns lamp commands into UDS requests using the loaded catalog.
//!
//! The engine deals in "lamp 0x30 at 100%"; this translates that into the exact
//! bytes the connected car expects, which is entirely catalog-driven because the
//! identifiers differ per chassis and are not something Strobelight can ship.

use std::collections::HashMap;
use std::sync::Arc;

use slt_catalog::{Catalog, CatalogError};
use slt_transport::EcuAddress;
use slt_uds::{UdsClient, UdsError};

/// Parameter names the lamp action is expected to use.
///
/// A convention rather than a schema rule, so catalogs stay close to BMW's own
/// naming while the engine still knows which value is which.
pub const PARAM_LAMP: &str = "lamp";
pub const PARAM_LEVEL: &str = "level";

/// The catalog action id used for lamp control.
pub const LAMP_ACTION: &str = "lamp.set";

/// Actuator failures.
#[derive(Debug, thiserror::Error)]
pub enum ActuatorError {
    #[error(transparent)]
    Catalog(#[from] CatalogError),

    #[error(transparent)]
    Uds(#[from] UdsError),

    #[error("the catalog has no '{0}' action, so lamps cannot be controlled")]
    MissingLampAction(String),
}

type Result<T> = std::result::Result<T, ActuatorError>;

/// Applies lamp commands to a vehicle.
pub struct Actuator {
    client: Arc<UdsClient>,
    catalog: Arc<Catalog>,
    /// Resolved once at construction so the hot path avoids a lookup per command.
    action_id: String,
    address: EcuAddress,
    /// Whether the action carries a level parameter. Some modules only support
    /// on/off, in which case the catalog omits it.
    supports_level: bool,
}

impl Actuator {
    /// Binds an actuator to a client and catalog.
    pub fn new(client: Arc<UdsClient>, catalog: Arc<Catalog>) -> Result<Self> {
        let action = catalog
            .action(LAMP_ACTION)
            .map_err(|_| ActuatorError::MissingLampAction(LAMP_ACTION.to_string()))?;
        let address = catalog.action_address(action)?;
        let supports_level = action.params.iter().any(|p| p.name == PARAM_LEVEL);

        Ok(Self {
            client,
            catalog: Arc::clone(&catalog),
            action_id: action.id.clone(),
            address,
            supports_level,
        })
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn catalog_verified(&self) -> bool {
        self.catalog.fully_verified()
    }

    pub fn address(&self) -> EcuAddress {
        self.address
    }

    /// The per-lamp dwell floor this chassis requires.
    pub fn min_dwell_ms(&self) -> u64 {
        self.catalog.min_step_ms()
    }

    /// Opens the diagnostic session the catalog's lamp action requires.
    pub async fn begin_session(&self) -> Result<()> {
        let action = self.catalog.action(&self.action_id)?;
        let session = match action.session {
            0x03 => slt_uds::Session::Extended,
            _ => slt_uds::Session::Default,
        };
        self.client.start_session(self.address, session).await?;
        Ok(())
    }

    /// Commands one lamp to a brightness.
    pub async fn set_lamp(&self, lamp: u8, level: u8) -> Result<()> {
        let action = self.catalog.action(&self.action_id)?;

        let mut values = HashMap::new();
        values.insert(PARAM_LAMP.to_string(), f64::from(lamp));
        if self.supports_level {
            values.insert(PARAM_LEVEL.to_string(), f64::from(level));
        }

        let payload = action.encode_actuate(&values)?;
        self.client.request(self.address, &payload).await?;
        Ok(())
    }

    /// Hands every output back to the ECU in one request.
    ///
    /// Uses the release form with no lamp parameter, which per ISO 14229 returns
    /// control for the whole identifier. One request is important here: this runs
    /// on the panic path and when a connection is failing, so the fewer round
    /// trips the better.
    pub async fn release_all(&self) -> Result<()> {
        let action = self.catalog.action(&self.action_id)?;
        let payload = action.encode_release();
        self.client.request(self.address, &payload).await?;
        Ok(())
    }

    /// Releases a specific lamp.
    pub async fn release_lamp(&self, lamp: u8) -> Result<()> {
        let action = self.catalog.action(&self.action_id)?;
        let mut payload = action.encode_release();
        payload.push(lamp);
        self.client.request(self.address, &payload).await?;
        Ok(())
    }

    /// Reads per-lamp short-circuit counters for the preflight check.
    ///
    /// The identifier is chassis-specific and optional; when the catalog does not
    /// define a `lamp.counters` action this returns an empty map and the
    /// preflight simply has less to go on.
    pub async fn read_short_circuit_counters(&self) -> HashMap<u8, u8> {
        let Ok(action) = self.catalog.action("lamp.counters") else {
            tracing::debug!(
                "catalog defines no 'lamp.counters' action; skipping short-circuit check"
            );
            return HashMap::new();
        };
        let Ok(address) = self.catalog.action_address(action) else {
            return HashMap::new();
        };

        match self
            .client
            .read_data_by_identifier(address, action.identifier)
            .await
        {
            Ok(data) => parse_counter_pairs(&data),
            Err(e) => {
                tracing::warn!(error = %e, "could not read short-circuit counters");
                HashMap::new()
            }
        }
    }
}

/// Parses a counter response as `[lamp, count]` pairs.
fn parse_counter_pairs(data: &[u8]) -> HashMap<u8, u8> {
    data.chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_pairs_parse() {
        let counters = parse_counter_pairs(&[0x03, 12, 0x04, 0, 0x30, 50]);
        assert_eq!(counters.get(&0x03), Some(&12));
        assert_eq!(counters.get(&0x04), Some(&0));
        assert_eq!(counters.get(&0x30), Some(&50));
    }

    #[test]
    fn trailing_odd_byte_is_ignored() {
        // A truncated response must not panic or invent a counter.
        let counters = parse_counter_pairs(&[0x03, 12, 0x04]);
        assert_eq!(counters.len(), 1);
    }

    #[test]
    fn empty_response_yields_no_counters() {
        assert!(parse_counter_pairs(&[]).is_empty());
    }
}
