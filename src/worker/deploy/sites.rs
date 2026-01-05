//! Site metadata and route restoration
//!
//! Manages persistent metadata for deployed sites and restores Caddy routes on startup.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use super::caddy::configure_caddy_route;

/// Metadata stored with each deployed site
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteMetadata {
    /// Site identifier (e.g., "nullislabs-website-pr-42")
    pub site_id: String,
    /// Full domain for this site (e.g., "pr-42-website.nxm.rs")
    pub domain: String,
}

const METADATA_FILE: &str = ".catapult.json";

/// Write site metadata to the site directory
pub async fn write_site_metadata(site_dir: &Path, metadata: &SiteMetadata) -> Result<()> {
    let metadata_path = site_dir.join(METADATA_FILE);
    let content =
        serde_json::to_string_pretty(metadata).context("Failed to serialize site metadata")?;

    tokio::fs::write(&metadata_path, content)
        .await
        .context("Failed to write site metadata")?;

    tracing::debug!(
        site_id = %metadata.site_id,
        domain = %metadata.domain,
        "Wrote site metadata"
    );

    Ok(())
}

/// Read site metadata from a site directory
pub async fn read_site_metadata(site_dir: &Path) -> Result<Option<SiteMetadata>> {
    let metadata_path = site_dir.join(METADATA_FILE);

    if !metadata_path.exists() {
        return Ok(None);
    }

    let content = tokio::fs::read_to_string(&metadata_path)
        .await
        .context("Failed to read site metadata")?;

    let metadata: SiteMetadata =
        serde_json::from_str(&content).context("Failed to parse site metadata")?;

    Ok(Some(metadata))
}

/// Restore all Caddy routes from existing site deployments
///
/// Scans the sites directory and configures Caddy routes for all sites
/// that have metadata files. This should be called on worker startup.
pub async fn restore_all_routes(
    http_client: &reqwest::Client,
    caddy_admin_api: &str,
    sites_dir: &Path,
) -> Result<usize> {
    if !sites_dir.exists() {
        tracing::debug!(sites_dir = %sites_dir.display(), "Sites directory doesn't exist, nothing to restore");
        return Ok(0);
    }

    let mut restored = 0;
    let mut entries = tokio::fs::read_dir(sites_dir)
        .await
        .context("Failed to read sites directory")?;

    while let Some(entry) = entries.next_entry().await? {
        let site_dir = entry.path();

        // Skip non-directories
        if !site_dir.is_dir() {
            continue;
        }

        // Try to read metadata
        match read_site_metadata(&site_dir).await {
            Ok(Some(metadata)) => {
                tracing::info!(
                    site_id = %metadata.site_id,
                    domain = %metadata.domain,
                    "Restoring Caddy route"
                );

                match configure_caddy_route(
                    http_client,
                    caddy_admin_api,
                    &metadata.site_id,
                    &site_dir,
                    &metadata.domain,
                )
                .await
                {
                    Ok(()) => {
                        restored += 1;
                        tracing::info!(
                            site_id = %metadata.site_id,
                            domain = %metadata.domain,
                            "Restored Caddy route"
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            site_id = %metadata.site_id,
                            error = %e,
                            "Failed to restore Caddy route"
                        );
                    }
                }
            }
            Ok(None) => {
                // No metadata file - this might be a manually created directory
                // or from before metadata was implemented
                let dir_name = site_dir.file_name().unwrap_or_default().to_string_lossy();
                tracing::debug!(
                    site_dir = %dir_name,
                    "No metadata file found, skipping"
                );
            }
            Err(e) => {
                let dir_name = site_dir.file_name().unwrap_or_default().to_string_lossy();
                tracing::warn!(
                    site_dir = %dir_name,
                    error = %e,
                    "Failed to read site metadata"
                );
            }
        }
    }

    tracing::info!(count = restored, "Route restoration complete");

    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_write_and_read_metadata() {
        let dir = tempdir().unwrap();
        let site_dir = dir.path();

        let metadata = SiteMetadata {
            site_id: "test-site-pr-42".to_string(),
            domain: "pr-42-test.example.com".to_string(),
        };

        // Write metadata
        write_site_metadata(site_dir, &metadata).await.unwrap();

        // Read it back
        let read_back = read_site_metadata(site_dir).await.unwrap().unwrap();
        assert_eq!(read_back.site_id, metadata.site_id);
        assert_eq!(read_back.domain, metadata.domain);
    }

    #[tokio::test]
    async fn test_read_missing_metadata() {
        let dir = tempdir().unwrap();
        let result = read_site_metadata(dir.path()).await.unwrap();
        assert!(result.is_none());
    }
}
