use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};

use crate::central::db;
use crate::central::github::GitHubClient;
use crate::central::server::AppState;
use crate::shared::{JobStatus, StatusUpdate, auth::verify_signature};

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
    let state_clone = state.clone();
    tokio::spawn(async move {
        if let Err(e) = process_status_update(&state_clone, status_update).await {
            tracing::error!(error = %e, "Failed to process status update");
        }
    });

    StatusCode::OK
}

async fn process_status_update(state: &AppState, update: StatusUpdate) -> anyhow::Result<()> {
    // Look up job context
    let context = match db::get_job_context(&state.db, update.job_id).await? {
        Some(ctx) => ctx,
        None => {
            tracing::warn!(job_id = %update.job_id, "No job context found for job_id");
            return Ok(());
        }
    };

    tracing::info!(
        job_id = %update.job_id,
        status = %update.status,
        org = %context.github_org,
        repo = %context.github_repo,
        "Received status update"
    );

    // Update GitHub PR comment if we have a comment_id
    if let Some(comment_id) = context.github_comment_id {
        // Skip building status (we already posted "Building..." initially)
        if update.status == JobStatus::Building {
            return Ok(());
        }

        // Skip pending and cleaned statuses
        if update.status == JobStatus::Pending || update.status == JobStatus::Cleaned {
            return Ok(());
        }

        // Get a fresh installation token
        let token = state
            .github_app
            .get_installation_token(&state.http_client, context.installation_id as u64)
            .await?;

        let github_client = GitHubClient::new(token.token);

        // Build the comment body based on status
        let comment_body = match update.status {
            JobStatus::Success => {
                let url = update
                    .deployed_url
                    .as_deref()
                    .unwrap_or("(URL not available)");
                GitHubClient::success_comment(&context.commit_sha, url)
            }
            JobStatus::Failed => {
                let error = update.error_message.as_deref().unwrap_or("Unknown error");
                GitHubClient::failure_comment(&context.commit_sha, error)
            }
            _ => return Ok(()),
        };

        // Update the comment
        github_client
            .update_comment(
                &context.github_org,
                &context.github_repo,
                comment_id,
                &comment_body,
            )
            .await?;

        tracing::info!(
            job_id = %update.job_id,
            comment_id = comment_id,
            status = %update.status,
            "Updated GitHub PR comment"
        );
    }

    Ok(())
}
