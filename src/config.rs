use std::collections::HashMap;
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

    /// Worker endpoints by environment name
    /// e.g., {"production": "https://deployer.example.com", "staging": "https://deployer-staging.example.com"}
    pub workers: HashMap<String, String>,
}

impl CentralConfig {
    /// Load configuration from environment variables and CLI arguments
    ///
    /// Workers are specified via CLI: `--worker zone=https://endpoint`
    pub fn from_env_and_args(worker_args: Vec<String>) -> Result<Self> {
        let workers = Self::parse_worker_args(worker_args)?;

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

            workers,
        })
    }

    /// Parse worker arguments from CLI
    ///
    /// Each argument should be in the format: `zone=https://endpoint`
    fn parse_worker_args(args: Vec<String>) -> Result<HashMap<String, String>> {
        let mut workers = HashMap::new();

        for arg in args {
            let (zone, endpoint) = arg
                .split_once('=')
                .with_context(|| format!("Invalid worker format '{}', expected 'zone=https://endpoint'", arg))?;

            let zone = zone.trim();
            let endpoint = endpoint.trim();

            if zone.is_empty() {
                anyhow::bail!("Empty zone name in worker argument '{}'", arg);
            }
            if endpoint.is_empty() {
                anyhow::bail!("Empty endpoint URL in worker argument '{}'", arg);
            }
            if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
                anyhow::bail!("Worker endpoint must be a URL: '{}'", endpoint);
            }

            if workers.insert(zone.to_string(), endpoint.to_string()).is_some() {
                anyhow::bail!("Duplicate worker zone: '{}'", zone);
            }
        }

        Ok(workers)
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

    /// Whether to use container isolation for builds
    pub use_containers: bool,

    /// Container image for builds (must have nix installed)
    pub build_image: String,

    /// Memory limit for build containers (in bytes)
    pub container_memory_limit: u64,

    /// CPU limit for build containers (number of CPUs * 100000)
    pub container_cpu_quota: i64,

    /// PID limit for build containers
    pub container_pids_limit: i64,
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
                .unwrap_or_else(|_| Self::detect_podman_socket())
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

            use_containers: std::env::var("USE_CONTAINERS")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(true), // Default to using containers

            build_image: std::env::var("BUILD_IMAGE")
                .unwrap_or_else(|_| "nixos/nix:latest".to_string()),

            container_memory_limit: std::env::var("CONTAINER_MEMORY_LIMIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(4 * 1024 * 1024 * 1024), // 4GB default

            container_cpu_quota: std::env::var("CONTAINER_CPU_QUOTA")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(200000), // 2 CPUs default

            container_pids_limit: std::env::var("CONTAINER_PIDS_LIMIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1000),
        })
    }

    /// Detect the best available Podman socket
    ///
    /// Prefers the system socket (for production with iptables support),
    /// falls back to user socket if system socket isn't available.
    fn detect_podman_socket() -> String {
        // First try system socket (preferred for production - supports iptables)
        let system_socket = "/run/podman/podman.sock";
        if std::path::Path::new(system_socket).exists() {
            return system_socket.to_string();
        }

        // Fall back to user socket (for development/rootless mode)
        // SAFETY: getuid is safe to call and returns the real user ID
        let uid = unsafe { libc::getuid() };
        let user_socket = format!("/run/user/{}/podman/podman.sock", uid);
        if std::path::Path::new(&user_socket).exists() {
            return user_socket;
        }

        // Default to system socket even if it doesn't exist
        // (will fail with a clear error when used)
        system_socket.to_string()
    }
}
