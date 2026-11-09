#!/bin/bash

set -e
# set -o xtrace

# Recreate two ext4 partitions on a Lethe test disk and mount them.
# Usage: testing/vagrant-recreate-partitions.sh /dev/vdb

DEVICE="${1:?Usage: $0 <block-device>}"

if [[ ! -b "$DEVICE" ]]; then
  echo "Not a block device: $DEVICE" >&2
  exit 2
fi

# reset
umount /mnt/extra1 || true
umount /mnt/extra2 || true
parted "$DEVICE" rm 1 || true
parted "$DEVICE" rm 2 || true

# create partitions
parted "$DEVICE" mklabel gpt --script
parted "$DEVICE" mkpart primary ext4 0% 70% --script
parted "$DEVICE" mkpart primary ext4 70% 100% --script

sleep 1 # having eventual consistency issue on Ubuntu

mkfs -F -t ext4 "${DEVICE}1"
mkfs -F -t ext4 "${DEVICE}2"
e2label "${DEVICE}1" test1
e2label "${DEVICE}2" test2

# mount
mkdir -p /mnt/extra1
mkdir -p /mnt/extra2
mount "${DEVICE}1" /mnt/extra1
mount "${DEVICE}2" /mnt/extra2
