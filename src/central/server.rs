use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    Router,
    routing::{get, post},
};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tower_http::trace::TraceLayer;

use crate::central::db;
use crate::central::github::GitHubApp;
use crate::central::handlers::{
    delete_authorized_org, handle_heartbeat, handle_status, handle_webhook, list_authorized_orgs,
    upsert_authorized_org,
};
use crate::central::worker_monitor::{MonitorConfig, WorkerMonitor};
use crate::config::CentralConfig;

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<CentralConfig>,
    pub db: PgPool,
    pub github_app: Arc<GitHubApp>,
    pub http_client: reqwest::Client,
}

/// Run the Central HTTP server
pub async fn run(config: CentralConfig) -> Result<()> {
    // Load GitHub App private key
    let private_key = config.load_private_key()?;

    // Initialize GitHub App
    let github_app = GitHubApp::new(config.github_app_id, &private_key)
        .context("Failed to initialize GitHub App")?;

    // Connect to database
    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
        .context("Failed to connect to database")?;

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .context("Failed to run database migrations")?;

    tracing::info!("Database connected and migrations applied");

    // Sync workers from config to database
    if !config.workers.is_empty() {
        let worker_count = db::sync_workers(&db, &config.workers)
            .await
            .context("Failed to sync workers to database")?;
        tracing::info!(count = worker_count, "Workers synced to database");

        for (zone, endpoint) in &config.workers {
            tracing::info!(zone = %zone, endpoint = %endpoint, "Worker registered");
        }

        // Start worker health monitor
        let monitor =
            WorkerMonitor::new(db.clone(), config.workers.clone(), MonitorConfig::default());
        monitor.start();
    } else {
        tracing::warn!("No workers configured - deployments will fail until workers are added");
    }

    // Build application state
    let state = AppState {
        config: Arc::new(config.clone()),
        db,
        github_app: Arc::new(github_app),
        http_client: reqwest::Client::new(),
    };

    // Build router
    let app = Router::new()
        .route("/webhook/github", post(handle_webhook))
        .route("/api/status", post(handle_status))
        .route("/api/workers/heartbeat", post(handle_heartbeat))
        // Admin API for managing authorizations
        .route("/api/admin/auth", get(list_authorized_orgs))
        .route("/api/admin/auth", post(upsert_authorized_org))
        .route(
            "/api/admin/auth",
            axum::routing::delete(delete_authorized_org),
        )
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
