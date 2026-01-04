use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};

use crate::shared::{auth::verify_signature, BuildJob, JobStatus, StatusUpdate};
use crate::worker::callback::send_status_update;
use crate::worker::server::AppState;

/// Handle incoming build job requests
pub async fn handle_build(
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

    // Parse build job
    let job: BuildJob = match serde_json::from_slice(&body) {
        Ok(job) => job,
        Err(e) => {
            tracing::error!(error = %e, "Failed to parse build job");
            return StatusCode::BAD_REQUEST;
        }
    };

    tracing::info!(
        job_id = %job.job_id,
        repo = %job.repo_name,
        branch = %job.branch,
        pr = job.pr_number,
        "Received build job"
    );

    // Spawn async build task
    let state_clone = state.clone();
    tokio::spawn(async move {
        execute_build(state_clone, job).await;
    });

    // Return 202 Accepted immediately
    StatusCode::ACCEPTED
}

async fn execute_build(state: AppState, job: BuildJob) {
    let job_id = job.job_id;
    let callback_url = job.callback_url.clone();

    // Send building status
    if let Err(e) = send_status_update(
        &state.http_client,
        &callback_url,
        &state.config.worker_shared_secret,
        StatusUpdate {
            job_id,
            status: JobStatus::Building,
            deployed_url: None,
            error_message: None,
        },
    )
    .await
    {
        tracing::error!(error = %e, "Failed to send building status");
    }

    // Execute the build pipeline
    match run_build_pipeline(&state, &job).await {
        Ok(deployed_url) => {
            tracing::info!(job_id = %job_id, url = %deployed_url, "Build successful");

            if let Err(e) = send_status_update(
                &state.http_client,
                &callback_url,
                &state.config.worker_shared_secret,
                StatusUpdate {
                    job_id,
                    status: JobStatus::Success,
                    deployed_url: Some(deployed_url),
                    error_message: None,
                },
            )
            .await
            {
                tracing::error!(error = %e, "Failed to send success status");
            }
        }
        Err(e) => {
            tracing::error!(job_id = %job_id, error = %e, "Build failed");

            if let Err(e2) = send_status_update(
                &state.http_client,
                &callback_url,
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

async fn run_build_pipeline(state: &AppState, job: &BuildJob) -> anyhow::Result<String> {
    use crate::shared::generate_site_id;
    use crate::worker::builder::{clone_repository, run_build};
    use crate::worker::deploy::configure_caddy_route;

    let site_id = generate_site_id(&job.org_name, &job.repo_name, job.pr_number);

    // Create work directory
    let work_dir = std::env::temp_dir().join(format!("catapult-{}", job.job_id));
    tokio::fs::create_dir_all(&work_dir).await?;

    // Clone repository
    tracing::info!(job_id = %job.job_id, "Cloning repository");
    let repo_dir = clone_repository(&job.repo_url, &job.git_token, &job.commit_sha, &work_dir).await?;

    // Run build in container
    tracing::info!(job_id = %job.job_id, "Running build");
    let output_dir = run_build(state, job, &repo_dir).await?;

    // Deploy to sites directory
    let site_dir = state.config.sites_dir.join(&site_id);
    tracing::info!(job_id = %job.job_id, site_dir = %site_dir.display(), "Deploying artifacts");

    // Remove old deployment if exists
    if site_dir.exists() {
        tokio::fs::remove_dir_all(&site_dir).await?;
    }

    // Copy build artifacts
    copy_dir_recursive(&output_dir, &site_dir).await?;

    // Configure Caddy route
    let deployed_url = crate::shared::generate_preview_url(
        &job.domain,
        &job.repo_name,
        job.pr_number,
    );

    configure_caddy_route(
        &state.http_client,
        &state.config.caddy_admin_api,
        &site_id,
        &site_dir,
        &job.domain,
        &job.repo_name,
        job.pr_number,
    )
    .await?;

    // Configure Cloudflare DNS and tunnel ingress
    // The domain field contains the full hostname (e.g., "pr-42-website.nxm.rs")
    if state.cloudflare.is_enabled() {
        tracing::info!(job_id = %job.job_id, hostname = %job.domain, "Configuring Cloudflare route");
        if let Err(e) = state.cloudflare.ensure_route(&job.domain).await {
            // Log but don't fail the build - Caddy is already configured
            tracing::error!(error = %e, hostname = %job.domain, "Failed to configure Cloudflare route");
        }
    }

    // Cleanup work directory
    let _ = tokio::fs::remove_dir_all(&work_dir).await;

    Ok(deployed_url)
}

async fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(dst).await?;

    let mut entries = tokio::fs::read_dir(src).await?;
    while let Some(entry) = entries.next_entry().await? {
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if entry.file_type().await?.is_dir() {
            Box::pin(copy_dir_recursive(&src_path, &dst_path)).await?;
        } else {
            tokio::fs::copy(&src_path, &dst_path).await?;
        }
    }

    Ok(())
}
