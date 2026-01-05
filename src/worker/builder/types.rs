use crate::shared::{DeployConfig, SiteType};

/// Build context with resolved configuration
#[derive(Debug)]
pub struct BuildContext {
    /// Resolved site type
    pub site_type: SiteType,

    /// Build command to execute
    pub build_command: String,

    /// Output directory containing build artifacts
    pub output_dir: String,

    /// Nix flake reference for the build environment
    pub flake_ref: Option<String>,
}

impl BuildContext {
    /// Create a build context from a site type and optional deploy config
    pub fn new(site_type: SiteType, deploy_config: Option<DeployConfig>) -> Self {
        let deploy_config = deploy_config.unwrap_or_default();

        // Resolve site type (deploy config can override)
        let resolved_type = deploy_config.build_type.unwrap_or(site_type);

        // Resolve build command
        let build_command = deploy_config
            .build_command
            .or_else(|| resolved_type.default_build_command().map(String::from))
            .unwrap_or_else(|| "echo 'No build command specified'".to_string());

        // Resolve output directory
        let output_dir = deploy_config
            .output_dir
            .or_else(|| resolved_type.default_output_dir().map(String::from))
            .unwrap_or_else(|| "dist".to_string());

        // Get flake reference
        let flake_ref = resolved_type.flake_ref().map(String::from);

        Self {
            site_type: resolved_type,
            build_command,
            output_dir,
            flake_ref,
        }
    }
}

/// Try to auto-detect the site type from repository contents
pub async fn detect_site_type(repo_dir: &std::path::Path) -> SiteType {
    // Check for SvelteKit
    if repo_dir.join("svelte.config.js").exists() || repo_dir.join("svelte.config.ts").exists() {
        return SiteType::SvelteKit;
    }

    // Check for Vite
    if repo_dir.join("vite.config.js").exists() || repo_dir.join("vite.config.ts").exists() {
        return SiteType::Vite;
    }

    // Check for Zola
    if repo_dir.join("config.toml").exists()
        && let Ok(contents) = tokio::fs::read_to_string(repo_dir.join("config.toml")).await
            && contents.contains("base_url") && contents.contains("[markdown]") {
                return SiteType::Zola;
            }

    // Check for custom flake
    if repo_dir.join("flake.nix").exists() {
        return SiteType::Custom;
    }

    // Fallback - if there's a package.json, assume Vite
    if repo_dir.join("package.json").exists() {
        return SiteType::Vite;
    }

    SiteType::Auto
}

/// Load .deploy.json from repository if it exists
pub async fn load_deploy_config(repo_dir: &std::path::Path) -> Option<DeployConfig> {
    let config_path = repo_dir.join(".deploy.json");

    if !config_path.exists() {
        return None;
    }

    match tokio::fs::read_to_string(&config_path).await {
        Ok(contents) => match serde_json::from_str(&contents) {
            Ok(config) => Some(config),
            Err(e) => {
                tracing::warn!(error = %e, "Failed to parse .deploy.json");
                None
            }
        },
        Err(e) => {
            tracing::warn!(error = %e, "Failed to read .deploy.json");
            None
        }
    }
}
