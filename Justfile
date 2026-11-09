set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]

alias b := build
alias c := check
alias f := fmt
alias l := clippy
alias t := test

# List available recipes
default:
    @just --list

# Format source code
fmt:
    cargo fmt --all

# Check formatting without modifying files
fmt-check:
    cargo fmt --all -- --check

# Run Clippy on all targets
clippy:
    cargo clippy --locked --all-targets --all-features

# Type-check all targets without producing binaries
check:
    cargo check --locked --all-targets --all-features

# Build the debug binary; extra flags are forwarded
build *args:
    cargo build --locked {{args}}

# Build the optimized release binary
release:
    cargo build --locked --release

# Run tests; extra flags are forwarded
test *args:
    cargo test --locked {{args}}

# Run the app with the given arguments
run *args:
    cargo run --locked -- {{args}}

# Print CLI help
help:
    cargo run --locked -- --help

# List storage devices (requires administrator/root)
devices:
    cargo run --locked -- list

# Wipe a device, e.g. `just wipe /dev/sdX random yes 1m`
wipe device scheme="zero" verify="no" blocksize="128k" *args:
    cargo run --locked -- wipe --scheme {{scheme}} --verify {{verify}} --blocksize {{blocksize}} {{args}} {{device}}

# Verify cargo-dist configuration is up to date
dist-check:
    dist generate --check

# Preview the cargo-dist release plan for a tag
dist-plan tag:
    dist plan --tag={{tag}}

# Build API documentation
doc:
    cargo doc --locked --no-deps

# Update dependencies
update *args:
    cargo update {{args}}

# Install the binary from the current source tree
install *args:
    cargo install --locked --path . {{args}}

# Remove build artifacts
clean:
    cargo clean

# Show toolchain versions
info:
    @cargo --version
    @rustc --version
    @cargo fmt --version
    @cargo clippy --version

# Run the full CI check suite
ci: fmt-check clippy test dist-check release

# Run CI checks without the release build
ci-quick: fmt-check clippy test dist-check
