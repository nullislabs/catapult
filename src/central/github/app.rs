use anyhow::{Context, Result};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
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
            anyhow::bail!("GitHub API error {}: {}", status, body);
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

    // Test RSA key for unit tests only - generated specifically for testing
    // DO NOT use this key in production!
    const TEST_PRIVATE_KEY: &str = r#"-----BEGIN RSA PRIVATE KEY-----
MIIEowIBAAKCAQEAmXoJ3f8mWvgWoQM1BRWXBBltU2N9dFP+l0YIMh6nqwTXbzRy
GAMB8DJ4P0zXsiOUxHM9vBqNO/HE/EPN1FG/i4bYwl6464FtaXXzrtH2olNnE3lM
ueUFEHvOwsfhy0XZfWiTg7uMI3X5ecynqf2M7Mt69MXfjnMwcVMnKsQXel8Yg5Y8
DTEeSVDYnDBCOwi0umHyhS/hka0weCRiUpZToxcyRcV1/HgwEnWnf/LszjJoCKeR
i4x9vwn0JOyqic4RYZkkivGhwhVjupp1ntx1J7ZGbCyfXtKXg83c022EfEH0eIPG
GVL6LO39kekQ6Wky6Wv+wm7GoP3O2VzeEtj8iwIDAQABAoIBABocOoCDj3VrfoIP
Bx6h0SrX3pMQun+naIk4110bfP/p13LqU7zFCjfghjeSraF3TzBqwKZ3R+7aT15x
dJt5+uHUY50Ru1kJkGEgOYBU5SYxlTgpa0W28jkfLwpRMhdAf2NH/syrBAjmYbZ4
fE+9vJNufLEW1tUnwKaO9Htgl/Pv5RWCK/bmzWjMSyFrJMmqnCcFypAvFPCO70/K
XZH6UWLdNlRdzZOes/DZcMHPfVlYvzvZFclWef0yOJwxhW2FaVFtEaG43HPZNHbN
fHkvY1pjGyDMnlPrAQsR5HHhzowuHXaRESnRqv267zZi3DnD3Xv/JbM6fqlZ3zQF
v3WUFKUCgYEA1qzIC2jTH2Q4yjvIAh8jD0sfZrb6OgaEV89zAy7ayQdU7pWZRjIj
TbvxvadoztDpWEuBimhKyb50ZMDa3LgCKLnSNsOzPQa2+RktM8yXIq+EqK5xQTLU
Si7rHLB7g1KqCXAG0KL6+h8YBlA5NKqRhgJlDR6CQNCpVu+WhLJuDxUCgYEAtwVl
8v2whUr6L5t3niv13FGtKXoc4IdcyU34ny40iStRdEVp9rNOnU40zM5rcCKMYVyj
NNTLQ7WLPmYq3N6IKsNMS2e9LaT6gema0+SqXBVt7vppSsH84SCylxaHgMonVrEK
raTXzbzycysACVNstRdSZk5s5foDqDagEvjCxR8CgYEAj4xDzBVRL2mF6/0jlf+a
IwzZt4ZdNlXLQyhtwNAg7lHfwhX4ww6dusoVMPtzwu/BSRBcU9+/Or4G+KRY9UR5
9R+kaIheH02RJmpmZn/FBCWXsG/NPYqul9hd0PZV8Q9isiLd+78v0fbeysH0Lrpr
ys9pIOeos4yT35Uf8iWaIK0CgYA+i/S8ZyB1XRtFO89UWderFKql+xp1TS1TincG
B2di3U/3+WTuL3cVYU3AFGc5KkVpXJxWCMbye897YrURSGemnZmsR2aqe7A0x53m
/kWONLCeNCvZpZQDaAZAhi2GwQ9SnCx3DVfG8uS0oSRhC4aiGLdLSVAEBD5NtWVd
NnBxpwKBgEazX1mIiCKfUm5qu8l0CHvJR0vjBpjNWkJ5eCZ9qovMB6Mk2AxIMXWi
wEzQc0txmJr9jEGWm3jTicXo0/P+5by3M/OPvqzVX+mFFXhEgxfbwdTI38Y+48or
p8NnJtidt6eNVGzsVler1Ha26Kch8P3EucuTK59xTMkJUV4igAMN
-----END RSA PRIVATE KEY-----"#;

    #[test]
    fn test_jwt_generation() {
        let app = GitHubApp::new(12345, TEST_PRIVATE_KEY).unwrap();
        let jwt = app.generate_jwt().unwrap();

        // JWT should have 3 parts (header.payload.signature)
        assert_eq!(jwt.split('.').count(), 3);

        // Verify the JWT can be decoded (header only, we can't verify signature without public key)
        let parts: Vec<&str> = jwt.split('.').collect();
        let header =
            base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, parts[0])
                .expect("Failed to decode header");
        let header: serde_json::Value =
            serde_json::from_slice(&header).expect("Failed to parse header");

        assert_eq!(header["alg"], "RS256");
        assert_eq!(header["typ"], "JWT");
    }

    #[test]
    fn test_github_app_creation() {
        let app = GitHubApp::new(12345, TEST_PRIVATE_KEY);
        assert!(app.is_ok());
    }

    #[test]
    fn test_github_app_invalid_key() {
        let result = GitHubApp::new(12345, "not a valid key");
        assert!(result.is_err());
    }
}
