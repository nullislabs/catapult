use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    Router,
    routing::{get, post},
};
use tower_http::trace::TraceLayer;

use crate::config::WorkerConfig;
use crate::worker::deploy::{
    CloudflareClient, CloudflareConfig, restore_all_routes, wait_for_caddy_ready,
};
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
    let http_client = reqwest::Client::new();
    let state = AppState {
        config: Arc::new(config.clone()),
        http_client: http_client.clone(),
        cloudflare,
    };

    // Wait for Caddy admin API to be ready before restoring routes
    if let Err(e) = wait_for_caddy_ready(&http_client, &config.caddy_admin_api).await {
        tracing::error!(error = %e, "Caddy admin API not available, skipping route restoration");
    } else {
        // Restore Caddy routes for existing site deployments
        match restore_all_routes(&http_client, &config.caddy_admin_api, &config.sites_dir).await {
            Ok(count) => {
                if count > 0 {
                    tracing::info!(count, "Restored Caddy routes for existing sites");
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to restore Caddy routes");
            }
        }
    }

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

    axum::serve(listener, app).await.context("Server error")?;

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
        (Some(api_token), Some(account_id), Some(tunnel_id)) => Some(CloudflareConfig {
            api_token: api_token.clone(),
            account_id: account_id.clone(),
            tunnel_id: tunnel_id.clone(),
            service_url: config.cloudflare_service_url.clone(),
        }),
        _ => None,
    };

    match cf_config {
        Some(cfg) => CloudflareClient::new(cfg),
        None => CloudflareClient::disabled(),
    }
}
