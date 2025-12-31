use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::shared::{BuildJob, SiteType};
use crate::worker::builder::types::{detect_site_type, load_deploy_config, BuildContext};
use crate::worker::server::AppState;

/// Run a build in a Podman container
pub async fn run_build(
    state: &AppState,
    job: &BuildJob,
    repo_dir: &Path,
) -> Result<PathBuf> {
    // Load deploy config if present
    let deploy_config = load_deploy_config(repo_dir).await;

    // Resolve site type (auto-detect if needed)
    let site_type = if job.site_type == SiteType::Auto {
        detect_site_type(repo_dir).await
    } else {
        job.site_type
    };

    if site_type == SiteType::Auto {
        anyhow::bail!("Could not auto-detect site type and no explicit type provided");
    }

    // Build context with resolved configuration
    let context = BuildContext::new(site_type, deploy_config);

    tracing::info!(
        site_type = %context.site_type,
        build_command = %context.build_command,
        output_dir = %context.output_dir,
        "Resolved build context"
    );

    // For now, run the build directly (Podman integration can be added later)
    // This is a simplified version that runs the build command directly
    run_build_command(&context, repo_dir).await?;

    // Return the output directory path
    let output_path = repo_dir.join(&context.output_dir);
    if !output_path.exists() {
        anyhow::bail!(
            "Build output directory does not exist: {}",
            output_path.display()
        );
    }

    Ok(output_path)
}

/// Run the build command in the repository directory
async fn run_build_command(context: &BuildContext, repo_dir: &Path) -> Result<()> {
    use tokio::process::Command;

    // Use nix develop if we have a flake reference
    let output = if let Some(flake_ref) = &context.flake_ref {
        tracing::info!(flake = %flake_ref, "Running build with nix develop");

        Command::new("nix")
            .args([
                "develop",
                flake_ref,
                "--command",
                "sh",
                "-c",
                &context.build_command,
            ])
            .current_dir(repo_dir)
            .output()
            .await
            .context("Failed to execute nix develop")?
    } else {
        // Run directly (for custom builds)
        Command::new("sh")
            .args(["-c", &context.build_command])
            .current_dir(repo_dir)
            .output()
            .await
            .context("Failed to execute build command")?
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        anyhow::bail!(
            "Build command failed:\nstdout: {}\nstderr: {}",
            stdout,
            stderr
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    tracing::debug!(stdout = %stdout, "Build output");

    Ok(())
}

/// Run build in an isolated Podman container (full implementation)
#[allow(dead_code)]
async fn run_build_in_container(
    state: &AppState,
    context: &BuildContext,
    repo_dir: &Path,
) -> Result<PathBuf> {
    use bollard::Docker;
    use bollard::container::{Config, CreateContainerOptions, StartContainerOptions};
    use bollard::models::{HostConfig, Mount, MountTypeEnum};

    // Connect to Podman via Docker-compatible API
    let docker = Docker::connect_with_unix(
        state.config.podman_socket.to_str().unwrap(),
        120,
        bollard::API_DEFAULT_VERSION,
    )
    .context("Failed to connect to Podman")?;

    let output_dir = std::env::temp_dir().join(format!("catapult-output-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&output_dir).await?;

    // Build the container command
    let command = if let Some(flake_ref) = &context.flake_ref {
        format!(
            "nix develop {} --command sh -c '{}' && cp -r {} /output/",
            flake_ref, context.build_command, context.output_dir
        )
    } else {
        format!(
            "{} && cp -r {} /output/",
            context.build_command, context.output_dir
        )
    };

    let container_config = Config {
        image: Some("nixos/nix:latest"),
        cmd: Some(vec!["sh", "-c", &command]),
        working_dir: Some("/workspace"),
        host_config: Some(HostConfig {
            mounts: Some(vec![
                Mount {
                    target: Some("/workspace".to_string()),
                    source: Some(repo_dir.to_string_lossy().to_string()),
                    typ: Some(MountTypeEnum::BIND),
                    read_only: Some(true),
                    ..Default::default()
                },
                Mount {
                    target: Some("/output".to_string()),
                    source: Some(output_dir.to_string_lossy().to_string()),
                    typ: Some(MountTypeEnum::BIND),
                    read_only: Some(false),
                    ..Default::default()
                },
            ]),
            memory: Some(4 * 1024 * 1024 * 1024), // 4GB
            cpu_period: Some(100000),
            cpu_quota: Some(200000), // 2 CPUs
            pids_limit: Some(1000),
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            cap_drop: Some(vec!["ALL".to_string()]),
            ..Default::default()
        }),
        ..Default::default()
    };

    let container_name = format!("catapult-build-{}", uuid::Uuid::new_v4());

    docker
        .create_container(
            Some(CreateContainerOptions::<String> {
                name: container_name.clone(),
                ..Default::default()
            }),
            container_config,
        )
        .await
        .context("Failed to create container")?;

    docker
        .start_container(&container_name, None::<StartContainerOptions<String>>)
        .await
        .context("Failed to start container")?;

    // Wait for container to finish
    let result = docker
        .wait_container(&container_name, None::<bollard::container::WaitContainerOptions<String>>)
        .try_collect::<Vec<_>>()
        .await;

    // Cleanup container
    let _ = docker
        .remove_container(&container_name, None)
        .await;

    // Check result
    match result {
        Ok(responses) => {
            if let Some(response) = responses.first() {
                if response.status_code != 0 {
                    anyhow::bail!("Container exited with code {}", response.status_code);
                }
            }
        }
        Err(e) => {
            anyhow::bail!("Failed to wait for container: {}", e);
        }
    }

    Ok(output_dir)
}

// Need this import for the container code
use futures::TryStreamExt;
