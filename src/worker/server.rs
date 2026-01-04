use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    routing::{get, post},
    Router,
};
use tower_http::trace::TraceLayer;

use crate::config::WorkerConfig;
use crate::worker::deploy::{CloudflareClient, CloudflareConfig};
use crate::worker::handlers::{handle_build, handle_cleanup};

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<WorkerConfig>,
    pub http_client: reqwest::Client,
    pub cloudflare: CloudflareClient,
}

/// Run the Worker HTTP server
pub async fn run(config: WorkerConfig) -> Result<()> {
    // Verify sites directory exists
    if !config.sites_dir.exists() {
        tokio::fs::create_dir_all(&config.sites_dir)
            .await
            .context("Failed to create sites directory")?;
    }

    // Create Cloudflare client
    let cloudflare = create_cloudflare_client(&config);

    if cloudflare.is_enabled() {
        tracing::info!("Cloudflare integration enabled");
    } else {
        tracing::info!("Cloudflare integration disabled (missing config)");
    }

    // Build application state
    let state = AppState {
        config: Arc::new(config.clone()),
        http_client: reqwest::Client::new(),
        cloudflare,
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

/// Create Cloudflare client from configuration
///
/// Requires all of: CLOUDFLARE_API_TOKEN, CLOUDFLARE_ACCOUNT_ID, CLOUDFLARE_TUNNEL_ID
/// If any are missing, Cloudflare integration is disabled.
/// Zone IDs are looked up dynamically based on the domain being deployed.
fn create_cloudflare_client(config: &WorkerConfig) -> CloudflareClient {
    // Check if all required config is present
    let cf_config = match (
        &config.cloudflare_api_token,
        &config.cloudflare_account_id,
        &config.cloudflare_tunnel_id,
    ) {
        (Some(api_token), Some(account_id), Some(tunnel_id)) => {
            Some(CloudflareConfig {
                api_token: api_token.clone(),
                account_id: account_id.clone(),
                tunnel_id: tunnel_id.clone(),
                service_url: config.cloudflare_service_url.clone(),
            })
        }
        _ => None,
    };

    match cf_config {
        Some(cfg) => CloudflareClient::new(cfg),
        None => CloudflareClient::disabled(),
    }
}
