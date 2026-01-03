# NixOS module for Catapult deployment
#
# This module provides systemd services for both Central and Worker modes.
#
# Usage in your NixOS configuration (~/.config/nixos/configuration.nix):
#
#   imports = [ ./path/to/catapult/nixos/catapult.nix ];
#
#   services.catapult.central = {
#     enable = true;
#     databaseUrl = "postgresql://catapult:password@localhost/catapult";
#     githubAppId = 123456;
#     githubPrivateKeyFile = "/var/lib/catapult/github-private-key.pem";
#     githubWebhookSecretFile = "/var/lib/catapult/webhook-secret";
#     workerSharedSecretFile = "/var/lib/catapult/worker-secret";
#     workers = {
#       nullislabs = "https://deployer.nullislabs.io";
#       nullispl = "https://deployer.nullis.pl";
#     };
#   };
#
#   services.catapult.worker = {
#     enable = true;
#     centralUrl = "https://catapult.example.com";
#     workerSharedSecretFile = "/var/lib/catapult/worker-secret";
#   };
#
{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.catapult;

  # Build the catapult binary from source
  # In production, you'd likely use a pre-built package
  catapultPackage = pkgs.rustPlatform.buildRustPackage rec {
    pname = "catapult";
    version = "0.1.0";
    src = ./..;
    cargoLock.lockFile = ./../Cargo.lock;
    nativeBuildInputs = [ pkgs.pkg-config ];
    buildInputs = [ pkgs.openssl ];
  };
in
{
  options.services.catapult = {
    # ==================== CENTRAL OPTIONS ====================
    central = {
      enable = mkEnableOption "Catapult Central server";

      package = mkOption {
        type = types.package;
        default = catapultPackage;
        description = "The catapult package to use";
      };

      user = mkOption {
        type = types.str;
        default = "catapult";
        description = "User to run the central service";
      };

      group = mkOption {
        type = types.str;
        default = "catapult";
        description = "Group to run the central service";
      };

      listenAddress = mkOption {
        type = types.str;
        default = "127.0.0.1:8080";
        description = "Address and port to listen on";
      };

      databaseUrl = mkOption {
        type = types.str;
        description = "PostgreSQL connection URL";
        example = "postgresql://catapult:password@localhost/catapult";
      };

      githubAppId = mkOption {
        type = types.int;
        description = "GitHub App ID";
      };

      githubPrivateKeyFile = mkOption {
        type = types.path;
        description = "Path to GitHub App private key PEM file";
      };

      githubWebhookSecretFile = mkOption {
        type = types.path;
        description = "Path to file containing GitHub webhook secret";
      };

      workerSharedSecretFile = mkOption {
        type = types.path;
        description = "Path to file containing worker shared secret";
      };

      logLevel = mkOption {
        type = types.str;
        default = "catapult=info,tower_http=info";
        description = "RUST_LOG filter string";
      };

      openFirewall = mkOption {
        type = types.bool;
        default = false;
        description = "Open firewall port for the listen address";
      };

      workers = mkOption {
        type = types.attrsOf types.str;
        default = { };
        description = ''
          Worker endpoints by zone name.
          Each zone is a deployment target (tenant) with a dedicated worker.
          Example: { nullislabs = "https://deployer.nullislabs.io"; }
        '';
        example = {
          nullislabs = "https://deployer.nullislabs.io";
          nullispl = "https://deployer.nullis.pl";
        };
      };
    };

    # ==================== WORKER OPTIONS ====================
    worker = {
      enable = mkEnableOption "Catapult Worker server";

      package = mkOption {
        type = types.package;
        default = catapultPackage;
        description = "The catapult package to use";
      };

      user = mkOption {
        type = types.str;
        default = "catapult-worker";
        description = "User to run the worker service";
      };

      group = mkOption {
        type = types.str;
        default = "catapult-worker";
        description = "Group to run the worker service";
      };

      listenAddress = mkOption {
        type = types.str;
        default = "127.0.0.1:8081";
        description = "Address and port to listen on";
      };

      centralUrl = mkOption {
        type = types.str;
        description = "URL of the Central server";
        example = "https://catapult.example.com";
      };

      workerSharedSecretFile = mkOption {
        type = types.path;
        description = "Path to file containing worker shared secret";
      };

      podmanSocket = mkOption {
        type = types.str;
        default = "/run/podman/podman.sock";
        description = "Path to Podman socket";
      };

      caddyAdminApi = mkOption {
        type = types.str;
        default = "http://localhost:2019";
        description = "URL of Caddy admin API";
      };

      sitesDir = mkOption {
        type = types.path;
        default = "/var/www/sites";
        description = "Directory where sites are deployed";
      };

      useContainers = mkOption {
        type = types.bool;
        default = true;
        description = "Use container isolation for builds";
      };

      buildImage = mkOption {
        type = types.str;
        default = "nixos/nix:latest";
        description = "Container image for builds";
      };

      containerMemoryLimit = mkOption {
        type = types.int;
        default = 4294967296; # 4GB
        description = "Memory limit for build containers in bytes";
      };

      containerCpuQuota = mkOption {
        type = types.int;
        default = 200000; # 2 CPUs
        description = "CPU quota for build containers (CPUs * 100000)";
      };

      containerPidsLimit = mkOption {
        type = types.int;
        default = 1000;
        description = "PID limit for build containers";
      };

      logLevel = mkOption {
        type = types.str;
        default = "catapult=info,tower_http=info";
        description = "RUST_LOG filter string";
      };
    };
  };

  config = mkMerge [
    # ==================== CENTRAL CONFIG ====================
    (mkIf cfg.central.enable {
      # Create user and group
      users.users.${cfg.central.user} = {
        isSystemUser = true;
        group = cfg.central.group;
        description = "Catapult Central service user";
      };
      users.groups.${cfg.central.group} = { };

      # Systemd service for Central
      systemd.services.catapult-central = {
        description = "Catapult Central - GitHub webhook orchestrator";
        after = [ "network.target" "postgresql.service" ];
        wants = [ "postgresql.service" ];
        wantedBy = [ "multi-user.target" ];

        environment = {
          RUST_LOG = cfg.central.logLevel;
          DATABASE_URL = cfg.central.databaseUrl;
          GITHUB_APP_ID = toString cfg.central.githubAppId;
          GITHUB_PRIVATE_KEY_PATH = cfg.central.githubPrivateKeyFile;
          LISTEN_ADDR = cfg.central.listenAddress;
        };

        serviceConfig = {
          Type = "simple";
          User = cfg.central.user;
          Group = cfg.central.group;
          ExecStart = "${cfg.central.package}/bin/catapult central";
          Restart = "always";
          RestartSec = 5;

          # Load secrets from files
          LoadCredential = [
            "webhook-secret:${cfg.central.githubWebhookSecretFile}"
            "worker-secret:${cfg.central.workerSharedSecretFile}"
          ];

          # Security hardening
          NoNewPrivileges = true;
          ProtectSystem = "strict";
          ProtectHome = true;
          PrivateTmp = true;
          PrivateDevices = true;
          ProtectKernelTunables = true;
          ProtectKernelModules = true;
          ProtectControlGroups = true;
          RestrictAddressFamilies = [ "AF_INET" "AF_INET6" "AF_UNIX" ];
          RestrictNamespaces = true;
          LockPersonality = true;
          MemoryDenyWriteExecute = true;
          RestrictRealtime = true;
          RestrictSUIDSGID = true;
          CapabilityBoundingSet = "";
        };

        # Read secrets and set environment variables
        script = let
          # Build --worker arguments from config
          workerArgs = lib.concatStringsSep " " (
            lib.mapAttrsToList (zone: endpoint: "--worker ${zone}=${endpoint}") cfg.central.workers
          );
        in ''
          export GITHUB_WEBHOOK_SECRET="$(cat $CREDENTIALS_DIRECTORY/webhook-secret)"
          export WORKER_SHARED_SECRET="$(cat $CREDENTIALS_DIRECTORY/worker-secret)"
          exec ${cfg.central.package}/bin/catapult central ${workerArgs}
        '';
      };

      # Open firewall if requested
      networking.firewall = mkIf cfg.central.openFirewall {
        allowedTCPPorts = [
          (lib.toInt (lib.last (lib.splitString ":" cfg.central.listenAddress)))
        ];
      };
    })

    # ==================== WORKER CONFIG ====================
    (mkIf cfg.worker.enable {
      # Create user and group
      users.users.${cfg.worker.user} = {
        isSystemUser = true;
        group = cfg.worker.group;
        description = "Catapult Worker service user";
        extraGroups = [ "podman" ];
      };
      users.groups.${cfg.worker.group} = { };

      # Ensure sites directory exists
      systemd.tmpfiles.rules = [
        "d ${cfg.worker.sitesDir} 0755 ${cfg.worker.user} ${cfg.worker.group} -"
      ];

      # Enable Podman
      virtualisation.podman = {
        enable = true;
        dockerCompat = true;
      };

      # Systemd service for Worker
      systemd.services.catapult-worker = {
        description = "Catapult Worker - build executor";
        after = [ "network.target" "podman.socket" ];
        wants = [ "podman.socket" ];
        wantedBy = [ "multi-user.target" ];

        environment = {
          RUST_LOG = cfg.worker.logLevel;
          CENTRAL_URL = cfg.worker.centralUrl;
          LISTEN_ADDR = cfg.worker.listenAddress;
          PODMAN_SOCKET = cfg.worker.podmanSocket;
          CADDY_ADMIN_API = cfg.worker.caddyAdminApi;
          SITES_DIR = cfg.worker.sitesDir;
          USE_CONTAINERS = if cfg.worker.useContainers then "true" else "false";
          BUILD_IMAGE = cfg.worker.buildImage;
          CONTAINER_MEMORY_LIMIT = toString cfg.worker.containerMemoryLimit;
          CONTAINER_CPU_QUOTA = toString cfg.worker.containerCpuQuota;
          CONTAINER_PIDS_LIMIT = toString cfg.worker.containerPidsLimit;
        };

        serviceConfig = {
          Type = "simple";
          User = cfg.worker.user;
          Group = cfg.worker.group;
          ExecStart = "${cfg.worker.package}/bin/catapult worker";
          Restart = "always";
          RestartSec = 5;

          # Load secrets from files
          LoadCredential = [
            "worker-secret:${cfg.worker.workerSharedSecretFile}"
          ];

          # Worker needs more permissions for Podman
          NoNewPrivileges = true;
          ProtectHome = true;
          PrivateTmp = true;
          ProtectKernelTunables = true;
          ProtectKernelModules = true;
          RestrictRealtime = true;
          RestrictSUIDSGID = true;

          # Allow access to Podman socket and sites directory
          ReadWritePaths = [ cfg.worker.sitesDir "/tmp" ];
          SupplementaryGroups = [ "podman" ];
        };

        # Read secrets and set environment variables
        script = ''
          export WORKER_SHARED_SECRET="$(cat $CREDENTIALS_DIRECTORY/worker-secret)"
          exec ${cfg.worker.package}/bin/catapult worker
        '';
      };
    })
  ];
}
