use crate::config::WorkerConfig;
use anyhow::Result;

mod builder;
mod callback;
mod deploy;
mod handlers;
mod server;

/// Run the Worker build executor
pub async fn run(config: WorkerConfig) -> Result<()> {
    tracing::info!(
        listen_addr = %config.listen_addr,
        "Starting Catapult Worker"
    );

    server::run(config).await
}
