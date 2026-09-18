//! The `csms` binary.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use csms::{AppState, Config, SiteConfig};
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about = "OCPP 2.0.1 charging station management system")]
struct Cli {
    /// Address for the OCPP WebSocket endpoint.
    #[arg(long, default_value = "0.0.0.0:9000")]
    listen: SocketAddr,
    /// Heartbeat interval given to stations at boot, in seconds.
    #[arg(long, default_value_t = 300)]
    heartbeat_interval: u64,
    /// Seconds a station has to answer a CALL from the CSMS.
    #[arg(long, default_value_t = 30)]
    call_timeout: u64,
    /// Accept only this id token in Authorize (repeat for more). Without it, every token is
    /// accepted.
    #[arg(long = "allow-id-token", value_name = "TOKEN")]
    allowed_id_tokens: Vec<String>,
    /// The site's grid limit, in W.
    #[arg(long, default_value_t = SiteConfig::default().grid_limit_w)]
    grid_limit_w: f64,
    /// The rating of every EVSE, in W.
    #[arg(long, default_value_t = SiteConfig::default().evse_max_w)]
    evse_max_w: f64,
    /// What an EVSE may draw before it has a plan (its TxDefaultProfile), in W.
    #[arg(long, default_value_t = SiteConfig::default().fallback_limit_w)]
    fallback_limit_w: f64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let cli = Cli::parse();

    let id_token_allowlist =
        (!cli.allowed_id_tokens.is_empty()).then(|| HashSet::from_iter(cli.allowed_id_tokens));
    let state = AppState::new(Config {
        heartbeat_interval: Duration::from_secs(cli.heartbeat_interval),
        call_timeout: Duration::from_secs(cli.call_timeout),
        id_token_allowlist,
        site: SiteConfig {
            grid_limit_w: cli.grid_limit_w,
            evse_max_w: cli.evse_max_w,
            fallback_limit_w: cli.fallback_limit_w,
            ..SiteConfig::default()
        },
        ..Config::default()
    });
    let listener = TcpListener::bind(cli.listen)
        .await
        .with_context(|| format!("cannot listen on {}", cli.listen))?;
    info!(
        "OCPP endpoint at ws://{}/ocpp/{{stationId}}",
        listener.local_addr()?
    );
    axum::serve(listener, csms::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")
}

async fn shutdown_signal() {
    if tokio::signal::ctrl_c().await.is_ok() {
        info!("shutting down");
    }
}
