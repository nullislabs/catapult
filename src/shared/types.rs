use derive_more::Display;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Build job dispatched from Central to Worker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildJob {
    /// Unique job identifier
    pub job_id: Uuid,

    /// GitHub repository clone URL
    pub repo_url: String,

    /// GitHub installation access token for cloning
    pub git_token: String,

    /// Branch to build
    pub branch: String,

    /// Commit SHA to checkout
    pub commit_sha: String,

    /// PR number (None for main branch deployments)
    pub pr_number: Option<u32>,

    /// Domain for the deployment (e.g., "example.com")
    pub domain: String,

    /// Site type for build configuration
    pub site_type: SiteType,

    /// URL to POST status updates to
    pub callback_url: String,

    /// Repository name (e.g., "website")
    pub repo_name: String,

    /// Organization/user name (e.g., "nullisLabs")
    pub org_name: String,

    /// Subdomain for main branch deployment (None for PR deployments)
    pub subdomain: Option<String>,
}

/// Cleanup job dispatched from Central to Worker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupJob {
    /// Unique job identifier
    pub job_id: Uuid,

    /// Site identifier to clean up (e.g., "nullislabs-website-pr-42")
    pub site_id: String,

    /// URL to POST status updates to
    pub callback_url: String,
}

/// Status update sent from Worker to Central
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusUpdate {
    /// Job identifier this status is for
    pub job_id: Uuid,

    /// Current status
    pub status: JobStatus,

    /// Deployed URL (if successful)
    pub deployed_url: Option<String>,

    /// Error message (if failed)
    pub error_message: Option<String>,
}

/// Job status values
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Job received, starting
    #[display("pending")]
    Pending,
    /// Build in progress
    #[display("building")]
    Building,
    /// Build and deployment successful
    #[display("success")]
    Success,
    /// Build or deployment failed
    #[display("failed")]
    Failed,
    /// PR deployment cleaned up
    #[display("cleaned")]
    Cleaned,
}

/// Build/site type configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Display)]
#[serde(rename_all = "snake_case")]
pub enum SiteType {
    /// SvelteKit application
    #[display("sveltekit")]
    SvelteKit,
    /// Vite-based application
    #[display("vite")]
    Vite,
    /// Zola static site generator
    #[display("zola")]
    Zola,
    /// Custom build (uses repo's flake.nix)
    #[display("custom")]
    Custom,
    /// Auto-detect based on repo contents
    #[default]
    #[display("auto")]
    Auto,
}

impl SiteType {
    /// Get the default build command for this site type
    pub fn default_build_command(&self) -> Option<&'static str> {
        match self {
            SiteType::SvelteKit => Some("npm ci && npm run build"),
            SiteType::Vite => Some("npm ci && npm run build"),
            SiteType::Zola => Some("zola build"),
            SiteType::Custom => None,
            SiteType::Auto => None,
        }
    }

    /// Get the default output directory for this site type
    pub fn default_output_dir(&self) -> Option<&'static str> {
        match self {
            SiteType::SvelteKit => Some("build"),
            SiteType::Vite => Some("dist"),
            SiteType::Zola => Some("public"),
            SiteType::Custom => None,
            SiteType::Auto => None,
        }
    }

    /// Get the Nix flake reference for this site type
    pub fn flake_ref(&self) -> Option<&'static str> {
        match self {
            SiteType::SvelteKit => Some("github:nullisLabs/catapult#sveltekit"),
            SiteType::Vite => Some("github:nullisLabs/catapult#vite"),
            SiteType::Zola => Some("github:nullisLabs/catapult#zola"),
            SiteType::Custom => None,
            SiteType::Auto => None,
        }
    }
}

impl std::str::FromStr for SiteType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "sveltekit" => Ok(SiteType::SvelteKit),
            "vite" => Ok(SiteType::Vite),
            "zola" => Ok(SiteType::Zola),
            "custom" => Ok(SiteType::Custom),
            "auto" => Ok(SiteType::Auto),
            _ => Err(format!("Unknown site type: {}", s)),
        }
    }
}

/// Repository deployment configuration (from .deploy.json)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeployConfig {
    /// Build type override
    #[serde(default)]
    pub build_type: Option<SiteType>,

    /// Custom build command
    #[serde(default)]
    pub build_command: Option<String>,

    /// Output directory override
    #[serde(default)]
    pub output_dir: Option<String>,
}

/// Generate a site ID for a deployment
pub fn generate_site_id(org: &str, repo: &str, pr_number: Option<u32>) -> String {
    match pr_number {
        Some(pr) => format!("{}-{}-pr-{}", org.to_lowercase(), repo.to_lowercase(), pr),
        None => format!("{}-{}-main", org.to_lowercase(), repo.to_lowercase()),
    }
}

/// Generate the preview URL for a deployment
pub fn generate_preview_url(domain: &str, repo: &str, pr_number: Option<u32>) -> String {
    match pr_number {
        Some(pr) => format!("https://pr-{}-{}.{}", pr, repo.to_lowercase(), domain),
        None => format!("https://{}", domain),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_site_id() {
        assert_eq!(
            generate_site_id("NullisLabs", "Website", Some(42)),
            "nullislabs-website-pr-42"
        );
        assert_eq!(
            generate_site_id("NullisLabs", "Website", None),
            "nullislabs-website-main"
        );
    }

    #[test]
    fn test_generate_preview_url() {
        assert_eq!(
            generate_preview_url("example.com", "Website", Some(42)),
            "https://pr-42-website.example.com"
        );
        assert_eq!(
            generate_preview_url("example.com", "Website", None),
            "https://example.com"
        );
    }

    #[test]
    fn test_site_type_from_str() {
        assert_eq!("sveltekit".parse::<SiteType>().unwrap(), SiteType::SvelteKit);
        assert_eq!("VITE".parse::<SiteType>().unwrap(), SiteType::Vite);
        assert!("unknown".parse::<SiteType>().is_err());
    }
}
