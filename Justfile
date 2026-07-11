# commet — common dev tasks
#
# Run `just` (no args) to list recipes. Install with `cargo install just` or
# `brew install just`. CI continues to call `cargo` directly so this file is
# never on the critical path of a release build.

# Default recipe: list everything.
default:
    @just --list --unsorted

# Build the crate (debug profile).
build:
    cargo build

# Build the crate in release mode.
build-release:
    cargo build --release

# Run the `cc` binary; pass extra args after `--`, e.g. `just run -- --help`.
run *args:
    cargo run -- {{args}}

# Run all tests (unit + integration + doc). `--all-features` matches CI
# and enables the `mock` provider the e2e suite needs.
test:
    cargo test --all-features

# Run a single test by substring match (cargo test's built-in filter).
test-one name:
    cargo test --all-features {{name}}

# Format every Rust file in the workspace.
fmt:
    cargo fmt --all

# Verify formatting without writing — matches CI.
fmt-check:
    cargo fmt --all --check

# Clippy with deny-warnings — matches CI.
lint:
    cargo clippy --all-targets --all-features -- -D warnings

# Everything CI runs, in CI order. Fails fast on the first miss.
ci: fmt-check lint test

# Auto-fix formatting, then run lint + tests. Use before pushing.
pre-push: fmt lint test

# Open rustdoc for this crate in the browser.
doc:
    cargo doc --no-deps --open

# `cargo clean` — wipe `target/`.
clean:
    cargo clean
