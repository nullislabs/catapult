//! Worker health monitoring
//!
//! This module provides a background task that periodically checks worker health
//! and updates the `last_seen` timestamp in the database.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use sqlx::PgPool;
use tokio::time::{interval, sleep};

use crate::central::db;

/// Configuration for the worker monitor
#[derive(Debug, Clone)]
pub struct MonitorConfig {
    /// How often to check worker health (default: 30 seconds)
    pub check_interval: Duration,
    /// Timeout for health check requests (default: 5 seconds)
    pub request_timeout: Duration,
    /// Maximum retries before marking worker as unhealthy
    pub max_retries: u32,
    /// Initial retry delay (doubles each retry, up to max_delay)
    pub initial_retry_delay: Duration,
    /// Maximum retry delay
    pub max_retry_delay: Duration,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            check_interval: Duration::from_secs(30),
            request_timeout: Duration::from_secs(5),
            max_retries: 3,
            initial_retry_delay: Duration::from_secs(1),
            max_retry_delay: Duration::from_secs(30),
        }
    }
}

/// Worker health monitor
///
/// Runs as a background task and periodically checks worker health endpoints.
pub struct WorkerMonitor {
    db: PgPool,
    http_client: reqwest::Client,
    workers: Arc<HashMap<String, String>>,
    config: MonitorConfig,
}

impl WorkerMonitor {
    /// Create a new worker monitor
    pub fn new(db: PgPool, workers: HashMap<String, String>, config: MonitorConfig) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(config.request_timeout)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            db,
            http_client,
            workers: Arc::new(workers),
            config,
        }
    }

    /// Start the monitor as a background task
    ///
    /// Returns a handle to the spawned task.
    pub fn start(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            self.run().await;
        })
    }

    /// Run the monitoring loop
    async fn run(self) {
        tracing::info!(
            interval_secs = self.config.check_interval.as_secs(),
            worker_count = self.workers.len(),
            "Starting worker health monitor"
        );

        // Initial check with retries for workers that might not be ready yet
        self.initial_check().await;

        // Regular health check interval
        let mut check_interval = interval(self.config.check_interval);

        loop {
            check_interval.tick().await;
            self.check_all_workers().await;
        }
    }

    /// Initial health check with exponential backoff
    ///
    /// Workers might not be ready when Central starts, so we retry with backoff.
    async fn initial_check(&self) {
        tracing::info!("Performing initial worker health check");

        for (zone, endpoint) in self.workers.iter() {
            let mut delay = self.config.initial_retry_delay;
            let mut attempt = 0;

            loop {
                attempt += 1;
                match self.check_worker_health(zone, endpoint).await {
                    Ok(()) => {
                        tracing::info!(zone = %zone, endpoint = %endpoint, "Worker is healthy");
                        break;
                    }
                    Err(e) => {
                        if attempt >= self.config.max_retries {
                            tracing::warn!(
                                zone = %zone,
                                endpoint = %endpoint,
                                error = %e,
                                attempts = attempt,
                                "Worker unreachable after max retries"
                            );
                            break;
                        }

                        tracing::debug!(
                            zone = %zone,
                            endpoint = %endpoint,
                            error = %e,
                            attempt = attempt,
                            retry_in_secs = delay.as_secs(),
                            "Worker not ready, retrying"
                        );

                        sleep(delay).await;
                        delay = std::cmp::min(delay * 2, self.config.max_retry_delay);
                    }
                }
            }
        }
    }

    /// Check all workers
    async fn check_all_workers(&self) {
        for (zone, endpoint) in self.workers.iter() {
            if let Err(e) = self.check_worker_health(zone, endpoint).await {
                tracing::warn!(
                    zone = %zone,
                    endpoint = %endpoint,
                    error = %e,
                    "Worker health check failed"
                );
            }
        }
    }

    /// Check a single worker's health
    async fn check_worker_health(&self, zone: &str, endpoint: &str) -> Result<()> {
        let health_url = format!("{}/health", endpoint);

        let response = self.http_client.get(&health_url).send().await?;

        if !response.status().is_success() {
            anyhow::bail!("Health check returned status {}", response.status());
        }

        // Update last_seen in database
        db::update_worker_heartbeat(&self.db, zone).await?;

        tracing::trace!(zone = %zone, "Worker health check passed");

        Ok(())
    }
}
