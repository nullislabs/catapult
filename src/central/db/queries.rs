use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use super::models::{DeploymentConfig, DeploymentHistory, Worker};
use crate::shared::JobStatus;

/// Get deployment configuration for a repository
pub async fn get_deployment_config(
    pool: &PgPool,
    org: &str,
    repo: &str,
) -> Result<Option<DeploymentConfig>> {
    let config = sqlx::query_as::<_, DeploymentConfig>(
        r#"
        SELECT id, github_org, github_repo, installation_id, environment, domain, subdomain,
               site_type, enabled, created_at, updated_at
        FROM deployment_config
        WHERE github_org = $1 AND github_repo = $2 AND enabled = true
        "#,
    )
    .bind(org)
    .bind(repo)
    .fetch_optional(pool)
    .await?;

    Ok(config)
}

/// Update installation_id for a deployment config (cached from webhook)
pub async fn update_installation_id(
    pool: &PgPool,
    config_id: i32,
    installation_id: u64,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE deployment_config
        SET installation_id = $1
        WHERE id = $2
        "#,
    )
    .bind(installation_id as i64)
    .bind(config_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Get worker endpoint for an environment
pub async fn get_worker(pool: &PgPool, environment: &str) -> Result<Option<Worker>> {
    let worker = sqlx::query_as::<_, Worker>(
        r#"
        SELECT id, environment, endpoint, enabled, last_seen, created_at, updated_at
        FROM workers
        WHERE environment = $1 AND enabled = true
        "#,
    )
    .bind(environment)
    .fetch_optional(pool)
    .await?;

    Ok(worker)
}

/// Create a new deployment history record
pub async fn create_deployment(
    pool: &PgPool,
    config_id: i32,
    job_id: Uuid,
    deployment_type: &str,
    pr_number: Option<i32>,
    branch: &str,
    commit_sha: &str,
) -> Result<i32> {
    let row = sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO deployment_history (config_id, job_id, deployment_type, pr_number, branch, commit_sha, status)
        VALUES ($1, $2, $3, $4, $5, $6, 'pending')
        RETURNING id
        "#,
    )
    .bind(config_id)
    .bind(job_id)
    .bind(deployment_type)
    .bind(pr_number)
    .bind(branch)
    .bind(commit_sha)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

/// Get deployment by job_id (for status updates from workers)
pub async fn get_deployment_by_job_id(
    pool: &PgPool,
    job_id: Uuid,
) -> Result<Option<DeploymentHistory>> {
    let deployment = sqlx::query_as::<_, DeploymentHistory>(
        r#"
        SELECT id, config_id, job_id, deployment_type, pr_number, branch, commit_sha,
               status, started_at, completed_at, deployed_url, error_message, github_comment_id
        FROM deployment_history
        WHERE job_id = $1
        "#,
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await?;

    Ok(deployment)
}

/// Get deployment config by ID
pub async fn get_deployment_config_by_id(
    pool: &PgPool,
    config_id: i32,
) -> Result<Option<DeploymentConfig>> {
    let config = sqlx::query_as::<_, DeploymentConfig>(
        r#"
        SELECT id, github_org, github_repo, installation_id, environment, domain, subdomain,
               site_type, enabled, created_at, updated_at
        FROM deployment_config
        WHERE id = $1
        "#,
    )
    .bind(config_id)
    .fetch_optional(pool)
    .await?;

    Ok(config)
}

/// Update deployment status
pub async fn update_deployment_status(
    pool: &PgPool,
    deployment_id: i32,
    status: JobStatus,
    deployed_url: Option<&str>,
    error_message: Option<&str>,
) -> Result<()> {
    let completed_at = match status {
        JobStatus::Success | JobStatus::Failed | JobStatus::Cleaned => {
            Some(chrono::Utc::now())
        }
        _ => None,
    };

    sqlx::query(
        r#"
        UPDATE deployment_history
        SET status = $1, deployed_url = $2, error_message = $3, completed_at = $4
        WHERE id = $5
        "#,
    )
    .bind(status.to_string())
    .bind(deployed_url)
    .bind(error_message)
    .bind(completed_at)
    .bind(deployment_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Set GitHub comment ID for a deployment
pub async fn set_github_comment_id(
    pool: &PgPool,
    deployment_id: i32,
    comment_id: i64,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE deployment_history
        SET github_comment_id = $1
        WHERE id = $2
        "#,
    )
    .bind(comment_id)
    .bind(deployment_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Get deployment by ID
pub async fn get_deployment(pool: &PgPool, deployment_id: i32) -> Result<Option<DeploymentHistory>> {
    let deployment = sqlx::query_as::<_, DeploymentHistory>(
        r#"
        SELECT id, config_id, job_id, deployment_type, pr_number, branch, commit_sha,
               status, started_at, completed_at, deployed_url, error_message, github_comment_id
        FROM deployment_history
        WHERE id = $1
        "#,
    )
    .bind(deployment_id)
    .fetch_optional(pool)
    .await?;

    Ok(deployment)
}

/// Find active PR deployment for cleanup
pub async fn find_active_pr_deployment(
    pool: &PgPool,
    config_id: i32,
    pr_number: i32,
) -> Result<Option<DeploymentHistory>> {
    let deployment = sqlx::query_as::<_, DeploymentHistory>(
        r#"
        SELECT id, config_id, job_id, deployment_type, pr_number, branch, commit_sha,
               status, started_at, completed_at, deployed_url, error_message, github_comment_id
        FROM deployment_history
        WHERE config_id = $1 AND pr_number = $2 AND status = 'success'
        ORDER BY started_at DESC
        LIMIT 1
        "#,
    )
    .bind(config_id)
    .bind(pr_number)
    .fetch_optional(pool)
    .await?;

    Ok(deployment)
}
