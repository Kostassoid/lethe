#!/usr/bin/env bash

set -euo pipefail

export DEBIAN_FRONTEND=noninteractive

TARGET_USER="${SUDO_USER:-vagrant}"
TARGET_HOME="$(getent passwd "$TARGET_USER" | cut -d: -f6)"
ARCHIVE="$TARGET_HOME/lethe-src.tar.gz"

if [[ ! -f "$ARCHIVE" ]]; then
  # The file provisioner uploads to the SSH user's home. If SUDO_USER was not
  # preserved, fall back to the first uploaded archive we can find.
  ARCHIVE="$(find /home -maxdepth 2 -name lethe-src.tar.gz -print -quit)"
  TARGET_USER="$(stat -c '%U' "$ARCHIVE")"
  TARGET_HOME="$(getent passwd "$TARGET_USER" | cut -d: -f6)"
fi

apt-get update
apt-get install -y --no-install-recommends \
  build-essential \
  ca-certificates \
  curl \
  git \
  parted \
  tar

if ! sudo -u "$TARGET_USER" -H test -x "$TARGET_HOME/.cargo/bin/cargo"; then
  sudo -u "$TARGET_USER" -H bash -lc \
    'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal'
fi

install -d -o "$TARGET_USER" -g "$TARGET_USER" "$TARGET_HOME/lethe"
tar -xzf "$ARCHIVE" -C "$TARGET_HOME/lethe"
chmod +x "$TARGET_HOME"/lethe/testing/*.sh
chown -R "$TARGET_USER:$TARGET_USER" "$TARGET_HOME/lethe"

# Marks the guest as a disposable Lethe test VM. The test harness refuses to
# auto-select a disk without this marker.
touch /etc/lethe-test-vm

echo "Lethe test VM is ready."
