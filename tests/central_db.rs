//! Integration tests for Central database operations

mod common;

use catapult::central::db;
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
async fn test_worker_disabled() {
    let db = TestDatabase::new().await;

    // Insert a disabled worker
    sqlx::query(
        r#"
        INSERT INTO workers (environment, endpoint, enabled)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind("staging")
    .bind("https://staging-worker.example.com")
    .bind(false)
    .execute(&db.pool)
    .await
    .expect("Failed to insert worker");

    // Should not find disabled worker
    let worker = db::get_worker(&db.pool, "staging")
        .await
        .expect("Failed to query worker");

    assert!(worker.is_none());
}

#[tokio::test]
async fn test_job_context_crud() {
    let db = TestDatabase::new().await;

    let job_id = Uuid::new_v4();
    let installation_id: u64 = 12345678;
    let org = "testorg";
    let repo = "testrepo";
    let comment_id: i64 = 987654321;
    let commit_sha = "abc123def456";

    // Store job context
    db::store_job_context(
        &db.pool,
        job_id,
        installation_id,
        org,
        repo,
        Some(comment_id),
        commit_sha,
    )
    .await
    .expect("Failed to store job context");

    // Get job context
    let context = db::get_job_context(&db.pool, job_id)
        .await
        .expect("Failed to get job context")
        .expect("Job context not found");

    assert_eq!(context.job_id, job_id);
    assert_eq!(context.installation_id, installation_id as i64);
    assert_eq!(context.github_org, org);
    assert_eq!(context.github_repo, repo);
    assert_eq!(context.github_comment_id, Some(comment_id));
    assert_eq!(context.commit_sha, commit_sha);
}

#[tokio::test]
async fn test_job_context_not_found() {
    let db = TestDatabase::new().await;

    let context = db::get_job_context(&db.pool, Uuid::new_v4())
        .await
        .expect("Failed to query job context");

    assert!(context.is_none());
}

#[tokio::test]
async fn test_job_context_upsert() {
    let db = TestDatabase::new().await;

    let job_id = Uuid::new_v4();
    let installation_id: u64 = 12345678;
    let org = "testorg";
    let repo = "testrepo";
    let commit_sha = "abc123";

    // Store job context with initial comment ID
    db::store_job_context(
        &db.pool,
        job_id,
        installation_id,
        org,
        repo,
        Some(111),
        commit_sha,
    )
    .await
    .expect("Failed to store job context");

    // Update with new comment ID
    db::store_job_context(
        &db.pool,
        job_id,
        installation_id,
        org,
        repo,
        Some(222),
        commit_sha,
    )
    .await
    .expect("Failed to update job context");

    // Verify the comment ID was updated
    let context = db::get_job_context(&db.pool, job_id)
        .await
        .expect("Failed to get job context")
        .expect("Job context not found");

    assert_eq!(context.github_comment_id, Some(222));
}

#[tokio::test]
async fn test_sync_workers() {
    let db = TestDatabase::new().await;

    use std::collections::HashMap;

    // Initial sync with two workers
    let mut workers = HashMap::new();
    workers.insert(
        "zone1".to_string(),
        "https://worker1.example.com".to_string(),
    );
    workers.insert(
        "zone2".to_string(),
        "https://worker2.example.com".to_string(),
    );

    let count = db::sync_workers(&db.pool, &workers)
        .await
        .expect("Failed to sync workers");

    assert_eq!(count, 2);

    // Verify workers exist
    let worker1 = db::get_worker(&db.pool, "zone1")
        .await
        .expect("Failed to get worker")
        .expect("Worker not found");
    assert_eq!(worker1.endpoint, "https://worker1.example.com");

    let worker2 = db::get_worker(&db.pool, "zone2")
        .await
        .expect("Failed to get worker")
        .expect("Worker not found");
    assert_eq!(worker2.endpoint, "https://worker2.example.com");

    // Sync again with only zone1 - zone2 should be disabled
    workers.remove("zone2");
    db::sync_workers(&db.pool, &workers)
        .await
        .expect("Failed to sync workers");

    // zone2 should now be disabled (not found)
    let worker2 = db::get_worker(&db.pool, "zone2")
        .await
        .expect("Failed to get worker");
    assert!(worker2.is_none());

    // zone1 should still exist
    let worker1 = db::get_worker(&db.pool, "zone1")
        .await
        .expect("Failed to get worker");
    assert!(worker1.is_some());
}

// ==================== Authorization Tests ====================

#[tokio::test]
async fn test_authorized_org_crud() {
    let db = TestDatabase::new().await;

    let zones = vec!["production".to_string(), "staging".to_string()];
    let domain_patterns = vec!["*.example.com".to_string(), "example.com".to_string()];

    // Create authorized org
    let org = db::upsert_authorized_org(&db.pool, "testorg", &zones, &domain_patterns)
        .await
        .expect("Failed to create authorized org");

    assert_eq!(org.github_org, "testorg");
    assert_eq!(org.zones, zones);
    assert_eq!(org.domain_patterns, domain_patterns);
    assert!(org.enabled);

    // Get authorized org
    let fetched = db::get_authorized_org(&db.pool, "testorg")
        .await
        .expect("Failed to get authorized org")
        .expect("Org not found");

    assert_eq!(fetched.github_org, "testorg");
    assert_eq!(fetched.zones, zones);
}

#[tokio::test]
async fn test_authorized_org_case_insensitive() {
    let db = TestDatabase::new().await;

    let zones = vec!["production".to_string()];
    let domain_patterns = vec!["*.example.com".to_string()];

    // Create with lowercase
    db::upsert_authorized_org(&db.pool, "MyOrg", &zones, &domain_patterns)
        .await
        .expect("Failed to create authorized org");

    // Fetch with different case
    let fetched = db::get_authorized_org(&db.pool, "myorg")
        .await
        .expect("Failed to get authorized org");
    assert!(fetched.is_some());

    let fetched = db::get_authorized_org(&db.pool, "MYORG")
        .await
        .expect("Failed to get authorized org");
    assert!(fetched.is_some());
}

#[tokio::test]
async fn test_authorized_org_not_found() {
    let db = TestDatabase::new().await;

    let fetched = db::get_authorized_org(&db.pool, "nonexistent")
        .await
        .expect("Failed to query authorized org");

    assert!(fetched.is_none());
}

#[tokio::test]
async fn test_authorized_org_update() {
    let db = TestDatabase::new().await;

    // Create initial
    let zones1 = vec!["production".to_string()];
    let domains1 = vec!["*.example.com".to_string()];
    db::upsert_authorized_org(&db.pool, "testorg", &zones1, &domains1)
        .await
        .expect("Failed to create authorized org");

    // Update with new values
    let zones2 = vec!["production".to_string(), "staging".to_string()];
    let domains2 = vec!["*.example.com".to_string(), "*.test.com".to_string()];
    let updated = db::upsert_authorized_org(&db.pool, "testorg", &zones2, &domains2)
        .await
        .expect("Failed to update authorized org");

    assert_eq!(updated.zones, zones2);
    assert_eq!(updated.domain_patterns, domains2);
}

#[tokio::test]
async fn test_authorized_org_delete() {
    let db = TestDatabase::new().await;

    let zones = vec!["production".to_string()];
    let domains = vec!["*.example.com".to_string()];

    // Create
    db::upsert_authorized_org(&db.pool, "testorg", &zones, &domains)
        .await
        .expect("Failed to create authorized org");

    // Delete
    let deleted = db::delete_authorized_org(&db.pool, "testorg")
        .await
        .expect("Failed to delete authorized org");
    assert!(deleted);

    // Should not be found (disabled)
    let fetched = db::get_authorized_org(&db.pool, "testorg")
        .await
        .expect("Failed to query authorized org");
    assert!(fetched.is_none());
}

#[tokio::test]
async fn test_list_authorized_orgs() {
    let db = TestDatabase::new().await;

    // Create multiple orgs
    db::upsert_authorized_org(
        &db.pool,
        "org1",
        &vec!["zone1".to_string()],
        &vec!["*.org1.com".to_string()],
    )
    .await
    .expect("Failed to create org1");

    db::upsert_authorized_org(
        &db.pool,
        "org2",
        &vec!["zone2".to_string()],
        &vec!["*.org2.com".to_string()],
    )
    .await
    .expect("Failed to create org2");

    // List all
    let orgs = db::list_authorized_orgs(&db.pool)
        .await
        .expect("Failed to list authorized orgs");

    assert_eq!(orgs.len(), 2);
}
