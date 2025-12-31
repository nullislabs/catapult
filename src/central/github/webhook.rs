use crate::shared::auth::verify_github_signature;
use serde::Deserialize;

/// Verify a GitHub webhook signature
pub fn verify_webhook_signature(secret: &str, payload: &[u8], signature: &str) -> bool {
    verify_github_signature(secret.as_bytes(), payload, signature)
}

/// Parsed webhook event
#[derive(Debug)]
pub enum WebhookEvent {
    PullRequest(PullRequestEvent),
    Push(PushEvent),
    Ping,
    Unknown(String),
}

/// Pull request event payload
#[derive(Debug, Clone, Deserialize)]
pub struct PullRequestEvent {
    pub action: PullRequestAction,
    pub number: u32,
    pub pull_request: PullRequest,
    pub repository: Repository,
    pub installation: Option<Installation>,
}

/// Pull request action type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestAction {
    Opened,
    Synchronize,
    Closed,
    Reopened,
    #[serde(other)]
    Other,
}

/// Pull request details
#[derive(Debug, Clone, Deserialize)]
pub struct PullRequest {
    pub head: PullRequestHead,
    pub merged: Option<bool>,
}

/// Pull request head (source branch)
#[derive(Debug, Clone, Deserialize)]
pub struct PullRequestHead {
    #[serde(rename = "ref")]
    pub branch: String,
    pub sha: String,
}

/// Push event payload
#[derive(Debug, Clone, Deserialize)]
pub struct PushEvent {
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub after: String,
    pub repository: Repository,
    pub installation: Option<Installation>,
}

impl PushEvent {
    /// Check if this is a push to the main branch
    pub fn is_main_branch(&self) -> bool {
        self.git_ref == "refs/heads/main" || self.git_ref == "refs/heads/master"
    }

    /// Get the branch name from the ref
    pub fn branch_name(&self) -> Option<&str> {
        self.git_ref.strip_prefix("refs/heads/")
    }
}

/// Repository information
#[derive(Debug, Clone, Deserialize)]
pub struct Repository {
    pub name: String,
    pub full_name: String,
    pub clone_url: String,
    pub owner: RepositoryOwner,
}

impl Repository {
    /// Get the organization/user name
    pub fn org_name(&self) -> &str {
        &self.owner.login
    }
}

/// Repository owner
#[derive(Debug, Clone, Deserialize)]
pub struct RepositoryOwner {
    pub login: String,
}

/// GitHub App installation
#[derive(Debug, Clone, Deserialize)]
pub struct Installation {
    pub id: u64,
}

/// Parse a webhook event from the event type and payload
pub fn parse_webhook_event(event_type: &str, payload: &[u8]) -> Result<WebhookEvent, serde_json::Error> {
    match event_type {
        "pull_request" => {
            let event: PullRequestEvent = serde_json::from_slice(payload)?;
            Ok(WebhookEvent::PullRequest(event))
        }
        "push" => {
            let event: PushEvent = serde_json::from_slice(payload)?;
            Ok(WebhookEvent::Push(event))
        }
        "ping" => Ok(WebhookEvent::Ping),
        other => Ok(WebhookEvent::Unknown(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pull_request_event() {
        let payload = r#"{
            "action": "opened",
            "number": 42,
            "pull_request": {
                "head": {
                    "ref": "feature-branch",
                    "sha": "abc123"
                },
                "merged": false
            },
            "repository": {
                "name": "website",
                "full_name": "nullisLabs/website",
                "clone_url": "https://github.com/nullisLabs/website.git",
                "owner": {
                    "login": "nullisLabs"
                }
            },
            "installation": {
                "id": 12345
            }
        }"#;

        let event = parse_webhook_event("pull_request", payload.as_bytes()).unwrap();
        match event {
            WebhookEvent::PullRequest(pr) => {
                assert_eq!(pr.action, PullRequestAction::Opened);
                assert_eq!(pr.number, 42);
                assert_eq!(pr.pull_request.head.branch, "feature-branch");
                assert_eq!(pr.repository.org_name(), "nullisLabs");
            }
            _ => panic!("Expected PullRequest event"),
        }
    }

    #[test]
    fn test_parse_push_event() {
        let payload = r#"{
            "ref": "refs/heads/main",
            "after": "def456",
            "repository": {
                "name": "website",
                "full_name": "nullisLabs/website",
                "clone_url": "https://github.com/nullisLabs/website.git",
                "owner": {
                    "login": "nullisLabs"
                }
            },
            "installation": {
                "id": 12345
            }
        }"#;

        let event = parse_webhook_event("push", payload.as_bytes()).unwrap();
        match event {
            WebhookEvent::Push(push) => {
                assert!(push.is_main_branch());
                assert_eq!(push.branch_name(), Some("main"));
                assert_eq!(push.after, "def456");
            }
            _ => panic!("Expected Push event"),
        }
    }
}
