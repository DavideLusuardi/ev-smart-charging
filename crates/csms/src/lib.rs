//! Charging Station Management System: an OCPP 2.0.1 server with smart charging.
//!
//! Stations connect to `ws://host:port/ocpp/{stationId}` with the `ocpp2.0.1` subprotocol.
//! Each connection runs as its own actor task (see `connection`), and the [`Registry`]
//! keeps track of which stations are connected.

mod connection;
mod handlers;
pub mod registry;
pub mod station_id;
mod ws;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;

pub use registry::Registry;
pub use station_id::StationId;

/// The WebSocket subprotocol a station must offer.
pub const SUBPROTOCOL: &str = "ocpp2.0.1";

/// Server settings.
#[derive(Debug, Clone, Default)]
pub struct Config {}

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
