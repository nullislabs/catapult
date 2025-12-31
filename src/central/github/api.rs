use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// GitHub API client for interacting with repositories
pub struct GitHubClient {
    http_client: reqwest::Client,
    token: String,
}

#[derive(Debug, Serialize)]
struct CreateCommentRequest {
    body: String,
}

#[derive(Debug, Deserialize)]
pub struct CommentResponse {
    pub id: i64,
}

impl GitHubClient {
    /// Create a new GitHub client with an installation access token
    pub fn new(token: String) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            token,
        }
    }

    /// Create a comment on a pull request
    pub async fn create_pr_comment(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u32,
        body: &str,
    ) -> Result<CommentResponse> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}/comments",
            owner, repo, pr_number
        );

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "catapult")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&CreateCommentRequest {
                body: body.to_string(),
            })
            .send()
            .await
            .context("Failed to create PR comment")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        response
            .json()
            .await
            .context("Failed to parse comment response")
    }

    /// Update an existing comment
    pub async fn update_comment(
        &self,
        owner: &str,
        repo: &str,
        comment_id: i64,
        body: &str,
    ) -> Result<()> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/comments/{}",
            owner, repo, comment_id
        );

        let response = self
            .http_client
            .patch(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "catapult")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&CreateCommentRequest {
                body: body.to_string(),
            })
            .send()
            .await
            .context("Failed to update comment")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        Ok(())
    }

    /// Generate a "Building..." comment body
    pub fn building_comment(commit_sha: &str) -> String {
        format!(
            "🚀 **Deployment in progress**\n\n\
             Building commit `{}`...\n\n\
             _This comment will be updated when the deployment completes._",
            &commit_sha[..7.min(commit_sha.len())]
        )
    }

    /// Generate a success comment body
    pub fn success_comment(commit_sha: &str, deployed_url: &str) -> String {
        format!(
            "✅ **Deployment successful**\n\n\
             Commit `{}` has been deployed.\n\n\
             🔗 **Preview URL:** {}\n\n\
             _This deployment will be automatically cleaned up when the PR is closed._",
            &commit_sha[..7.min(commit_sha.len())],
            deployed_url
        )
    }

    /// Generate a failure comment body
    pub fn failure_comment(commit_sha: &str, error: &str) -> String {
        format!(
            "❌ **Deployment failed**\n\n\
             Failed to deploy commit `{}`.\n\n\
             **Error:**\n```\n{}\n```\n\n\
             _Please check the build logs for more details._",
            &commit_sha[..7.min(commit_sha.len())],
            error
        )
    }
}
