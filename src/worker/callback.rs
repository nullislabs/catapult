use anyhow::{Context, Result};

use crate::shared::{auth::sign_request, StatusUpdate};

/// Send a status update to Central
pub async fn send_status_update(
    http_client: &reqwest::Client,
    callback_url: &str,
    shared_secret: &str,
    status: StatusUpdate,
) -> Result<()> {
    let body = serde_json::to_vec(&status).context("Failed to serialize status update")?;

    let (signature, timestamp) = sign_request(shared_secret.as_bytes(), &body);

    let response = http_client
        .post(callback_url)
        .header("Content-Type", "application/json")
        .header("X-Worker-Signature", signature)
        .header("X-Request-Timestamp", timestamp.to_string())
        .body(body)
        .send()
        .await
        .context("Failed to send status update to Central")?;

    if !response.status().is_success() {
        let status_code = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Central returned error {}: {}", status_code, body);
    }

    Ok(())
}
