use std::collections::HashMap;

use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use super::models::{AuthorizedOrg, Worker};

/// Get worker endpoint for an environment (zone)
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

/// Update worker last_seen timestamp (for heartbeat/health checks)
#[allow(dead_code)]
pub async fn update_worker_heartbeat(pool: &PgPool, environment: &str) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE workers
        SET last_seen = NOW()
        WHERE environment = $1 AND enabled = true
        "#,
    )
    .bind(environment)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Sync workers from configuration to database
///
/// - Upserts workers that are in the config (creates new or updates existing)
/// - Disables workers that are no longer in the config
/// - Returns the number of workers synced
pub async fn sync_workers(pool: &PgPool, workers: &HashMap<String, String>) -> Result<usize> {
    let mut tx = pool.begin().await?;

    // Upsert each worker from config
    for (environment, endpoint) in workers {
        sqlx::query(
            r#"
            INSERT INTO workers (environment, endpoint, enabled, last_seen)
            VALUES ($1, $2, true, NOW())
            ON CONFLICT (environment) DO UPDATE SET
                endpoint = EXCLUDED.endpoint,
                enabled = true,
                last_seen = NOW(),
                updated_at = NOW()
            "#,
        )
        .bind(environment)
        .bind(endpoint)
        .execute(&mut *tx)
        .await?;
    }

    // Disable workers that are not in the config
    if !workers.is_empty() {
        let environments: Vec<&str> = workers.keys().map(|s| s.as_str()).collect();
        sqlx::query(
            r#"
            UPDATE workers
            SET enabled = false, updated_at = NOW()
            WHERE environment != ALL($1) AND enabled = true
            "#,
        )
        .bind(&environments)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    Ok(workers.len())
}

// ==================== Job Context ====================

/// Job context for correlating status updates with GitHub comments
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct JobContext {
    #[allow(dead_code)]
    pub job_id: Uuid,
    pub installation_id: i64,
    pub github_org: String,
    pub github_repo: String,
    pub github_comment_id: Option<i64>,
    pub commit_sha: String,
}

/// Store job context for status update correlation
pub async fn store_job_context(
    pool: &PgPool,
    job_id: Uuid,
    installation_id: u64,
    org: &str,
    repo: &str,
    comment_id: Option<i64>,
    commit_sha: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO job_context (job_id, installation_id, github_org, github_repo, github_comment_id, commit_sha)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (job_id) DO UPDATE SET
            github_comment_id = COALESCE(EXCLUDED.github_comment_id, job_context.github_comment_id)
        "#,
    )
    .bind(job_id)
    .bind(installation_id as i64)
    .bind(org)
    .bind(repo)
    .bind(comment_id)
    .bind(commit_sha)
    .execute(pool)
    .await?;

    Ok(())
}

/// Get job context by job_id
pub async fn get_job_context(pool: &PgPool, job_id: Uuid) -> Result<Option<JobContext>> {
    let context = sqlx::query_as::<_, JobContext>(
        r#"
        SELECT job_id, installation_id, github_org, github_repo, github_comment_id, commit_sha
        FROM job_context
        WHERE job_id = $1
        "#,
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await?;

    Ok(context)
}

// ==================== Authorization ====================

/// Get authorized org by GitHub org name (case-insensitive)
pub async fn get_authorized_org(pool: &PgPool, github_org: &str) -> Result<Option<AuthorizedOrg>> {
    let org = sqlx::query_as::<_, AuthorizedOrg>(
        r#"
        SELECT id, github_org, zones, domain_patterns, enabled, created_at, updated_at
        FROM authorized_orgs
        WHERE LOWER(github_org) = LOWER($1) AND enabled = true
        "#,
    )
    .bind(github_org)
    .fetch_optional(pool)
    .await?;

    Ok(org)
}

/// List all authorized orgs
pub async fn list_authorized_orgs(pool: &PgPool) -> Result<Vec<AuthorizedOrg>> {
    let orgs = sqlx::query_as::<_, AuthorizedOrg>(
        r#"
        SELECT id, github_org, zones, domain_patterns, enabled, created_at, updated_at
        FROM authorized_orgs
        ORDER BY github_org
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(orgs)
}

/// Create or update an authorized org
pub async fn upsert_authorized_org(
    pool: &PgPool,
    github_org: &str,
    zones: &[String],
    domain_patterns: &[String],
) -> Result<AuthorizedOrg> {
    let org = sqlx::query_as::<_, AuthorizedOrg>(
        r#"
        INSERT INTO authorized_orgs (github_org, zones, domain_patterns, enabled)
        VALUES ($1, $2, $3, true)
        ON CONFLICT (github_org) DO UPDATE SET
            zones = EXCLUDED.zones,
            domain_patterns = EXCLUDED.domain_patterns,
            enabled = true,
            updated_at = NOW()
        RETURNING id, github_org, zones, domain_patterns, enabled, created_at, updated_at
        "#,
    )
    .bind(github_org)
    .bind(zones)
    .bind(domain_patterns)
    .fetch_one(pool)
    .await?;

    Ok(org)
}

/// Delete (disable) an authorized org
pub async fn delete_authorized_org(pool: &PgPool, github_org: &str) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE authorized_orgs
        SET enabled = false, updated_at = NOW()
        WHERE LOWER(github_org) = LOWER($1)
        "#,
    )
    .bind(github_org)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

// ==================== PR Comments ====================

/// Get the existing comment ID for a PR deployment
pub async fn get_pr_comment(
    pool: &PgPool,
    org: &str,
    repo: &str,
    pr_number: u32,
) -> Result<Option<i64>> {
    let result = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT comment_id
        FROM pr_comments
        WHERE LOWER(github_org) = LOWER($1)
          AND LOWER(github_repo) = LOWER($2)
          AND pr_number = $3
        "#,
    )
    .bind(org)
    .bind(repo)
    .bind(pr_number as i32)
    .fetch_optional(pool)
    .await?;

    Ok(result)
}

/// Store or update the comment ID for a PR deployment
pub async fn upsert_pr_comment(
    pool: &PgPool,
    org: &str,
    repo: &str,
    pr_number: u32,
    comment_id: i64,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO pr_comments (github_org, github_repo, pr_number, comment_id)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (github_org, github_repo, pr_number) DO UPDATE SET
            comment_id = EXCLUDED.comment_id,
            updated_at = NOW()
        "#,
    )
    .bind(org)
    .bind(repo)
    .bind(pr_number as i32)
    .bind(comment_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Delete PR comment tracking when PR is closed
pub async fn delete_pr_comment(
    pool: &PgPool,
    org: &str,
    repo: &str,
    pr_number: u32,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        DELETE FROM pr_comments
        WHERE LOWER(github_org) = LOWER($1)
          AND LOWER(github_repo) = LOWER($2)
          AND pr_number = $3
        "#,
    )
    .bind(org)
    .bind(repo)
    .bind(pr_number as i32)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}
