use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

const CADDY_READY_TIMEOUT: Duration = Duration::from_secs(60);
const CADDY_READY_INTERVAL: Duration = Duration::from_millis(500);

/// Wait for Caddy admin API to be ready
///
/// Polls the Caddy admin API until it responds or timeout is reached.
/// This should be called before attempting to restore routes on startup.
pub async fn wait_for_caddy_ready(
    http_client: &reqwest::Client,
    caddy_admin_api: &str,
) -> Result<()> {
    let start = std::time::Instant::now();
    let url = format!("{}/config/", caddy_admin_api);

    tracing::info!(
        caddy_admin_api = caddy_admin_api,
        timeout_secs = CADDY_READY_TIMEOUT.as_secs(),
        "Waiting for Caddy to be ready"
    );

    loop {
        match http_client.get(&url).send().await {
            Ok(response) if response.status().is_success() => {
                tracing::info!(
                    elapsed_ms = start.elapsed().as_millis() as u64,
                    "Caddy admin API is ready"
                );
                return Ok(());
            }
            Ok(response) => {
                tracing::debug!(
                    status = %response.status(),
                    "Caddy not ready yet (unexpected status)"
                );
            }
            Err(e) => {
                tracing::debug!(error = %e, "Caddy not ready yet");
            }
        }

        if start.elapsed() >= CADDY_READY_TIMEOUT {
            anyhow::bail!(
                "Caddy admin API not ready after {:?}",
                CADDY_READY_TIMEOUT
            );
        }

        tokio::time::sleep(CADDY_READY_INTERVAL).await;
    }
}

/// Configure a Caddy route for a deployment via the admin API
///
/// The domain is already fully resolved by central server (includes PR subdomain if applicable),
/// so we use it directly as the hostname.
pub async fn configure_caddy_route(
    http_client: &reqwest::Client,
    caddy_admin_api: &str,
    site_id: &str,
    site_dir: &Path,
    domain: &str,
) -> Result<()> {
    // Domain is already the full hostname (resolved by central server)
    let hostname = domain;

    // Build the route configuration
    let route = CaddyRoute {
        id: site_id.to_string(),
        match_rules: vec![CaddyMatch {
            host: vec![hostname.to_string()],
        }],
        handle: vec![CaddyHandler::FileServer {
            root: site_dir.to_string_lossy().to_string(),
            index_names: vec!["index.html".to_string()],
        }],
        terminal: true,
    };

    // First, try to delete any existing route with this ID
    let _ = remove_caddy_route(http_client, caddy_admin_api, site_id).await;

    // Find the position to insert (before any catch-all route)
    let insert_index = find_catch_all_index(http_client, caddy_admin_api).await?;

    // Add the route (PUT to insert at index, or POST to append)
    add_caddy_route(http_client, caddy_admin_api, &route, insert_index).await?;

    tracing::info!(
        site_id = site_id,
        hostname = hostname,
        site_dir = %site_dir.display(),
        insert_index = ?insert_index,
        "Configured Caddy route"
    );

    Ok(())
}

/// Find the index of a catch-all route (one without match rules)
/// Returns None if no catch-all is found (append to end)
async fn find_catch_all_index(
    http_client: &reqwest::Client,
    caddy_admin_api: &str,
) -> Result<Option<usize>> {
    let url = format!("{}/config/apps/http/servers/main/routes", caddy_admin_api);

    let response = http_client
        .get(&url)
        .send()
        .await
        .context("Failed to get Caddy routes")?;

    if !response.status().is_success() {
        return Ok(None);
    }

    let routes: Vec<serde_json::Value> = response
        .json()
        .await
        .context("Failed to parse Caddy routes")?;

    // Find first route without a "match" field (catch-all)
    for (idx, route) in routes.iter().enumerate() {
        if route.get("match").is_none() {
            tracing::debug!(index = idx, "Found catch-all route");
            return Ok(Some(idx));
        }
    }

    Ok(None)
}

/// Add a route to Caddy, inserting before catch-all if one exists
///
/// Uses PUT to insert at a specific index (Caddy API: PUT to /routes/N inserts at N)
/// or POST to append if no catch-all exists.
async fn add_caddy_route(
    http_client: &reqwest::Client,
    caddy_admin_api: &str,
    route: &CaddyRoute,
    insert_index: Option<usize>,
) -> Result<()> {
    let (method, url) = match insert_index {
        Some(idx) => (
            reqwest::Method::PUT,
            format!(
                "{}/config/apps/http/servers/main/routes/{}",
                caddy_admin_api, idx
            ),
        ),
        None => (
            reqwest::Method::POST,
            format!("{}/config/apps/http/servers/main/routes", caddy_admin_api),
        ),
    };

    let response = http_client
        .request(method.clone(), &url)
        .json(route)
        .send()
        .await
        .context("Failed to add Caddy route")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Caddy API error {}: {}", status, body);
    }

    tracing::info!(
        method = %method,
        insert_index = ?insert_index,
        "Added route to Caddy"
    );

    Ok(())
}

/// Remove a Caddy route by ID
///
/// Uses Caddy's /id/ endpoint which allows direct access to objects by their @id field.
/// This is cleaner than traversing the config path since routes are stored in an array.
pub async fn remove_caddy_route(
    http_client: &reqwest::Client,
    caddy_admin_api: &str,
    site_id: &str,
) -> Result<()> {
    // Use /id/{id} endpoint to delete by @id field directly
    let url = format!("{}/id/{}", caddy_admin_api, site_id);

    let response = http_client
        .delete(&url)
        .send()
        .await
        .context("Failed to remove Caddy route")?;

    // 404 is fine - route may not exist
    if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND {
        tracing::info!(site_id = site_id, "Removed Caddy route");
        Ok(())
    } else {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Caddy API error {}: {}", status, body)
    }
}

/// Caddy route configuration
#[derive(Debug, Serialize, Deserialize)]
struct CaddyRoute {
    #[serde(rename = "@id")]
    id: String,
    #[serde(rename = "match")]
    match_rules: Vec<CaddyMatch>,
    handle: Vec<CaddyHandler>,
    terminal: bool,
}

/// Caddy match rules
#[derive(Debug, Serialize, Deserialize)]
struct CaddyMatch {
    host: Vec<String>,
}

/// Caddy handlers
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "handler", rename_all = "snake_case")]
enum CaddyHandler {
    FileServer {
        root: String,
        index_names: Vec<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_caddy_route_serialization() {
        let route = CaddyRoute {
            id: "test-site".to_string(),
            match_rules: vec![CaddyMatch {
                host: vec!["pr-42-website.example.com".to_string()],
            }],
            handle: vec![CaddyHandler::FileServer {
                root: "/var/www/sites/test-site".to_string(),
                index_names: vec!["index.html".to_string()],
            }],
            terminal: true,
        };

        let json = serde_json::to_string_pretty(&route).unwrap();
        assert!(json.contains("@id"));
        assert!(json.contains("pr-42-website.example.com"));
        assert!(json.contains("file_server"));
    }
}
