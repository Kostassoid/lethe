#!/usr/bin/env bash

set -euo pipefail

TARGET_USER="${SUDO_USER:-admin}"
TARGET_HOME="$(dscl . -read "/Users/$TARGET_USER" NFSHomeDirectory 2>/dev/null | awk '{print $2}')"
if [[ -z "$TARGET_HOME" || ! -d "$TARGET_HOME" ]]; then
  TARGET_HOME="$(eval echo "~$TARGET_USER")"
fi

ARCHIVE="$TARGET_HOME/lethe-src.tar.gz"
if [[ ! -f "$ARCHIVE" ]]; then
  ARCHIVE="$(find /Users -maxdepth 2 -name lethe-src.tar.gz -print -quit)"
  if [[ -z "$ARCHIVE" ]]; then
    echo "Unable to find lethe-src.tar.gz." >&2
    exit 1
  fi
  TARGET_USER="$(stat -f '%Su' "$ARCHIVE")"
  TARGET_HOME="$(dscl . -read "/Users/$TARGET_USER" NFSHomeDirectory | awk '{print $2}')"
fi

TARGET_GROUP="$(id -gn "$TARGET_USER")"

if ! xcode-select -p >/dev/null 2>&1; then
  echo "Xcode Command Line Tools are required. Use a Tart image with the Command Line Tools installed." >&2
  exit 1
fi

if ! sudo -u "$TARGET_USER" -H test -x "$TARGET_HOME/.cargo/bin/cargo"; then
  sudo -u "$TARGET_USER" -H bash -lc \
    'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal'

  if ! sudo -u "$TARGET_USER" -H test -x "$TARGET_HOME/.cargo/bin/cargo"; then
    echo "Rust installation failed. Check that the guest can reach the internet" >&2
    echo "(on macOS hosts, a VPN such as Mullvad can block shared VM networking)." >&2
    exit 1
  fi
fi

if ! sudo -u "$TARGET_USER" sudo -n true >/dev/null 2>&1; then
  echo "$TARGET_USER ALL=(ALL) NOPASSWD: ALL" > /etc/sudoers.d/lethe-vagrant
  chmod 440 /etc/sudoers.d/lethe-vagrant
  visudo -cf /etc/sudoers.d/lethe-vagrant >/dev/null
fi

install -d -o "$TARGET_USER" -g "$TARGET_GROUP" "$TARGET_HOME/lethe"
tar -xzf "$ARCHIVE" -C "$TARGET_HOME/lethe"
chmod +x "$TARGET_HOME"/lethe/testing/*.sh
chown -R "$TARGET_USER:$TARGET_GROUP" "$TARGET_HOME/lethe"

EXPECTED_BYTES=$((1024 * 1024 * 1024))

find_scratch_disk() {
  local disk size

  for disk in $(diskutil list |
    awk '/^[[:space:]]*\/dev\/(disk[0-9]+)([[:space:]].*)?:/ { sub("/dev/", "", $1); print $1 }'); do
    size="$(diskutil info -plist "$disk" 2>/dev/null | plutil -extract Size raw -o - - 2>/dev/null || true)"
    if [[ -n "$size" ]] && (( size >= EXPECTED_BYTES * 8 / 10 && size <= EXPECTED_BYTES * 12 / 10 )); then
      printf '/dev/r%s\n' "$disk"
      return 0
    fi
  done

  return 1
}

if DEVICE="$(find_scratch_disk)"; then
  printf '%s\n' "$DEVICE" > "$TARGET_HOME/.lethe-test-device"
  chown "$TARGET_USER:$TARGET_GROUP" "$TARGET_HOME/.lethe-test-device"
  printf '%s\n' "$DEVICE" > /etc/lethe-test-vm
  echo "Lethe scratch disk: $DEVICE"
else
  echo "Warning: no ~1 GiB scratch disk was found." >&2
fi
