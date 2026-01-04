//! Deploy configuration fetching from GitHub repositories
//!
//! Fetches and merges `.deploy.json` files from:
//! 1. Organization defaults: `{org}/.github/.deploy.json`
//! 2. Repository overrides: `{org}/{repo}/.deploy.json`

use anyhow::{Context, Result};
use base64::Engine;

use crate::shared::DeployConfig;

/// Fetch and merge deploy configuration for a repository
///
/// Tries to fetch configuration from:
/// 1. `{org}/.github/.deploy.json` - Organization defaults
/// 2. `{org}/{repo}/.deploy.json` - Repository-specific overrides
///
/// Returns merged config, or None if neither file exists.
pub async fn fetch_deploy_config(
    http_client: &reqwest::Client,
    token: &str,
    org: &str,
    repo: &str,
) -> Result<Option<DeployConfig>> {
    // Fetch org-level defaults from .github repo
    let org_config = fetch_config_file(http_client, token, org, ".github", ".deploy.json").await?;

    // Fetch repo-level overrides
    let repo_config = fetch_config_file(http_client, token, org, repo, ".deploy.json").await?;

    // Merge configs
    match (org_config, repo_config) {
        (None, None) => Ok(None),
        (Some(org), None) => Ok(Some(org)),
        (None, Some(repo)) => Ok(Some(repo)),
        (Some(mut org), Some(repo)) => {
            org.merge(&repo);
            Ok(Some(org))
        }
    }
}

/// Fetch a single config file from a GitHub repository
async fn fetch_config_file(
    http_client: &reqwest::Client,
    token: &str,
    org: &str,
    repo: &str,
    path: &str,
) -> Result<Option<DeployConfig>> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/contents/{}",
        org, repo, path
    );

    let response = http_client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "catapult")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .context("Failed to fetch config file from GitHub")?;

    // 404 means file doesn't exist - that's OK
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        tracing::debug!(org, repo, path, "No .deploy.json found");
        return Ok(None);
    }

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("GitHub API error {}: {}", status, body);
    }

    // Parse the contents response
    let content_response: GitHubContentResponse = response
        .json()
        .await
        .context("Failed to parse GitHub content response")?;

    // Decode base64 content
    let content_bytes = base64::engine::general_purpose::STANDARD
        .decode(content_response.content.replace('\n', ""))
        .context("Failed to decode base64 content")?;

    let content = String::from_utf8(content_bytes)
        .context("Config file is not valid UTF-8")?;

    // Parse JSON
    let config: DeployConfig = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse .deploy.json in {}/{}", org, repo))?;

    tracing::debug!(org, repo, path, "Loaded .deploy.json");

    Ok(Some(config))
}

#[derive(Debug, serde::Deserialize)]
struct GitHubContentResponse {
    content: String,
    #[allow(dead_code)]
    encoding: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deploy_config_merge() {
        let mut org_config = DeployConfig {
            zone: Some("nxm".to_string()),
            domain_pattern: Some("{repo}.nxm.rs".to_string()),
            pr_pattern: Some("pr-{pr}-{repo}.nxm.rs".to_string()),
            ..Default::default()
        };

        let repo_config = DeployConfig {
            domain: Some("nxm.rs".to_string()),
            subdomain: Some("www".to_string()),
            build_type: Some(crate::shared::SiteType::SvelteKit),
            ..Default::default()
        };

        org_config.merge(&repo_config);

        // Org values preserved
        assert_eq!(org_config.zone, Some("nxm".to_string()));
        assert_eq!(org_config.domain_pattern, Some("{repo}.nxm.rs".to_string()));

        // Repo overrides applied
        assert_eq!(org_config.domain, Some("nxm.rs".to_string()));
        assert_eq!(org_config.subdomain, Some("www".to_string()));
        assert_eq!(org_config.build_type, Some(crate::shared::SiteType::SvelteKit));
    }

    #[test]
    fn test_resolve_domain() {
        let config = DeployConfig {
            domain_pattern: Some("{repo}.nxm.rs".to_string()),
            ..Default::default()
        };

        assert_eq!(config.resolve_domain("website"), Some("website.nxm.rs".to_string()));
        assert_eq!(config.resolve_domain("MyRepo"), Some("myrepo.nxm.rs".to_string()));
    }

    #[test]
    fn test_resolve_domain_explicit_override() {
        let config = DeployConfig {
            domain: Some("nxm.rs".to_string()),
            domain_pattern: Some("{repo}.nxm.rs".to_string()),
            ..Default::default()
        };

        // Explicit domain takes precedence
        assert_eq!(config.resolve_domain("website"), Some("nxm.rs".to_string()));
    }

    #[test]
    fn test_resolve_pr_domain() {
        let config = DeployConfig {
            pr_pattern: Some("pr-{pr}-{repo}.preview.nxm.rs".to_string()),
            ..Default::default()
        };

        assert_eq!(
            config.resolve_pr_domain("website", 42),
            Some("pr-42-website.preview.nxm.rs".to_string())
        );
    }

    #[test]
    fn test_resolve_pr_domain_fallback() {
        let config = DeployConfig {
            domain: Some("nxm.rs".to_string()),
            ..Default::default()
        };

        // Falls back to default pattern
        assert_eq!(
            config.resolve_pr_domain("website", 42),
            Some("pr-42-website.nxm.rs".to_string())
        );
    }

    #[test]
    fn test_is_deployable() {
        let mut config = DeployConfig::default();
        assert!(!config.is_deployable()); // No zone

        config.zone = Some("nxm".to_string());
        assert!(config.is_deployable());

        config.enabled = false;
        assert!(!config.is_deployable()); // Disabled
    }
}
