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

Catapult reads deployment configuration from `.deploy.json` files in your repositories:

1. **Organization defaults:** `{org}/.github/.deploy.json`
2. **Repository overrides:** `{org}/{repo}/.deploy.json`

Repository settings override organization defaults.

#### Example Organization Config (`{org}/.github/.deploy.json`)

```json
{
  "zone": "nullislabs",
  "domain_pattern": "{repo}.nxm.rs",
  "pr_pattern": "pr-{pr}-{repo}.nxm.rs",
  "enabled": true
}
```

#### Example Repository Config (`{org}/{repo}/.deploy.json`)

```json
{
  "build_type": "sveltekit",
  "build_command": "pnpm build",
  "output_dir": "build"
}
```

#### Configuration Options

| Field | Description | Example |
|-------|-------------|---------|
| `zone` | Worker zone to deploy to | `"nullislabs"` |
| `domain_pattern` | Domain pattern for main branch | `"{repo}.nxm.rs"` |
| `pr_pattern` | Domain pattern for PR previews | `"pr-{pr}-{repo}.nxm.rs"` |
| `domain` | Explicit domain (overrides pattern) | `"nxm.rs"` |
| `subdomain` | Subdomain prefix (used with `domain`) | `"www"` |
| `build_type` | Build type: `sveltekit`, `vite`, `zola`, `custom` | `"sveltekit"` |
| `build_command` | Custom build command | `"npm run build"` |
| `output_dir` | Build output directory | `"dist"` |
| `enabled` | Enable/disable deployments | `true` |

#### Domain Resolution Examples

**Apex domain (`nxm.rs`):**
```json
{ "domain": "nxm.rs" }
```

**WWW subdomain (`www.nxm.rs`):**
```json
{ "domain": "nxm.rs", "subdomain": "www" }
```

**Pattern-based (`{repo}.nxm.rs`):**
```json
{ "domain_pattern": "{repo}.nxm.rs" }
```
For repo "website", resolves to `website.nxm.rs`.

**PR previews:** Always use the `pr_pattern`:
```json
{ "pr_pattern": "pr-{pr}-{repo}.nxm.rs" }
```
For PR #42 on repo "website", resolves to `pr-42-website.nxm.rs`.

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

    # Cloudflare Tunnel integration (optional)
    # Creates DNS records and tunnel ingress rules automatically
    cloudflare = {
      enable = true;
      apiTokenFile = "/var/lib/catapult/cloudflare-token";
      accountId = "your-cloudflare-account-id";
      zoneId = "your-zone-id";
      tunnelId = "your-tunnel-id";
      serviceUrl = "http://localhost:8080";  # Where Caddy listens
    };
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

### Cloudflare Tunnel Setup (Optional)

Catapult can automatically manage DNS records and tunnel ingress rules when
deploying sites. This requires a remotely-managed Cloudflare Tunnel.

#### 1. Create a Tunnel

In the Cloudflare Zero Trust dashboard:

1. Go to **Networks → Tunnels**
2. Create a new tunnel (note the **Tunnel ID**)
3. Configure it as **remotely managed** (not config file based)
4. Install and run `cloudflared` with the provided token

#### 2. Create an API Token

In Cloudflare dashboard:

1. Go to **My Profile → API Tokens**
2. Create a token with these permissions:
   - **Zone > DNS > Edit** (for DNS record management)
   - **Account > Cloudflare Tunnel > Edit** (for tunnel ingress rules)
3. Save the token to `/var/lib/catapult/cloudflare-token`

```bash
echo "your-cloudflare-api-token" > /var/lib/catapult/cloudflare-token
chmod 600 /var/lib/catapult/cloudflare-token
chown catapult-worker:catapult-worker /var/lib/catapult/cloudflare-token
```

#### 3. Find Your IDs

- **Account ID:** Dashboard URL or Overview page sidebar
- **Zone ID:** Domain overview page sidebar
- **Tunnel ID:** From tunnel creation or Tunnels list

#### 4. Configure Worker

```nix
services.catapult.worker.cloudflare = {
  enable = true;
  apiTokenFile = "/var/lib/catapult/cloudflare-token";
  accountId = "f9ff5c79365f8f1851aa90ec0a0c7932";
  zoneId = "1234567890abcdef";
  tunnelId = "abc123-def456-ghi789";
  serviceUrl = "http://localhost:8080";  # Caddy's listen address
};
```

#### How It Works

When Catapult deploys a site:

1. **Creates tunnel ingress rule:** `pr-42-website.nxm.rs → http://localhost:8080`
2. **Creates DNS record:** CNAME `pr-42-website.nxm.rs → {tunnel-id}.cfargotunnel.com`
3. Traffic flows: User → Cloudflare → Tunnel → Caddy → Site

When a PR is closed:

1. DNS record is deleted
2. Tunnel ingress rule is removed

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
