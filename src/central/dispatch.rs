use anyhow::{Context, Result};

use crate::shared::{auth::sign_request, BuildJob, CleanupJob};

/// Dispatch a build job to a worker
pub async fn dispatch_build_job(
    http_client: &reqwest::Client,
    worker_endpoint: &str,
    shared_secret: &str,
    job: &BuildJob,
) -> Result<()> {
    let url = format!("{}/build", worker_endpoint);
    let body = serde_json::to_vec(job).context("Failed to serialize build job")?;

    let (signature, timestamp) = sign_request(shared_secret.as_bytes(), &body);

    let response = http_client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("X-Central-Signature", signature)
        .header("X-Request-Timestamp", timestamp.to_string())
        .body(body)
        .send()
        .await
        .context("Failed to dispatch build job to worker")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Worker returned error {}: {}", status, body);
    }

    Ok(())
}

/// Dispatch a cleanup job to a worker
pub async fn dispatch_cleanup_job(
    http_client: &reqwest::Client,
    worker_endpoint: &str,
    shared_secret: &str,
    job: &CleanupJob,
) -> Result<()> {
    let url = format!("{}/cleanup", worker_endpoint);
    let body = serde_json::to_vec(job).context("Failed to serialize cleanup job")?;

    let (signature, timestamp) = sign_request(shared_secret.as_bytes(), &body);

    let response = http_client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("X-Central-Signature", signature)
        .header("X-Request-Timestamp", timestamp.to_string())
        .body(body)
        .send()
        .await
        .context("Failed to dispatch cleanup job to worker")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Worker returned error {}: {}", status, body);
    }

    Ok(())
}
