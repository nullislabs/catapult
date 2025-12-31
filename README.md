# Catapult

Automated deployment runner for GitHub webhooks. Handles PR preview deployments and main branch deployments with Nix-based build isolation and multi-environment support.

## Architecture Overview

Catapult uses a **central + worker** architecture:

```
GitHub App (single application)
    ↓ webhooks
┌─────────────────────────────────────────────────────────────┐
│ Catapult Central (catapult.nullislabs.io)                   │
│   ├─ Receives all GitHub webhooks                           │
│   ├─ Verifies webhook signatures                            │
│   ├─ Looks up deployment config (PostgreSQL)                │
│   ├─ Generates GitHub installation access tokens            │
│   ├─ Posts "Building..." comment to PR                      │
│   └─ Dispatches build jobs to appropriate worker            │
└─────────────────────────────────────────────────────────────┘
                    ↓ authenticated job dispatch
    ┌───────────────┴───────────────┐
    ↓                               ↓
┌─────────────────────────┐ ┌─────────────────────────┐
│ Caddy Container         │ │ Caddy Container         │
│ (nullislabs)            │ │ (nullispl)              │
│ ┌─────────────────────┐ │ │ ┌─────────────────────┐ │
│ │ Catapult Worker     │ │ │ │ Catapult Worker     │ │
│ │ deployer.nullislabs │ │ │ │ deployer.nullis.pl  │ │
│ │ .io                 │ │ │ │                     │ │
│ │ - Clones repo       │ │ │ │ - Clones repo       │ │
│ │ - Builds (Podman)   │ │ │ │ - Builds (Podman)   │ │
│ │ - Deploys files     │ │ │ │ - Deploys files     │ │
│ │ - Configures Caddy  │ │ │ │ - Configures Caddy  │ │
│ └─────────────────────┘ │ │ └─────────────────────┘ │
│ Caddy Server            │ │ Caddy Server            │
└─────────────────────────┘ └─────────────────────────┘
                    ↓ status updates
┌─────────────────────────────────────────────────────────────┐
│ Catapult Central                                            │
│   └─ Posts success/failure comment to PR                    │
└─────────────────────────────────────────────────────────────┘
```

## Components

### Catapult Central (`catapult.nullislabs.io`)

The orchestrator that handles GitHub integration:

- Receives webhooks from the GitHub App
- Verifies webhook signatures
- Manages deployment configuration database
- Generates short-lived GitHub installation tokens
- Dispatches jobs to environment-specific workers
- Posts PR comments (building, success, failure)

### Catapult Worker (`deployer.<zone>`)

Runs inside each Caddy container, executes builds and deployments:

- Receives authenticated build jobs from Central
- Clones repositories (using provided GitHub token)
- Runs builds in isolated Podman containers (NixOS-based)
- Writes build artifacts to `/var/www/sites/`
- Configures Caddy routes via admin API (localhost:2019)
- Reports status back to Central

**Single binary, two modes:**
```bash
catapult central   # Run as orchestrator
catapult worker    # Run as build executor
```

## Key Features

- **Single GitHub App** - One app installation covers all environments
- **Multi-environment support** - Deploy to different Caddy instances
- **Private repository support** - Uses GitHub App installation access tokens
- **PR preview deployments** - Automatic previews for every PR
- **Main branch deployments** - Automatic production deployments
- **Automatic cleanup** - Removes PR deployments on merge/close
- **Nix-based build isolation** - Podman containers with flake-defined environments
- **Custom build commands** - Via `.deploy.json` in repository

## Build System

Workers use **Podman** to run isolated build containers. Each container runs a minimal NixOS image that uses **Nix flakes** to bring up the appropriate build environment.

```
Catapult Worker (deployer.<zone>)
  ↓
Podman (container-in-container)
  ↓
NixOS Container
  ├─ nix develop (from build flake)
  ├─ git clone (with token)
  ├─ Run build command
  └─ Output artifacts → /var/www/sites/
```

### Build Flakes

| Type | Flake | Provides |
|------|-------|----------|
| `sveltekit` | `github:nullisLabs/catapult#sveltekit` | Node.js, npm |
| `vite` | `github:nullisLabs/catapult#vite` | Node.js, npm |
| `zola` | `github:nullisLabs/catapult#zola` | Zola static site generator |
| `custom` | (from repo's flake.nix) | User-defined |
| `auto` | (detected) | Based on repo contents |

## Configuration

### Central Environment Variables

```bash
# Database
DATABASE_URL=postgresql://user:pass@host:5432/dbname

# GitHub App
GITHUB_APP_ID=123456
GITHUB_PRIVATE_KEY_PATH=/path/to/private-key.pem
GITHUB_WEBHOOK_SECRET=your-webhook-secret

# Server
LISTEN_ADDR=0.0.0.0:8080

# Worker authentication
WORKER_SHARED_SECRET=your-shared-secret
```

### Worker Environment Variables

```bash
# Central connection
CENTRAL_URL=https://catapult.nullislabs.io
WORKER_SHARED_SECRET=your-shared-secret

# Server
LISTEN_ADDR=0.0.0.0:8080

# Build system
PODMAN_SOCKET=/run/podman/podman.sock

# Caddy
CADDY_ADMIN_API=http://localhost:2019
SITES_DIR=/var/www/sites
```

## Repository Configuration

Repositories can include a `.deploy.json` file:

```json
{
  "build_type": "sveltekit",
  "build_command": "npm run build",
  "output_dir": "build"
}
```

### Build Types

| Type | Default Command | Output Dir |
|------|-----------------|------------|
| `sveltekit` | `npm run build` | `build` |
| `vite` | `npm run build` | `dist` |
| `zola` | `zola build` | `public` |
| `custom` | (from config) | (from config) |
| `auto` | (detected) | (detected) |

## URL Structure

### PR Deployments
```
https://pr-{number}-{repo}.{domain}/

Examples:
https://pr-42-website.wilmen.co/
https://pr-123-docs.nullis.xyz/
```

### Main Branch Deployments
```
https://{subdomain}.{domain}/
or
https://{domain}/
```

## Security

### GitHub App Permissions

- Contents: Read (clone repos)
- Pull Requests: Read & Write (comment on PRs)

### Central ↔ Worker Authentication

Workers authenticate with Central using a shared secret (HMAC-signed requests).

### Build Isolation

Builds run in isolated Podman containers with:
- NixOS-based minimal image
- Read-only root filesystem (except build directory)
- Network restrictions (blocks RFC1918, allows internet)
- Memory/CPU limits
- PID limits (prevent fork bombs)
- Dropped capabilities

## Development

```bash
# Enter development environment
nix develop

# Run central
cargo run -- central

# Run worker
cargo run -- worker

# Run tests
cargo test

# Watch mode
cargo watch -x "run -- central"
```

## License

AGPL-3.0-only
