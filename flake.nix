{
  description = "Catapult - automated deployment system for GitHub webhooks";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, crane, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        # Rust toolchain - stable for catapult
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-analyzer" "rust-src" ];
        };

        # Configure crane with our toolchain
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # Source filtering - include migrations for SQLx
        # Must filter from original source, not cleanCargoSource (which excludes .sql)
        sqlFilter = path: _type: builtins.match ".*\.sql$" path != null;
        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = path: type:
            (sqlFilter path type) || (craneLib.filterCargoSources path type);
        };

        # Common args for crane builds
        commonArgs = {
          inherit src;
          pname = "catapult";
          version = "0.1.0";
          strictDeps = true;

          nativeBuildInputs = with pkgs; [ pkg-config ];
          buildInputs = with pkgs; [ openssl ]
            ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
              pkgs.darwin.apple_sdk.frameworks.Security
              pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
            ];
        };

        # Build dependencies only (for caching)
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        # Build the catapult binary
        catapult = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          doCheck = false; # Tests require testcontainers/postgres
        });

      in {
        packages = {
          default = catapult;
          catapult = catapult;
        };

        # NixOS module for easy deployment
        nixosModules.default = { config, lib, pkgs, ... }:
          let
            cfg = config.services.catapult;
          in {
            options.services.catapult = {
              central = {
                enable = lib.mkEnableOption "Catapult Central server";

                package = lib.mkOption {
                  type = lib.types.package;
                  default = catapult;
                  description = "The catapult package to use";
                };

                listenAddr = lib.mkOption {
                  type = lib.types.str;
                  default = "0.0.0.0:8080";
                  description = "Address to listen on";
                };

                databaseUrl = lib.mkOption {
                  type = lib.types.str;
                  description = "PostgreSQL connection URL";
                };

                githubAppId = lib.mkOption {
                  type = lib.types.int;
                  description = "GitHub App ID";
                };

                githubPrivateKeyFile = lib.mkOption {
                  type = lib.types.path;
                  description = "Path to GitHub App private key PEM file";
                };

                githubWebhookSecretFile = lib.mkOption {
                  type = lib.types.path;
                  description = "Path to file containing GitHub webhook secret";
                };

                workerSharedSecretFile = lib.mkOption {
                  type = lib.types.path;
                  description = "Path to file containing worker shared secret";
                };

                adminApiKeyFile = lib.mkOption {
                  type = lib.types.path;
                  description = "Path to file containing admin API key";
                };

                workers = lib.mkOption {
                  type = lib.types.attrsOf lib.types.str;
                  default = {};
                  example = { nullislabs = "https://deployer.nullislabs.io"; };
                  description = "Worker endpoints by zone name";
                };
              };

              worker = {
                enable = lib.mkEnableOption "Catapult Worker server";

                package = lib.mkOption {
                  type = lib.types.package;
                  default = catapult;
                  description = "The catapult package to use";
                };

                listenAddr = lib.mkOption {
                  type = lib.types.str;
                  default = "0.0.0.0:8080";
                  description = "Address to listen on";
                };

                centralUrl = lib.mkOption {
                  type = lib.types.str;
                  description = "URL of the Central server";
                };

                workerSharedSecretFile = lib.mkOption {
                  type = lib.types.path;
                  description = "Path to file containing worker shared secret";
                };

                sitesDir = lib.mkOption {
                  type = lib.types.path;
                  default = "/var/www/sites";
                  description = "Directory where sites are deployed";
                };

                caddyAdminApi = lib.mkOption {
                  type = lib.types.str;
                  default = "http://localhost:2019";
                  description = "Caddy admin API URL";
                };

                cloudflare = {
                  enable = lib.mkEnableOption "Cloudflare Tunnel DNS integration";

                  apiTokenFile = lib.mkOption {
                    type = lib.types.nullOr lib.types.path;
                    default = null;
                    description = "Path to file containing Cloudflare API token";
                  };

                  accountId = lib.mkOption {
                    type = lib.types.nullOr lib.types.str;
                    default = null;
                    description = "Cloudflare Account ID";
                  };

                  tunnelId = lib.mkOption {
                    type = lib.types.nullOr lib.types.str;
                    default = null;
                    description = "Cloudflare Tunnel ID";
                  };

                  serviceUrl = lib.mkOption {
                    type = lib.types.str;
                    default = "http://localhost:8080";
                    description = "Local service URL for tunnel routing";
                  };
                };
              };
            };

            config = lib.mkMerge [
              (lib.mkIf cfg.central.enable {
                systemd.services.catapult-central = {
                  description = "Catapult Central Server";
                  wantedBy = [ "multi-user.target" ];
                  after = [ "network.target" "postgresql.service" ];

                  serviceConfig = {
                    Type = "simple";
                    ExecStart = let
                      workerArgs = lib.concatStringsSep " " (
                        lib.mapAttrsToList (zone: endpoint: "--worker ${zone}=${endpoint}") cfg.central.workers
                      );
                    in "${cfg.central.package}/bin/catapult central ${workerArgs}";
                    Restart = "always";
                    RestartSec = 5;

                    # Security hardening
                    DynamicUser = true;
                    ProtectSystem = "strict";
                    ProtectHome = true;
                    NoNewPrivileges = true;
                    PrivateTmp = true;
                  };

                  environment = {
                    LISTEN_ADDR = cfg.central.listenAddr;
                    DATABASE_URL = cfg.central.databaseUrl;
                    GITHUB_APP_ID = toString cfg.central.githubAppId;
                    GITHUB_PRIVATE_KEY_PATH = cfg.central.githubPrivateKeyFile;
                  };

                  script = ''
                    export GITHUB_WEBHOOK_SECRET=$(cat ${cfg.central.githubWebhookSecretFile})
                    export WORKER_SHARED_SECRET=$(cat ${cfg.central.workerSharedSecretFile})
                    export ADMIN_API_KEY=$(cat ${cfg.central.adminApiKeyFile})
                    exec ${cfg.central.package}/bin/catapult central ${
                      lib.concatStringsSep " " (
                        lib.mapAttrsToList (zone: endpoint: "--worker ${zone}=${endpoint}") cfg.central.workers
                      )
                    }
                  '';
                };
              })

              (lib.mkIf cfg.worker.enable {
                systemd.services.catapult-worker = {
                  description = "Catapult Worker Server";
                  wantedBy = [ "multi-user.target" ];
                  after = [ "network.target" "podman.socket" ];

                  serviceConfig = {
                    Type = "simple";
                    Restart = "always";
                    RestartSec = 5;

                    # Worker needs more privileges for podman
                    User = "catapult-worker";
                    Group = "catapult-worker";
                    SupplementaryGroups = [ "podman" ];
                  };

                  environment = {
                    LISTEN_ADDR = cfg.worker.listenAddr;
                    CENTRAL_URL = cfg.worker.centralUrl;
                    SITES_DIR = cfg.worker.sitesDir;
                    CADDY_ADMIN_API = cfg.worker.caddyAdminApi;
                  } // lib.optionalAttrs cfg.worker.cloudflare.enable {
                    CLOUDFLARE_ACCOUNT_ID = cfg.worker.cloudflare.accountId;
                    CLOUDFLARE_TUNNEL_ID = cfg.worker.cloudflare.tunnelId;
                    CLOUDFLARE_SERVICE_URL = cfg.worker.cloudflare.serviceUrl;
                  };

                  script = ''
                    export WORKER_SHARED_SECRET=$(cat ${cfg.worker.workerSharedSecretFile})
                    ${lib.optionalString (cfg.worker.cloudflare.enable && cfg.worker.cloudflare.apiTokenFile != null) ''
                      export CLOUDFLARE_API_TOKEN=$(cat ${cfg.worker.cloudflare.apiTokenFile})
                    ''}
                    exec ${cfg.worker.package}/bin/catapult worker
                  '';
                };

                users.users.catapult-worker = {
                  isSystemUser = true;
                  group = "catapult-worker";
                  home = "/var/lib/catapult-worker";
                  createHome = true;
                };

                users.groups.catapult-worker = {};
              })
            ];
          };

        devShells = {
          # Default development shell for working on catapult itself
          default = pkgs.mkShell {
            buildInputs = with pkgs; [
              rustToolchain
              openssl
              pkg-config
              just
              cargo-audit
              cargo-watch
              cargo-llvm-cov
              sqlx-cli
            ];

            OPENSSL_DIR = "${pkgs.openssl.dev}";
            OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
            RUST_BACKTRACE = "1";

            shellHook = ''
              echo "🚀 Catapult development environment loaded"
              echo "🦀 Rust: $(rustc --version)"
              echo ""
              echo "💡 Commands:"
              echo "   - cargo build           # Build catapult"
              echo "   - cargo test            # Run tests"
              echo "   - cargo watch -x run    # Development with hot reload"
              echo "   - nix build             # Build Nix package"
            '';
          };

          # Build environment for SvelteKit projects (used by worker)
          sveltekit = pkgs.mkShell {
            buildInputs = with pkgs; [
              nodejs_22
              nodePackages.npm
              git
              cacert
            ];

            shellHook = ''
              export SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt
              export NODE_OPTIONS="--max-old-space-size=4096"
            '';
          };

          # Build environment for Vite projects (used by worker)
          vite = pkgs.mkShell {
            buildInputs = with pkgs; [
              nodejs_22
              nodePackages.npm
              git
              cacert
            ];

            shellHook = ''
              export SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt
              export NODE_OPTIONS="--max-old-space-size=4096"
            '';
          };

          # Build environment for Zola static sites (used by worker)
          zola = pkgs.mkShell {
            buildInputs = with pkgs; [
              zola
              git
            ];
          };
        };
      }
    ) // {
      # System-independent NixOS module
      nixosModules.default = { config, lib, pkgs, ... }:
        let
          cfg = config.services.catapult;
          catapultPkg = self.packages.${pkgs.system}.default;
        in {
          options.services.catapult = {
            central = {
              enable = lib.mkEnableOption "Catapult Central server";

              package = lib.mkOption {
                type = lib.types.package;
                default = catapultPkg;
                description = "The catapult package to use";
              };

              listenAddr = lib.mkOption {
                type = lib.types.str;
                default = "0.0.0.0:8080";
                description = "Address to listen on";
              };

              databaseUrl = lib.mkOption {
                type = lib.types.str;
                description = "PostgreSQL connection URL";
              };

              githubAppId = lib.mkOption {
                type = lib.types.int;
                description = "GitHub App ID";
              };

              githubPrivateKeyFile = lib.mkOption {
                type = lib.types.path;
                description = "Path to GitHub App private key PEM file";
              };

              githubWebhookSecretFile = lib.mkOption {
                type = lib.types.path;
                description = "Path to file containing GitHub webhook secret";
              };

              workerSharedSecretFile = lib.mkOption {
                type = lib.types.path;
                description = "Path to file containing worker shared secret";
              };

              adminApiKeyFile = lib.mkOption {
                type = lib.types.path;
                description = "Path to file containing admin API key";
              };

              workers = lib.mkOption {
                type = lib.types.attrsOf lib.types.str;
                default = {};
                example = { nullislabs = "https://deployer.nullislabs.io"; };
                description = "Worker endpoints by zone name";
              };
            };

            worker = {
              enable = lib.mkEnableOption "Catapult Worker server";

              package = lib.mkOption {
                type = lib.types.package;
                default = catapultPkg;
                description = "The catapult package to use";
              };

              listenAddr = lib.mkOption {
                type = lib.types.str;
                default = "0.0.0.0:8080";
                description = "Address to listen on";
              };

              centralUrl = lib.mkOption {
                type = lib.types.str;
                description = "URL of the Central server";
              };

              workerSharedSecretFile = lib.mkOption {
                type = lib.types.path;
                description = "Path to file containing worker shared secret";
              };

              sitesDir = lib.mkOption {
                type = lib.types.path;
                default = "/var/www/sites";
                description = "Directory where sites are deployed";
              };

              caddyAdminApi = lib.mkOption {
                type = lib.types.str;
                default = "http://localhost:2019";
                description = "Caddy admin API URL";
              };

              cloudflare = {
                enable = lib.mkEnableOption "Cloudflare Tunnel DNS integration";

                apiTokenFile = lib.mkOption {
                  type = lib.types.nullOr lib.types.path;
                  default = null;
                  description = "Path to file containing Cloudflare API token";
                };

                accountId = lib.mkOption {
                  type = lib.types.nullOr lib.types.str;
                  default = null;
                  description = "Cloudflare Account ID";
                };

                tunnelId = lib.mkOption {
                  type = lib.types.nullOr lib.types.str;
                  default = null;
                  description = "Cloudflare Tunnel ID";
                };

                serviceUrl = lib.mkOption {
                  type = lib.types.str;
                  default = "http://localhost:8080";
                  description = "Local service URL for tunnel routing";
                };
              };
            };
          };

          config = lib.mkMerge [
            (lib.mkIf cfg.central.enable {
              systemd.services.catapult-central = {
                description = "Catapult Central Server";
                wantedBy = [ "multi-user.target" ];
                after = [ "network.target" "postgresql.service" ];

                serviceConfig = {
                  Type = "simple";
                  Restart = "always";
                  RestartSec = 5;
                  DynamicUser = true;
                  ProtectSystem = "strict";
                  ProtectHome = true;
                  NoNewPrivileges = true;
                  PrivateTmp = true;
                };

                environment = {
                  LISTEN_ADDR = cfg.central.listenAddr;
                  DATABASE_URL = cfg.central.databaseUrl;
                  GITHUB_APP_ID = toString cfg.central.githubAppId;
                  GITHUB_PRIVATE_KEY_PATH = cfg.central.githubPrivateKeyFile;
                };

                script = ''
                  export GITHUB_WEBHOOK_SECRET=$(cat ${cfg.central.githubWebhookSecretFile})
                  export WORKER_SHARED_SECRET=$(cat ${cfg.central.workerSharedSecretFile})
                  export ADMIN_API_KEY=$(cat ${cfg.central.adminApiKeyFile})
                  exec ${cfg.central.package}/bin/catapult central ${
                    lib.concatStringsSep " " (
                      lib.mapAttrsToList (zone: endpoint: "--worker ${zone}=${endpoint}") cfg.central.workers
                    )
                  }
                '';
              };
            })

            (lib.mkIf cfg.worker.enable {
              systemd.services.catapult-worker = {
                description = "Catapult Worker Server";
                wantedBy = [ "multi-user.target" ];
                after = [ "network.target" "podman.socket" ];

                serviceConfig = {
                  Type = "simple";
                  Restart = "always";
                  RestartSec = 5;
                  User = "catapult-worker";
                  Group = "catapult-worker";
                  SupplementaryGroups = [ "podman" ];
                };

                environment = {
                  LISTEN_ADDR = cfg.worker.listenAddr;
                  CENTRAL_URL = cfg.worker.centralUrl;
                  SITES_DIR = cfg.worker.sitesDir;
                  CADDY_ADMIN_API = cfg.worker.caddyAdminApi;
                } // lib.optionalAttrs cfg.worker.cloudflare.enable {
                  CLOUDFLARE_ACCOUNT_ID = cfg.worker.cloudflare.accountId;
                  CLOUDFLARE_TUNNEL_ID = cfg.worker.cloudflare.tunnelId;
                  CLOUDFLARE_SERVICE_URL = cfg.worker.cloudflare.serviceUrl;
                };

                script = ''
                  export WORKER_SHARED_SECRET=$(cat ${cfg.worker.workerSharedSecretFile})
                  ${lib.optionalString (cfg.worker.cloudflare.enable && cfg.worker.cloudflare.apiTokenFile != null) ''
                    export CLOUDFLARE_API_TOKEN=$(cat ${cfg.worker.cloudflare.apiTokenFile})
                  ''}
                  exec ${cfg.worker.package}/bin/catapult worker
                '';
              };

              users.users.catapult-worker = {
                isSystemUser = true;
                group = "catapult-worker";
                home = "/var/lib/catapult-worker";
                createHome = true;
              };

              users.groups.catapult-worker = {};
            })
          ];
        };
    };
}
