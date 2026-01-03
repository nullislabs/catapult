use clap::{Parser, Subcommand};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod central;
mod config;
mod shared;
mod worker;

#[derive(Parser)]
#[command(name = "catapult")]
#[command(about = "Automated deployment runner for GitHub webhooks")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run as Central orchestrator (receives GitHub webhooks, dispatches to workers)
    Central {
        /// Worker endpoints in format zone=https://endpoint
        ///
        /// Each worker serves a deployment zone (tenant). Can be specified multiple times.
        ///
        /// Example: --worker nullislabs=https://deployer.nullislabs.io
        #[arg(long = "worker", value_name = "ZONE=URL")]
        workers: Vec<String>,
    },
    /// Run as Worker (executes builds, deploys to Caddy)
    Worker,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "catapult=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Central { workers } => {
            let config = config::CentralConfig::from_env_and_args(workers)?;
            central::run(config).await?;
        }
        Command::Worker => {
            let config = config::WorkerConfig::from_env()?;
            worker::run(config).await?;
        }
    }

    Ok(())
}
