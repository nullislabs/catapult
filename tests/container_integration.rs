//! Container integration tests for Worker build operations
//!
//! These tests require:
//! - A running Podman daemon (rootless preferred, rootful supported)
//! - Properly configured subuid/subgid mappings (for rootless)
//! - The alpine:latest image available (or BUILD_IMAGE env var)
//!
//! ## NixOS Setup (Rootless - Recommended)
//!
//! 1. Ensure your NixOS configuration includes:
//!    ```nix
//!    virtualisation.podman.enable = true;
//!    users.users.youruser = {
//!      subUidRanges = [{ startUid = 100000; count = 65536; }];
//!      subGidRanges = [{ startGid = 100000; count = 65536; }];
//!    };
//!    ```
//!
//! 2. Start the rootless Podman socket:
//!    ```bash
//!    systemctl --user start podman.socket
//!    ```
//!
//! 3. Run tests:
//!    ```bash
//!    cargo test --test container_integration -- --ignored
//!    ```
//!
//! ## Socket Selection
//!
//! The tests automatically select the Podman socket in this order:
//! 1. PODMAN_SOCKET environment variable (if set)
//! 2. User's rootless socket at /run/user/$UID/podman/podman.sock
//! 3. System socket at /run/podman/podman.sock (requires proper permissions)

use anyhow::Result;
use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, LogsOptions, RemoveContainerOptions, StartContainerOptions,
    WaitContainerOptions,
};
use bollard::models::{HostConfig, Mount, MountTypeEnum};
use bollard::network::ListNetworksOptions;
use futures::StreamExt;
use tempfile::TempDir;

/// Get the Podman socket path, preferring rootless socket
fn podman_socket() -> String {
    // First check for explicit override
    if let Ok(socket) = std::env::var("PODMAN_SOCKET") {
        return socket;
    }

    // Try rootless socket (preferred for development/testing)
    let uid = unsafe { libc::getuid() };
    let user_socket = format!("/run/user/{}/podman/podman.sock", uid);
    if std::path::Path::new(&user_socket).exists() {
        return user_socket;
    }

    // Fall back to system socket
    "/run/podman/podman.sock".to_string()
}

/// Get the build image from environment or use default
fn build_image() -> String {
    std::env::var("BUILD_IMAGE").unwrap_or_else(|_| "docker.io/library/alpine:latest".to_string())
}

/// Connect to Podman via Docker-compatible API
async fn connect_podman() -> Result<Docker> {
    let socket = podman_socket();
    let docker = Docker::connect_with_unix(&socket, 120, bollard::API_DEFAULT_VERSION)?;
    Ok(docker)
}

/// Create a temporary directory with test content
fn create_test_workspace() -> TempDir {
    let dir = tempfile::tempdir().expect("Failed to create temp dir");

    // Create a simple test file
    std::fs::write(dir.path().join("test.txt"), "Hello from test workspace\n")
        .expect("Failed to write test file");

    // Create a simple script
    std::fs::write(
        dir.path().join("build.sh"),
        "#!/bin/sh\necho 'Build successful'\necho 'test output' > /output/result.txt\n",
    )
    .expect("Failed to write build script");

    dir
}

#[tokio::test]
#[ignore = "Requires Podman: systemctl --user start podman.socket"]
async fn test_podman_connection() {
    let docker = connect_podman().await.expect("Failed to connect to Podman");

    // Verify connection by pinging
    let version = docker.version().await.expect("Failed to get version");

    println!("Connected to container engine:");
    println!("  Version: {:?}", version.version);
    println!("  API Version: {:?}", version.api_version);

    assert!(version.version.is_some());
}

#[tokio::test]
#[ignore = "Requires Podman: systemctl --user start podman.socket"]
async fn test_list_networks() {
    let docker = connect_podman().await.expect("Failed to connect to Podman");

    let networks = docker
        .list_networks(Some(ListNetworksOptions::<String>::default()))
        .await
        .expect("Failed to list networks");

    println!("Found {} networks:", networks.len());
    for network in &networks {
        println!(
            "  - {} (driver: {:?})",
            network.name.as_deref().unwrap_or("unnamed"),
            network.driver
        );
    }

    // Should have at least the default bridge network
    assert!(!networks.is_empty());
}

#[tokio::test]
#[ignore = "Requires Podman: systemctl --user start podman.socket"]
async fn test_simple_container_run() {
    let docker = connect_podman().await.expect("Failed to connect to Podman");
    let image = build_image();

    // Pull image if needed (ignore errors - it might already exist)
    let _ = docker
        .create_image(
            Some(bollard::image::CreateImageOptions {
                from_image: image.clone(),
                ..Default::default()
            }),
            None,
            None,
        )
        .collect::<Vec<_>>()
        .await;

    // Use file-based output verification (more reliable than log streaming)
    let output_dir = tempfile::tempdir().expect("Failed to create output dir");
    let container_name = format!("catapult-test-{}", uuid::Uuid::new_v4());

    // Create container that writes output to a file
    let config = Config {
        image: Some(image),
        cmd: Some(vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo 'Hello from container' > /output/result.txt".to_string(),
        ]),
        host_config: Some(HostConfig {
            mounts: Some(vec![Mount {
                target: Some("/output".to_string()),
                source: Some(output_dir.path().to_string_lossy().to_string()),
                typ: Some(MountTypeEnum::BIND),
                read_only: Some(false),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    };

    docker
        .create_container(
            Some(CreateContainerOptions {
                name: container_name.clone(),
                ..Default::default()
            }),
            config,
        )
        .await
        .expect("Failed to create container");

    // Start container
    docker
        .start_container(&container_name, None::<StartContainerOptions<String>>)
        .await
        .expect("Failed to start container");

    // Wait for container to finish
    let mut wait_stream = docker.wait_container(
        &container_name,
        Some(WaitContainerOptions {
            condition: "not-running",
        }),
    );

    let exit_code = match wait_stream.next().await {
        Some(Ok(response)) => response.status_code,
        Some(Err(e)) => panic!("Wait failed: {}", e),
        None => panic!("Wait stream ended unexpectedly"),
    };

    // Cleanup container
    docker
        .remove_container(
            &container_name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
        .expect("Failed to remove container");

    assert_eq!(exit_code, 0);

    // Verify output via file
    let output_file = output_dir.path().join("result.txt");
    assert!(output_file.exists(), "Output file should exist");

    let content = std::fs::read_to_string(&output_file).expect("Failed to read output");
    println!("Container output: {}", content.trim());
    assert!(content.contains("Hello from container"));
}

#[tokio::test]
#[ignore = "Requires Podman: systemctl --user start podman.socket"]
async fn test_container_with_mounts() {
    let docker = connect_podman().await.expect("Failed to connect to Podman");
    let image = build_image();

    // Pull image if needed
    let _ = docker
        .create_image(
            Some(bollard::image::CreateImageOptions {
                from_image: image.clone(),
                ..Default::default()
            }),
            None,
            None,
        )
        .collect::<Vec<_>>()
        .await;

    // Create test workspace
    let workspace = create_test_workspace();
    let output_dir = tempfile::tempdir().expect("Failed to create output dir");

    let container_name = format!("catapult-test-mount-{}", uuid::Uuid::new_v4());

    // Create container with mounts
    let config = Config {
        image: Some(image),
        cmd: Some(vec![
            "sh".to_string(),
            "-c".to_string(),
            "cat /workspace/test.txt && echo 'output data' > /output/result.txt".to_string(),
        ]),
        host_config: Some(HostConfig {
            mounts: Some(vec![
                Mount {
                    target: Some("/workspace".to_string()),
                    source: Some(workspace.path().to_string_lossy().to_string()),
                    typ: Some(MountTypeEnum::BIND),
                    read_only: Some(true),
                    ..Default::default()
                },
                Mount {
                    target: Some("/output".to_string()),
                    source: Some(output_dir.path().to_string_lossy().to_string()),
                    typ: Some(MountTypeEnum::BIND),
                    read_only: Some(false),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        }),
        ..Default::default()
    };

    docker
        .create_container(
            Some(CreateContainerOptions {
                name: container_name.clone(),
                ..Default::default()
            }),
            config,
        )
        .await
        .expect("Failed to create container");

    // Start and wait
    docker
        .start_container(&container_name, None::<StartContainerOptions<String>>)
        .await
        .expect("Failed to start container");

    let mut wait_stream = docker.wait_container(
        &container_name,
        Some(WaitContainerOptions {
            condition: "not-running",
        }),
    );

    let exit_code = match wait_stream.next().await {
        Some(Ok(response)) => response.status_code,
        Some(Err(e)) => panic!("Wait failed: {}", e),
        None => panic!("Wait stream ended unexpectedly"),
    };

    // Cleanup container
    docker
        .remove_container(
            &container_name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
        .expect("Failed to remove container");

    assert_eq!(exit_code, 0);

    // Verify output was written
    let output_file = output_dir.path().join("result.txt");
    assert!(output_file.exists(), "Output file should exist");

    let content = std::fs::read_to_string(&output_file).expect("Failed to read output");
    assert!(content.contains("output data"));

    println!("Mount test passed - output: {}", content.trim());
}

#[tokio::test]
#[ignore = "Requires Podman: systemctl --user start podman.socket"]
async fn test_container_resource_limits() {
    let docker = connect_podman().await.expect("Failed to connect to Podman");
    let image = build_image();

    // Pull image if needed
    let _ = docker
        .create_image(
            Some(bollard::image::CreateImageOptions {
                from_image: image.clone(),
                ..Default::default()
            }),
            None,
            None,
        )
        .collect::<Vec<_>>()
        .await;

    let container_name = format!("catapult-test-limits-{}", uuid::Uuid::new_v4());

    // Create container with resource limits
    let config = Config {
        image: Some(image),
        cmd: Some(vec!["echo".to_string(), "Resource limits test".to_string()]),
        host_config: Some(HostConfig {
            memory: Some(256 * 1024 * 1024), // 256MB
            cpu_period: Some(100000),
            cpu_quota: Some(50000), // 0.5 CPU
            pids_limit: Some(100),
            ..Default::default()
        }),
        ..Default::default()
    };

    docker
        .create_container(
            Some(CreateContainerOptions {
                name: container_name.clone(),
                ..Default::default()
            }),
            config,
        )
        .await
        .expect("Failed to create container with resource limits");

    // Inspect to verify limits
    let inspect = docker
        .inspect_container(&container_name, None)
        .await
        .expect("Failed to inspect container");

    let host_config = inspect.host_config.expect("Missing host config");

    println!("Container resource limits:");
    println!("  Memory: {:?}", host_config.memory);
    println!("  CPU Period: {:?}", host_config.cpu_period);
    println!("  CPU Quota: {:?}", host_config.cpu_quota);
    println!("  PIDs Limit: {:?}", host_config.pids_limit);

    assert_eq!(host_config.memory, Some(256 * 1024 * 1024));
    assert_eq!(host_config.cpu_period, Some(100000));
    assert_eq!(host_config.cpu_quota, Some(50000));
    assert_eq!(host_config.pids_limit, Some(100));

    // Cleanup
    docker
        .remove_container(
            &container_name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
        .expect("Failed to remove container");
}

#[tokio::test]
#[ignore = "Requires Podman: systemctl --user start podman.socket"]
async fn test_container_security_options() {
    let docker = connect_podman().await.expect("Failed to connect to Podman");
    let image = build_image();

    // Pull image if needed
    let _ = docker
        .create_image(
            Some(bollard::image::CreateImageOptions {
                from_image: image.clone(),
                ..Default::default()
            }),
            None,
            None,
        )
        .collect::<Vec<_>>()
        .await;

    let container_name = format!("catapult-test-security-{}", uuid::Uuid::new_v4());

    // Create container with security hardening
    let config = Config {
        image: Some(image),
        cmd: Some(vec!["id".to_string()]),
        host_config: Some(HostConfig {
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            cap_drop: Some(vec!["ALL".to_string()]),
            ..Default::default()
        }),
        ..Default::default()
    };

    docker
        .create_container(
            Some(CreateContainerOptions {
                name: container_name.clone(),
                ..Default::default()
            }),
            config,
        )
        .await
        .expect("Failed to create container with security options");

    // Start and wait
    docker
        .start_container(&container_name, None::<StartContainerOptions<String>>)
        .await
        .expect("Failed to start container");

    let mut wait_stream = docker.wait_container(
        &container_name,
        Some(WaitContainerOptions {
            condition: "not-running",
        }),
    );

    let exit_code = match wait_stream.next().await {
        Some(Ok(response)) => response.status_code,
        Some(Err(e)) => panic!("Wait failed: {}", e),
        None => panic!("Wait stream ended unexpectedly"),
    };

    // Cleanup
    docker
        .remove_container(
            &container_name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
        .expect("Failed to remove container");

    assert_eq!(exit_code, 0);
    println!("Security options test passed");
}

#[tokio::test]
#[ignore = "Requires Podman and network access"]
async fn test_container_network_external_access() {
    let docker = connect_podman().await.expect("Failed to connect to Podman");
    let image = build_image();

    // Pull image if needed
    let _ = docker
        .create_image(
            Some(bollard::image::CreateImageOptions {
                from_image: image.clone(),
                ..Default::default()
            }),
            None,
            None,
        )
        .collect::<Vec<_>>()
        .await;

    // Use file-based output verification
    let output_dir = tempfile::tempdir().expect("Failed to create output dir");
    let container_name = format!("catapult-test-network-{}", uuid::Uuid::new_v4());

    // Create container that tries to access external network and writes result to file
    // Note: Using wget instead of ping as alpine's ping requires special capabilities
    let config = Config {
        image: Some(image),
        cmd: Some(vec![
            "sh".to_string(),
            "-c".to_string(),
            // Try to reach external network using wget (more reliable than ping in containers)
            "wget -q -O /dev/null --timeout=5 http://1.1.1.1 && echo 'NETWORK_OK' > /output/result.txt || echo 'NETWORK_FAIL' > /output/result.txt".to_string(),
        ]),
        host_config: Some(HostConfig {
            mounts: Some(vec![Mount {
                target: Some("/output".to_string()),
                source: Some(output_dir.path().to_string_lossy().to_string()),
                typ: Some(MountTypeEnum::BIND),
                read_only: Some(false),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    };

    docker
        .create_container(
            Some(CreateContainerOptions {
                name: container_name.clone(),
                ..Default::default()
            }),
            config,
        )
        .await
        .expect("Failed to create container");

    // Start and wait
    docker
        .start_container(&container_name, None::<StartContainerOptions<String>>)
        .await
        .expect("Failed to start container");

    let mut wait_stream = docker.wait_container(
        &container_name,
        Some(WaitContainerOptions {
            condition: "not-running",
        }),
    );

    let _ = wait_stream.next().await;

    // Cleanup container
    docker
        .remove_container(
            &container_name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
        .expect("Failed to remove container");

    // Verify output via file
    let output_file = output_dir.path().join("result.txt");
    assert!(output_file.exists(), "Output file should exist");

    let output = std::fs::read_to_string(&output_file).expect("Failed to read output");
    println!("Network test output: {}", output.trim());

    // With default network, external access should work
    assert!(
        output.contains("NETWORK_OK"),
        "External network access should work with default network"
    );
}

#[tokio::test]
#[ignore = "Requires rootful Podman for iptables rules"]
async fn test_build_network_blocks_rfc1918() {
    use catapult::worker::builder::network::{BUILD_NETWORK_NAME, ensure_build_network};

    // This test requires rootful Podman because:
    // 1. The build network uses iptables rules to block RFC1918
    // 2. iptables requires root privileges
    //
    // To run: PODMAN_SOCKET=/run/podman/podman.sock sudo -E cargo test ...

    let docker = connect_podman().await.expect("Failed to connect to Podman");
    let image = build_image();

    // Ensure build network exists (will fail in rootless mode due to iptables)
    if let Err(e) = ensure_build_network(&docker).await {
        eprintln!("Skipping RFC1918 test: {}", e);
        eprintln!("This test requires rootful Podman with iptables access");
        return;
    }

    // Pull image if needed
    let _ = docker
        .create_image(
            Some(bollard::image::CreateImageOptions {
                from_image: image.clone(),
                ..Default::default()
            }),
            None,
            None,
        )
        .collect::<Vec<_>>()
        .await;

    // Use file-based output verification
    let output_dir = tempfile::tempdir().expect("Failed to create output dir");
    let container_name = format!("catapult-test-rfc1918-{}", uuid::Uuid::new_v4());

    // Create container on isolated network that tries to access RFC1918 addresses
    // Using timeout-based TCP connection attempts instead of ping (more reliable)
    let config = Config {
        image: Some(image),
        cmd: Some(vec![
            "sh".to_string(),
            "-c".to_string(),
            // Try to connect to RFC1918 addresses - should be blocked by iptables
            // Use timeout with nc (netcat) for reliable connection testing
            concat!(
                "(",
                "timeout 2 sh -c 'echo | nc -w 1 10.0.0.1 80 2>/dev/null' && echo 'RFC1918_10_OK' || echo 'RFC1918_10_BLOCKED'; ",
                "timeout 2 sh -c 'echo | nc -w 1 172.16.0.1 80 2>/dev/null' && echo 'RFC1918_172_OK' || echo 'RFC1918_172_BLOCKED'; ",
                "timeout 2 sh -c 'echo | nc -w 1 192.168.1.1 80 2>/dev/null' && echo 'RFC1918_192_OK' || echo 'RFC1918_192_BLOCKED'",
                ") > /output/result.txt 2>&1"
            ).to_string(),
        ]),
        host_config: Some(HostConfig {
            network_mode: Some(BUILD_NETWORK_NAME.to_string()),
            mounts: Some(vec![Mount {
                target: Some("/output".to_string()),
                source: Some(output_dir.path().to_string_lossy().to_string()),
                typ: Some(MountTypeEnum::BIND),
                read_only: Some(false),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    };

    docker
        .create_container(
            Some(CreateContainerOptions {
                name: container_name.clone(),
                ..Default::default()
            }),
            config,
        )
        .await
        .expect("Failed to create container on build network");

    // Start and wait
    docker
        .start_container(&container_name, None::<StartContainerOptions<String>>)
        .await
        .expect("Failed to start container");

    let mut wait_stream = docker.wait_container(
        &container_name,
        Some(WaitContainerOptions {
            condition: "not-running",
        }),
    );

    let _ = wait_stream.next().await;

    // Cleanup container
    docker
        .remove_container(
            &container_name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
        .expect("Failed to remove container");

    // Verify output via file
    let output_file = output_dir.path().join("result.txt");
    let output = if output_file.exists() {
        std::fs::read_to_string(&output_file).expect("Failed to read output")
    } else {
        // If no output file, the command likely failed entirely
        String::new()
    };

    println!("RFC1918 blocking test output:\n{}", output);

    // All RFC1918 addresses should be blocked
    assert!(
        output.contains("RFC1918_10_BLOCKED"),
        "10.0.0.0/8 should be blocked"
    );
    assert!(
        output.contains("RFC1918_172_BLOCKED"),
        "172.16.0.0/12 should be blocked"
    );
    assert!(
        output.contains("RFC1918_192_BLOCKED"),
        "192.168.0.0/16 should be blocked"
    );
}

#[tokio::test]
#[ignore = "Requires Podman: systemctl --user start podman.socket"]
async fn test_full_build_simulation() {
    let docker = connect_podman().await.expect("Failed to connect to Podman");

    // Use Alpine for the test (NixOS image would be better but is large)
    let image = build_image();

    // Pull image if needed
    let _ = docker
        .create_image(
            Some(bollard::image::CreateImageOptions {
                from_image: image.clone(),
                ..Default::default()
            }),
            None,
            None,
        )
        .collect::<Vec<_>>()
        .await;

    // Create workspace with a simple project
    let workspace = tempfile::tempdir().expect("Failed to create workspace");
    let output_dir = tempfile::tempdir().expect("Failed to create output dir");

    // Create a simple build script
    std::fs::write(
        workspace.path().join("build.sh"),
        r#"#!/bin/sh
set -e
echo "==> Starting build"
echo "==> Copying files"
mkdir -p /output/dist
echo "<html><body>Hello World</body></html>" > /output/dist/index.html
echo "==> Build complete"
"#,
    )
    .expect("Failed to write build script");

    let container_name = format!("catapult-test-build-{}", uuid::Uuid::new_v4());

    // Simulate a real build with all the options we use
    let build_script = r#"
set -e
echo '==> Copying workspace to /tmp/build'
cp -r /workspace /tmp/build
cd /tmp/build
echo '==> Running build'
sh build.sh
echo '==> Build complete'
"#;

    let config = Config {
        image: Some(image),
        cmd: Some(vec![
            "sh".to_string(),
            "-c".to_string(),
            build_script.to_string(),
        ]),
        working_dir: Some("/workspace".to_string()),
        host_config: Some(HostConfig {
            mounts: Some(vec![
                Mount {
                    target: Some("/workspace".to_string()),
                    source: Some(workspace.path().to_string_lossy().to_string()),
                    typ: Some(MountTypeEnum::BIND),
                    read_only: Some(true),
                    ..Default::default()
                },
                Mount {
                    target: Some("/output".to_string()),
                    source: Some(output_dir.path().to_string_lossy().to_string()),
                    typ: Some(MountTypeEnum::BIND),
                    read_only: Some(false),
                    ..Default::default()
                },
            ]),
            memory: Some(512 * 1024 * 1024), // 512MB
            cpu_quota: Some(100000),         // 1 CPU
            pids_limit: Some(500),
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            cap_drop: Some(vec!["ALL".to_string()]),
            tmpfs: Some(
                [("/tmp".to_string(), "size=100M,mode=1777".to_string())]
                    .into_iter()
                    .collect(),
            ),
            ..Default::default()
        }),
        ..Default::default()
    };

    docker
        .create_container(
            Some(CreateContainerOptions {
                name: container_name.clone(),
                ..Default::default()
            }),
            config,
        )
        .await
        .expect("Failed to create build container");

    // Start container
    docker
        .start_container(&container_name, None::<StartContainerOptions<String>>)
        .await
        .expect("Failed to start container");

    // Stream logs
    let mut logs = docker.logs::<String>(
        &container_name,
        Some(LogsOptions {
            follow: true,
            stdout: true,
            stderr: true,
            ..Default::default()
        }),
    );

    let mut log_output = Vec::new();
    while let Some(log) = logs.next().await {
        match log {
            Ok(l) => {
                let line = l.to_string();
                println!("[BUILD] {}", line.trim());
                log_output.push(line);
            }
            Err(e) => {
                eprintln!("Log error: {}", e);
                break;
            }
        }
    }

    // Wait for completion
    let mut wait_stream = docker.wait_container(
        &container_name,
        Some(WaitContainerOptions {
            condition: "not-running",
        }),
    );

    let exit_code = match wait_stream.next().await {
        Some(Ok(response)) => response.status_code,
        Some(Err(e)) => panic!("Wait failed: {}", e),
        None => panic!("Wait stream ended unexpectedly"),
    };

    // Cleanup
    docker
        .remove_container(
            &container_name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
        .expect("Failed to remove container");

    // Verify results
    assert_eq!(exit_code, 0, "Build should succeed");

    let output_index = output_dir.path().join("dist/index.html");
    assert!(output_index.exists(), "Output file should exist");

    let content = std::fs::read_to_string(&output_index).expect("Failed to read output");
    assert!(content.contains("Hello World"));

    println!("\nFull build simulation passed!");
    println!("Output: {}", content.trim());
}
