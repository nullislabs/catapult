//! Catapult - Automated deployment runner for GitHub webhooks
//!
//! This crate provides two operational modes:
//! - **Central**: Receives GitHub webhooks, orchestrates deployments
//! - **Worker**: Executes builds in containers, deploys to Caddy

pub mod central;
pub mod config;
pub mod shared;
pub mod worker;
