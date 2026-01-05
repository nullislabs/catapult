# Deployment Guide

NixOS deployment for Catapult Central and Worker.

## Prerequisites

- NixOS 24.05+
- PostgreSQL (Central)
- Podman + Caddy (Worker)
- GitHub App configured

## GitHub App Setup

1. Create GitHub App at Settings → Developer settings → GitHub Apps
2. Configure:
   - **Webhook URL:** `https://catapult.example.com/webhook/github`
   - **Permissions:** Contents (Read), Pull requests (Read & Write)
   - **Events:** Pull request, Push
3. Generate and download the private key

## Secrets

```bash
# Generate shared secret (same on Central and Worker)
openssl rand -base64 32 > /var/lib/catapult/worker-secret

# GitHub webhook secret
echo "your-webhook-secret" > /var/lib/catapult/webhook-secret

# GitHub App private key
cp downloaded-key.pem /var/lib/catapult/github-private-key.pem

chmod 600 /var/lib/catapult/*
```

## Central Configuration

```nix
{ config, pkgs, ... }:
{
  imports = [ /path/to/catapult/nixos/catapult.nix ];

  services.postgresql = {
    enable = true;
    ensureDatabases = [ "catapult" ];
    ensureUsers = [{ name = "catapult"; ensureDBOwnership = true; }];
  };

  services.catapult.central = {
    enable = true;
    databaseUrl = "postgresql://catapult@localhost/catapult";
    githubAppId = 123456;
    githubPrivateKeyFile = "/var/lib/catapult/github-private-key.pem";
    githubWebhookSecretFile = "/var/lib/catapult/webhook-secret";
    workerSharedSecretFile = "/var/lib/catapult/worker-secret";

    # Workers by zone (tenant)
    workers = {
      acme-corp = "https://deployer.acme.example.com";
      contoso = "https://deployer.contoso.example.com";
    };
  };

  services.caddy.virtualHosts."catapult.example.com".extraConfig = ''
    reverse_proxy localhost:8080
  '';
}
```

## Worker Configuration

```nix
{ config, pkgs, ... }:
{
  imports = [
    /path/to/catapult/nixos/catapult.nix
    /path/to/catapult/nixos/podman.nix
  ];

  services.catapult-podman = {
    enable = true;
    user = "catapult-worker";
  };

  services.catapult.worker = {
    enable = true;
    centralUrl = "https://catapult.example.com";
    workerSharedSecretFile = "/var/lib/catapult/worker-secret";
    caddyAdminApi = "http://localhost:2019";
    sitesDir = "/var/www/sites";

    # Optional: Cloudflare Tunnel for DNS management
    cloudflare = {
      enable = true;
      apiTokenFile = "/var/lib/catapult/cloudflare-token";
      accountId = "your-account-id";
      tunnelId = "your-tunnel-id";
      serviceUrl = "http://localhost:8080";
    };
  };

  services.caddy = {
    enable = true;
    globalConfig = "admin localhost:2019";

    virtualHosts."example.com".extraConfig = ''
      # Static config - Catapult adds dynamic routes via admin API
    '';
    virtualHosts."*.example.com".extraConfig = ''
      # Wildcard for PR previews
    '';
  };
}
```

## Repository Configuration

### Organization defaults (`{org}/.github/.deploy.json`)

```json
{
  "zone": "acme-corp",
  "domain_pattern": "{repo}.example.com",
  "pr_pattern": "pr-{pr}-{repo}.example.com"
}
```

### Repository config (`{org}/{repo}/.deploy.json`)

```json
{
  "build_type": "sveltekit",
  "build_command": "npm run build",
  "output_dir": "build"
}
```

### Options

| Field | Description | Example |
|-------|-------------|---------|
| `zone` | Worker zone | `"acme-corp"` |
| `domain_pattern` | Main branch domain | `"{repo}.example.com"` |
| `pr_pattern` | PR preview domain | `"pr-{pr}-{repo}.example.com"` |
| `domain` | Explicit domain | `"example.com"` |
| `subdomain` | Subdomain prefix | `"www"` |
| `build_type` | `sveltekit`, `vite`, `zola`, `custom` | `"sveltekit"` |
| `build_command` | Custom build command | `"npm run build"` |
| `output_dir` | Output directory | `"build"` |

## Cloudflare Tunnel (Optional)

For automatic DNS record and tunnel ingress management:

1. Create tunnel in Cloudflare Zero Trust (remotely managed)
2. Create API token with DNS:Edit and Cloudflare Tunnel:Edit permissions
3. Configure `services.catapult.worker.cloudflare` with IDs

On deploy, Catapult creates:
- Tunnel ingress rule: `pr-42-website.example.com → http://localhost:8080`
- DNS CNAME: `pr-42-website.example.com → {tunnel-id}.cfargotunnel.com`

## Verification

```bash
# Central
systemctl status catapult-central
journalctl -u catapult-central -f
curl http://localhost:8080/health

# Worker
systemctl status catapult-worker
journalctl -u catapult-worker -f
```

## Troubleshooting

**Webhook signature invalid:** Check webhook secret matches GitHub App config

**Podman connection failed:**
```bash
ls -la /run/podman/podman.sock
systemctl start podman.socket
```

**Caddy route fails:**
```bash
curl http://localhost:2019/config/
```
