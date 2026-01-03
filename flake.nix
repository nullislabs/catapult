{
  description = "catapult - automated deployment runner for GitHub webhooks";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      nixpkgs,
      rust-overlay,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
      in
      {
        devShells = {
          # Default development shell for working on catapult itself
          default = with pkgs; mkShell {
            buildInputs = [
              # System dependencies
              openssl
              pkg-config

              # Rust toolchain (stable with coverage support)
              (rust-bin.stable.latest.default.override {
                extensions = [
                  "rust-src"
                  "rust-analyzer"
                  "llvm-tools-preview"  # Required for cargo-llvm-cov
                ];
              })

              # Development tools
              just
              cargo-audit
              cargo-watch
              cargo-llvm-cov
            ];

            # Set OpenSSL environment variables
            OPENSSL_DIR = "${openssl.dev}";
            OPENSSL_LIB_DIR = "${openssl.out}/lib";
          };

          # Build environment for SvelteKit projects
          sveltekit = with pkgs; mkShell {
            buildInputs = [
              nodejs_22
              nodePackages.npm
              # Common build tools
              git
              cacert
            ];

            shellHook = ''
              export SSL_CERT_FILE=${cacert}/etc/ssl/certs/ca-bundle.crt
              export NODE_OPTIONS="--max-old-space-size=4096"
            '';
          };

          # Build environment for Vite projects
          vite = with pkgs; mkShell {
            buildInputs = [
              nodejs_22
              nodePackages.npm
              # Common build tools
              git
              cacert
            ];

            shellHook = ''
              export SSL_CERT_FILE=${cacert}/etc/ssl/certs/ca-bundle.crt
              export NODE_OPTIONS="--max-old-space-size=4096"
            '';
          };

          # Build environment for Zola static sites
          zola = with pkgs; mkShell {
            buildInputs = [
              zola
              # Common build tools
              git
            ];
          };
        };

        packages.default = pkgs.hello; # Placeholder until we have a proper build
      }
    );
}
