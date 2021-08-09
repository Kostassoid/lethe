#!/bin/bash

set -e
# set -o xtrace

# reset
umount /mnt/extra1 || true
umount /mnt/extra2 || true
parted /dev/sdb rm 1 || true
parted /dev/sdb rm 2 || true

#create partitions
parted /dev/sdb mklabel gpt --script
parted /dev/sdb mkpart primary ext4 0% 70% --script
parted /dev/sdb mkpart primary ext4 70% 100% --script

sleep 1 # having eventual consistency issue on Ubuntu

mkfs -F -t ext4 /dev/sdb1
mkfs -F -t ext4 /dev/sdb2
e2label /dev/sdb1 test1
e2label /dev/sdb2 test2

# mount
mkdir -p /mnt/extra1
mkdir -p /mnt/extra2
mount /dev/sdb1 /mnt/extra1
mount /dev/sdb2 /mnt/extra2
