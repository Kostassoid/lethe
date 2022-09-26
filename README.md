# Lethe

[![Build Status](https://travis-ci.com/Kostassoid/lethe.svg?branch=master)](https://travis-ci.com/Kostassoid/lethe)

A secure, free, cross-platform and open-source drive wiping utility.

Should work with any HDD, SSD (read limitations) and flash drives.

The usual methods for wiping (or sanitization) a drive, including those (allegedly) used by government agencies are based
on destructive writes. In other words, on overwriting existing data with multiple layers of randomly generated data or some
static pattern.
This is basically what this tool does.

There are other similar applications around (including multiple built-in Linux tools). Most of them are proprietary, or slow,
or non-cross-platform, which was a requirement for me. So I wrote this application.

## Features

- Supports Windows (but not WSL), macOS and Linux.
- Validates the data (reads back) to make sure all write commands were successful
- Uses fast cryptographic random generator
- Allows to override OS recommended block size for possibly faster operations
- Tracks & skips bad blocks and other localized errors automatically (Experimental)

## Limitations

- For SSD, it's impossible to reliable wipe all the data because of the various optimizations performed by modern SSD controllers, namely wear leveling and compression. The best approach currently is to use multiple wiping rounds with random data. Later, a support for Secure Erase ATA commands may be added to make the process more reliable.
- The maximum number of blocks per storage device is 2<sup>32</sup>, or 4,294,967,296. For example, using a block size of 1 MB the size of the storage can be up to 4096 TB.
- The application hasn't even been tested on RAID storages, beware.

## Current status

The initial active development phase is done.
I have been using the application for some time for personal needs on all supported platforms. It does what it was designed to do. Didn't have to deal with forensics experts yet though.
I still make some additions/changes occasionally, but there's no exact roadmap.
I would love to learn about other people's experience with the application. Let me know if you have any issues!

## Download

Current release: **v0.8.2** [Changelog](CHANGELOG.md)

Download and unzip binaries for your OS:
- [Windows x86-64](https://github.com/Kostassoid/lethe/releases/download/v0.8.2/lethe-v0.8.2-x86_64-pc-windows-gnu.zip)
- [macOS Intel](https://github.com/Kostassoid/lethe/releases/download/v0.8.2/lethe-v0.8.2-x86_64-apple-darwin.tar.gz)
- [macOS Apple](https://github.com/Kostassoid/lethe/releases/download/v0.8.2/lethe-v0.8.2-aarch64-apple-darwin.tar.gz) (Experimental)
- [Linux x86-64](https://github.com/Kostassoid/lethe/releases/download/v0.8.2/lethe-v0.8.2-x86_64-unknown-linux-musl.tar.gz)

Or install `lethe` from sources using the latest [Rust toolchain](https://www.rust-lang.org/tools/install):

```
cargo install lethe
```

## Usage

`lethe` is a CLI (command-line interface). Run it without parameters or use `help` command to display usage information.

```
lethe help
```

You can also use `help` command to get more information about any particular command.

```
lethe help wipe
```

Note that `lethe` operates on a low level and will require a root/administrator access (e.g. `sudo`) to work with any real drives.

## Benchmarks

### macOS

Measured on Macbook Pro 2015 with macOS 10.14.4 (Mojave) using a Sandisk 64G Flash Drive with USB 3.0 interface. OS recommended block size is 128k.

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
