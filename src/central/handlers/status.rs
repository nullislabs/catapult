use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};

use crate::central::db;
use crate::central::github::GitHubClient;
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
    let state_clone = state.clone();
    tokio::spawn(async move {
        if let Err(e) = process_status_update(&state_clone, status_update).await {
            tracing::error!(error = %e, "Failed to process status update");
        }
    });

    StatusCode::OK
}

async fn process_status_update(state: &AppState, update: StatusUpdate) -> anyhow::Result<()> {
    // Look up deployment by job_id
    let deployment = match db::get_deployment_by_job_id(&state.db, update.job_id).await? {
        Some(d) => d,
        None => {
            tracing::warn!(job_id = %update.job_id, "No deployment found for job_id");
            return Ok(());
        }
    };

    // Update deployment status in database
    db::update_deployment_status(
        &state.db,
        deployment.id,
        update.status,
        update.deployed_url.as_deref(),
        update.error_message.as_deref(),
    )
    .await?;

    tracing::info!(
        job_id = %update.job_id,
        deployment_id = deployment.id,
        status = %update.status,
        "Updated deployment status"
    );

    // Update GitHub PR comment if this is a PR deployment with a comment
    if deployment.deployment_type == "pr" && deployment.github_comment_id.is_some() {
        let comment_id = deployment.github_comment_id.unwrap();

        // Get the deployment config for org/repo info
        let config = match db::get_deployment_config_by_id(&state.db, deployment.config_id).await? {
            Some(c) => c,
            None => {
                tracing::warn!(
                    config_id = deployment.config_id,
                    "Deployment config not found"
                );
                return Ok(());
            }
        };

        // Get installation_id
        let installation_id = match config.installation_id {
            Some(id) => id as u64,
            None => {
                tracing::warn!(
                    config_id = config.id,
                    "No installation_id cached for config, cannot update comment"
                );
                return Ok(());
            }
        };

        // Get a fresh installation token
        let token = state
            .github_app
            .get_installation_token(&state.http_client, installation_id)
            .await?;

        let github_client = GitHubClient::new(token.token);

        // Build the comment body based on status
        let comment_body = match update.status {
            JobStatus::Success => {
                let url = update.deployed_url.as_deref().unwrap_or("(URL not available)");
                GitHubClient::success_comment(&deployment.commit_sha, url)
            }
            JobStatus::Failed => {
                let error = update.error_message.as_deref().unwrap_or("Unknown error");
                GitHubClient::failure_comment(&deployment.commit_sha, error)
            }
            JobStatus::Building => {
                // Don't update for building status (we already posted "Building..." initially)
                return Ok(());
            }
            JobStatus::Pending | JobStatus::Cleaned => {
                // Don't update for these statuses
                return Ok(());
            }
        };

        // Update the comment
        github_client
            .update_comment(&config.github_org, &config.github_repo, comment_id, &comment_body)
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
