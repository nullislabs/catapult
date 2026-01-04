use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Cloudflare integration configuration
#[derive(Debug, Clone)]
pub struct CloudflareConfig {
    /// Cloudflare API token with DNS and Tunnel edit permissions
    pub api_token: String,
    /// Cloudflare Account ID (for tunnel management)
    pub account_id: String,
    /// Zone ID for the domain (for DNS management)
    pub zone_id: String,
    /// Tunnel ID
    pub tunnel_id: String,
    /// Local service URL that the tunnel routes to (e.g., "http://localhost:8080")
    pub service_url: String,
}

/// Cloudflare client for managing deployment DNS records and tunnel routes
///
/// This manages both:
/// 1. DNS records (CNAME pointing to tunnel)
/// 2. Tunnel ingress rules (hostname → local service)
#[derive(Clone)]
pub struct CloudflareClient {
    http_client: reqwest::Client,
    config: Option<CloudflareConfig>,
}

impl CloudflareClient {
    /// Create a new Cloudflare client (enabled)
    pub fn new(config: CloudflareConfig) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            config: Some(config),
        }
    }

    /// Create a disabled Cloudflare client
    pub fn disabled() -> Self {
        Self {
            http_client: reqwest::Client::new(),
            config: None,
        }
    }

    /// Check if Cloudflare integration is enabled
    pub fn is_enabled(&self) -> bool {
        self.config.is_some()
    }

    /// Ensure DNS record and tunnel ingress rule exist for a hostname
    pub async fn ensure_route(&self, hostname: &str) -> Result<()> {
        let config = match &self.config {
            Some(c) => c,
            None => return Ok(()),
        };

        // Add tunnel ingress rule first (this routes traffic to local service)
        self.ensure_tunnel_ingress(hostname, config).await?;

        // Then create DNS record (this makes the hostname resolve to tunnel)
        self.ensure_dns_record(hostname, config).await?;

        Ok(())
    }

    /// Remove DNS record and tunnel ingress rule for a hostname
    pub async fn remove_route(&self, hostname: &str) -> Result<()> {
        let config = match &self.config {
            Some(c) => c,
            None => return Ok(()),
        };

        // Remove DNS first, then tunnel ingress
        self.remove_dns_record(hostname, config).await?;
        self.remove_tunnel_ingress(hostname, config).await?;

        Ok(())
    }

    // ==================== DNS Management ====================

    async fn ensure_dns_record(&self, hostname: &str, config: &CloudflareConfig) -> Result<()> {
        let tunnel_target = format!("{}.cfargotunnel.com", config.tunnel_id);

        let existing = self.get_dns_record(hostname, config).await?;

        if let Some(record) = existing {
            if record.content != tunnel_target {
                self.update_dns_record(&record.id, hostname, &tunnel_target, config)
                    .await?;
                tracing::info!(hostname = hostname, "Updated DNS record");
            } else {
                tracing::debug!(hostname = hostname, "DNS record already up to date");
            }
        } else {
            self.create_dns_record(hostname, &tunnel_target, config)
                .await?;
            tracing::info!(hostname = hostname, "Created DNS record");
        }

        Ok(())
    }

    async fn remove_dns_record(&self, hostname: &str, config: &CloudflareConfig) -> Result<()> {
        let existing = self.get_dns_record(hostname, config).await?;

        if let Some(record) = existing {
            let url = format!(
                "https://api.cloudflare.com/client/v4/zones/{}/dns_records/{}",
                config.zone_id, record.id
            );

            let response = self
                .http_client
                .delete(&url)
                .bearer_auth(&config.api_token)
                .send()
                .await
                .context("Failed to delete DNS record")?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                anyhow::bail!("Cloudflare DNS API error {}: {}", status, body);
            }

            tracing::info!(hostname = hostname, "Removed DNS record");
        }

        Ok(())
    }

    async fn get_dns_record(
        &self,
        hostname: &str,
        config: &CloudflareConfig,
    ) -> Result<Option<DnsRecord>> {
        let url = format!(
            "https://api.cloudflare.com/client/v4/zones/{}/dns_records?name={}",
            config.zone_id, hostname
        );

        let response = self
            .http_client
            .get(&url)
            .bearer_auth(&config.api_token)
            .send()
            .await
            .context("Failed to query DNS records")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Cloudflare DNS API error {}: {}", status, body);
        }

        let result: CloudflareResponse<Vec<DnsRecord>> = response
            .json()
            .await
            .context("Failed to parse Cloudflare response")?;

        Ok(result.result.into_iter().next())
    }

    async fn create_dns_record(
        &self,
        hostname: &str,
        target: &str,
        config: &CloudflareConfig,
    ) -> Result<()> {
        let url = format!(
            "https://api.cloudflare.com/client/v4/zones/{}/dns_records",
            config.zone_id
        );

        let request = CreateDnsRecord {
            record_type: "CNAME".to_string(),
            name: hostname.to_string(),
            content: target.to_string(),
            proxied: true,
            ttl: 1,
        };

        let response = self
            .http_client
            .post(&url)
            .bearer_auth(&config.api_token)
            .json(&request)
            .send()
            .await
            .context("Failed to create DNS record")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Cloudflare DNS API error {}: {}", status, body);
        }

        Ok(())
    }

    async fn update_dns_record(
        &self,
        record_id: &str,
        hostname: &str,
        target: &str,
        config: &CloudflareConfig,
    ) -> Result<()> {
        let url = format!(
            "https://api.cloudflare.com/client/v4/zones/{}/dns_records/{}",
            config.zone_id, record_id
        );

        let request = CreateDnsRecord {
            record_type: "CNAME".to_string(),
            name: hostname.to_string(),
            content: target.to_string(),
            proxied: true,
            ttl: 1,
        };

        let response = self
            .http_client
            .put(&url)
            .bearer_auth(&config.api_token)
            .json(&request)
            .send()
            .await
            .context("Failed to update DNS record")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Cloudflare DNS API error {}: {}", status, body);
        }

        Ok(())
    }

    // ==================== Tunnel Ingress Management ====================

    async fn ensure_tunnel_ingress(&self, hostname: &str, config: &CloudflareConfig) -> Result<()> {
        let mut tunnel_config = self.get_tunnel_config(config).await?;

        // Check if hostname already exists in ingress rules
        let exists = tunnel_config.config.ingress.iter().any(|rule| {
            rule.hostname.as_deref() == Some(hostname)
        });

        if exists {
            tracing::debug!(hostname = hostname, "Tunnel ingress rule already exists");
            return Ok(());
        }

        // Create new ingress rule
        let new_rule = TunnelIngressRule {
            hostname: Some(hostname.to_string()),
            service: config.service_url.clone(),
            origin_request: None,
        };

        // Insert before the catch-all rule (which should be last)
        // The catch-all has hostname: None
        let insert_pos = tunnel_config
            .config
            .ingress
            .iter()
            .position(|r| r.hostname.is_none())
            .unwrap_or(tunnel_config.config.ingress.len());

        tunnel_config.config.ingress.insert(insert_pos, new_rule);

        // Ensure there's a catch-all at the end
        if !tunnel_config.config.ingress.iter().any(|r| r.hostname.is_none()) {
            tunnel_config.config.ingress.push(TunnelIngressRule {
                hostname: None,
                service: "http_status:404".to_string(),
                origin_request: None,
            });
        }

        self.update_tunnel_config(config, &tunnel_config.config).await?;

        tracing::info!(
            hostname = hostname,
            service = %config.service_url,
            "Added tunnel ingress rule"
        );

        Ok(())
    }

    async fn remove_tunnel_ingress(&self, hostname: &str, config: &CloudflareConfig) -> Result<()> {
        let mut tunnel_config = self.get_tunnel_config(config).await?;

        let original_len = tunnel_config.config.ingress.len();
        tunnel_config.config.ingress.retain(|rule| {
            rule.hostname.as_deref() != Some(hostname)
        });

        if tunnel_config.config.ingress.len() == original_len {
            tracing::debug!(hostname = hostname, "Tunnel ingress rule not found");
            return Ok(());
        }

        self.update_tunnel_config(config, &tunnel_config.config).await?;

        tracing::info!(hostname = hostname, "Removed tunnel ingress rule");

        Ok(())
    }

    async fn get_tunnel_config(&self, config: &CloudflareConfig) -> Result<TunnelConfigResponse> {
        let url = format!(
            "https://api.cloudflare.com/client/v4/accounts/{}/cfd_tunnel/{}/configurations",
            config.account_id, config.tunnel_id
        );

        let response = self
            .http_client
            .get(&url)
            .bearer_auth(&config.api_token)
            .send()
            .await
            .context("Failed to get tunnel config")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Cloudflare Tunnel API error {}: {}", status, body);
        }

        let result: CloudflareResponse<TunnelConfigResponse> = response
            .json()
            .await
            .context("Failed to parse tunnel config")?;

        Ok(result.result)
    }

    async fn update_tunnel_config(
        &self,
        config: &CloudflareConfig,
        tunnel_config: &TunnelConfig,
    ) -> Result<()> {
        let url = format!(
            "https://api.cloudflare.com/client/v4/accounts/{}/cfd_tunnel/{}/configurations",
            config.account_id, config.tunnel_id
        );

        let request = TunnelConfigRequest {
            config: tunnel_config.clone(),
        };

        let response = self
            .http_client
            .put(&url)
            .bearer_auth(&config.api_token)
            .json(&request)
            .send()
            .await
            .context("Failed to update tunnel config")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Cloudflare Tunnel API error {}: {}", status, body);
        }

        Ok(())
    }
}

// ==================== API Types ====================

#[derive(Debug, Deserialize)]
struct CloudflareResponse<T> {
    result: T,
    #[allow(dead_code)]
    success: bool,
}

// --- DNS Types ---

#[derive(Debug, Deserialize)]
struct DnsRecord {
    id: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct CreateDnsRecord {
    #[serde(rename = "type")]
    record_type: String,
    name: String,
    content: String,
    proxied: bool,
    ttl: u32,
}

// --- Tunnel Types ---

#[derive(Debug, Deserialize)]
struct TunnelConfigResponse {
    config: TunnelConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TunnelConfig {
    ingress: Vec<TunnelIngressRule>,
    #[serde(flatten)]
    other: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TunnelIngressRule {
    #[serde(skip_serializing_if = "Option::is_none")]
    hostname: Option<String>,
    service: String,
    #[serde(rename = "originRequest", skip_serializing_if = "Option::is_none")]
    origin_request: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct TunnelConfigRequest {
    config: TunnelConfig,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cloudflare_disabled() {
        let client = CloudflareClient::disabled();
        assert!(!client.is_enabled());
    }

    #[test]
    fn test_cloudflare_enabled() {
        let client = CloudflareClient::new(CloudflareConfig {
            api_token: "token".into(),
            account_id: "account".into(),
            zone_id: "zone".into(),
            tunnel_id: "tunnel".into(),
            service_url: "http://localhost:8080".into(),
        });
        assert!(client.is_enabled());
    }
}
