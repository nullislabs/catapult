# NixOS module for Catapult Podman configuration
#
# This module provides the Podman configuration needed to run
# Catapult's container-based builds.
#
# Usage in your NixOS configuration:
#
#   imports = [ ./path/to/catapult/nixos/podman.nix ];
#
#   services.catapult-podman = {
#     enable = true;
#     user = "catapult";  # The user running the catapult worker
#   };
#
{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.catapult-podman;
in
{
  options.services.catapult-podman = {
    enable = mkEnableOption "Catapult Podman configuration";

    user = mkOption {
      type = types.str;
      default = "catapult";
      description = "The user that will run Catapult worker and needs Podman access";
    };

    useRootless = mkOption {
      type = types.bool;
      default = false;
      description = ''
        Use rootless Podman (recommended for development/testing).
        For production with RFC1918 network blocking, set to false.
      '';
    };
  };

  config = mkIf cfg.enable {
    # Enable containers and Podman
    virtualisation = {
      containers.enable = true;
      podman = {
        enable = true;
        # Enable Docker-compatible CLI and socket
        dockerCompat = true;
        # Enable DNS in the default network
        defaultNetwork.settings.dns_enabled = true;
      };
    };

    # Configure user with subuid/subgid ranges for rootless containers
    users.users.${cfg.user} = {
      subUidRanges = [{ startUid = 100000; count = 65536; }];
      subGidRanges = [{ startGid = 100000; count = 65536; }];
      extraGroups = [ "podman" ];
    };

    # For rootful mode, enable the system-wide Podman socket
    systemd.services.podman = mkIf (!cfg.useRootless) {
      enable = true;
      wantedBy = [ "multi-user.target" ];
    };

    # For rootful mode, allow the catapult user to access the Podman socket
    systemd.tmpfiles.rules = mkIf (!cfg.useRootless) [
      "d /run/podman 0755 root podman -"
    ];

    # Ensure the podman socket has correct permissions
    systemd.sockets.podman = mkIf (!cfg.useRootless) {
      socketConfig = {
        SocketMode = "0660";
        SocketGroup = "podman";
      };
    };

    # For rootless mode, enable user linger so the socket persists
    # This allows the user's Podman socket to stay active even when not logged in
    systemd.tmpfiles.settings = mkIf cfg.useRootless {
      "10-catapult-linger" = {
        "/var/lib/systemd/linger/${cfg.user}" = {
          f = {
            mode = "0644";
          };
        };
      };
    };
  };
}
