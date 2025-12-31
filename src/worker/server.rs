use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    routing::{get, post},
    Router,
};
use tower_http::trace::TraceLayer;

use crate::config::WorkerConfig;
use crate::worker::handlers::{handle_build, handle_cleanup};

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<WorkerConfig>,
    pub http_client: reqwest::Client,
}

/// Run the Worker HTTP server
pub async fn run(config: WorkerConfig) -> Result<()> {
    // Verify sites directory exists
    if !config.sites_dir.exists() {
        tokio::fs::create_dir_all(&config.sites_dir)
            .await
            .context("Failed to create sites directory")?;
    }

    // Build application state
    let state = AppState {
        config: Arc::new(config.clone()),
        http_client: reqwest::Client::new(),
    };

    // Build router
    let app = Router::new()
        .route("/build", post(handle_build))
        .route("/cleanup", post(handle_cleanup))
        .route("/health", get(health_check))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // Start server
    let listener = tokio::net::TcpListener::bind(&config.listen_addr)
        .await
        .context("Failed to bind to address")?;

    tracing::info!(addr = %config.listen_addr, "Server listening");

    axum::serve(listener, app)
        .await
        .context("Server error")?;

    Ok(())
}

async fn health_check() -> &'static str {
    "OK"
}
