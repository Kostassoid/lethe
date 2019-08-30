# Lethe

A secure, free, cross-platform and open-source drive wiping utility.

Should work with any HDD, SSD, flash drives, etc.

The usual method for wiping the disk is filling it with randomly generated data or zeroes (or any other constant value). But the best results are achieved by performing a combination of these fills. This is basically what this tool does.

For newer SSDs there's another method, ATA Secure Erase, which is not supported yet by `lethe`. This can potentially lower the effectiveness of the method used. So keeping your data encrypted from the start is a good practice.

## Why another tool like this?

Simply because there's no other tool I could find that's truly cross-platform, open-source, easy to use and not owned by a greedy company stating their application is free, but asking money for essential security features. I don't mind paying for a good product but these tactics don't exactly induce trust. So I decided I would pay with my time and effort. And maybe learn something new in the process.

## Features

- Supports Mac OS and Linux (Windows is planned)
- Validates the data (reads back) to make sure all write commands were successful
- Uses fast cryptographic random generator
- Allows to override OS recommended block size for possibly faster operations

## Roadmap
- Windows support
- Checkpoints (allow to stop wiping at any moment and later resume from that place)
- Benchmark a drive to pick the optimal buffer (block) size
- Bad blocks handling
- Secure Erase support
- Backend mode (simplifies using from another applications/scripts)

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

To get a list of available system drives use command `list`:

```
lethe list
```

Note that `lethe` operates on a low level and will likely require a root access (e.g. `sudo`) to work with any real drives (and not loopback devices, for example).

### Wiping a drive

To wipe a drive you just need to pass the name of the device as an argument to a `wipe` command.

```
lethe wipe /dev/rdisk3
```

This command will use a default configuration which should be fine for most normal needs.

But you can tune these parameters if needed:

#### --scheme=\<value\>

*Possible values:* `zero`

*Default value:* `random2`

Wiping scheme is basically a plan, a list of steps that should be performed.

#### --verify=\<value\>

*Possible values:* `no`, `last` or `all`

*Default value:* `last`


#### --blocksize=\<value\>

*Possible values:* a number of bytes

*Default value:* OS recommended block size

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
