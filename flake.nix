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
        devShells.default = with pkgs; mkShell {
          buildInputs = [
            # System dependencies
            openssl
            pkg-config

            # Rust toolchain (stable)
            (rust-bin.stable.latest.default.override {
              extensions = [
                "rust-src"
                "rust-analyzer"
              ];
            })

            # Development tools
            just
            cargo-audit
            cargo-watch
          ];

          # Set OpenSSL environment variables
          OPENSSL_DIR = "${openssl.dev}";
          OPENSSL_LIB_DIR = "${openssl.out}/lib";
        };

        packages.default = pkgs.hello; # Placeholder until we have a proper build
      }
    );
}
