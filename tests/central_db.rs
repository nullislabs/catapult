//! Integration tests for Central database operations

mod common;

use catapult::central::db;
use catapult::shared::JobStatus;
use common::TestDatabase;
use uuid::Uuid;

#[tokio::test]
async fn test_worker_crud() {
    let db = TestDatabase::new().await;

    // Insert a worker
    sqlx::query(
        r#"
        INSERT INTO workers (environment, endpoint, enabled)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind("production")
    .bind("https://worker.example.com")
    .bind(true)
    .execute(&db.pool)
    .await
    .expect("Failed to insert worker");

    // Get worker
    let worker = db::get_worker(&db.pool, "production")
        .await
        .expect("Failed to get worker")
        .expect("Worker not found");

    assert_eq!(worker.environment, "production");
    assert_eq!(worker.endpoint, "https://worker.example.com");
    assert!(worker.enabled);
}

#[tokio::test]
async fn test_worker_not_found() {
    let db = TestDatabase::new().await;

    let worker = db::get_worker(&db.pool, "nonexistent")
        .await
        .expect("Failed to query worker");

    assert!(worker.is_none());
}

#[tokio::test]
async fn test_deployment_config_crud() {
    let db = TestDatabase::new().await;

    // Create a worker first (FK constraint)
    db.create_test_worker("production").await;

    // Insert a deployment config
    sqlx::query(
        r#"
        INSERT INTO deployment_config (github_org, github_repo, environment, domain, site_type, enabled)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind("testorg")
    .bind("testrepo")
    .bind("production")
    .bind("example.com")
    .bind("sveltekit")
    .bind(true)
    .execute(&db.pool)
    .await
    .expect("Failed to insert deployment config");

    // Get deployment config
    let config = db::get_deployment_config(&db.pool, "testorg", "testrepo")
        .await
        .expect("Failed to get deployment config")
        .expect("Config not found");

    assert_eq!(config.github_org, "testorg");
    assert_eq!(config.github_repo, "testrepo");
    assert_eq!(config.domain, "example.com");
    assert_eq!(config.site_type, "sveltekit");
}

#[tokio::test]
async fn test_deployment_config_disabled() {
    let db = TestDatabase::new().await;

    // Create a worker first (FK constraint)
    db.create_test_worker("production").await;

    // Insert a disabled deployment config
    sqlx::query(
        r#"
        INSERT INTO deployment_config (github_org, github_repo, environment, domain, site_type, enabled)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind("testorg")
    .bind("testrepo")
    .bind("production")
    .bind("example.com")
    .bind("sveltekit")
    .bind(false) // disabled
    .execute(&db.pool)
    .await
    .expect("Failed to insert deployment config");

    // Should not find disabled config
    let config = db::get_deployment_config(&db.pool, "testorg", "testrepo")
        .await
        .expect("Failed to get deployment config");

    assert!(config.is_none());
}

#[tokio::test]
async fn test_deployment_history_lifecycle() {
    let db = TestDatabase::new().await;

    // Create a worker first (FK constraint)
    db.create_test_worker("production").await;

    // First create a deployment config
    let config_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO deployment_config (github_org, github_repo, environment, domain, site_type, enabled)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind("testorg")
    .bind("testrepo")
    .bind("production")
    .bind("example.com")
    .bind("sveltekit")
    .bind(true)
    .fetch_one(&db.pool)
    .await
    .expect("Failed to insert deployment config");

    // Create deployment
    let job_id = Uuid::new_v4();
    let deployment_id = db::create_deployment(
        &db.pool,
        config_id,
        job_id,
        "pr",
        Some(42),
        "feature-branch",
        "abc123def456",
    )
    .await
    .expect("Failed to create deployment");

    assert!(deployment_id > 0);

    // Get deployment by job_id
    let deployment = db::get_deployment_by_job_id(&db.pool, job_id)
        .await
        .expect("Failed to get deployment")
        .expect("Deployment not found");

    assert_eq!(deployment.config_id, config_id);
    assert_eq!(deployment.job_id, Some(job_id));
    assert_eq!(deployment.deployment_type, "pr");
    assert_eq!(deployment.pr_number, Some(42));
    assert_eq!(deployment.branch, "feature-branch");
    assert_eq!(deployment.commit_sha, "abc123def456");
    assert_eq!(deployment.status, "pending");

    // Update deployment status to success
    db::update_deployment_status(
        &db.pool,
        deployment_id,
        JobStatus::Success,
        Some("https://pr-42.example.com"),
        None,
    )
    .await
    .expect("Failed to update deployment status");

    // Verify update
    let updated = db::get_deployment_by_job_id(&db.pool, job_id)
        .await
        .expect("Failed to get deployment")
        .expect("Deployment not found");

    assert_eq!(updated.status, "success");
    assert_eq!(
        updated.deployed_url,
        Some("https://pr-42.example.com".to_string())
    );
    assert!(updated.completed_at.is_some());
}

#[tokio::test]
async fn test_deployment_failure() {
    let db = TestDatabase::new().await;

    // Create a worker first (FK constraint)
    db.create_test_worker("production").await;

    // Create config and deployment
    let config_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO deployment_config (github_org, github_repo, environment, domain, site_type, enabled)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind("testorg")
    .bind("testrepo")
    .bind("production")
    .bind("example.com")
    .bind("vite")
    .bind(true)
    .fetch_one(&db.pool)
    .await
    .expect("Failed to insert deployment config");

    let job_id = Uuid::new_v4();
    let deployment_id = db::create_deployment(
        &db.pool,
        config_id,
        job_id,
        "main",
        None,
        "main",
        "def456abc789",
    )
    .await
    .expect("Failed to create deployment");

    // Update to failed status
    db::update_deployment_status(
        &db.pool,
        deployment_id,
        JobStatus::Failed,
        None,
        Some("Build failed: npm install failed"),
    )
    .await
    .expect("Failed to update deployment status");

    // Verify
    let deployment = db::get_deployment_by_job_id(&db.pool, job_id)
        .await
        .expect("Failed to get deployment")
        .expect("Deployment not found");

    assert_eq!(deployment.status, "failed");
    assert!(deployment.deployed_url.is_none());
    assert_eq!(
        deployment.error_message,
        Some("Build failed: npm install failed".to_string())
    );
}

#[tokio::test]
async fn test_github_comment_id() {
    let db = TestDatabase::new().await;

    // Create a worker first (FK constraint)
    db.create_test_worker("production").await;

    // Create config and deployment
    let config_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO deployment_config (github_org, github_repo, environment, domain, site_type, enabled)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind("testorg")
    .bind("testrepo")
    .bind("production")
    .bind("example.com")
    .bind("sveltekit")
    .bind(true)
    .fetch_one(&db.pool)
    .await
    .expect("Failed to insert deployment config");

    let job_id = Uuid::new_v4();
    let deployment_id = db::create_deployment(
        &db.pool,
        config_id,
        job_id,
        "pr",
        Some(123),
        "feature",
        "commit123",
    )
    .await
    .expect("Failed to create deployment");

    // Set GitHub comment ID
    db::set_github_comment_id(&db.pool, deployment_id, 987654321)
        .await
        .expect("Failed to set comment ID");

    // Verify
    let deployment = db::get_deployment_by_job_id(&db.pool, job_id)
        .await
        .expect("Failed to get deployment")
        .expect("Deployment not found");

    assert_eq!(deployment.github_comment_id, Some(987654321));
}

#[tokio::test]
async fn test_installation_id_update() {
    let db = TestDatabase::new().await;

    // Create a worker first (FK constraint)
    db.create_test_worker("production").await;

    // Create config
    let config_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO deployment_config (github_org, github_repo, environment, domain, site_type, enabled)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind("testorg")
    .bind("testrepo")
    .bind("production")
    .bind("example.com")
    .bind("sveltekit")
    .bind(true)
    .fetch_one(&db.pool)
    .await
    .expect("Failed to insert deployment config");

    // Initially no installation_id
    let config = db::get_deployment_config_by_id(&db.pool, config_id)
        .await
        .expect("Failed to get config")
        .expect("Config not found");
    assert!(config.installation_id.is_none());

    // Update installation_id
    db::update_installation_id(&db.pool, config_id, 12345678)
        .await
        .expect("Failed to update installation_id");

    // Verify
    let config = db::get_deployment_config_by_id(&db.pool, config_id)
        .await
        .expect("Failed to get config")
        .expect("Config not found");
    assert_eq!(config.installation_id, Some(12345678));
}

#[tokio::test]
async fn test_find_active_pr_deployment() {
    let db = TestDatabase::new().await;

    // Create a worker first (FK constraint)
    db.create_test_worker("production").await;

    // Create config
    let config_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO deployment_config (github_org, github_repo, environment, domain, site_type, enabled)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind("testorg")
    .bind("testrepo")
    .bind("production")
    .bind("example.com")
    .bind("sveltekit")
    .bind(true)
    .fetch_one(&db.pool)
    .await
    .expect("Failed to insert deployment config");

    // Create a successful PR deployment
    let job_id = Uuid::new_v4();
    let deployment_id = db::create_deployment(
        &db.pool,
        config_id,
        job_id,
        "pr",
        Some(42),
        "feature",
        "commit123",
    )
    .await
    .expect("Failed to create deployment");

    db::update_deployment_status(
        &db.pool,
        deployment_id,
        JobStatus::Success,
        Some("https://pr-42.example.com"),
        None,
    )
    .await
    .expect("Failed to update status");

    // Find active deployment
    let active = db::find_active_pr_deployment(&db.pool, config_id, 42)
        .await
        .expect("Failed to find deployment")
        .expect("Deployment not found");

    assert_eq!(active.pr_number, Some(42));
    assert_eq!(active.status, "success");

    // Should not find non-existent PR
    let not_found = db::find_active_pr_deployment(&db.pool, config_id, 999)
        .await
        .expect("Failed to query");
    assert!(not_found.is_none());
}
