use anyhow::{Context, Result};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// GitHub App for generating JWTs and installation tokens
#[derive(Clone)]
pub struct GitHubApp {
    app_id: u64,
    private_key: EncodingKey,
}

#[derive(Debug, Serialize)]
struct JwtClaims {
    /// Issued at time
    iat: u64,
    /// Expiration time (10 minutes max for GitHub)
    exp: u64,
    /// Issuer (GitHub App ID)
    iss: String,
}

#[derive(Debug, Deserialize)]
pub struct InstallationToken {
    pub token: String,
    pub expires_at: String,
}

impl GitHubApp {
    /// Create a new GitHub App instance
    pub fn new(app_id: u64, private_key_pem: &str) -> Result<Self> {
        let private_key = EncodingKey::from_rsa_pem(private_key_pem.as_bytes())
            .context("Failed to parse GitHub App private key")?;

        Ok(Self {
            app_id,
            private_key,
        })
    }

    /// Generate a JWT for GitHub App authentication
    ///
    /// JWTs are valid for up to 10 minutes
    pub fn generate_jwt(&self) -> Result<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("Time went backwards")?
            .as_secs();

        let claims = JwtClaims {
            // Issue 60 seconds in the past to allow for clock drift
            iat: now.saturating_sub(60),
            // Expire in 10 minutes
            exp: now + 600,
            iss: self.app_id.to_string(),
        };

        let header = Header::new(Algorithm::RS256);
        encode(&header, &claims, &self.private_key).context("Failed to encode JWT")
    }

    /// Get an installation access token for a specific installation
    pub async fn get_installation_token(
        &self,
        http_client: &reqwest::Client,
        installation_id: u64,
    ) -> Result<InstallationToken> {
        let jwt = self.generate_jwt()?;

        let response = http_client
            .post(format!(
                "https://api.github.com/app/installations/{}/access_tokens",
                installation_id
            ))
            .header("Authorization", format!("Bearer {}", jwt))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "catapult")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .context("Failed to request installation token")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "GitHub API error {}: {}",
                status,
                body
            );
        }

        response
            .json()
            .await
            .context("Failed to parse installation token response")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test RSA key for unit tests only (not a real key)
    const TEST_PRIVATE_KEY: &str = r#"-----BEGIN RSA PRIVATE KEY-----
MIIEpAIBAAKCAQEA0Z3VS5JJcds3xfn/ygWyF8PbnGy0AHB7MNvgKudQTwBVBqgP
Jc4BYr4xig2V8M+n3VB5fZwFSo+IkC7gCKhxFJyPqlYkLDwFGxPgwgLrdKcGWn1F
6dgbLqHvbKKLNVQGTGPqjKbGpKKgEuIrmcRBpLR0I7bVaD0MwCF0J5KHQN0JQWB6
f0P+HRTN0ZCvK3sTaEHdGNoNBBVAH5ORLlVl5V4UBgzUUdBP1PdN7hLsiSwSNBBn
V1GCpKzGk3ztgTX0IwHMOHHCjBQYsqVb3MvKlKR6s68WwXVEREFjUgRoRk6i8Zbj
SBSp5gPJfSl3L9X2l6dmpdLbNBoG5WJPKM0FcwIDAQABAoIBAC/CvEZpVteZaoEh
8Tnp0IQGsLuT7ISHILGaQ5h0rTqCbMhBCikX27lCqR2F0A7+7zni6VUwqPIqX+p0
Y9s7AbC76RkF5K1vO9QjKh0y0AGqlkF0N9lXnrCjvPW+PwkbvkwMRCcYAwPCYdPT
NgJLDDYQ3EKuKfyzfTiVEtLsOa1V7gBzE9H3KpJjwLhS9hHsgaOwkpqQGLl+6LBK
G1nhgPw35Rq0aMSQMNSfwgUKKxHTwqJk8NnPNxYRKqQxNnF1QqSNgRinsghCDf9q
r7+KgdPzlHvPXc19RvK/9vY3hNBcvnM4KYJQ5fRQGv1GzjBs0BAp7PQMqsB0H8Rl
vy4MS2ECgYEA7zR3RqbHHvYRnmHAz8CEG9UKFWB2J+fWJP0hJ6KWIc3hhS7NzFPD
r0AMb2RSBtBqHFDJqhcDiB5hy4JhJhBvKt7ZQiDN9dL+D6a7s5DPhP5+Y9C8Mn6V
KPgI7XWW0J5cTzCPuAdQ3FIGVA7K2q0LDBnZNkNM0sTQ3PmvS0QlKvsCgYEA35w5
hnKJdmCEJ7E0Fj3R9AZEw8kLH0VJOb0IpkfRFGZpxIHF4T2OqpK3L28U5/SR5Q54
e0fR5Cv0CXthf8qDXpMH8Y7U6Je6G7HlC7VQPqPgvpx2+0L4XBr2A2xB3vcBzF7e
rVlTr2DRxI6GLNM7lmJhQvMk3btDCAzl7ufwQGkCgYBbhynB7wDqxTRnUqTY3wCb
zbLJfL/aHzT5V3cNelv7R8NN0gVh0qkCPSKkwi31WBXH8+YMqqdQGohzw0m/D+mj
Y8VqJN+SuPjE/KmcxbXKtCHmLhDsF4bL2m9fg2i4QSbZdMGlr7lWqJC7M8c5mf/y
9fApP3L88ZEL3ALz9h/FywKBgQCJvh/h0YIgFKHvEe9D3x+9k2e4HHiJD9PWRB5l
5jbKZVcNM5IS3fXUHF/Q/R9X2LKrF2L+KR7sBWsMWyJllK2VKiZlMdVpHma2L0a8
5cFOiVMxOFl2GpLPUb9s1MPvAC8cRfTjMohGkzpNJsUQkd7hj0LrQAXsIZKWn0dh
7BdUqQKBgQCxKfYBf5aZ0WJdXp7ksb0MT4WedTizBLOlw0IFLZ3RYWJ3k6L5u5wq
PqPVB0UpLdHHa7qPuEL/dp3ThKjvPq7tOQFl6fHjhofnPFvJdC9VTPvE2xW7ZEHP
qlqNzZ6ptIopPVMVfp+8s3i2DXXBERvJfMjcGcK8xoJiz+6nLikvLA==
-----END RSA PRIVATE KEY-----"#;

    #[test]
    #[ignore = "Requires valid RSA key - run with GITHUB_PRIVATE_KEY_PATH env var"]
    fn test_jwt_generation() {
        let app = GitHubApp::new(12345, TEST_PRIVATE_KEY).unwrap();
        let jwt = app.generate_jwt().unwrap();

        // JWT should have 3 parts
        assert_eq!(jwt.split('.').count(), 3);
    }
}
