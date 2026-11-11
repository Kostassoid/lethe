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
release-build:
    cargo build --locked --release

# Release a new version (dry run by default; add `--execute` to release): e.g. `just release 0.9.0`
release version *args:
    cargo release {{version}} {{args}}

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

# Install the Vagrant QEMU provider plugin (run once)
vm-setup:
    vagrant plugin install vagrant-qemu

# Install the macOS VM prerequisites (Apple Silicon; run once)
vm-setup-macos:
    #!/usr/bin/env bash
    set -euo pipefail

    if ! command -v tart >/dev/null 2>&1; then
      if ! command -v brew >/dev/null 2>&1; then
        echo "Homebrew is required to install Tart. See https://tart.run/quick-start/." >&2
        exit 1
      fi

      # Tart moved from the cirruslabs tap to openai/tools. Homebrew 6 also
      # requires trusting third-party taps before loading their formulae.
      brew tap openai/tools
      if brew help trust >/dev/null 2>&1; then
        brew trust openai/tools
      fi
      brew install openai/tools/tart
    fi

    tart --version
    vagrant plugin install vagrant-tart

# Show the state of the test VMs
vm-status:
    vagrant status

# Start a QEMU test VM: linux-x64, linux-arm64, windows-x64 or windows-arm64
vm-up platform="linux-x64":
    vagrant up {{platform}} --provider=qemu

# Start the macOS test VM on Apple Silicon
vm-up-macos platform="macos-arm64":
    vagrant up {{platform}} --provider=tart

# Open a shell in a test VM
vm-ssh platform="linux-x64":
    vagrant ssh {{platform}}

# Re-upload the working tree and re-run provisioning on a test VM
vm-sync platform="linux-x64":
    vagrant provision {{platform}}

# Run the integration tests inside a Linux test VM
vm-test-linux platform="linux-x64":
    vagrant upload testing/run-tests.sh /home/vagrant/lethe/testing/run-tests.sh {{platform}}
    vagrant ssh {{platform}} -c "cd ~/lethe && bash testing/run-tests.sh"

# Run the integration tests inside a Windows test VM
vm-test-windows platform="windows-x64":
    vagrant upload testing/run-tests.ps1 C:/Users/vagrant/lethe/testing/run-tests.ps1 {{platform}}
    vagrant ssh {{platform}} -c "powershell -NoProfile -ExecutionPolicy Bypass -File C:/Users/vagrant/lethe/testing/run-tests.ps1"

# Run the integration tests inside a macOS test VM
vm-test-macos platform="macos-arm64" user="admin":
    vagrant upload testing/run-tests.sh /Users/{{user}}/lethe/testing/run-tests.sh {{platform}}
    vagrant ssh {{platform}} -c "cd ~/lethe && bash testing/run-tests.sh"

# Stop and delete one test VM
vm-destroy platform="linux-x64":
    vagrant destroy -f {{platform}}

# Stop and delete all test VMs
vm-destroy-all:
    vagrant destroy -f

# Run integration tests on a macOS/Linux host (set LETHE_TEST_DEVICE)
test-integration:
    bash testing/run-tests.sh

# Run integration tests on a Windows host (set LETHE_TEST_DEVICE)
test-integration-win:
    powershell -NoProfile -ExecutionPolicy Bypass -File testing/run-tests.ps1

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
ci: fmt-check clippy test dist-check release-build

# Run CI checks without the release build
ci-quick: fmt-check clippy test dist-check
