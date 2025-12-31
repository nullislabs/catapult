use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Configure a Caddy route for a deployment via the admin API
pub async fn configure_caddy_route(
    http_client: &reqwest::Client,
    caddy_admin_api: &str,
    site_id: &str,
    site_dir: &Path,
    domain: &str,
    repo_name: &str,
    pr_number: Option<u32>,
) -> Result<()> {
    // Generate the hostname for this deployment
    let hostname = match pr_number {
        Some(pr) => format!("pr-{}-{}.{}", pr, repo_name.to_lowercase(), domain),
        None => domain.to_string(),
    };

    // Build the route configuration
    let route = CaddyRoute {
        id: site_id.to_string(),
        match_rules: vec![CaddyMatch {
            host: vec![hostname.clone()],
        }],
        handle: vec![CaddyHandler::FileServer {
            root: site_dir.to_string_lossy().to_string(),
            index_names: vec!["index.html".to_string()],
        }],
        terminal: true,
    };

    // First, try to delete any existing route with this ID
    let _ = remove_caddy_route(http_client, caddy_admin_api, site_id).await;

    // Add the new route
    let url = format!("{}/config/apps/http/servers/srv0/routes", caddy_admin_api);

    let response = http_client
        .post(&url)
        .json(&route)
        .send()
        .await
        .context("Failed to add Caddy route")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Caddy API error {}: {}", status, body);
    }

    tracing::info!(
        site_id = site_id,
        hostname = hostname,
        site_dir = %site_dir.display(),
        "Configured Caddy route"
    );

    Ok(())
}

/// Remove a Caddy route by ID
pub async fn remove_caddy_route(
    http_client: &reqwest::Client,
    caddy_admin_api: &str,
    site_id: &str,
) -> Result<()> {
    let url = format!(
        "{}/config/apps/http/servers/srv0/routes/{}",
        caddy_admin_api, site_id
    );

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
