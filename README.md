# Catapult

[![CI](https://github.com/nullisLabs/catapult/actions/workflows/ci.yml/badge.svg)](https://github.com/nullisLabs/catapult/actions/workflows/ci.yml)
[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org)
[![NixOS](https://img.shields.io/badge/NixOS-ready-5277C3.svg)](https://nixos.org)

Preview deploys without the Vercel price tag. Nix builds, your servers, zero BS.

## Overview

Catapult uses a **central + worker** architecture:

- **Central** receives GitHub webhooks, manages configuration, and dispatches jobs
- **Workers** execute builds in isolated Podman containers and deploy to Caddy

```
GitHub ──webhook──▶ Central ──dispatch──▶ Worker ──deploy──▶ Caddy
                       │                     │
                       ▼                     ▼
                   PostgreSQL            Podman + Nix
```

Single binary, two modes:
```bash
catapult central   # Run as orchestrator
catapult worker    # Run as build executor
```

## Features

- **PR Preview Deployments** - Automatic previews at `pr-{N}-{repo}.example.com`
- **Production Deployments** - Deploy on push to main branch
- **Multi-tenant** - Route deployments to different workers by zone
- **Nix Build Isolation** - Reproducible builds in Podman containers
- **GitHub App Integration** - Private repo support, PR comments
- **Cloudflare Tunnel Support** - Optional DNS and tunnel management

## Quick Start

### Repository Configuration

Add `.deploy.json` to your repository:

```json
{
  "build_type": "sveltekit",
  "build_command": "npm run build",
  "output_dir": "build"
}
```

Supported build types: `sveltekit`, `vite`, `zola`, `custom`, `auto`

### Organization Defaults

Add `.deploy.json` to `{org}/.github/` for organization-wide defaults:

```json
{
  "zone": "production",
  "domain_pattern": "{repo}.example.com",
  "pr_pattern": "pr-{pr}-{repo}.example.com"
}
```

## Deployment

See [docs/deployment.md](docs/deployment.md) for NixOS deployment instructions.

### NixOS Flake

```nix
{
  inputs.catapult.url = "github:nullisLabs/catapult";

  outputs = { self, nixpkgs, catapult, ... }: {
    nixosConfigurations.myserver = nixpkgs.lib.nixosSystem {
      modules = [
        catapult.nixosModules.default
        {
          services.catapult.central = {
            enable = true;
            databaseUrl = "postgresql://catapult@localhost/catapult";
            githubAppId = 123456;
            # ... see docs/deployment.md for full options
          };
        }
      ];
    };
  };
}
```

## Architecture

See [docs/architecture.md](docs/architecture.md) for detailed diagrams and API documentation.

## Development

```bash
# Enter development environment
nix develop

# Build
cargo build

# Run tests
cargo test

# Watch mode
cargo watch -x "run -- central"

# Run all CI checks
just ci
```

## License

AGPL-3.0-only
