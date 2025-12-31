use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};

use crate::central::server::AppState;
use crate::shared::{auth::verify_signature, JobStatus, StatusUpdate};

/// Handle status updates from workers
pub async fn handle_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Extract signature and timestamp headers
    let signature = match headers.get("x-worker-signature") {
        Some(sig) => sig.to_str().unwrap_or_default(),
        None => {
            tracing::warn!("Missing X-Worker-Signature header");
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
        tracing::warn!("Invalid worker signature");
        return StatusCode::UNAUTHORIZED;
    }

    // Parse status update
    let status_update: StatusUpdate = match serde_json::from_slice(&body) {
        Ok(update) => update,
        Err(e) => {
            tracing::error!(error = %e, "Failed to parse status update");
            return StatusCode::BAD_REQUEST;
        }
    };

    tracing::info!(
        job_id = %status_update.job_id,
        status = %status_update.status,
        url = status_update.deployed_url.as_deref(),
        "Received status update from worker"
    );

    // Process status update asynchronously
    tokio::spawn(async move {
        if let Err(e) = process_status_update(&state, status_update).await {
            tracing::error!(error = %e, "Failed to process status update");
        }
    });

    StatusCode::OK
}

async fn process_status_update(state: &AppState, update: StatusUpdate) -> anyhow::Result<()> {
    // TODO: Look up deployment by job_id and update status
    // TODO: Update GitHub PR comment if applicable

    // For now, just log the update
    match update.status {
        JobStatus::Success => {
            tracing::info!(
                job_id = %update.job_id,
                url = update.deployed_url.as_deref(),
                "Deployment successful"
            );
        }
        JobStatus::Failed => {
            tracing::error!(
                job_id = %update.job_id,
                error = update.error_message.as_deref(),
                "Deployment failed"
            );
        }
        JobStatus::Cleaned => {
            tracing::info!(
                job_id = %update.job_id,
                "PR deployment cleaned up"
            );
        }
        _ => {}
    }

    Ok(())
}
