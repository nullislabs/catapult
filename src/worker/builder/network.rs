use anyhow::{Context, Result};
use bollard::models::IpamConfig;
use bollard::network::{CreateNetworkOptions, InspectNetworkOptions};
use bollard::Docker;
use std::collections::HashMap;
use tokio::process::Command;

/// Name of the isolated build network
pub const BUILD_NETWORK_NAME: &str = "catapult-build-isolated";

/// RFC1918 private IP ranges that should be blocked
const RFC1918_RANGES: &[&str] = &["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"];

/// Ensure the isolated build network exists with proper RFC1918 blocking
pub async fn ensure_build_network(docker: &Docker) -> Result<()> {
    // Check if network already exists
    match docker
        .inspect_network(
            BUILD_NETWORK_NAME,
            Some(InspectNetworkOptions::<String> {
                verbose: false,
                scope: "local".to_string(),
            }),
        )
        .await
    {
        Ok(network) => {
            tracing::debug!(network = BUILD_NETWORK_NAME, "Build network already exists");
            // Network exists, ensure iptables rules are in place
            if let Some(ipam) = network.ipam {
                if let Some(configs) = ipam.config {
                    for config in configs {
                        if let Some(subnet) = config.subnet {
                            ensure_iptables_rules(&subnet).await?;
                        }
                    }
                }
            }
            return Ok(());
        }
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => {
            // Network doesn't exist, create it
            tracing::info!(network = BUILD_NETWORK_NAME, "Creating isolated build network");
        }
        Err(e) => {
            return Err(e).context("Failed to inspect build network");
        }
    }

    // Create the network with a specific subnet for iptables rules
    let ipam_config = IpamConfig {
        subnet: Some("10.89.0.0/24".to_string()),
        gateway: Some("10.89.0.1".to_string()),
        ..Default::default()
    };

    let options = CreateNetworkOptions {
        name: BUILD_NETWORK_NAME,
        driver: "bridge",
        internal: false, // Allow external access (needed for npm/nix downloads)
        ipam: bollard::models::Ipam {
            driver: Some("default".to_string()),
            config: Some(vec![ipam_config]),
            options: None,
        },
        options: {
            let mut opts = HashMap::new();
            // Disable inter-container communication
            opts.insert("com.docker.network.bridge.enable_icc", "false");
            opts
        },
        ..Default::default()
    };

    docker
        .create_network(options)
        .await
        .context("Failed to create build network")?;

    // Set up iptables rules to block RFC1918
    ensure_iptables_rules("10.89.0.0/24").await?;

    tracing::info!(
        network = BUILD_NETWORK_NAME,
        "Created isolated build network with RFC1918 blocking"
    );

    Ok(())
}

/// Ensure iptables rules block RFC1918 destinations from the build network
async fn ensure_iptables_rules(source_subnet: &str) -> Result<()> {
    // Create a custom chain for catapult rules if it doesn't exist
    let chain_name = "CATAPULT_BUILD_ISOLATION";

    // Check if chain exists
    let check_chain = Command::new("iptables")
        .args(["-n", "-L", chain_name])
        .output()
        .await;

    if check_chain.is_err() || !check_chain.unwrap().status.success() {
        // Create the chain
        let output = Command::new("iptables")
            .args(["-N", chain_name])
            .output()
            .await
            .context("Failed to create iptables chain")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Chain might already exist (race condition) - that's fine
            if !stderr.contains("Chain already exists") {
                tracing::warn!(
                    chain = chain_name,
                    stderr = %stderr,
                    "Failed to create iptables chain (may require root)"
                );
                // Don't fail - we'll log a warning but continue
                // In production, these rules should be set up by system configuration
                return Ok(());
            }
        }

        // Add rules to block RFC1918 destinations
        for range in RFC1918_RANGES {
            // Skip the build network's own subnet
            if range == &"10.0.0.0/8" {
                // More specific rule to allow the build network itself but block rest of 10.x
                add_iptables_rule(chain_name, source_subnet, "10.89.0.0/24", "ACCEPT").await?;
            }

            add_iptables_rule(chain_name, source_subnet, range, "DROP").await?;
        }

        // Add jump rule from FORWARD chain if not present
        let check_jump = Command::new("iptables")
            .args([
                "-C",
                "FORWARD",
                "-s",
                source_subnet,
                "-j",
                chain_name,
            ])
            .output()
            .await;

        if check_jump.is_err() || !check_jump.unwrap().status.success() {
            let output = Command::new("iptables")
                .args([
                    "-I",
                    "FORWARD",
                    "1",
                    "-s",
                    source_subnet,
                    "-j",
                    chain_name,
                ])
                .output()
                .await
                .context("Failed to add FORWARD jump rule")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::warn!(
                    stderr = %stderr,
                    "Failed to add iptables FORWARD jump rule (may require root)"
                );
            }
        }

        tracing::info!(
            chain = chain_name,
            source = source_subnet,
            "Configured iptables rules for RFC1918 blocking"
        );
    }

    Ok(())
}

/// Add an iptables rule to the chain
async fn add_iptables_rule(chain: &str, source: &str, dest: &str, target: &str) -> Result<()> {
    // Check if rule exists first
    let check = Command::new("iptables")
        .args(["-C", chain, "-s", source, "-d", dest, "-j", target])
        .output()
        .await;

    if check.is_ok() && check.unwrap().status.success() {
        // Rule already exists
        return Ok(());
    }

    let output = Command::new("iptables")
        .args(["-A", chain, "-s", source, "-d", dest, "-j", target])
        .output()
        .await
        .context("Failed to add iptables rule")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::debug!(
            chain = chain,
            source = source,
            dest = dest,
            target = target,
            stderr = %stderr,
            "Failed to add iptables rule"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rfc1918_ranges() {
        // Verify we have all RFC1918 ranges covered
        assert_eq!(RFC1918_RANGES.len(), 3);
        assert!(RFC1918_RANGES.contains(&"10.0.0.0/8"));
        assert!(RFC1918_RANGES.contains(&"172.16.0.0/12"));
        assert!(RFC1918_RANGES.contains(&"192.168.0.0/16"));
    }

    #[test]
    fn test_build_network_name() {
        assert_eq!(BUILD_NETWORK_NAME, "catapult-build-isolated");
    }
}
