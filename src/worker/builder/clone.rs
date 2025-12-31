use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Clone a repository and checkout a specific commit
pub async fn clone_repository(
    repo_url: &str,
    token: &str,
    commit_sha: &str,
    work_dir: &Path,
) -> Result<PathBuf> {
    let repo_dir = work_dir.join("repo");

    // Insert token into URL for authentication
    let auth_url = insert_token_in_url(repo_url, token)?;

    // Clone with depth 1 for speed (we'll fetch the specific commit)
    let output = Command::new("git")
        .args(["clone", "--depth", "1", &auth_url, repo_dir.to_str().unwrap()])
        .current_dir(work_dir)
        .output()
        .await
        .context("Failed to execute git clone")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Sanitize error message to not include token
        let sanitized = stderr.replace(token, "[REDACTED]");
        anyhow::bail!("git clone failed: {}", sanitized);
    }

    // Fetch the specific commit
    let output = Command::new("git")
        .args(["fetch", "origin", commit_sha, "--depth", "1"])
        .current_dir(&repo_dir)
        .output()
        .await
        .context("Failed to fetch commit")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let sanitized = stderr.replace(token, "[REDACTED]");
        anyhow::bail!("git fetch failed: {}", sanitized);
    }

    // Checkout the specific commit
    let output = Command::new("git")
        .args(["checkout", commit_sha])
        .current_dir(&repo_dir)
        .output()
        .await
        .context("Failed to checkout commit")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git checkout failed: {}", stderr);
    }

    tracing::info!(
        commit = commit_sha,
        repo_dir = %repo_dir.display(),
        "Repository cloned successfully"
    );

    Ok(repo_dir)
}

/// Insert authentication token into a GitHub URL
fn insert_token_in_url(url: &str, token: &str) -> Result<String> {
    // Handle HTTPS URLs: https://github.com/org/repo.git
    if let Some(rest) = url.strip_prefix("https://") {
        return Ok(format!("https://x-access-token:{}@{}", token, rest));
    }

    // Handle git:// URLs
    if let Some(rest) = url.strip_prefix("git://") {
        return Ok(format!("https://x-access-token:{}@{}", token, rest));
    }

    anyhow::bail!("Unsupported URL format: {}", url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_token_https() {
        let url = "https://github.com/nullisLabs/website.git";
        let token = "ghs_abc123";
        let result = insert_token_in_url(url, token).unwrap();
        assert_eq!(
            result,
            "https://x-access-token:ghs_abc123@github.com/nullisLabs/website.git"
        );
    }

    #[test]
    fn test_insert_token_git() {
        let url = "git://github.com/nullisLabs/website.git";
        let token = "ghs_abc123";
        let result = insert_token_in_url(url, token).unwrap();
        assert_eq!(
            result,
            "https://x-access-token:ghs_abc123@github.com/nullisLabs/website.git"
        );
    }
}
