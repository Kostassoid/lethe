<div align="center">
<img src="https://raw.githubusercontent.com/Kostassoid/lethe/master/assets/logo.svg" width="200" alt="Logo" />
</div>

# Lethe

A secure, free, cross-platform and open-source drive wiping utility for HDDs, SSDs (see [Limitations](#limitations))
and flash drives.

Most drive wiping methods, including those allegedly used by government agencies, rely on destructive writes:
overwriting existing data with random data or static patterns over one or more passes. Lethe does exactly that.

Similar tools exist, but most are proprietary, slow or not cross-platform. Lethe aims to be none of those.

## Features

- **Cross-platform** — Windows (x86-64 and ARM64; WSL is not supported), macOS (Apple Silicon) and Linux (x86-64 and
  ARM64).
- **Verified wiping** — reads the data back and compares it with the expected pattern.
- **Full or partial coverage** — verify every pass or only the last one, over the whole drive or just a sample of its
  beginning and end.
- **Fast random data** — uses a fast cryptographic random generator.
- **Tunable block size** — override the OS-recommended block size for faster operations.
- **Bad block handling** — tracks, skips and reports bad blocks and other localized errors automatically.
- **Sleep prevention** — blocks the OS from sleeping while a wipe is in progress.

## Limitations

- For SSD, it's impossible to reliably wipe all the data because of the various optimizations performed by modern SSD
  controllers, namely wear leveling and compression. The best approach currently is to use multiple wiping rounds with
  random data.
- In theory, Secure Erase ATA commands may be used additionally, but there's no guarantee regarding their semantics.
  This application doesn't support this method.
- The maximum number of blocks per storage device is 2<sup>32</sup>, or 4,294,967,296. For example, using a block size
  of 1 MB the size of the storage can be up to 4096 TB.
- The application hasn't been tested on RAID storages and other complex storage configurations, beware.

## Current status

This application is already fairly old and was used in many scenarios. I never received any complaints regarding its
functionality, apart from some minor issues. I believe it is stable enough for general use.

Saying that, I didn't have to deal with forensics experts and my area of expertise is not related to security. I would
love to hear from people with more experience in this area.

Also, while it's fun working on this project, I don't have a significant roadmap for it apart from a few improvements.
At the very least, I'm going to keep updating it from time to time to keep dependencies up to date and fix any issues
that may arise.
Let me know if you have any issues!

## Installation

Current release: **v1.0.1** [Changelog](CHANGELOG.md)

### Bash (Linux, macOS)

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Kostassoid/lethe/releases/latest/download/lethe-installer.sh | sh
```

### PowerShell (Windows)

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/Kostassoid/lethe/releases/latest/download/lethe-installer.ps1 | iex"
```

### From source

Install `lethe` using the latest [Rust toolchain](https://www.rust-lang.org/tools/install):

```sh
cargo install lethe
```

## Usage

> **Warning:** Wiping is irreversible. Double-check the device ID before answering `yes`.
> Raw device access requires root/administrator privileges (`sudo` or an elevated shell).

The CLI has two commands: `list` and `wipe`.

### List devices

```sh
sudo lethe list
```

Every device gets a full ID (`/dev/sda`, `/dev/rdisk2`, `\\.\PhysicalDrive0`) and a short ID. Both are accepted by
`wipe`.

### Wipe a device

```sh
sudo lethe wipe <device-id>
```

With no options, Lethe uses the `random2x` scheme, 1 MiB blocks and verifies the last pass. It prints the plan and
waits for confirmation:

```
Wiping:
    Device            /dev/sdb
    Size              16 GB (16000000000 bytes)
    Steps             fill, fill, verify
    Block size        1 MB
    Starting offset   0 bytes
    Total area size   16 GB (16000000000 bytes)
    Verification      Last stage only (100% coverage)

Are you sure? (type 'yes' to confirm):
```

### Options

| Option              | Default    | Description                                |
|---------------------|------------|--------------------------------------------|
| `--scheme`, `-s`    | `random2x` | Data sanitization scheme                   |
| `--verify`, `-v`    | `last`     | Verify `no`, `last` or `all` passes        |
| `--coverage`, `-c`  | `100`      | Verification coverage in percent           |
| `--blocksize`, `-b` | `1m`       | I/O block size; must be a power of two     |
| `--offset`, `-o`    | `0`        | Start position in bytes                    |
| `--retries`, `-r`   | `8`        | Maximum retries per step                   |
| `--yes`, `-y`       | off        | Skip the confirmation prompt               |

Sizes accept `k`, `m`, `g` and `t` suffixes (e.g. `128k`, `4m`, `1g`). Use `lethe help wipe` for the full list.

### Schemes

| Scheme      | Passes | Patterns                        |
|-------------|--------|---------------------------------|
| `zero`      | 1      | zeroes                          |
| `random`    | 1      | random                          |
| `random2x`  | 2      | random, random                  |
| `badblocks` | 4      | `aa`, `55`, `ff`, `00`          |
| `gost`      | 2      | zeroes, random                  |
| `dod`       | 3      | zeroes, ones, random            |
| `vsitr`     | 7      | zeroes/ones alternating, random |

```sh
# Default: two random passes
sudo lethe wipe /dev/sdb

# Fast: single zero pass, no verification
sudo lethe wipe /dev/sdb --scheme=zero --verify=no

# DoD-style: three passes, verify after each one
sudo lethe wipe /dev/sdb --scheme=dod --verify=all
```

### Verification

`--verify` controls when data is read back:

- `no` — never verify; fastest.
- `last` — verify only the final pass (default).
- `all` — verify after every pass; slowest but most thorough.

If verification finds a mismatch, the write step is retried from the failure point and checked again, up to
`--retries` times.

#### Partial coverage

`--coverage` limits how much of the wiped area is verified, as a percentage. It works with both `last` and `all`:

```sh
# Verify the entire last pass (default)
sudo lethe wipe /dev/sdb --coverage=100

# Spot-check only 10% of the last pass
sudo lethe wipe /dev/sdb --verify=last --coverage=10

# Check 50% after each pass
sudo lethe wipe /dev/sdb --verify=all --coverage=50
```

Partial coverage samples the **beginning and the end** of the wiped area, rounded outward to whole blocks, because
those regions are the most likely to reveal an incomplete wipe. Even `--coverage=0` still checks the first and last
block. This is a fast sanity check for very large drives, not a substitute for a full read-back.

### Block size

The default is 1 MiB and the value must be a power of two. Larger blocks are often faster over USB and on large
drives; the OS-recommended block size may be better in some cases.

```sh
sudo lethe wipe /dev/sdb --blocksize=4m
sudo lethe wipe /dev/sdb --blocksize=128k
```

### Wiping only part of a drive

`--offset` starts the wipe at the given byte position and continues to the end of the device:

```sh
# Leave the first 1 GiB untouched
sudo lethe wipe /dev/sdb --offset=1g
```

Offsets are rounded down to a block boundary. Make sure the skipped area does not belong to a mounted filesystem.

### Retries and bad blocks

`--retries` (default `8`) sets how many times a failed step is retried. Blocks that cannot be accessed are marked,
skipped and counted in the summary instead of aborting the whole operation.

```sh
sudo lethe wipe /dev/sdb --retries=3
```

### Unattended use

`--yes` skips the confirmation prompt:

```sh
sudo lethe wipe /dev/sdb --yes --scheme=random --verify=last
```

### Help

```sh
lethe help
lethe help wipe
```

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

## Benchmarks

### macOS

Measured on Macbook Pro 2015 with macOS 10.14.4 (Mojave) using a Sandisk 64G Flash Drive with USB 3.0 interface. OS
recommended block size is 128k.

**Zero fill**

| Command                                                             | Block size | Time taken (seconds) |
|---------------------------------------------------------------------|------------|----------------------|
| `dd if=/dev/zero of=/dev/rdisk3 bs=131072`                          | 128k       | 2667.21              |
| `lethe wipe --scheme=zero --blocksize=128k --verify=no /dev/rdisk3` | 128k       | 2725.77              |
| `dd if=/dev/zero of=/dev/rdisk3 bs=1m`                              | 1m         | 2134.99              |
| `lethe wipe --scheme=zero --blocksize=1m --verify=no /dev/rdisk3`   | 1m         | 2129.61              |

**Random fill**

| Command                                                               | Block size | Time taken (seconds) |
|-----------------------------------------------------------------------|------------|----------------------|
| `dd if=/dev/urandom of=/dev/rdisk3 bs=131072`                         | 128k       | 4546.48              |
| `lethe wipe --scheme=random --blocksize=128k --verify=no /dev/rdisk3` | 128k       | 2758.11              |

## License

`Lethe` is licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for the full license text.
