# Lethe

A secure, free, cross-platform and open-source drive wiping utility.

Should work with any HDD, SSD (read below) and flash drives.

The usual method for wiping a drive is filling it with randomly generated data or zeroes (or any other constant value). The best results are achieved by performing a combination of these steps. This is basically what this tool does.

In case of SSDs, however, it is practically impossible to prove the data was sucessfully wiped (or overwritten) because of various optimizations performed by modern SSD controllers, namely wear leveling and compression. The situation improves with a better implementations of native wiping features like Secure Erase but for now the best way of action is: 
- never store any sensitive data on SSD unencrypted,
- wipe using multiple random fill passes
- additionally perform Secure Erase if possible (not supported by `lethe` yet)

## Features

- Supports Mac OS and Linux (Windows is planned)
- Validates the data (reads back) to make sure all write commands were successful
- Uses fast cryptographic random generator
- Allows to override OS recommended block size for possibly faster operations

## Download

//todo

## Usage

`lethe` is a CLI (command-line interface). Run it without parameters or use `help` command to dispay usage information.

```
lethe help
```

You can also use `help` command to get more information about any particular command.

```
lethe help wipe
```

Note that `lethe` operates on a low level and will require a root access (e.g. `sudo`) to work with any real drives (and not loopback devices, for example).

## Benchmarks

### Mac OS

Tested on Macbook Pro 2015 with Mac OS 10.14.4 (Mojave) using a Sandisk 64G Flash Drive with USB 3.0 interface. OS recommended block size is 128k.

**Zero fill**

 Command | Block size | Time taken (seconds)
---------|------------|----------
 `dd if=/dev/zero of=/dev/rdisk3 bs=131072` | 128k | 2667.21
 `lethe wipe --scheme=zero --verify=no /dev/rdisk3` | 128k | 2725.77
 `dd if=/dev/zero of=/dev/rdisk3 bs=1m` | 1m | 2134.99
 `lethe wipe --scheme=zero --blocksize=1048576 --verify=no /dev/rdisk3` | 1m | 2129.61

**Random fill**

 Command | Block size | Time taken (seconds)
---------|------------|----------
 `dd if=/dev/urandom of=/dev/rdisk3 bs=131072` | 128k | 4546.48
 `lethe wipe --scheme=random --verify=no /dev/rdisk3` | 128k | 2758.11

## License

`Lethe` is licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for the full license text.