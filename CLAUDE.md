# CLAUDE.md

Development guidance for Claude Code when working on this repository.

## Project Overview

Catapult is a deployment automation system with two components:

1. **Central** (`catapult.nullislabs.io`) - Receives GitHub webhooks, orchestrates deployments
2. **Worker** (`deployer.<zone>`) - Executes builds inside Caddy containers

Single binary with two modes: `catapult central` and `catapult worker`.

## Architecture

See `docs/architecture.md` for detailed diagrams.

**Central:**
- Receives GitHub webhooks at `/webhook/github`
- Verifies signatures, looks up config in PostgreSQL
- Generates GitHub installation access tokens
- Posts PR comments
- Dispatches jobs to workers

**Worker:**
- Receives build jobs at `/build`
- Clones repos, runs Podman builds (NixOS + flakes)
- Deploys to `/var/www/sites/`
- Configures Caddy via admin API
- Reports status back to Central

## Commands

```bash
# Enter development environment
nix develop

# Build
cargo build

# Run central mode
cargo run -- central

# Run worker mode
cargo run -- worker

# Test
cargo test

# Watch mode
cargo watch -x "run -- central"

# Format
cargo fmt

# Lint
cargo clippy
```

## Code Style

- Use `thiserror` for error types
- Use `tracing` for logging (not `log`)
- Use `tokio` for async runtime
- Use `axum` for HTTP server
- Use `sqlx` for database access (with compile-time checked queries)
- Prefer explicit error handling over `.unwrap()`
- Use `clap` for CLI argument parsing

## Project Structure

```
catapult/
├── src/
│   ├── main.rs              # Entry point, CLI parsing
│   ├── config.rs            # Configuration loading (central vs worker)
│   │
│   ├── central/             # Central mode
│   │   ├── mod.rs
│   │   ├── server.rs        # Axum server setup
│   │   ├── handlers/
│   │   │   ├── mod.rs
│   │   │   ├── webhook.rs   # GitHub webhook handler
│   │   │   └── status.rs    # Worker status callback handler
│   │   ├── github/
│   │   │   ├── mod.rs
│   │   │   ├── app.rs       # App authentication, JWT generation
│   │   │   ├── webhook.rs   # Webhook signature verification
│   │   │   └── api.rs       # GitHub API client (comments, tokens)
│   │   ├── db/
│   │   │   ├── mod.rs
│   │   │   ├── models.rs    # SQLx models
│   │   │   └── queries.rs   # Database queries
│   │   └── dispatch.rs      # Job dispatch to workers
│   │
│   ├── worker/              # Worker mode
│   │   ├── mod.rs
│   │   ├── server.rs        # Axum server setup
│   │   ├── handlers/
│   │   │   ├── mod.rs
│   │   │   ├── build.rs     # Build job handler
│   │   │   └── cleanup.rs   # Cleanup job handler
│   │   ├── builder/
│   │   │   ├── mod.rs
│   │   │   ├── podman.rs    # Podman container management
│   │   │   ├── clone.rs     # Git clone operations
│   │   │   └── types.rs     # Build configuration types
│   │   ├── deploy/
│   │   │   ├── mod.rs
│   │   │   └── caddy.rs     # Caddy admin API integration
│   │   └── callback.rs      # Status callback to Central
│   │
│   └── shared/              # Shared types between central and worker
│       ├── mod.rs
│       ├── auth.rs          # HMAC signing/verification
│       └── types.rs         # Job payloads, status types
│
├── migrations/              # SQL migrations (Central only)
├── docs/                    # Documentation
│   └── architecture.md
├── flake.nix               # Nix flake for dev environment
└── Cargo.toml
```

## Key Dependencies

- `axum` - HTTP framework
- `tokio` - Async runtime
- `clap` - CLI argument parsing
- `sqlx` - Database (PostgreSQL, Central only)
- `reqwest` - HTTP client
- `serde` / `serde_json` - Serialization
- `tracing` / `tracing-subscriber` - Logging
- `thiserror` / `anyhow` - Error handling
- `jsonwebtoken` - GitHub App JWT (Central only)
- `hmac` / `sha2` - Signature verification
- `bollard` - Podman API client (Worker only)

## Database

Central uses PostgreSQL with SQLx compile-time checked queries.

```bash
# Run migrations
sqlx migrate run

# Prepare for offline compilation
cargo sqlx prepare
```

## Testing

- Unit tests in same file as implementation
- Integration tests in `tests/` directory
- Use `testcontainers` for database tests (Central)
- Mock Podman API for worker tests

## Security Notes

- Always verify webhook signatures (GitHub → Central)
- Always verify HMAC signatures (Central ↔ Worker)
- Include timestamp in signatures for replay protection
- Never log GitHub tokens or secrets
- Installation access tokens expire after 1 hour
- Build containers must be isolated from internal networks (block RFC1918)
