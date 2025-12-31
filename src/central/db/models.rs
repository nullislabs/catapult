use chrono::{DateTime, Utc};
use sqlx::FromRow;

use crate::shared::SiteType;

/// Worker registration record
#[derive(Debug, Clone, FromRow)]
pub struct Worker {
    pub id: i32,
    pub environment: String,
    pub endpoint: String,
    pub enabled: bool,
    pub last_seen: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Deployment configuration for a repository
#[derive(Debug, Clone, FromRow)]
pub struct DeploymentConfig {
    pub id: i32,
    pub github_org: String,
    pub github_repo: String,
    pub environment: String,
    pub domain: String,
    pub subdomain: Option<String>,
    pub site_type: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl DeploymentConfig {
    /// Parse the site_type string into a SiteType enum
    pub fn site_type(&self) -> SiteType {
        self.site_type.parse().unwrap_or_default()
    }
}

/// Deployment history record
#[derive(Debug, Clone, FromRow)]
pub struct DeploymentHistory {
    pub id: i32,
    pub config_id: i32,
    pub deployment_type: String,
    pub pr_number: Option<i32>,
    pub branch: String,
    pub commit_sha: String,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub deployed_url: Option<String>,
    pub error_message: Option<String>,
    pub github_comment_id: Option<i64>,
}
