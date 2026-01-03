//! Integration tests for Central API endpoints

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::{get, post},
    Router,
};
use catapult::central::db;
use catapult::shared::{auth::sign_request, JobStatus, StatusUpdate};
use common::TestDatabase;
use tower::util::ServiceExt;
use uuid::Uuid;

/// Create a minimal test router for status updates
fn create_test_router(
    db: sqlx::PgPool,
    worker_secret: String,
) -> Router {
    use axum::extract::State;
    use axum::http::HeaderMap;
    use bytes::Bytes;

    #[derive(Clone)]
    struct TestState {
        db: sqlx::PgPool,
        worker_secret: String,
    }

    async fn handle_status(
        State(state): State<TestState>,
        headers: HeaderMap,
        body: Bytes,
    ) -> StatusCode {
        let signature = match headers.get("x-worker-signature") {
            Some(sig) => sig.to_str().unwrap_or_default(),
            None => return StatusCode::UNAUTHORIZED,
        };

        let timestamp: u64 = match headers.get("x-request-timestamp") {
            Some(ts) => ts.to_str().unwrap_or("0").parse().unwrap_or(0),
            None => return StatusCode::UNAUTHORIZED,
        };

        if !catapult::shared::auth::verify_signature(
            state.worker_secret.as_bytes(),
            &body,
            signature,
            timestamp,
        ) {
            return StatusCode::UNAUTHORIZED;
        }

        let status_update: StatusUpdate = match serde_json::from_slice(&body) {
            Ok(update) => update,
            Err(_) => return StatusCode::BAD_REQUEST,
        };

        // Look up and update deployment
        match db::get_deployment_by_job_id(&state.db, status_update.job_id).await {
            Ok(Some(deployment)) => {
                if db::update_deployment_status(
                    &state.db,
                    deployment.id,
                    status_update.status,
                    status_update.deployed_url.as_deref(),
                    status_update.error_message.as_deref(),
                )
                .await
                .is_ok()
                {
                    StatusCode::OK
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }
            Ok(None) => StatusCode::OK, // Job not found is OK (idempotent)
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    async fn health_check() -> &'static str {
        "OK"
    }

    let state = TestState { db, worker_secret };

    Router::new()
        .route("/api/status", post(handle_status))
        .route("/health", get(health_check))
        .with_state(state)
}

#[tokio::test]
async fn test_health_check() {
    let db = TestDatabase::new().await;
    let app = create_test_router(db.pool, "test-secret".to_string());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_status_update_missing_signature() {
    let db = TestDatabase::new().await;
    let app = create_test_router(db.pool, "test-secret".to_string());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/status")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_status_update_invalid_signature() {
    let db = TestDatabase::new().await;
    let app = create_test_router(db.pool, "test-secret".to_string());

    let body = serde_json::to_string(&StatusUpdate {
        job_id: Uuid::new_v4(),
        status: JobStatus::Success,
        deployed_url: None,
        error_message: None,
    })
    .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/status")
                .header("content-type", "application/json")
                .header("x-worker-signature", "sha256=invalid")
                .header("x-request-timestamp", "12345")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_status_update_valid_signature() {
    let db = TestDatabase::new().await;
    let secret = "test-secret";
    let app = create_test_router(db.pool.clone(), secret.to_string());

    // Create a worker first (FK constraint)
    db.create_test_worker("production").await;

    // Create a deployment config and deployment
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
    db::create_deployment(
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

    // Create signed status update
    let status_update = StatusUpdate {
        job_id,
        status: JobStatus::Success,
        deployed_url: Some("https://pr-42.example.com".to_string()),
        error_message: None,
    };
    let body = serde_json::to_vec(&status_update).unwrap();
    let (signature, timestamp) = sign_request(secret.as_bytes(), &body);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/status")
                .header("content-type", "application/json")
                .header("x-worker-signature", signature)
                .header("x-request-timestamp", timestamp.to_string())
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Verify deployment was updated
    let deployment = db::get_deployment_by_job_id(&db.pool, job_id)
        .await
        .expect("Failed to get deployment")
        .expect("Deployment not found");

    assert_eq!(deployment.status, "success");
    assert_eq!(
        deployment.deployed_url,
        Some("https://pr-42.example.com".to_string())
    );
}

#[tokio::test]
async fn test_status_update_nonexistent_job() {
    let db = TestDatabase::new().await;
    let secret = "test-secret";
    let app = create_test_router(db.pool, secret.to_string());

    // Create status update for non-existent job
    let status_update = StatusUpdate {
        job_id: Uuid::new_v4(), // Random job that doesn't exist
        status: JobStatus::Success,
        deployed_url: Some("https://example.com".to_string()),
        error_message: None,
    };
    let body = serde_json::to_vec(&status_update).unwrap();
    let (signature, timestamp) = sign_request(secret.as_bytes(), &body);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/status")
                .header("content-type", "application/json")
                .header("x-worker-signature", signature)
                .header("x-request-timestamp", timestamp.to_string())
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should still return OK (idempotent)
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_status_update_failure() {
    let db = TestDatabase::new().await;
    let secret = "test-secret";
    let app = create_test_router(db.pool.clone(), secret.to_string());

    // Create a worker first (FK constraint)
    db.create_test_worker("production").await;

    // Create deployment
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
    db::create_deployment(
        &db.pool,
        config_id,
        job_id,
        "main",
        None,
        "main",
        "abc123",
    )
    .await
    .expect("Failed to create deployment");

    // Send failure status
    let status_update = StatusUpdate {
        job_id,
        status: JobStatus::Failed,
        deployed_url: None,
        error_message: Some("Build failed: npm install error".to_string()),
    };
    let body = serde_json::to_vec(&status_update).unwrap();
    let (signature, timestamp) = sign_request(secret.as_bytes(), &body);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/status")
                .header("content-type", "application/json")
                .header("x-worker-signature", signature)
                .header("x-request-timestamp", timestamp.to_string())
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Verify deployment was updated
    let deployment = db::get_deployment_by_job_id(&db.pool, job_id)
        .await
        .expect("Failed to get deployment")
        .expect("Deployment not found");

    assert_eq!(deployment.status, "failed");
    assert_eq!(
        deployment.error_message,
        Some("Build failed: npm install error".to_string())
    );
}
