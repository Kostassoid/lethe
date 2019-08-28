# Lethe

A secure, free, cross-platform and open-source drive wiping utility.

You might need this tool when you have data on your drive (HDD or SSD) you absolutely don't want anyone else to recover after you sell the disk of throw it away, even with specialized hardware. 

The usual method for wiping the disk is filling it with zeroes (or any other constant value) or with randomly generated data. For newer SSDs there's another option: Secure Wipe, but this one is not supported by this tool yet.

## Features:
- Supports Mac OS and Linux
- Fills disk with zeroes, random data or a combination of these steps
- Validates the data (reads back) to make sure all write commands were successful
- Uses fast cryptographic random generator
- Allows to override OS-recommended block size for possibly faster operations

## Roadmap
- Windows support
- Checkpoints (allow to stop wiping at any moment and later resume from that place)
- Benchmark a drive to pick the optimal buffer (block) size
- ATA Secure Wipe support

## Benchmarks

### Mac OS

Using a new 64G usb 3 flash drive. OS recommended block size 128k.

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

## Usage

