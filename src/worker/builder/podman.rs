use anyhow::{Context, Result};
use bollard::container::{
    Config, CreateContainerOptions, LogsOptions, RemoveContainerOptions, StartContainerOptions,
    WaitContainerOptions,
};
use bollard::image::CreateImageOptions;
use bollard::models::{HostConfig, Mount, MountTypeEnum};
use bollard::Docker;
use futures::StreamExt;
use std::path::{Path, PathBuf};

use crate::shared::{BuildJob, SiteType};
use crate::worker::builder::network::{ensure_build_network, BUILD_NETWORK_NAME};
use crate::worker::builder::types::{detect_site_type, load_deploy_config, BuildContext};
use crate::worker::server::AppState;

/// Run a build - either in a container or directly depending on config
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
        use_containers = state.config.use_containers,
        "Resolved build context"
    );

    if state.config.use_containers {
        run_build_in_container(state, &context, repo_dir).await
    } else {
        run_build_directly(&context, repo_dir).await
    }
}

/// Run the build command directly (no container isolation)
async fn run_build_directly(context: &BuildContext, repo_dir: &Path) -> Result<PathBuf> {
    use tokio::process::Command;

    tracing::warn!("Running build WITHOUT container isolation - this is less secure");

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

/// Run build in an isolated Podman container
async fn run_build_in_container(
    state: &AppState,
    context: &BuildContext,
    repo_dir: &Path,
) -> Result<PathBuf> {
    // Connect to Podman via Docker-compatible API
    let docker = Docker::connect_with_unix(
        state.config.podman_socket.to_str().unwrap(),
        120,
        bollard::API_DEFAULT_VERSION,
    )
    .context("Failed to connect to Podman")?;

    // Ensure the isolated build network exists with RFC1918 blocking
    ensure_build_network(&docker).await?;

    // Ensure the build image exists (pull if needed)
    ensure_image(&docker, &state.config.build_image).await?;

    // Create output directory
    let output_dir = std::env::temp_dir().join(format!("catapult-output-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&output_dir).await?;

    let container_name = format!("catapult-build-{}", uuid::Uuid::new_v4());

    tracing::info!(
        container = %container_name,
        image = %state.config.build_image,
        "Starting container build"
    );

    // Build the container command
    // The build runs in /workspace (read-only mount of repo)
    // Output is copied to /output (writable mount)
    let build_script = build_container_script(context);

    let container_config = Config {
        image: Some(state.config.build_image.clone()),
        cmd: Some(vec!["sh".to_string(), "-c".to_string(), build_script]),
        working_dir: Some("/workspace".to_string()),
        env: Some(vec![
            "NIX_CONFIG=experimental-features = nix-command flakes".to_string(),
            "HOME=/tmp".to_string(),
        ]),
        host_config: Some(HostConfig {
            mounts: Some(vec![
                // Mount repo as read-only
                Mount {
                    target: Some("/workspace".to_string()),
                    source: Some(repo_dir.to_string_lossy().to_string()),
                    typ: Some(MountTypeEnum::BIND),
                    read_only: Some(true),
                    ..Default::default()
                },
                // Mount output directory as writable
                Mount {
                    target: Some("/output".to_string()),
                    source: Some(output_dir.to_string_lossy().to_string()),
                    typ: Some(MountTypeEnum::BIND),
                    read_only: Some(false),
                    ..Default::default()
                },
            ]),
            // Resource limits
            memory: Some(state.config.container_memory_limit as i64),
            cpu_period: Some(100000),
            cpu_quota: Some(state.config.container_cpu_quota),
            pids_limit: Some(state.config.container_pids_limit),
            // Security hardening
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            cap_drop: Some(vec!["ALL".to_string()]),
            // Temp filesystem for build artifacts
            tmpfs: Some(
                [("/tmp".to_string(), "size=2G,mode=1777".to_string())]
                    .into_iter()
                    .collect(),
            ),
            // Use isolated network with RFC1918 blocking
            network_mode: Some(BUILD_NETWORK_NAME.to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };

    // Create container
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

    // Start container
    docker
        .start_container(&container_name, None::<StartContainerOptions<String>>)
        .await
        .context("Failed to start container")?;

    // Stream logs while container runs
    let log_stream = docker.logs::<String>(
        &container_name,
        Some(LogsOptions {
            follow: true,
            stdout: true,
            stderr: true,
            ..Default::default()
        }),
    );

    // Collect logs (limited to prevent memory issues)
    let mut logs = Vec::new();
    let mut log_stream = log_stream;
    while let Some(log_result) = log_stream.next().await {
        match log_result {
            Ok(log) => {
                let line = log.to_string();
                tracing::debug!(container = %container_name, "{}", line.trim());
                if logs.len() < 1000 {
                    logs.push(line);
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "Error reading container logs");
                break;
            }
        }
    }

    // Wait for container to finish
    let mut wait_stream = docker.wait_container(
        &container_name,
        Some(WaitContainerOptions {
            condition: "not-running",
        }),
    );

    let exit_code = match wait_stream.next().await {
        Some(Ok(response)) => response.status_code,
        Some(Err(e)) => {
            cleanup_container(&docker, &container_name).await;
            anyhow::bail!("Failed to wait for container: {}", e);
        }
        None => {
            cleanup_container(&docker, &container_name).await;
            anyhow::bail!("Container wait stream ended unexpectedly");
        }
    };

    // Cleanup container
    cleanup_container(&docker, &container_name).await;

    // Check exit code
    if exit_code != 0 {
        let log_output = logs.join("");
        anyhow::bail!(
            "Container build failed with exit code {}:\n{}",
            exit_code,
            log_output
        );
    }

    tracing::info!(
        container = %container_name,
        output_dir = %output_dir.display(),
        "Container build completed successfully"
    );

    Ok(output_dir)
}

/// Build the shell script that runs inside the container
fn build_container_script(context: &BuildContext) -> String {
    let mut script = String::new();

    // Enable strict mode
    script.push_str("set -e\n");

    // Copy workspace to writable location (since /workspace is read-only)
    script.push_str("echo '==> Copying workspace to /tmp/build'\n");
    script.push_str("cp -r /workspace /tmp/build\n");
    script.push_str("cd /tmp/build\n");

    // Run build command (with or without nix develop)
    if let Some(flake_ref) = &context.flake_ref {
        script.push_str(&format!(
            "echo '==> Running build with nix develop ({})'\n",
            flake_ref
        ));
        script.push_str(&format!(
            "nix develop '{}' --command sh -c '{}'\n",
            flake_ref, context.build_command
        ));
    } else {
        script.push_str("echo '==> Running build command'\n");
        script.push_str(&format!("{}\n", context.build_command));
    }

    // Copy output to /output
    script.push_str(&format!(
        "echo '==> Copying output from {} to /output'\n",
        context.output_dir
    ));
    script.push_str(&format!(
        "cp -r '{}'/. /output/\n",
        context.output_dir
    ));

    script.push_str("echo '==> Build complete'\n");

    script
}

/// Cleanup container (remove it)
async fn cleanup_container(docker: &Docker, container_name: &str) {
    if let Err(e) = docker
        .remove_container(
            container_name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
    {
        tracing::warn!(
            container = %container_name,
            error = %e,
            "Failed to remove container"
        );
    }
}

/// Ensure an image exists locally, pulling it if necessary
async fn ensure_image(docker: &Docker, image: &str) -> Result<()> {
    // Check if image already exists
    match docker.inspect_image(image).await {
        Ok(_) => {
            tracing::debug!(image = image, "Image already exists locally");
            return Ok(());
        }
        Err(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => {
            // Image doesn't exist, need to pull
            tracing::info!(image = image, "Pulling container image");
        }
        Err(e) => {
            return Err(e).context("Failed to inspect image");
        }
    }

    // Parse image name into repository and tag
    let (repo, tag) = if let Some((r, t)) = image.rsplit_once(':') {
        (r.to_string(), t.to_string())
    } else {
        (image.to_string(), "latest".to_string())
    };

    // Pull the image
    let options = CreateImageOptions {
        from_image: repo.clone(),
        tag: tag.clone(),
        ..Default::default()
    };

    let mut stream = docker.create_image(Some(options), None, None);

    while let Some(result) = stream.next().await {
        match result {
            Ok(info) => {
                if let Some(status) = info.status {
                    tracing::debug!(image = image, status = %status, "Pull progress");
                }
            }
            Err(e) => {
                return Err(e).context(format!("Failed to pull image: {}", image));
            }
        }
    }

    tracing::info!(image = image, "Image pulled successfully");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::SiteType;

    #[test]
    fn test_build_container_script_with_flake() {
        let context = BuildContext::new(SiteType::SvelteKit, None);
        let script = build_container_script(&context);

        assert!(script.contains("set -e"));
        assert!(script.contains("cp -r /workspace /tmp/build"));
        assert!(script.contains("nix develop"));
        assert!(script.contains("github:nullisLabs/catapult#sveltekit"));
        assert!(script.contains("npm ci && npm run build"));
        assert!(script.contains("cp -r 'build'/. /output/"));
    }

    #[test]
    fn test_build_container_script_without_flake() {
        let context = BuildContext::new(SiteType::Custom, None);
        let script = build_container_script(&context);

        assert!(script.contains("set -e"));
        assert!(!script.contains("nix develop"));
    }
}
