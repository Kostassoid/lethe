<div align="center">
<img src="https://raw.githubusercontent.com/Kostassoid/lethe/master/assets/logo.svg" width="200" alt="Logo" />
</div>

# Lethe

Lethe is a secure, open-source, cross-platform drive sanitization utility for HDDs, SSDs, and flash drives. It
combines verified overwrite wiping with device-level hardware erase, with first-class support for modern SSDs:
Lethe detects supported ATA and NVMe sanitize capabilities, selects an appropriate I/O queue depth, and executes
NVMe Sanitize or ATA SANITIZE where available.

For devices that do not support hardware erase, Lethe overwrites existing data with random or static patterns over
one or more passes and can read it back to verify the result. The supported methods are designed to align with the
Clear, Purge, and Destroy categories in [NIST SP 800-88 Rev. 2](#nist-sp-800-88-rev-2); see that section for the
method mapping, verification limits, and practical caveats.

Lethe is free of proprietary components and is developed as open source. Platform support and known limitations are
described below.

## Features

- **Cross-platform** — Windows (x86-64 and ARM64; WSL is not supported), macOS (Apple Silicon) and Linux (x86-64 and
  ARM64).
- **Verified wiping** — reads the data back and compares it with the expected pattern.
- **Full or partial coverage** — verify every pass or only the last one, over the whole drive or just a sample of its
  beginning and end.
- **Fast random data** — uses a fast cryptographic random generator.
- **Tunable block size** — override the OS-recommended block size for faster operations.
- **Queued I/O (experimental)** — automatically picks an I/O queue depth for the detected drive type, using
  `io_uring` on Linux and overlapped I/O on Windows.
- **Hardware erase (experimental)** — detects ATA Secure Erase/SANITIZE and NVMe Sanitize/Format NVM, and executes
  NVMe Sanitize and ATA SANITIZE on Windows.
- **Bad block handling** — tracks, skips and reports bad blocks and other localized errors automatically.
- **Sleep prevention** — blocks the OS from sleeping while a wipe is in progress.

## Limitations

- For SSD, it's impossible to reliably wipe all the data because of the various optimizations performed by modern SSD
  controllers, namely wear leveling and compression. The best approach currently is to use multiple wiping rounds with
  random data.
- Hardware sanitize commands are available on Windows for whole disks only. ATA Secure Erase and NVMe Format NVM
  are detected but not executed; drives that support only those commands cannot be hardware-erased by Lethe.
- The maximum number of blocks per storage device is 2<sup>32</sup>, or 4,294,967,296. For example, using a block size
  of 1 MB the size of the storage can be up to 4096 TB.
- The application hasn't been tested on RAID storages and other complex storage configurations, beware.

## NIST SP 800-88 Rev. 2

This section describes how Lethe maps to [NIST SP 800-88 Revision 2](https://csrc.nist.gov/pubs/sp/800/88/r2/final)
(September 2025). It is not a certification or a claim of compliance. NIST does not approve or certify sanitization
tools; validation and documentation remain the responsibility of the organization performing the sanitization.

Rev. 2 keeps the three sanitization methods and adds a dual-phase assurance model:

- **Verification** — the operational check that a technique completed without errors or anomalies.
- **Validation** — the organization's decision that the outcome is acceptable for the data's confidentiality level.

### Method support

| NIST method | Lethe support |
|-------------|---------------|
| **Clear** | **Yes.** Software overwrite of user-addressable storage through the normal read/write interface, with optional read-back verification. |
| **Purge** | **Partial.** NVMe Sanitize and ATA SANITIZE on Windows, whole disks only. These are standardized logical purge techniques. |
| **Destroy** | **No.** Physical destruction (shred, disintegrate, incinerate, etc.) is out of scope. |

#### Clear

Rev. 2 does not require multi-pass overwriting for Clear; a single overwrite pass or a dedicated sanitize command is
sufficient, provided the interface cannot retrieve the original data. Lethe supports single-pass and multi-pass
overwrites (`zero`, `random`, `random2x`, `badblocks`, `gost`, `dod`, `vsitr`).

`--verify` reads the data back and compares it with the expected pattern. This is **verification**, not validation.
Full verification is stronger; partial `--coverage` is a spot check of the beginning and end of the wiped area and is
not a substitute for validating the whole device. Bad blocks are skipped and reported, which reduces actual coverage.

For SSDs, overwriting user-addressable LBAs is a Clear-level technique at best; it does not reach overprovisioned,
remapped or otherwise hidden flash. Purge-level techniques are preferable where supported.

#### Purge

On Windows, `--hardware-erase=ata-sanitize` and `--hardware-erase=nvme-sanitize` issue dedicated whole-device sanitize
commands:

- **ATA SANITIZE**: crypto scramble, block erase, or overwrite, selected from the device's reported capabilities.
- **NVMe Sanitize**: crypto erase, block erase, or overwrite, selected from `SANICAP`.

Both are whole-disk operations; individual partitions and non-zero offsets are rejected. If a drive supports only
ATA Secure Erase or NVMe Format NVM, Lethe reports the capability in `list` but does not offer it as an erase option.
Linux and macOS hardware erase execution is not implemented yet.

Cryptographic erase requires more than issuing a command. Rev. 2 places conditions on CE, including encrypted media,
key management, no prior plaintext, and—for federal use—FIPS-validated cryptographic modules. Lethe can request a
crypto-erase action, but it cannot verify those conditions; that assessment belongs to the drive vendor and the
organization.

#### Destroy

Not supported. Use an approved physical destruction process for media that must meet the Destroy method.

### Assurance

Lethe provides **verification** signals:

- read-back comparison for software overwrite passes;
- completion and status polling for ATA SANITIZE and NVMe Sanitize;
- explicit reporting of I/O errors, sanitize failures, bad blocks and skipped blocks.

Lethe does **not** provide **validation**, issue a certificate of sanitization, or maintain the sanitization records
required by an enterprise program. Record the device identity, method, command output, verification result, skipped
blocks and any sanitize status evidence according to your organization's policy.

### Practical caveats

- NIST SP 800-88 Rev. 2 defers technique details to IEEE 2883, NSA guidance or an organizationally approved standard.
  Lethe implements techniques; it does not determine which technique your policy requires.
- Soft wipe coverage is limited to the user-addressable area. SSD wear leveling and overprovisioning remain a risk.
- ATA Secure Erase is intentionally not offered because its temporary-password failure mode can leave a drive locked.
- The tool is not FIPS-validated, and its random generator is not used for cryptographic erase.

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
`wipe`. The listing also shows the detected bus, drive kind, the queue depth that `--queue-depth=auto` will use, and
the hardware erase methods reported by the device.

Hardware erase operates on the whole drive, never on an individual partition, so partitions are left blank in this
column. Probing is currently implemented on Windows and may require administrator rights; USB bridges often need
elevated pass-through access.

Methods are listed in the recommended order:

| Code  | Meaning                                                                        |
|-------|--------------------------------------------------------------------------------|
| `NS*` | NVMe Sanitize — preferred for NVMe; covers the whole device                     |
| `NF*` | NVMe Format NVM — namespace-scoped fallback when Sanitize is unavailable        |
| `SA*` | ATA SANITIZE — preferred for SATA; covers the whole device                      |
| `SE*` | ATA Secure Erase — older command; effective on many drives but behavior varies  |
| `?`   | Not probed or unsupported on this platform                                      |
| `-`   | No hardware erase method reported                                               |

Prefer `NS` over `NF`, and `SA` over `SE`, when both are listed. There is no universal winner: crypto erase is
fastest, block erase physically erases, and overwrite writes a pattern; the actual behavior depends on the drive
firmware.

Only `NS*` and `SA*` are executable; `SE*` and `NF*` are reported for information.

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
| `--queue-depth`, `-q` | `auto`   | In-flight I/O requests, or a fixed `1`-`256` |
| `--hardware-erase`  | `off`      | Hardware erase: `auto`, `ata-sanitize`, `nvme-sanitize` |
| `--yes`, `-y`       | off        | Skip the confirmation prompt               |

Sizes accept `k`, `m`, `g` and `t` suffixes (e.g. `128k`, `4m`, `1g`). Use `lethe help wipe` for the full list.

### Schemes

| Scheme      | Passes | Patterns                                      |
|-------------|--------|-----------------------------------------------|
| `none`      | 0      | No software passes (hardware erase only)      |
| `zero`      | 1      | zeroes                                        |
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

### Queued I/O (experimental)

`--queue-depth` keeps multiple positional read/write requests in flight. On Linux this
uses `io_uring`; on Windows it uses overlapped I/O. Other platforms fall back to
synchronous I/O, so the effective depth remains `1`.

The default is `auto`, which picks a conservative depth from the detected drive kind:

| Drive kind                | Queue depth |
|---------------------------|-------------|
| NVMe SSD                  | 8           |
| SATA/other SSD            | 4           |
| HDD, removable, unknown   | 1           |

USB drives need command queueing (UAS) before using a depth above 1. For such drives the
OS's seek-penalty or TRIM information is used to distinguish an SSD from an HDD; bridges
that hide both stay at depth 1.

This can improve throughput on SSDs, especially for verification (read-back), but may not
help large sequential writes. The wipe semantics are unchanged: steps still run in order,
bad blocks are tracked the same way, and partial coverage is still computed over the whole
drive.

```sh
# Auto (recommended)
sudo lethe wipe /dev/nvme0n1

# Force a fixed depth
sudo lethe wipe /dev/nvme0n1 --queue-depth=4

# Force the sequential path
sudo lethe wipe /dev/nvme0n1 --queue-depth=1
```

### Hardware erase (experimental)

`--hardware-erase` runs a final device-level erase stage after the software wipe and
verification:

- `auto` picks NVMe Sanitize for NVMe SSDs or ATA SANITIZE for ATA/SATA SSDs when
  supported.
- `nvme-sanitize` or `ata-sanitize` selects a method explicitly.
- `off` keeps the normal software-only wipe (default).

Only whole disks are supported; individual partitions and non-zero `--offset` values are
rejected, since hardware erase always covers the entire drive. For a hardware-erase-only
run, use `--scheme=none`. The current Windows implementation executes NVMe Sanitize and
ATA SANITIZE. ATA Secure Erase and NVMe Format NVM are still reported by `list`, but they
are not offered as erase options. Linux and macOS execution is not implemented yet.

Both ATA SANITIZE and NVMe Sanitize run in the background; Lethe polls the device until
they complete, which can take a while. Some USB bridges accept ATA SANITIZE but do not
expose its status. After a short grace period, Lethe reports the command as accepted but
unconfirmed instead of failing, and warns you not to retry until the erase has finished.
Because completion cannot be verified through such a bridge, check the drive before
reuse. If neither method is supported, hardware erase is not available for that device.

Hardware erase is a destructive final stage. If it fails, the software wipe and
verification have already completed; the error is reported separately.

```sh
# Best supported method for this drive
sudo lethe wipe '\\.\PhysicalDrive3' --hardware-erase=auto

# Hardware erase only, without software passes
sudo lethe wipe '\\.\PhysicalDrive3' --scheme=none --hardware-erase=auto

# Force ATA SANITIZE
sudo lethe wipe '\\.\PhysicalDrive3' --hardware-erase=ata-sanitize

# Force NVMe Sanitize
sudo lethe wipe /dev/nvme0n1 --hardware-erase=nvme-sanitize
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

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, integration testing, benchmarks and release procedures.

## License

`Lethe` is licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for the full license text.
