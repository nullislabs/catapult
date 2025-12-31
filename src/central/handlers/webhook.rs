use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use uuid::Uuid;

use crate::central::dispatch::dispatch_build_job;
use crate::central::github::{
    parse_webhook_event, verify_webhook_signature, GitHubClient, PullRequestAction, WebhookEvent,
};
use crate::central::server::AppState;
use crate::central::db;
use crate::shared::{BuildJob, CleanupJob, generate_site_id};

/// Handle incoming GitHub webhooks
pub async fn handle_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Extract required headers
    let signature = match headers.get("x-hub-signature-256") {
        Some(sig) => sig.to_str().unwrap_or_default(),
        None => {
            tracing::warn!("Missing X-Hub-Signature-256 header");
            return StatusCode::UNAUTHORIZED;
        }
    };

    let event_type = match headers.get("x-github-event") {
        Some(et) => et.to_str().unwrap_or_default(),
        None => {
            tracing::warn!("Missing X-GitHub-Event header");
            return StatusCode::BAD_REQUEST;
        }
    };

    // Verify signature
    if !verify_webhook_signature(&state.config.github_webhook_secret, &body, signature) {
        tracing::warn!("Invalid webhook signature");
        return StatusCode::UNAUTHORIZED;
    }

    // Parse event
    let event = match parse_webhook_event(event_type, &body) {
        Ok(event) => event,
        Err(e) => {
            tracing::error!(error = %e, "Failed to parse webhook payload");
            return StatusCode::BAD_REQUEST;
        }
    };

    // Process event asynchronously
    tokio::spawn(async move {
        if let Err(e) = process_webhook_event(&state, event).await {
            tracing::error!(error = %e, "Failed to process webhook event");
        }
    });

    StatusCode::OK
}

async fn process_webhook_event(state: &AppState, event: WebhookEvent) -> anyhow::Result<()> {
    match event {
        WebhookEvent::PullRequest(pr_event) => {
            let org = pr_event.repository.org_name();
            let repo = &pr_event.repository.name;

            tracing::info!(
                org = org,
                repo = repo,
                pr = pr_event.number,
                action = ?pr_event.action,
                "Processing pull request event"
            );

            // Look up deployment config
            let config = match db::get_deployment_config(&state.db, org, repo).await? {
                Some(config) => config,
                None => {
                    tracing::debug!(org, repo, "No deployment config found, ignoring");
                    return Ok(());
                }
            };

            // Get installation ID
            let installation_id = pr_event
                .installation
                .as_ref()
                .map(|i| i.id)
                .ok_or_else(|| anyhow::anyhow!("Missing installation ID in webhook"))?;

            // Cache installation_id on config for later use (status updates)
            if config.installation_id.is_none() || config.installation_id != Some(installation_id as i64) {
                db::update_installation_id(&state.db, config.id, installation_id).await?;
            }

            match pr_event.action {
                PullRequestAction::Opened | PullRequestAction::Synchronize | PullRequestAction::Reopened => {
                    // Generate job_id upfront
                    let job_id = Uuid::new_v4();

                    // Get installation token
                    let token = state
                        .github_app
                        .get_installation_token(&state.http_client, installation_id)
                        .await?;

                    // Create deployment record with job_id
                    let deployment_id = db::create_deployment(
                        &state.db,
                        config.id,
                        job_id,
                        "pr",
                        Some(pr_event.number as i32),
                        &pr_event.pull_request.head.branch,
                        &pr_event.pull_request.head.sha,
                    )
                    .await?;

                    // Post "Building..." comment
                    let github_client = GitHubClient::new(token.token.clone());
                    let comment = github_client
                        .create_pr_comment(
                            org,
                            repo,
                            pr_event.number,
                            &GitHubClient::building_comment(&pr_event.pull_request.head.sha),
                        )
                        .await?;

                    // Store comment ID for later updates
                    db::set_github_comment_id(&state.db, deployment_id, comment.id).await?;

                    // Get worker for this environment
                    let worker = db::get_worker(&state.db, &config.environment)
                        .await?
                        .ok_or_else(|| {
                            anyhow::anyhow!("No worker found for environment: {}", config.environment)
                        })?;

                    // Dispatch build job with same job_id
                    let job = BuildJob {
                        job_id,
                        repo_url: pr_event.repository.clone_url.clone(),
                        git_token: token.token,
                        branch: pr_event.pull_request.head.branch.clone(),
                        commit_sha: pr_event.pull_request.head.sha.clone(),
                        pr_number: Some(pr_event.number),
                        domain: config.domain.clone(),
                        site_type: config.site_type(),
                        callback_url: format!(
                            "https://{}/api/status",
                            state.config.listen_addr
                        ),
                        repo_name: repo.to_string(),
                        org_name: org.to_string(),
                        subdomain: config.subdomain.clone(),
                    };

                    dispatch_build_job(
                        &state.http_client,
                        &worker.endpoint,
                        &state.config.worker_shared_secret,
                        &job,
                    )
                    .await?;

                    tracing::info!(
                        job_id = %job_id,
                        deployment_id = deployment_id,
                        pr = pr_event.number,
                        "Dispatched build job"
                    );
                }
                PullRequestAction::Closed => {
                    // Clean up PR deployment
                    if let Some(_deployment) =
                        db::find_active_pr_deployment(&state.db, config.id, pr_event.number as i32).await?
                    {
                        // Get worker
                        let worker = db::get_worker(&state.db, &config.environment)
                            .await?
                            .ok_or_else(|| {
                                anyhow::anyhow!("No worker found for environment: {}", config.environment)
                            })?;

                        // Dispatch cleanup job
                        let job = CleanupJob {
                            job_id: Uuid::new_v4(),
                            site_id: generate_site_id(org, repo, Some(pr_event.number)),
                            callback_url: format!(
                                "https://{}/api/status",
                                state.config.listen_addr
                            ),
                        };

                        crate::central::dispatch::dispatch_cleanup_job(
                            &state.http_client,
                            &worker.endpoint,
                            &state.config.worker_shared_secret,
                            &job,
                        )
                        .await?;

                        tracing::info!(
                            job_id = %job.job_id,
                            pr = pr_event.number,
                            "Dispatched cleanup job"
                        );
                    }
                }
                _ => {
                    tracing::debug!(action = ?pr_event.action, "Ignoring PR action");
                }
            }
        }
        WebhookEvent::Push(push_event) => {
            // Only process pushes to main branch
            if !push_event.is_main_branch() {
                tracing::debug!(ref_name = push_event.git_ref, "Ignoring non-main branch push");
                return Ok(());
            }

            let org = push_event.repository.org_name();
            let repo = &push_event.repository.name;

            tracing::info!(
                org = org,
                repo = repo,
                commit = &push_event.after,
                "Processing main branch push"
            );

            // Look up deployment config
            let config = match db::get_deployment_config(&state.db, org, repo).await? {
                Some(config) => config,
                None => {
                    tracing::debug!(org, repo, "No deployment config found, ignoring");
                    return Ok(());
                }
            };

            // Get installation ID
            let installation_id = push_event
                .installation
                .as_ref()
                .map(|i| i.id)
                .ok_or_else(|| anyhow::anyhow!("Missing installation ID in webhook"))?;

            // Cache installation_id on config
            if config.installation_id.is_none() || config.installation_id != Some(installation_id as i64) {
                db::update_installation_id(&state.db, config.id, installation_id).await?;
            }

            // Generate job_id upfront
            let job_id = Uuid::new_v4();

            // Get installation token
            let token = state
                .github_app
                .get_installation_token(&state.http_client, installation_id)
                .await?;

            // Create deployment record with job_id
            let deployment_id = db::create_deployment(
                &state.db,
                config.id,
                job_id,
                "main",
                None,
                push_event.branch_name().unwrap_or("main"),
                &push_event.after,
            )
            .await?;

            // Get worker for this environment
            let worker = db::get_worker(&state.db, &config.environment)
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!("No worker found for environment: {}", config.environment)
                })?;

            // Dispatch build job with same job_id
            let job = BuildJob {
                job_id,
                repo_url: push_event.repository.clone_url.clone(),
                git_token: token.token,
                branch: push_event.branch_name().unwrap_or("main").to_string(),
                commit_sha: push_event.after.clone(),
                pr_number: None,
                domain: config.domain.clone(),
                site_type: config.site_type(),
                callback_url: format!("https://{}/api/status", state.config.listen_addr),
                repo_name: repo.to_string(),
                org_name: org.to_string(),
                subdomain: config.subdomain.clone(),
            };

            dispatch_build_job(
                &state.http_client,
                &worker.endpoint,
                &state.config.worker_shared_secret,
                &job,
            )
            .await?;

            tracing::info!(
                job_id = %job_id,
                deployment_id = deployment_id,
                commit = &push_event.after,
                "Dispatched main branch build job"
            );
        }
        WebhookEvent::Ping => {
            tracing::info!("Received ping event");
        }
        WebhookEvent::Unknown(event_type) => {
            tracing::debug!(event_type, "Ignoring unknown event type");
        }
    }

    Ok(())
}
