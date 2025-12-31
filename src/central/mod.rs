use crate::config::CentralConfig;
use anyhow::Result;

mod db;
mod dispatch;
mod github;
mod handlers;
mod server;

/// Run the Central orchestrator
pub async fn run(config: CentralConfig) -> Result<()> {
    tracing::info!(
        listen_addr = %config.listen_addr,
        "Starting Catapult Central"
    );

    server::run(config).await
}
