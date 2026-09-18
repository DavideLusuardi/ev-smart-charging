//! Charging Station Management System: an OCPP 2.0.1 server with smart charging.
//!
//! Stations connect to `ws://host:port/ocpp/{stationId}` with the `ocpp2.0.1` subprotocol.
//! Each connection runs as its own actor task (see `connection`), and the [`Registry`]
//! keeps track of which stations are connected. What the stations report, including the
//! sessions in progress, is kept in the [`SiteState`].

mod charging_needs;
pub mod clock;
mod connection;
pub mod handle;
mod handlers;
mod meter;
pub mod registry;
pub mod state;
pub mod station_id;
mod transactions;
mod ws;

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use axum::Router;
use axum::routing::get;
use optimizer::NeedsDefaults;
use tokio::sync::Notify;

pub use charging_needs::ReportedNeeds;
pub use clock::Clock;
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
    pub site: SiteConfig,
}

/// The site's electrical setup.
#[derive(Debug, Clone, PartialEq)]
pub struct SiteConfig {
    /// The grid limit at startup.
    pub grid_limit_w: f64,
    /// The rating of every EVSE.
    pub evse_max_w: f64,
    /// The TxDefaultProfile limit: what an EVSE may draw without a plan.
    pub fallback_limit_w: f64,
    /// Phases wired to every AC EVSE.
    pub ac_phases: u32,
    /// What the CSMS assumes for EVs that don't report their needs.
    pub needs_defaults: NeedsDefaults,
}

impl Default for SiteConfig {
    fn default() -> Self {
        Self {
            grid_limit_w: 100_000.0,
            evse_max_w: 22_000.0,
            fallback_limit_w: 1_400.0,
            ac_phases: 3,
            // A workday at the office, and a typical commute's worth of energy.
            needs_defaults: NeedsDefaults {
                dwell_secs: 8 * 3_600,
                energy_wh: 20_000.0,
            },
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            heartbeat_interval: Duration::from_secs(300),
            call_timeout: Duration::from_secs(30),
            call_queue_len: 32,
            id_token_allowlist: None,
            site: SiteConfig::default(),
        }
    }
}

/// State shared by every connection and the HTTP handlers.
#[derive(Debug)]
pub struct AppState {
    pub config: Config,
    pub registry: Registry,
    pub clock: Clock,
    site: Mutex<SiteState>,
    /// Woken when something changed that calls for a new plan.
    pub(crate) replan: Notify,
}

impl AppState {
    pub fn new(config: Config) -> Arc<Self> {
        Self::with_clock(config, Clock::new())
    }

    pub fn with_clock(config: Config, clock: Clock) -> Arc<Self> {
        Arc::new(Self {
            site: Mutex::new(SiteState::new(config.site.grid_limit_w)),
            config,
            registry: Registry::default(),
            clock,
            replan: Notify::new(),
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
