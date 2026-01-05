use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use uuid::Uuid;

use crate::central::db;
use crate::central::deploy_config::fetch_deploy_config;
use crate::central::dispatch::dispatch_build_job;
use crate::central::github::{
    GitHubClient, PullRequestAction, WebhookEvent, parse_webhook_event, verify_webhook_signature,
};
use crate::central::server::AppState;
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

            // Get installation ID
            let installation_id = pr_event
                .installation
                .as_ref()
                .map(|i| i.id)
                .ok_or_else(|| anyhow::anyhow!("Missing installation ID in webhook"))?;

            // Get installation token (needed to fetch .deploy.json)
            let token = state
                .github_app
                .get_installation_token(&state.http_client, installation_id)
                .await?;

            // Fetch deploy config from org/.github and repo
            let deploy_config =
                fetch_deploy_config(&state.http_client, &token.token, org, repo).await?;

            let deploy_config = match deploy_config {
                Some(config) if config.is_deployable() => config,
                Some(_) => {
                    tracing::debug!(
                        org,
                        repo,
                        "Deployment disabled or no zone configured, ignoring"
                    );
                    return Ok(());
                }
                None => {
                    tracing::debug!(org, repo, "No .deploy.json found, ignoring");
                    return Ok(());
                }
            };

            let zone = deploy_config.zone.as_ref().unwrap(); // Safe: is_deployable checks this

            // Check authorization
            let auth = db::get_authorized_org(&state.db, org)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Organization '{}' is not authorized", org))?;

            if !auth.can_use_zone(zone) {
                anyhow::bail!(
                    "Organization '{}' is not authorized to use zone '{}'",
                    org,
                    zone
                );
            }

            // Get worker for this zone
            let worker = db::get_worker(&state.db, zone)
                .await?
                .ok_or_else(|| anyhow::anyhow!("No worker configured for zone: {}", zone))?;

            match pr_event.action {
                PullRequestAction::Opened
                | PullRequestAction::Synchronize
                | PullRequestAction::Reopened => {
                    // Resolve PR domain
                    let pr_domain = deploy_config
                        .resolve_pr_domain(repo, pr_event.number)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "Cannot resolve PR domain - no domain or pattern configured"
                            )
                        })?;

                    // Verify domain is allowed
                    if !auth.can_use_domain(&pr_domain) {
                        anyhow::bail!(
                            "Organization '{}' is not authorized to use domain '{}'",
                            org,
                            pr_domain
                        );
                    }

                    // Generate job_id
                    let job_id = Uuid::new_v4();

                    // Create or update the PR comment
                    let github_client = GitHubClient::new(token.token.clone());
                    let comment_id =
                        match db::get_pr_comment(&state.db, org, repo, pr_event.number).await? {
                            Some(existing_comment_id) => {
                                // Update existing comment
                                tracing::debug!(
                                    pr = pr_event.number,
                                    comment_id = existing_comment_id,
                                    "Updating existing PR comment"
                                );
                                github_client
                                    .update_comment(
                                        org,
                                        repo,
                                        existing_comment_id,
                                        &GitHubClient::building_comment(
                                            &pr_event.pull_request.head.sha,
                                        ),
                                    )
                                    .await?;
                                existing_comment_id
                            }
                            None => {
                                // Create new comment
                                tracing::debug!(pr = pr_event.number, "Creating new PR comment");
                                let comment = github_client
                                    .create_pr_comment(
                                        org,
                                        repo,
                                        pr_event.number,
                                        &GitHubClient::building_comment(
                                            &pr_event.pull_request.head.sha,
                                        ),
                                    )
                                    .await?;
                                // Store the comment ID for future updates
                                db::upsert_pr_comment(
                                    &state.db,
                                    org,
                                    repo,
                                    pr_event.number,
                                    comment.id,
                                )
                                .await?;
                                comment.id
                            }
                        };

                    // Dispatch build job
                    let job = BuildJob {
                        job_id,
                        repo_url: pr_event.repository.clone_url.clone(),
                        git_token: token.token,
                        branch: pr_event.pull_request.head.branch.clone(),
                        commit_sha: pr_event.pull_request.head.sha.clone(),
                        pr_number: Some(pr_event.number),
                        domain: pr_domain.clone(),
                        site_type: deploy_config.build_type.unwrap_or_default(),
                        callback_url: format!("{}/api/status", state.config.callback_base_url),
                        repo_name: repo.to_string(),
                        org_name: org.to_string(),
                        subdomain: None, // PRs don't use subdomain
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
                        pr = pr_event.number,
                        domain = %pr_domain,
                        zone = %zone,
                        "Dispatched PR build job"
                    );

                    // Store deployment info for status updates
                    // We store the comment_id with the job_id for later correlation
                    store_deployment_context(
                        state,
                        job_id,
                        installation_id,
                        org,
                        repo,
                        Some(comment_id),
                        &pr_event.pull_request.head.sha,
                    )
                    .await?;
                }
                PullRequestAction::Closed => {
                    // Resolve PR domain for cleanup
                    let pr_domain = deploy_config.resolve_pr_domain(repo, pr_event.number);

                    // Dispatch cleanup job
                    let job = CleanupJob {
                        job_id: Uuid::new_v4(),
                        site_id: generate_site_id(org, repo, Some(pr_event.number)),
                        callback_url: format!("{}/api/status", state.config.callback_base_url),
                        domain: pr_domain,
                    };

                    crate::central::dispatch::dispatch_cleanup_job(
                        &state.http_client,
                        &worker.endpoint,
                        &state.config.worker_shared_secret,
                        &job,
                    )
                    .await?;

                    // Clean up the PR comment tracking
                    if let Err(e) =
                        db::delete_pr_comment(&state.db, org, repo, pr_event.number).await
                    {
                        tracing::warn!(
                            error = %e,
                            pr = pr_event.number,
                            "Failed to delete PR comment tracking"
                        );
                    }

                    tracing::info!(
                        job_id = %job.job_id,
                        pr = pr_event.number,
                        zone = %zone,
                        "Dispatched cleanup job"
                    );
                }
                _ => {
                    tracing::debug!(action = ?pr_event.action, "Ignoring PR action");
                }
            }
        }
        WebhookEvent::Push(push_event) => {
            // Only process pushes to main branch
            if !push_event.is_main_branch() {
                tracing::debug!(
                    ref_name = push_event.git_ref,
                    "Ignoring non-main branch push"
                );
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

            // Get installation ID
            let installation_id = push_event
                .installation
                .as_ref()
                .map(|i| i.id)
                .ok_or_else(|| anyhow::anyhow!("Missing installation ID in webhook"))?;

            // Get installation token
            let token = state
                .github_app
                .get_installation_token(&state.http_client, installation_id)
                .await?;

            // Fetch deploy config
            let deploy_config =
                fetch_deploy_config(&state.http_client, &token.token, org, repo).await?;

            let deploy_config = match deploy_config {
                Some(config) if config.is_deployable() => config,
                Some(_) => {
                    tracing::debug!(
                        org,
                        repo,
                        "Deployment disabled or no zone configured, ignoring"
                    );
                    return Ok(());
                }
                None => {
                    tracing::debug!(org, repo, "No .deploy.json found, ignoring");
                    return Ok(());
                }
            };

            let zone = deploy_config.zone.as_ref().unwrap();

            // Check authorization
            let auth = db::get_authorized_org(&state.db, org)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Organization '{}' is not authorized", org))?;

            if !auth.can_use_zone(zone) {
                anyhow::bail!(
                    "Organization '{}' is not authorized to use zone '{}'",
                    org,
                    zone
                );
            }

            // Resolve main branch domain
            let main_domain = deploy_config.resolve_domain(repo).ok_or_else(|| {
                anyhow::anyhow!("Cannot resolve domain - no domain or pattern configured")
            })?;

            // Verify domain is allowed
            if !auth.can_use_domain(&main_domain) {
                anyhow::bail!(
                    "Organization '{}' is not authorized to use domain '{}'",
                    org,
                    main_domain
                );
            }

            // Get worker for this zone
            let worker = db::get_worker(&state.db, zone)
                .await?
                .ok_or_else(|| anyhow::anyhow!("No worker configured for zone: {}", zone))?;

            // Generate job_id
            let job_id = Uuid::new_v4();

            // Dispatch build job
            let job = BuildJob {
                job_id,
                repo_url: push_event.repository.clone_url.clone(),
                git_token: token.token,
                branch: push_event.branch_name().unwrap_or("main").to_string(),
                commit_sha: push_event.after.clone(),
                pr_number: None,
                domain: main_domain.clone(),
                site_type: deploy_config.build_type.unwrap_or_default(),
                callback_url: format!("{}/api/status", state.config.callback_base_url),
                repo_name: repo.to_string(),
                org_name: org.to_string(),
                subdomain: deploy_config.subdomain.clone(),
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
                commit = &push_event.after,
                domain = %main_domain,
                zone = %zone,
                "Dispatched main branch build job"
            );

            // Store deployment info for status updates
            // Push events don't have PR comments, so comment_id is None
            store_deployment_context(
                state,
                job_id,
                installation_id,
                org,
                repo,
                None,
                &push_event.after,
            )
            .await?;
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

/// Store deployment context for status update correlation
///
/// This stores the minimum info needed to update GitHub comments when
/// status updates arrive from workers.
/// For push events, comment_id is None since we don't create PR comments.
async fn store_deployment_context(
    state: &AppState,
    job_id: Uuid,
    installation_id: u64,
    org: &str,
    repo: &str,
    comment_id: Option<i64>,
    commit_sha: &str,
) -> anyhow::Result<()> {
    db::store_job_context(
        &state.db,
        job_id,
        installation_id,
        org,
        repo,
        comment_id,
        commit_sha,
    )
    .await?;

    Ok(())
}
