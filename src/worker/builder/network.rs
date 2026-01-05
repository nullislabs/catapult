use anyhow::{Context, Result};
use bollard::Docker;
use bollard::models::IpamConfig;
use bollard::network::{CreateNetworkOptions, InspectNetworkOptions, ListNetworksOptions};
use std::collections::HashSet;
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
            if let Some(ipam) = network.ipam
                && let Some(configs) = ipam.config
            {
                for config in configs {
                    if let Some(subnet) = config.subnet {
                        ensure_iptables_rules(&subnet).await?;
                    }
                }
            }
            return Ok(());
        }
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => {
            // Network doesn't exist, create it
            tracing::info!(
                network = BUILD_NETWORK_NAME,
                "Creating isolated build network"
            );
        }
        Err(e) => {
            return Err(e).context("Failed to inspect build network");
        }
    }

    // Find an available subnet that doesn't conflict with existing networks
    let subnet = find_available_subnet(docker).await?;
    let gateway = subnet.replace(".0/24", ".1");

    tracing::debug!(subnet = %subnet, "Selected subnet for build network");

    // Create the network with the selected subnet
    let ipam_config = IpamConfig {
        subnet: Some(subnet.clone()),
        gateway: Some(gateway),
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
        options: Default::default(),
        ..Default::default()
    };

    docker
        .create_network(options)
        .await
        .context("Failed to create build network")?;

    // Set up iptables rules to block RFC1918
    ensure_iptables_rules(&subnet).await?;

    tracing::info!(
        network = BUILD_NETWORK_NAME,
        subnet = %subnet,
        "Created isolated build network with RFC1918 blocking"
    );

    Ok(())
}

/// Find an available subnet in the 10.89.x.0/24 range
async fn find_available_subnet(docker: &Docker) -> Result<String> {
    // List all networks to find used subnets
    let networks = docker
        .list_networks(Some(ListNetworksOptions::<String>::default()))
        .await
        .context("Failed to list networks")?;

    // Collect all subnets in use
    let mut used_subnets: HashSet<String> = HashSet::new();
    for network in networks {
        if let Some(ipam) = network.ipam
            && let Some(configs) = ipam.config
        {
            for config in configs {
                if let Some(subnet) = config.subnet {
                    used_subnets.insert(subnet);
                }
            }
        }
    }

    // Try subnets in the 10.89.x.0/24 range (x from 0 to 255)
    for x in 0..=255u8 {
        let subnet = format!("10.89.{}.0/24", x);
        if !used_subnets.contains(&subnet) {
            // Also check for overlapping ranges (though /24s in different octets won't overlap)
            let overlaps = used_subnets
                .iter()
                .any(|used| subnets_overlap(&subnet, used));
            if !overlaps {
                return Ok(subnet);
            }
        }
    }

    anyhow::bail!("No available subnet found in 10.89.x.0/24 range")
}

/// Check if two CIDR subnets overlap (simplified for /24 networks)
fn subnets_overlap(a: &str, b: &str) -> bool {
    // Parse subnet and mask
    fn parse_subnet(s: &str) -> Option<(u32, u32)> {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 2 {
            return None;
        }
        let mask_bits: u32 = parts[1].parse().ok()?;
        let octets: Vec<&str> = parts[0].split('.').collect();
        if octets.len() != 4 {
            return None;
        }
        let ip: u32 = (octets[0].parse::<u32>().ok()? << 24)
            | (octets[1].parse::<u32>().ok()? << 16)
            | (octets[2].parse::<u32>().ok()? << 8)
            | octets[3].parse::<u32>().ok()?;
        let mask = if mask_bits == 0 {
            0
        } else {
            !0u32 << (32 - mask_bits)
        };
        Some((ip & mask, mask))
    }

    let Some((net_a, mask_a)) = parse_subnet(a) else {
        return false;
    };
    let Some((net_b, mask_b)) = parse_subnet(b) else {
        return false;
    };

    // Use the larger network's mask (smaller mask value = fewer bits = larger network)
    // Two networks overlap if they share any addresses, which happens when the smaller
    // mask (larger network) applied to both results in the same value
    let common_mask = mask_a.min(mask_b);
    (net_a & common_mask) == (net_b & common_mask)
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
            // Skip the build network's own subnet (allow self-communication)
            if range == &"10.0.0.0/8" {
                // More specific rule to allow the build network itself but block rest of 10.x
                add_iptables_rule(chain_name, source_subnet, source_subnet, "ACCEPT").await?;
            }

            add_iptables_rule(chain_name, source_subnet, range, "DROP").await?;
        }

        // Add jump rule from FORWARD chain if not present
        let check_jump = Command::new("iptables")
            .args(["-C", "FORWARD", "-s", source_subnet, "-j", chain_name])
            .output()
            .await;

        if check_jump.is_err() || !check_jump.unwrap().status.success() {
            let output = Command::new("iptables")
                .args(["-I", "FORWARD", "1", "-s", source_subnet, "-j", chain_name])
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

    #[test]
    fn test_subnets_overlap_same() {
        assert!(subnets_overlap("10.89.0.0/24", "10.89.0.0/24"));
    }

    #[test]
    fn test_subnets_overlap_different() {
        assert!(!subnets_overlap("10.89.0.0/24", "10.89.1.0/24"));
        assert!(!subnets_overlap("10.89.0.0/24", "192.168.1.0/24"));
    }

    #[test]
    fn test_subnets_overlap_larger_contains_smaller() {
        // 10.0.0.0/8 contains 10.89.0.0/24
        assert!(subnets_overlap("10.0.0.0/8", "10.89.0.0/24"));
        assert!(subnets_overlap("10.89.0.0/24", "10.0.0.0/8"));
    }

    #[test]
    fn test_subnets_overlap_172_range() {
        assert!(subnets_overlap("172.16.0.0/12", "172.17.0.0/24"));
        assert!(!subnets_overlap("172.16.0.0/12", "172.32.0.0/24"));
    }
}
