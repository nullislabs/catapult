# Catapult development commands
# Run `just --list` to see all available commands

# Default recipe - show help
default:
    @just --list

# Build the project
build:
    cargo build

# Build in release mode
build-release:
    cargo build --release

# Run all tests
test:
    cargo test

# Run tests with output
test-verbose:
    cargo test -- --nocapture

# Run container integration tests (requires Podman)
test-containers:
    cargo test --test container_integration -- --ignored

# Run all tests including integration tests
test-all:
    cargo test
    cargo test --test container_integration -- --ignored

# Check code without building
check:
    cargo check

# Run clippy lints
lint:
    cargo clippy -- -D warnings

# Format code
fmt:
    cargo fmt

# Check formatting
fmt-check:
    cargo fmt -- --check

# Run security audit
audit:
    cargo audit

# Watch for changes and run tests
watch:
    cargo watch -x test

# Watch for changes and run (central mode)
watch-central:
    cargo watch -x "run -- central"

# Watch for changes and run (worker mode)
watch-worker:
    cargo watch -x "run -- worker"

# === Coverage Commands ===

# Run tests with coverage and show summary (unit tests only)
coverage:
    cargo llvm-cov --all-features

# Run tests with coverage and generate HTML report (unit tests only)
coverage-html:
    cargo llvm-cov --all-features --html
    @echo "Coverage report generated at target/llvm-cov/html/index.html"

# Run ALL tests with coverage (including container integration tests)
# Requires: systemctl --user start podman.socket
coverage-all:
    cargo llvm-cov --all-features -- --include-ignored

# Run ALL tests and generate HTML report
# Requires: systemctl --user start podman.socket
coverage-all-html:
    cargo llvm-cov --all-features --html -- --include-ignored
    @echo "Coverage report generated at target/llvm-cov/html/index.html"

# Run tests with coverage and generate lcov report (for CI)
coverage-lcov:
    cargo llvm-cov --all-features --lcov --output-path target/llvm-cov/lcov.info

# Run ALL tests and generate lcov report
coverage-all-lcov:
    cargo llvm-cov --all-features --lcov --output-path target/llvm-cov/lcov.info -- --include-ignored

# Run tests with coverage and open HTML report in browser
coverage-open: coverage-html
    open target/llvm-cov/html/index.html || xdg-open target/llvm-cov/html/index.html

# Run ALL tests with coverage and open HTML report
coverage-all-open: coverage-all-html
    open target/llvm-cov/html/index.html || xdg-open target/llvm-cov/html/index.html

# Show uncovered lines in terminal
coverage-uncovered:
    cargo llvm-cov --all-features --show-missing-lines

# Show uncovered lines for ALL tests
coverage-all-uncovered:
    cargo llvm-cov --all-features --show-missing-lines -- --include-ignored

# Clean coverage data
coverage-clean:
    cargo llvm-cov clean --workspace

# === CI Commands ===

# Run all CI checks (what GitHub Actions runs)
ci: fmt-check lint test coverage-lcov
    @echo "All CI checks passed!"

# Clean build artifacts
clean:
    cargo clean
