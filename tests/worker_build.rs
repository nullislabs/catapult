//! Integration tests for Worker build operations

mod common;

use catapult::shared::{DeployConfig, SiteType};
use catapult::worker::builder::types::{detect_site_type, load_deploy_config, BuildContext};
use std::fs;
use tempfile::TempDir;

/// Create a temporary directory with test files
fn create_test_repo() -> TempDir {
    tempfile::tempdir().expect("Failed to create temp dir")
}

#[tokio::test]
async fn test_detect_sveltekit() {
    let dir = create_test_repo();
    fs::write(dir.path().join("svelte.config.js"), "// svelte config").unwrap();

    let site_type = detect_site_type(dir.path()).await;
    assert_eq!(site_type, SiteType::SvelteKit);
}

#[tokio::test]
async fn test_detect_sveltekit_ts() {
    let dir = create_test_repo();
    fs::write(dir.path().join("svelte.config.ts"), "// svelte config").unwrap();

    let site_type = detect_site_type(dir.path()).await;
    assert_eq!(site_type, SiteType::SvelteKit);
}

#[tokio::test]
async fn test_detect_vite() {
    let dir = create_test_repo();
    fs::write(dir.path().join("vite.config.js"), "// vite config").unwrap();

    let site_type = detect_site_type(dir.path()).await;
    assert_eq!(site_type, SiteType::Vite);
}

#[tokio::test]
async fn test_detect_vite_ts() {
    let dir = create_test_repo();
    fs::write(dir.path().join("vite.config.ts"), "// vite config").unwrap();

    let site_type = detect_site_type(dir.path()).await;
    assert_eq!(site_type, SiteType::Vite);
}

#[tokio::test]
async fn test_detect_zola() {
    let dir = create_test_repo();
    fs::write(
        dir.path().join("config.toml"),
        r#"
base_url = "https://example.com"
title = "Test Site"

[markdown]
highlight_code = true
"#,
    )
    .unwrap();

    let site_type = detect_site_type(dir.path()).await;
    assert_eq!(site_type, SiteType::Zola);
}

#[tokio::test]
async fn test_detect_custom_flake() {
    let dir = create_test_repo();
    fs::write(
        dir.path().join("flake.nix"),
        r#"{ description = "custom flake"; }"#,
    )
    .unwrap();

    let site_type = detect_site_type(dir.path()).await;
    assert_eq!(site_type, SiteType::Custom);
}

#[tokio::test]
async fn test_detect_fallback_to_vite_with_package_json() {
    let dir = create_test_repo();
    fs::write(
        dir.path().join("package.json"),
        r#"{"name": "test", "version": "1.0.0"}"#,
    )
    .unwrap();

    let site_type = detect_site_type(dir.path()).await;
    assert_eq!(site_type, SiteType::Vite);
}

#[tokio::test]
async fn test_detect_auto_when_unknown() {
    let dir = create_test_repo();
    // Empty directory

    let site_type = detect_site_type(dir.path()).await;
    assert_eq!(site_type, SiteType::Auto);
}

#[tokio::test]
async fn test_sveltekit_takes_priority_over_vite() {
    let dir = create_test_repo();
    // Both svelte and vite configs present
    fs::write(dir.path().join("svelte.config.js"), "// svelte").unwrap();
    fs::write(dir.path().join("vite.config.js"), "// vite").unwrap();

    let site_type = detect_site_type(dir.path()).await;
    assert_eq!(site_type, SiteType::SvelteKit);
}

#[tokio::test]
async fn test_load_deploy_config_missing() {
    let dir = create_test_repo();

    let config = load_deploy_config(dir.path()).await;
    assert!(config.is_none());
}

#[tokio::test]
async fn test_load_deploy_config_valid() {
    let dir = create_test_repo();
    fs::write(
        dir.path().join(".deploy.json"),
        r#"{
            "build_type": "vite",
            "build_command": "npm run build:prod",
            "output_dir": "dist/production"
        }"#,
    )
    .unwrap();

    let config = load_deploy_config(dir.path()).await;
    assert!(config.is_some());

    let config = config.unwrap();
    assert_eq!(config.build_type, Some(SiteType::Vite));
    assert_eq!(config.build_command, Some("npm run build:prod".to_string()));
    assert_eq!(config.output_dir, Some("dist/production".to_string()));
}

#[tokio::test]
async fn test_load_deploy_config_invalid_json() {
    let dir = create_test_repo();
    fs::write(dir.path().join(".deploy.json"), "not valid json").unwrap();

    let config = load_deploy_config(dir.path()).await;
    assert!(config.is_none());
}

#[test]
fn test_build_context_sveltekit_defaults() {
    let context = BuildContext::new(SiteType::SvelteKit, None);

    assert_eq!(context.site_type, SiteType::SvelteKit);
    assert_eq!(context.build_command, "npm ci && npm run build");
    assert_eq!(context.output_dir, "build");
    assert_eq!(
        context.flake_ref,
        Some("github:nullisLabs/catapult#sveltekit".to_string())
    );
}

#[test]
fn test_build_context_vite_defaults() {
    let context = BuildContext::new(SiteType::Vite, None);

    assert_eq!(context.site_type, SiteType::Vite);
    assert_eq!(context.build_command, "npm ci && npm run build");
    assert_eq!(context.output_dir, "dist");
    assert_eq!(
        context.flake_ref,
        Some("github:nullisLabs/catapult#vite".to_string())
    );
}

#[test]
fn test_build_context_zola_defaults() {
    let context = BuildContext::new(SiteType::Zola, None);

    assert_eq!(context.site_type, SiteType::Zola);
    assert_eq!(context.build_command, "zola build");
    assert_eq!(context.output_dir, "public");
    assert_eq!(
        context.flake_ref,
        Some("github:nullisLabs/catapult#zola".to_string())
    );
}

#[test]
fn test_build_context_custom_no_flake() {
    let context = BuildContext::new(SiteType::Custom, None);

    assert_eq!(context.site_type, SiteType::Custom);
    assert!(context.flake_ref.is_none());
}

#[test]
fn test_build_context_with_deploy_config_override() {
    let deploy_config = DeployConfig {
        build_type: Some(SiteType::Vite),
        build_command: Some("yarn build".to_string()),
        output_dir: Some("out".to_string()),
        ..Default::default()
    };

    // Start with SvelteKit but deploy config overrides to Vite
    let context = BuildContext::new(SiteType::SvelteKit, Some(deploy_config));

    assert_eq!(context.site_type, SiteType::Vite);
    assert_eq!(context.build_command, "yarn build");
    assert_eq!(context.output_dir, "out");
}

#[test]
fn test_build_context_partial_deploy_config() {
    let deploy_config = DeployConfig {
        build_type: None,
        build_command: Some("custom build".to_string()),
        output_dir: None,
        ..Default::default()
    };

    let context = BuildContext::new(SiteType::SvelteKit, Some(deploy_config));

    assert_eq!(context.site_type, SiteType::SvelteKit);
    assert_eq!(context.build_command, "custom build"); // overridden
    assert_eq!(context.output_dir, "build"); // default
}

#[test]
fn test_site_type_flake_refs() {
    assert_eq!(
        SiteType::SvelteKit.flake_ref(),
        Some("github:nullisLabs/catapult#sveltekit")
    );
    assert_eq!(
        SiteType::Vite.flake_ref(),
        Some("github:nullisLabs/catapult#vite")
    );
    assert_eq!(
        SiteType::Zola.flake_ref(),
        Some("github:nullisLabs/catapult#zola")
    );
    assert_eq!(SiteType::Custom.flake_ref(), None);
    assert_eq!(SiteType::Auto.flake_ref(), None);
}

#[test]
fn test_site_type_default_build_commands() {
    assert_eq!(
        SiteType::SvelteKit.default_build_command(),
        Some("npm ci && npm run build")
    );
    assert_eq!(
        SiteType::Vite.default_build_command(),
        Some("npm ci && npm run build")
    );
    assert_eq!(SiteType::Zola.default_build_command(), Some("zola build"));
    assert_eq!(SiteType::Custom.default_build_command(), None);
    assert_eq!(SiteType::Auto.default_build_command(), None);
}

#[test]
fn test_site_type_default_output_dirs() {
    assert_eq!(SiteType::SvelteKit.default_output_dir(), Some("build"));
    assert_eq!(SiteType::Vite.default_output_dir(), Some("dist"));
    assert_eq!(SiteType::Zola.default_output_dir(), Some("public"));
    assert_eq!(SiteType::Custom.default_output_dir(), None);
    assert_eq!(SiteType::Auto.default_output_dir(), None);
}
