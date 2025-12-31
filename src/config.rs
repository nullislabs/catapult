use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};

/// Configuration for Central mode
#[derive(Debug, Clone)]
pub struct CentralConfig {
    /// PostgreSQL connection URL
    pub database_url: String,

    /// GitHub App ID
    pub github_app_id: u64,

    /// Path to GitHub App private key PEM file
    pub github_private_key_path: PathBuf,

    /// GitHub webhook secret for signature verification
    pub github_webhook_secret: String,

    /// Shared secret for worker authentication
    pub worker_shared_secret: String,

    /// Address to listen on
    pub listen_addr: SocketAddr,
}

impl CentralConfig {
    /// Load configuration from environment variables
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            database_url: std::env::var("DATABASE_URL")
                .context("DATABASE_URL environment variable required")?,

            github_app_id: std::env::var("GITHUB_APP_ID")
                .context("GITHUB_APP_ID environment variable required")?
                .parse()
                .context("GITHUB_APP_ID must be a valid integer")?,

            github_private_key_path: std::env::var("GITHUB_PRIVATE_KEY_PATH")
                .context("GITHUB_PRIVATE_KEY_PATH environment variable required")?
                .into(),

            github_webhook_secret: std::env::var("GITHUB_WEBHOOK_SECRET")
                .context("GITHUB_WEBHOOK_SECRET environment variable required")?,

            worker_shared_secret: std::env::var("WORKER_SHARED_SECRET")
                .context("WORKER_SHARED_SECRET environment variable required")?,

            listen_addr: std::env::var("LISTEN_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
                .parse()
                .context("LISTEN_ADDR must be a valid socket address")?,
        })
    }

    /// Load the GitHub App private key from disk
    pub fn load_private_key(&self) -> Result<String> {
        std::fs::read_to_string(&self.github_private_key_path).with_context(|| {
            format!(
                "Failed to read GitHub private key from {:?}",
                self.github_private_key_path
            )
        })
    }
}

/// Configuration for Worker mode
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// URL of the Central server
    pub central_url: String,

    /// Shared secret for authentication with Central
    pub worker_shared_secret: String,

    /// Path to Podman socket
    pub podman_socket: PathBuf,

    /// Caddy admin API URL
    pub caddy_admin_api: String,

    /// Directory where sites are deployed
    pub sites_dir: PathBuf,

    /// Address to listen on
    pub listen_addr: SocketAddr,
}

impl WorkerConfig {
    /// Load configuration from environment variables
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            central_url: std::env::var("CENTRAL_URL")
                .context("CENTRAL_URL environment variable required")?,

            worker_shared_secret: std::env::var("WORKER_SHARED_SECRET")
                .context("WORKER_SHARED_SECRET environment variable required")?,

            podman_socket: std::env::var("PODMAN_SOCKET")
                .unwrap_or_else(|_| "/run/podman/podman.sock".to_string())
                .into(),

            caddy_admin_api: std::env::var("CADDY_ADMIN_API")
                .unwrap_or_else(|_| "http://localhost:2019".to_string()),

            sites_dir: std::env::var("SITES_DIR")
                .unwrap_or_else(|_| "/var/www/sites".to_string())
                .into(),

            listen_addr: std::env::var("LISTEN_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
                .parse()
                .context("LISTEN_ADDR must be a valid socket address")?,
        })
    }
}
