use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};

use crate::shared::{auth::verify_signature, CleanupJob, JobStatus, StatusUpdate};
use crate::worker::callback::send_status_update;
use crate::worker::deploy::remove_caddy_route;
use crate::worker::server::AppState;

/// Handle cleanup job requests
pub async fn handle_cleanup(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Extract signature and timestamp headers
    let signature = match headers.get("x-central-signature") {
        Some(sig) => sig.to_str().unwrap_or_default(),
        None => {
            tracing::warn!("Missing X-Central-Signature header");
            return StatusCode::UNAUTHORIZED;
        }
    };

    let timestamp: u64 = match headers.get("x-request-timestamp") {
        Some(ts) => ts.to_str().unwrap_or("0").parse().unwrap_or(0),
        None => {
            tracing::warn!("Missing X-Request-Timestamp header");
            return StatusCode::UNAUTHORIZED;
        }
    };

    // Verify signature
    if !verify_signature(
        state.config.worker_shared_secret.as_bytes(),
        &body,
        signature,
        timestamp,
    ) {
        tracing::warn!("Invalid central signature");
        return StatusCode::UNAUTHORIZED;
    }

    // Parse cleanup job
    let job: CleanupJob = match serde_json::from_slice(&body) {
        Ok(job) => job,
        Err(e) => {
            tracing::error!(error = %e, "Failed to parse cleanup job");
            return StatusCode::BAD_REQUEST;
        }
    };

    tracing::info!(
        job_id = %job.job_id,
        site_id = %job.site_id,
        "Received cleanup job"
    );

    // Spawn async cleanup task
    let state_clone = state.clone();
    tokio::spawn(async move {
        execute_cleanup(state_clone, job).await;
    });

    // Return 202 Accepted immediately
    StatusCode::ACCEPTED
}

async fn execute_cleanup(state: AppState, job: CleanupJob) {
    let job_id = job.job_id;

    match run_cleanup(&state, &job).await {
        Ok(()) => {
            tracing::info!(job_id = %job_id, site_id = %job.site_id, "Cleanup successful");

            if let Err(e) = send_status_update(
                &state.http_client,
                &job.callback_url,
                &state.config.worker_shared_secret,
                StatusUpdate {
                    job_id,
                    status: JobStatus::Cleaned,
                    deployed_url: None,
                    error_message: None,
                },
            )
            .await
            {
                tracing::error!(error = %e, "Failed to send cleanup status");
            }
        }
        Err(e) => {
            tracing::error!(job_id = %job_id, error = %e, "Cleanup failed");

            if let Err(e2) = send_status_update(
                &state.http_client,
                &job.callback_url,
                &state.config.worker_shared_secret,
                StatusUpdate {
                    job_id,
                    status: JobStatus::Failed,
                    deployed_url: None,
                    error_message: Some(e.to_string()),
                },
            )
            .await
            {
                tracing::error!(error = %e2, "Failed to send failure status");
            }
        }
    }
}

async fn run_cleanup(state: &AppState, job: &CleanupJob) -> anyhow::Result<()> {
    // Remove Caddy route
    remove_caddy_route(&state.http_client, &state.config.caddy_admin_api, &job.site_id).await?;

    // Remove site directory
    let site_dir = state.config.sites_dir.join(&job.site_id);
    if site_dir.exists() {
        tokio::fs::remove_dir_all(&site_dir).await?;
        tracing::info!(site_dir = %site_dir.display(), "Removed site directory");
    }

    Ok(())
}
