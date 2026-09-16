//! The `csms` binary.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use csms::{AppState, Config};
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about = "OCPP 2.0.1 charging station management system")]
struct Cli {
    /// Address for the OCPP WebSocket endpoint.
    #[arg(long, default_value = "0.0.0.0:9000")]
    listen: SocketAddr,
    /// Seconds a station has to answer a CALL from the CSMS.
    #[arg(long, default_value_t = 30)]
    call_timeout: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let cli = Cli::parse();

    let state = AppState::new(Config {
        call_timeout: Duration::from_secs(cli.call_timeout),
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
