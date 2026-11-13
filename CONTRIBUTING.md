# Contributing to Lethe

Thanks for your interest in improving Lethe. This document covers development, testing, benchmarks
and release procedures.

## Development quick start

Install the latest Rust toolchain and [`just`](https://github.com/casey/just). From the repository
root:

```sh
just fmt
just check
just clippy
just test
```

The integration test suite runs inside disposable VMs and verifies real wiping behavior.

## Benchmarks

Measured on MacBook Air (M3, 2024) running macOS 27.2.1 against a 10 GiB (10,737,418,240 bytes) partition on a removable drive, using 1 MiB blocks.
Times include process startup and teardown.

**Zero fill**

| Command                                                                    | Time taken (seconds) | Throughput |
|----------------------------------------------------------------------------|----------------------|------------|
| `dd if=/dev/zero of=/dev/rdisk4s2 bs=1m`                                   | 32.34                | 332 MB/s   |
| `lethe wipe --scheme=zero --blocksize=1m --verify=no --yes /dev/rdisk4s2`  | 34.20                | 314 MB/s   |

**Random fill**

| Command                                                                      | Time taken (seconds) | Throughput |
|------------------------------------------------------------------------------|----------------------|------------|
| `dd if=/dev/urandom of=/dev/rdisk4s2 bs=1m`                                  | 49.33                | 218 MB/s   |
| `lethe wipe --scheme=random --blocksize=1m --verify=no --yes /dev/rdisk4s2`  | 34.18                | 314 MB/s   |

## Testing

The integration test suite runs inside disposable VMs and verifies real wiping behavior. The
recipes below use the [`just`](https://github.com/casey/just) command runner.

The `Vagrantfile` provisions disposable VMs with an extra 1 GiB scratch disk that the integration tests wipe. Linux and
Windows guests use QEMU through the [`vagrant-qemu`](https://github.com/ppggff/vagrant-qemu) plugin. macOS guests on
Apple Silicon use Apple's Virtualization.framework through [Tart](https://tart.run/) and the
[`vagrant-tart`](https://github.com/letiemble/vagrant-tart) plugin.

### Prerequisites

Linux and Windows hosts need [Vagrant](https://developer.hashicorp.com/vagrant), [QEMU](https://www.qemu.org/) and
`vagrant-qemu`:

```sh
vagrant plugin install vagrant-qemu
```

QEMU is auto-detected in the usual install locations (including `C:\Program Files\qemu` on Windows and the
Homebrew/`/usr` prefixes elsewhere). Set `QEMU_DIR` if your installation lives somewhere else. On Windows the provider
uses TCG software emulation by default because WHPX is unreliable or unavailable on many hosts. Set
`LETHE_QEMU_ACCEL=whpx` to opt into hardware acceleration; Linux and macOS use KVM/HVF automatically.

macOS VM testing requires an Apple Silicon Mac, Tart and `vagrant-tart`. The `vm-setup-macos` recipe installs both;
to do it manually, run:

```sh
brew tap openai/tools
brew trust openai/tools
brew install openai/tools/tart
vagrant plugin install vagrant-tart
```

Tart moved from the `cirruslabs/cli` tap to `openai/tools`. On macOS 13-15 the Homebrew formula's `softnet` dependency
requires macOS Tahoe, so install Tart from the release archive instead (see the
[Tart quick start](https://tart.run/quick-start/)).

### Supported machines

| Machine         | Guest                      | Notes                                                         |
|-----------------|----------------------------|---------------------------------------------------------------|
| `linux-x64`     | Ubuntu 24.04 x86-64        | Native KVM/WHV/HVF acceleration where available               |
| `linux-arm64`   | Ubuntu 24.04 ARM64         | TCG emulation on non-ARM hosts                                |
| `windows-x64`   | Windows Server 2025 x86-64 | Builds with the Rust GNU toolchain inside the VM              |
| `windows-arm64` | Windows ARM64              | Requires `LETHE_WINDOWS_ARM64_BOX`; no public QEMU box exists |
| `macos-arm64`   | macOS on Apple Silicon     | Apple Silicon host only; uses Tart instead of QEMU            |

Apple only permits macOS virtualization on Apple-branded hardware, so `macos-arm64` is only defined on Apple Silicon
hosts. GitHub-hosted macOS runners do not support nested virtualization; run this machine on a self-hosted Apple Silicon
Mac. The default image is `ghcr.io/cirruslabs/macos-sequoia-base:latest` (about 25 GB compressed and a 50 GB virtual
disk); override it with `LETHE_MACOS_IMAGE`. `LETHE_MACOS_DISK_GB` defaults to the
selected image's disk size; Tart cannot shrink an existing VM, so destroy and
recreate the VM after switching images. Use `LETHE_MACOS_VM_NAME`, `LETHE_MACOS_USER`, and `LETHE_MACOS_PASSWORD` to
customize the VM name and SSH credentials. The first boot of a freshly cloned image can take 15+ minutes while macOS
finishes first-boot setup, so the macOS machine uses a 30-minute boot timeout by default (`LETHE_MACOS_BOOT_TIMEOUT`).
See [macOS VMs with bridged networking](#macos-vms-with-bridged-networking) if a host VPN blocks the VM's network access.

### Running the tests

Each `vm-test-*` recipe uploads the current test script, rebuilds the working tree inside the guest, wipes the extra
1 GiB scratch disk with the `zero` scheme using full verification, checks independently that the disk contains only
zeroes, then repeats with the `random` scheme.

Linux:

```sh
just vm-setup          # once: installs vagrant-qemu
just vm-up linux-x64
just vm-test-linux linux-x64
just vm-destroy linux-x64
```

Windows:

```sh
just vm-setup
just vm-up windows-x64
just vm-test-windows windows-x64
just vm-destroy windows-x64
```

macOS on Apple Silicon:

```sh
just vm-setup-macos    # once: installs Tart and vagrant-tart
just vm-up-macos
just vm-test-macos
just vm-destroy macos-arm64
```

`vm-test-macos` uploads only `testing/run-tests.sh`. After changing application source, run
`just vm-sync macos-arm64` to re-upload the working tree and re-run provisioning, then `just vm-test-macos`.

### macOS VMs with bridged networking

Tart uses shared (NAT) networking by default. Some host VPNs, notably Mullvad, block forwarded traffic from the VM: the
guest can resolve DNS but cannot open connections, so `rustup` and `cargo` time out during provisioning. Use bridged
networking when that happens; the guest gets an address from your LAN and its traffic bypasses the host VPN.

1. Find the active LAN interface. Wi-Fi is usually `en0`:

   ```sh
   ifconfig | grep -B2 'inet '
   ```

2. Export the bridged interface and start the VM:

   ```sh
   export LETHE_MACOS_NET_BRIDGED=en0
   just vm-up-macos
   ```

3. Once the VM is running (it may still be running even if `vm-up-macos` stopped with a "not ready for SSH" message),
   get its LAN address and keep it for the rest of the session:

   ```sh
   export LETHE_MACOS_SSH_HOST="$(tart ip --resolver arp --wait 30 lethe-macos-arm64)"
   ```

   If `tart ip` returns nothing, the ARP entry has not been populated yet. Find the address in your router's DHCP
   leases or from the guest console (`ipconfig getifaddr en0`), then set it explicitly:

   ```sh
   export LETHE_MACOS_SSH_HOST=192.168.0.91
   ```

4. Run the tests:

   ```sh
   just vm-test-macos
   ```

Keep both exports for the whole session. `LETHE_MACOS_NET_BRIDGED` must be set whenever the VM is (re)started;
`LETHE_MACOS_SSH_HOST` avoids flaky ARP-based discovery while the VM is idle. If the VM is destroyed and recreated, the
DHCP lease normally stays the same, but refresh the address if it changed:

```sh
export LETHE_MACOS_SSH_HOST="$(tart ip --resolver arp --wait 30 lethe-macos-arm64)"
```

To switch back to shared networking, destroy the VM first, unset both variables, then start it again:

```sh
just vm-destroy macos-arm64
unset LETHE_MACOS_NET_BRIDGED LETHE_MACOS_SSH_HOST
just vm-up-macos
```

With bridged networking the VM is visible on your LAN and its traffic does not go through the host VPN. If you prefer
not to use bridged networking, disconnect the host VPN while provisioning and testing instead:

```sh
mullvad disconnect
just vm-up-macos
just vm-test-macos
mullvad connect
```

### Troubleshooting

- `Host unreachable. Retrying...` during the first boot is expected for a freshly cloned macOS image; first-boot setup
  and Spotlight indexing can take 15+ minutes. The macOS VM has a 30-minute boot timeout (`LETHE_MACOS_BOOT_TIMEOUT`).
  Check `just vm-status`; the VM may still be booting after Vagrant prints retries.
- `The provider ... is not yet ready for SSH`: with bridged networking, the ARP cache entry for the guest probably
  expired. Set `LETHE_MACOS_SSH_HOST` as described above.
- `new disk size ... should be larger than the current disk size ...`: this is a `vagrant-tart` quirk when its disk
  configuration is smaller than the image. The `Vagrantfile` sets `LETHE_MACOS_DISK_GB` automatically (50 GB for
  base/vanilla images, 140 GB for xcode images). Tart cannot shrink a disk, so destroy and recreate the VM after
  switching images.
- `Unmatched argument at index 0` from `just`: the wrong `just` binary is on `PATH`; see the note at the top of this
  section.

### Source archive

The source archive sent to the VMs is generated from the working tree (excluding `.git`, `target`, `.vagrant` and
`testing/.cache`). Set `LETHE_SKIP_SOURCE_ARCHIVE=1` to reuse the existing archive.

`testing/run-tests.sh` and `testing/run-tests.ps1` can also be run directly on a macOS/Windows host when
`LETHE_TEST_DEVICE` points at a dedicated scratch disk. Auto-detection of the scratch disk is only enabled inside the
provisioned test VMs.

## Releasing

Releases are driven by a version tag. Install [`cargo-release`](https://github.com/crate-ci/cargo-release), make sure
the working tree is clean, then preview the release:

```sh
just release 0.9.0
```

This runs `cargo-release` in dry-run mode, showing the version bump, file changes, commit and tag without modifying
anything. To perform the release, pass `--execute`:

```sh
just release 0.9.0 --execute
```

This bumps the version, updates the changelog and README, commits the changes, creates the `v0.9.0` tag and pushes it.
The tag triggers the GitHub Actions release workflow, which builds the supported platforms and attaches the artifacts
to the GitHub release. A second tag-triggered workflow publishes the crate to crates.io.

Set the `CARGO_REGISTRY_TOKEN` repository secret to a crates.io API token before the first release.
