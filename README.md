# Lethe

A secure, free, cross-platform and open-source drive wiping utility.

You might need this tool when you have data on your drive (HDD or SSD) you absolutely don't want anyone else to recover after you sell the disk of throw it away, even with specialized hardware. 

The usual method for wiping the disk is filling it with zeroes (or any other constant value) or with randomly generated data. For newer SSDs there's another option: Secure Wipe, but this one is not supported by this tool yet.

## Why another tool like this?



## Features
- Supports Mac OS and Linux
- Fills a drive with zero or random data or a combination of these steps for improved security
- Validates the data (reads back) to make sure all write commands were successful
- Uses fast cryptographic random generator
- Allows to override OS recommended block size for possibly faster operations

## Roadmap
- Windows support
- Checkpoints (allow to stop wiping at any moment and later resume from that place)
- Benchmark a drive to pick the optimal buffer (block) size
- Bad blocks handling
- ATA Secure Wipe support

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

### Getting a list of drives

To get a list of available system drives, use command `list`:

```
lethe list
```

Note that `lethe` operates on a low level and will likely require a root access (e.g. `sudo`) to work with any real drives (and not loopback devices, for example).

### Wiping a drive

To wipe a drive you just need to pass the name of the device as an argument to a `wipe` command.

```
lethe wipe /dev/rdisk3
```

This command will use default values for all the other parameters. 

#### --scheme=\<value\>

*Possible values:* `zero`

Wiping scheme is basically a plan, a list of steps that should be performed.

#### --verify=\<value\>

*Possible values:* `no`, `last` (default) or `all`


#### --blocksize=\<value\>

*Possible values:* a number of bytes

You can override an OS recommended block size with this parameter. The value is a number of bytes. Often, multiplying a base block size 2 or 4 times can improve the performance. But going too low or using unaligned sizes can result in error. 

#### --yes

This flag prevents `lethe` from asking for any confirmation. The assumed answer is `yes`. This is useful for scripting.

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

There is no practical difference in speed between `lethe` and `dd` here.

**Random fill**

 Command | Block size | Time taken (seconds)
---------|------------|----------
 `dd if=/dev/urandom of=/dev/rdisk3 bs=131072` | 128k | 4546.48
 `lethe wipe --scheme=random --verify=no /dev/rdisk3` | 128k | 2758.11

 `/dev/urandom` is notoriously slow, and `lethe` uses a fast version of ChaCha CSPRNG.

## License

//todo
