//! Charging Station Management System: an OCPP 2.0.1 server with smart charging.
//!
//! Stations connect to `ws://host:port/ocpp/{stationId}` with the `ocpp2.0.1` subprotocol.
//! Each connection runs as its own actor task (see `connection`), and the [`Registry`]
//! keeps track of which stations are connected.

mod connection;
pub mod handle;
mod handlers;
pub mod registry;
pub mod station_id;
mod ws;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::routing::get;

pub use handle::{CallError, StationHandle};
pub use registry::Registry;
pub use station_id::StationId;

/// The WebSocket subprotocol a station must offer.
pub const SUBPROTOCOL: &str = "ocpp2.0.1";

/// Server settings.
#[derive(Debug, Clone)]
pub struct Config {
    /// How long a station has to answer a CALL.
    pub call_timeout: Duration,
    /// CALLs that can wait per connection while one is in flight; senders wait beyond that.
    pub call_queue_len: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            call_timeout: Duration::from_secs(30),
            call_queue_len: 32,
        }
    }
}

/// State shared by every connection and the HTTP handlers.
#[derive(Debug)]
pub struct AppState {
    pub config: Config,
    pub registry: Registry,
}

impl AppState {
    pub fn new(config: Config) -> Arc<Self> {
        Arc::new(Self {
            config,
            registry: Registry::default(),
        })
    }
}

/// The HTTP router with the OCPP WebSocket endpoint.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/ocpp/{station_id}", get(ws::upgrade))
        .with_state(state)
}
