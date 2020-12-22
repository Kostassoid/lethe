# Lethe

[![Build Status](https://travis-ci.org/Kostassoid/lethe.svg?branch=master)](https://travis-ci.org/Kostassoid/lethe)

A secure, free, cross-platform and open-source drive wiping utility.

Should work with any HDD, SSD (read limitations) and flash drives.

The usual method for wiping a drive is filling it with randomly generated data or zeroes (or any other constant value). The best results can be achieved by performing a combination of these steps. This is basically what this tool does.

## Features

- Supports Windows (but not WSL), macOS and Linux.
- Validates the data (reads back) to make sure all write commands were successful
- Uses fast cryptographic random generator
- Allows to override OS recommended block size for possibly faster operations
- Tracks & skips bad blocks

## Limitations

- For SSD, it's impossible to reliable wipe all the data because of the various optimizations performed by modern SSD controllers, namely wear leveling and compression. The best approach currently is to use multiple wiping rounds with random data. Later, a support for Secure Erase ops may be added to make the process more reliable.
- The maximum number of blocks per storage device is 2<sup>32</sup>, or 4,294,967,296. For example, using a block size of 1 MB the size of the storage can be up to 4096 TB.

## Download

Current release: **v0.4.0** [Changelog](CHANGELOG.md)

Download and unzip binaries for your OS:
- [Windows x64](https://github.com/Kostassoid/lethe/releases/download/v0.4.0/lethe-v0.4.0-x86_64-pc-windows-gnu.zip)
- [macOS x64](https://github.com/Kostassoid/lethe/releases/download/v0.4.0/lethe-v0.4.0-x86_64-apple-darwin.tar.gz)
- [Linux x64](https://github.com/Kostassoid/lethe/releases/download/v0.4.0/lethe-v0.4.0-x86_64-unknown-linux-musl.tar.gz)

Or install `lethe` from sources using latest [Rust toolchain](https://www.rust-lang.org/tools/install):

```
cargo install lethe
```

## Usage

`lethe` is a CLI (command-line interface). Run it without parameters or use `help` command to dispay usage information.

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

Tested on Macbook Pro 2015 with macOS 10.14.4 (Mojave) using a Sandisk 64G Flash Drive with USB 3.0 interface. OS recommended block size is 128k.

**Zero fill**

 Command | Block size | Time taken (seconds)
---------|------------|----------
 `dd if=/dev/zero of=/dev/rdisk3 bs=131072` | 128k | 2667.21
 `lethe wipe --scheme=zero --blocksize=128k --verify=no /dev/rdisk3` | 128k | 2725.77
 `dd if=/dev/zero of=/dev/rdisk3 bs=1m` | 1m | 2134.99
 `lethe wipe --scheme=zero --blocksize=1m --verify=no /dev/rdisk3` | 1m | 2129.61

**Random fill**

 Command | Block size | Time taken (seconds)
---------|------------|----------
 `dd if=/dev/urandom of=/dev/rdisk3 bs=131072` | 128k | 4546.48
 `lethe wipe --scheme=random --blocksize=128k --verify=no /dev/rdisk3` | 128k | 2758.11

## License

`Lethe` is licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for the full license text.
