//! Charging Station Management System: an OCPP 2.0.1 server with smart charging.
//!
//! Stations connect to `ws://host:port/ocpp/{stationId}` with the `ocpp2.0.1` subprotocol.
//! Each connection runs as its own actor task (see `connection`), and the [`Registry`]
//! keeps track of which stations are connected. What the stations report is kept in the
//! [`SiteState`].

mod connection;
pub mod handle;
mod handlers;
pub mod registry;
pub mod state;
pub mod station_id;
mod ws;

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use axum::Router;
use axum::routing::get;

pub use handle::{CallError, StationHandle};
pub use registry::Registry;
pub use state::SiteState;
pub use station_id::StationId;

/// The WebSocket subprotocol a station must offer.
pub const SUBPROTOCOL: &str = "ocpp2.0.1";

/// Server settings.
#[derive(Debug, Clone)]
pub struct Config {
    /// Heartbeat interval given to stations when they boot.
    pub heartbeat_interval: Duration,
    /// How long a station has to answer a CALL.
    pub call_timeout: Duration,
    /// CALLs that can wait per connection while one is in flight; senders wait beyond that.
    pub call_queue_len: usize,
    /// Id tokens that `Authorize` accepts; `None` accepts every token.
    pub id_token_allowlist: Option<HashSet<String>>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            heartbeat_interval: Duration::from_secs(300),
            call_timeout: Duration::from_secs(30),
            call_queue_len: 32,
            id_token_allowlist: None,
        }
    }
}

/// State shared by every connection and the HTTP handlers.
#[derive(Debug)]
pub struct AppState {
    pub config: Config,
    pub registry: Registry,
    site: Mutex<SiteState>,
}

impl AppState {
    pub fn new(config: Config) -> Arc<Self> {
        Arc::new(Self {
            config,
            registry: Registry::default(),
            site: Mutex::default(),
        })
    }

    /// Locks the site state. Never hold the guard across an `.await`.
    pub fn site(&self) -> MutexGuard<'_, SiteState> {
        self.site.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The HTTP router with the OCPP WebSocket endpoint.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/ocpp/{station_id}", get(ws::upgrade))
        .with_state(state)
}
