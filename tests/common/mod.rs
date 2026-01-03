//! Common test utilities and fixtures

use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres;

/// Test database container with connection pool
pub struct TestDatabase {
    pub pool: PgPool,
    _container: ContainerAsync<Postgres>,
}

impl TestDatabase {
    /// Start a PostgreSQL container and run migrations
    pub async fn new() -> Self {
        let container = Postgres::default()
            .start()
            .await
            .expect("Failed to start postgres container");

        let host = container.get_host().await.expect("Failed to get host");
        let port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get port");

        let database_url = format!(
            "postgres://postgres:postgres@{}:{}/postgres",
            host, port
        );

        // Create connection pool
        let pool = PgPool::connect(&database_url)
            .await
            .expect("Failed to connect to database");

        // Run migrations
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("Failed to run migrations");

        Self {
            pool,
            _container: container,
        }
    }

    /// Create a test worker in the database (needed for FK constraint)
    pub async fn create_test_worker(&self, environment: &str) {
        sqlx::query(
            r#"
            INSERT INTO workers (environment, endpoint, enabled)
            VALUES ($1, $2, $3)
            ON CONFLICT (environment) DO NOTHING
            "#,
        )
        .bind(environment)
        .bind(format!("https://worker.{}.example.com", environment))
        .bind(true)
        .execute(&self.pool)
        .await
        .expect("Failed to create test worker");
    }
}

/// Create a test HTTP client for API testing
pub fn test_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("Failed to create HTTP client")
}

/// Generate a random test secret
pub fn random_secret() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("test-secret-{}", timestamp)
}
