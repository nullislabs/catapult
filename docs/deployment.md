# Catapult Deployment Guide

This guide covers deploying Catapult on NixOS in a self-hosted environment.

## Prerequisites

- NixOS (tested on 24.05+)
- PostgreSQL (for Central)
- Podman (for Worker)
- Caddy (for Worker)
- A GitHub App configured

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│ Central Server (e.g., catapult.example.com)                 │
│                                                             │
│  ┌──────────────────┐    ┌──────────────────┐              │
│  │ catapult-central │───▶│    PostgreSQL    │              │
│  │    :8080         │    │      :5432       │              │
│  └──────────────────┘    └──────────────────┘              │
│          │                                                  │
│          │ (reverse proxy)                                  │
│          ▼                                                  │
│  ┌──────────────────┐                                      │
│  │      Caddy       │◀─── GitHub webhooks                  │
│  │   :80/:443       │                                      │
│  └──────────────────┘                                      │
└─────────────────────────────────────────────────────────────┘
                        │
                        │ HTTPS + HMAC
                        ▼
┌─────────────────────────────────────────────────────────────┐
│ Worker Server (e.g., deployer.example.com)                  │
│                                                             │
│  ┌──────────────────┐    ┌──────────────────┐              │
│  │ catapult-worker  │───▶│     Podman       │              │
│  │    :8081         │    │   (containers)   │              │
│  └──────────────────┘    └──────────────────┘              │
│          │                       │                          │
│          │                       ▼                          │
│          │               ┌──────────────────┐              │
│          └──────────────▶│      Caddy       │              │
│                          │   :80/:443/:2019 │              │
│                          │ /var/www/sites/  │              │
│                          └──────────────────┘              │
└─────────────────────────────────────────────────────────────┘
```

## Step 1: GitHub App Setup

1. Go to GitHub → Settings → Developer settings → GitHub Apps
2. Create a new GitHub App:
   - **Name:** Catapult (or your choice)
   - **Homepage URL:** https://catapult.example.com
   - **Webhook URL:** https://catapult.example.com/webhook/github
   - **Webhook secret:** Generate a secure random string

3. Permissions:
   - **Repository permissions:**
     - Contents: Read (for cloning)
     - Pull requests: Read and Write (for commenting)
   - **Subscribe to events:**
     - Pull request
     - Push

4. Generate and download the private key (PEM file)

5. Note down:
   - App ID
   - Webhook secret
   - Private key file

## Step 2: Generate Secrets

```bash
# Generate worker shared secret (same on Central and Worker)
openssl rand -base64 32 > /var/lib/catapult/worker-secret

# Copy webhook secret
echo "your-github-webhook-secret" > /var/lib/catapult/webhook-secret

# Copy GitHub App private key
cp /path/to/downloaded-key.pem /var/lib/catapult/github-private-key.pem

# Set permissions
chmod 600 /var/lib/catapult/*
chown catapult:catapult /var/lib/catapult/*
```

## Step 3: Central Server Configuration

### NixOS Configuration

Add to your NixOS configuration (e.g., `~/.config/nixos/catapult-central.nix`):

```nix
{ config, pkgs, ... }:

{
  # Import the catapult module
  imports = [ /path/to/catapult/nixos/catapult.nix ];

  # PostgreSQL for Central
  services.postgresql = {
    enable = true;
    ensureDatabases = [ "catapult" ];
    ensureUsers = [
      {
        name = "catapult";
        ensureDBOwnership = true;
      }
    ];
    authentication = ''
      local catapult catapult trust
      host catapult catapult 127.0.0.1/32 trust
    '';
  };

  # Catapult Central
  services.catapult.central = {
    enable = true;
    databaseUrl = "postgresql://catapult@localhost/catapult";
    githubAppId = 123456;  # Your GitHub App ID
    githubPrivateKeyFile = "/var/lib/catapult/github-private-key.pem";
    githubWebhookSecretFile = "/var/lib/catapult/webhook-secret";
    workerSharedSecretFile = "/var/lib/catapult/worker-secret";
    listenAddress = "127.0.0.1:8080";
    logLevel = "catapult=info,tower_http=info";

    # Worker endpoints by zone (tenant)
    # Each zone has a dedicated worker that handles deployments for that zone
    workers = {
      nullislabs = "https://deployer.nullislabs.io";
      nullispl = "https://deployer.nullis.pl";
    };
  };

  # Caddy as reverse proxy
  services.caddy = {
    enable = true;
    virtualHosts."catapult.example.com".extraConfig = ''
      reverse_proxy localhost:8080
    '';
  };

  # Secrets directory
  systemd.tmpfiles.rules = [
    "d /var/lib/catapult 0750 catapult catapult -"
  ];
}
```

### Database Setup

After enabling PostgreSQL, create the tables:

```bash
# Tables are created automatically on first run via migrations
sudo systemctl start catapult-central
```

### Worker Registration

Workers are automatically registered from your Central configuration on startup.
The `workers` attribute in your NixOS config defines all available workers:

```nix
workers = {
  nullislabs = "https://deployer.nullislabs.io";
  nullispl = "https://deployer.nullis.pl";
};
```

Each key is a **zone** (tenant identifier) that matches the `environment` field
in your deployment configs. Workers are synced to the database on Central startup,
and workers not in the config are automatically disabled.

### Configure Deployments

Register repositories for deployment. The `environment` field must match a zone
defined in your `workers` configuration:

```sql
INSERT INTO deployment_config
  (github_org, github_repo, environment, domain, subdomain, site_type, enabled)
VALUES
  ('nullislabs', 'website', 'nullislabs', 'nullislabs.io', 'www', 'sveltekit', true),
  ('nullislabs', 'docs', 'nullislabs', 'docs.nullislabs.io', NULL, 'vite', true),
  ('nullispl', 'portfolio', 'nullispl', 'nullis.pl', NULL, 'zola', true);
```

## Step 4: Worker Server Configuration

### NixOS Configuration

Add to your NixOS configuration (e.g., `~/.config/nixos/catapult-worker.nix`):

```nix
{ config, pkgs, ... }:

{
  # Import the catapult modules
  imports = [
    /path/to/catapult/nixos/catapult.nix
    /path/to/catapult/nixos/podman.nix
  ];

  # Podman configuration for builds
  services.catapult-podman = {
    enable = true;
    user = "catapult-worker";
    useRootless = false;  # Use rootful for iptables network isolation
  };

  # Catapult Worker
  services.catapult.worker = {
    enable = true;
    centralUrl = "https://catapult.example.com";
    workerSharedSecretFile = "/var/lib/catapult/worker-secret";
    listenAddress = "127.0.0.1:8081";

    # Podman configuration
    podmanSocket = "/run/podman/podman.sock";
    useContainers = true;
    buildImage = "nixos/nix:latest";

    # Resource limits
    containerMemoryLimit = 4294967296;  # 4GB
    containerCpuQuota = 200000;         # 2 CPUs
    containerPidsLimit = 1000;

    # Caddy integration
    caddyAdminApi = "http://localhost:2019";
    sitesDir = "/var/www/sites";

    logLevel = "catapult=info,tower_http=info";
  };

  # Caddy for serving sites
  #
  # Catapult dynamically adds routes via the admin API.
  # Domain-level static config (Matrix well-known, etc.) goes in virtualHosts.
  # Dynamic deployment routes are merged with this config automatically.
  services.caddy = {
    enable = true;
    globalConfig = ''
      admin localhost:2019
    '';

    # Example: Domain config with Matrix well-known
    virtualHosts."nxm.rs".extraConfig = ''
      # Matrix server delegation (static, always present)
      handle /.well-known/matrix/* {
        header Content-Type application/json
        respond `{"m.server":"matrix.nxm.rs:443"}`
      }

      # Catapult adds file_server routes via admin API
      # They merge with this config automatically
    '';

    # PR preview wildcard domain
    virtualHosts."*.nxm.rs".extraConfig = ''
      # Catapult adds routes for pr-{N}-{repo}.nxm.rs
    '';
  };

  # Secrets directory
  systemd.tmpfiles.rules = [
    "d /var/lib/catapult 0750 catapult-worker catapult-worker -"
    "d /var/www/sites 0755 catapult-worker catapult-worker -"
  ];

  # Firewall
  networking.firewall.allowedTCPPorts = [ 80 443 ];
}
```

### Worker Secrets

Copy the same worker secret used on Central:

```bash
# On worker server
mkdir -p /var/lib/catapult
# Copy worker-secret from central server
scp central:/var/lib/catapult/worker-secret /var/lib/catapult/
chmod 600 /var/lib/catapult/worker-secret
chown catapult-worker:catapult-worker /var/lib/catapult/worker-secret
```

## Step 5: Verification

### Check Central

```bash
# Check service status
sudo systemctl status catapult-central

# View logs
sudo journalctl -u catapult-central -f

# Test health endpoint
curl http://localhost:8080/health
```

### Check Worker

```bash
# Check service status
sudo systemctl status catapult-worker

# View logs
sudo journalctl -u catapult-worker -f

# Test Podman connection
podman --remote info
```

### Test Webhook

1. Install the GitHub App on a test repository
2. Create a PR
3. Check Central logs for webhook receipt
4. Verify "Building..." comment appears on PR
5. Check Worker logs for build execution
6. Verify deployment URL in PR comment

## Caddy Configuration Architecture

Catapult uses a **hybrid static + dynamic** approach for Caddy configuration:

```
┌─────────────────────────────────────────────────────────────────────┐
│  Static Config (NixOS Caddyfile)                                    │
│  - Domain-level settings (Matrix well-known, headers, redirects)    │
│  - TLS/ACME configuration                                           │
│  - Base virtual hosts                                               │
│  - Managed by NixOS, requires rebuild to change                     │
└─────────────────────────────────────────────────────────────────────┘
                              +
┌─────────────────────────────────────────────────────────────────────┐
│  Dynamic Routes (Caddy Admin API)                                   │
│  - Per-deployment file_server routes                                │
│  - Added/removed by Catapult worker                                 │
│  - Changes take effect immediately, no reload needed                │
│  - Merged with static config automatically                          │
└─────────────────────────────────────────────────────────────────────┘
```

### How It Works

1. **Caddy starts** with your NixOS-defined Caddyfile (static config)
2. **Admin API enabled** on `localhost:2019`
3. **Catapult deploys** a site → POSTs route to admin API
4. **Route is merged** with existing config, takes effect immediately
5. **PR closed** → Catapult DELETEs the route via admin API

### Domain-Level Config Examples

Put static, domain-level configuration in your NixOS Caddy config:

```nix
services.caddy.virtualHosts."example.com".extraConfig = ''
  # Matrix server delegation
  handle /.well-known/matrix/* {
    header Content-Type application/json
    respond `{"m.server":"matrix.example.com:443"}`
  }

  # Security headers
  header {
    X-Content-Type-Options nosniff
    X-Frame-Options DENY
    Referrer-Policy strict-origin-when-cross-origin
  }

  # Redirect www to apex
  @www host www.example.com
  redir @www https://example.com{uri} permanent
'';
```

Catapult's dynamic routes handle the actual site content (`file_server` for
the deployed static files).

## Troubleshooting

### Central Issues

**Database connection failed:**
```bash
# Check PostgreSQL is running
sudo systemctl status postgresql

# Check connection manually
sudo -u catapult psql -d catapult -c "SELECT 1"
```

**Webhook signature invalid:**
- Verify webhook secret matches in GitHub App and config
- Check for trailing newlines in secret file

### Worker Issues

**Podman connection failed:**
```bash
# Check Podman socket exists
ls -la /run/podman/podman.sock

# Start Podman socket if needed
sudo systemctl start podman.socket

# Check user is in podman group
groups catapult-worker
```

**Build container fails:**
```bash
# Test container manually
sudo podman run --rm docker.io/library/alpine:latest echo "Hello"

# Check network isolation
sudo iptables -L CATAPULT_BUILD_ISOLATION -v
```

**Caddy route configuration fails:**
```bash
# Check Caddy admin API
curl http://localhost:2019/config/

# Check Caddy is running
sudo systemctl status caddy
```

### Network Issues

**RFC1918 blocking not working:**
```bash
# Verify iptables rules exist
sudo iptables -L CATAPULT_BUILD_ISOLATION -v

# If missing, the network module needs root access
# Check worker is using system Podman socket, not rootless
```

## Security Checklist

- [ ] GitHub App private key has 600 permissions
- [ ] All secret files have 600 permissions
- [ ] Worker shared secret is the same on Central and Worker
- [ ] Webhook secret matches GitHub App configuration
- [ ] Central is behind HTTPS (via Caddy/nginx)
- [ ] Worker is behind HTTPS for Central communication
- [ ] Podman socket has proper permissions
- [ ] Sites directory is writable by worker user
- [ ] Firewall only allows necessary ports

## Monitoring

### Log Levels

Adjust `logLevel` for more/less verbosity:

```nix
logLevel = "catapult=debug,tower_http=debug";  # Verbose
logLevel = "catapult=info,tower_http=warn";    # Normal
logLevel = "catapult=warn";                     # Quiet
```

### Metrics

The application logs include timing information. For production monitoring, consider:

1. Log aggregation (Loki, Elasticsearch)
2. Adding a metrics endpoint (future enhancement)
3. Alerting on error log patterns

## Backup

### Central

Backup PostgreSQL database:

```bash
sudo -u postgres pg_dump catapult > catapult-backup.sql
```

Backup secrets:

```bash
tar -czf catapult-secrets.tar.gz /var/lib/catapult/
```

### Worker

No persistent state beyond deployed sites. Sites are rebuilt on each deployment.
