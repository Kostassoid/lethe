#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(dirname "$SCRIPT_DIR")"
cd "$REPO_DIR"

# Non-interactive SSH sessions do not source ~/.profile or ~/.bashrc.
export PATH="$HOME/.cargo/bin:$PATH"

BIN="${LETHE_BIN:-$REPO_DIR/target/release/lethe}"
BLOCK_SIZE="${LETHE_TEST_BLOCK_SIZE:-1m}"
EXPECTED_BYTES="${LETHE_TEST_DISK_BYTES:-1073741824}"

if [[ $EUID -eq 0 ]]; then
  SUDO=()
else
  SUDO=(sudo)
fi

find_linux_test_disk() {
  local minimum=$((EXPECTED_BYTES * 8 / 10))
  local maximum=$((EXPECTED_BYTES * 12 / 10))
  local candidates=()

  while read -r name size type ro; do
    [[ "$type" == "disk" ]] || continue
    [[ "$ro" == "0" ]] || continue
    (( size >= minimum && size <= maximum )) || continue

    # Never touch a disk that has anything mounted on it (the boot disk, an
    # LVM/RAID member, etc.).
    if [[ -n "$(lsblk -n -o MOUNTPOINTS "/dev/$name" 2>/dev/null | tr -d '[:space:]')" ]]; then
      continue
    fi

    candidates+=("/dev/$name")
  done < <(lsblk -b -dn -o NAME,SIZE,TYPE,RO)

  if (( ${#candidates[@]} == 1 )); then
    printf '%s\n' "${candidates[0]}"
    return 0
  fi

  if (( ${#candidates[@]} == 0 )); then
    echo "No detached ~${EXPECTED_BYTES}-byte disk found." >&2
  else
    echo "Multiple candidate disks found: ${candidates[*]}" >&2
  fi
  echo "Set LETHE_TEST_DEVICE to the scratch disk explicitly." >&2
  return 1
}

if [[ -n "${LETHE_TEST_DEVICE:-}" ]]; then
  DEVICE="$LETHE_TEST_DEVICE"
elif [[ "$(uname -s)" == "Linux" && -f /etc/lethe-test-vm ]]; then
  DEVICE="$(find_linux_test_disk)"
elif [[ "$(uname -s)" == "Linux" ]]; then
  echo "Refusing to guess a disk outside a Lethe test VM." >&2
  echo "Set LETHE_TEST_DEVICE=/dev/sdX to run against a dedicated scratch disk." >&2
  exit 2
else
  echo "Set LETHE_TEST_DEVICE to a dedicated scratch disk on this platform." >&2
  exit 2
fi

if [[ ! -b "$DEVICE" && "$(uname -s)" != "Darwin" ]]; then
  echo "Not a block device: $DEVICE" >&2
  exit 2
fi

echo "==> Building Lethe"
cargo build --locked --release

echo "==> Device list (${DEVICE})"
"${SUDO[@]}" "$BIN" list

if ! "${SUDO[@]}" "$BIN" list | grep -Fq "$DEVICE"; then
  echo "FAIL: $DEVICE is not reported by lethe list." >&2
  exit 1
fi

echo "==> Filling ${DEVICE} with non-zero data"
"${SUDO[@]}" dd if=/dev/urandom of="$DEVICE" bs=1m \
  count=$((EXPECTED_BYTES / 1048576)) 2>/dev/null || true

echo "==> Zero wipe with full verification"
"${SUDO[@]}" "$BIN" wipe \
  --scheme=zero \
  --verify=all \
  --blocksize="$BLOCK_SIZE" \
  --yes \
  "$DEVICE"

echo "==> Independently checking that ${DEVICE} is all zeroes"
if ! "${SUDO[@]}" cmp "$DEVICE" /dev/zero; then
  echo "FAIL: device still contains non-zero data after the zero wipe." >&2
  exit 1
fi

echo "==> Random wipe with partial verification"
"${SUDO[@]}" "$BIN" wipe \
  --scheme=random \
  --verify=last \
  --coverage=10 \
  --blocksize="$BLOCK_SIZE" \
  --yes \
  "$DEVICE"

echo "==> Independently checking that ${DEVICE} is no longer all zeroes"
set +e
"${SUDO[@]}" cmp "$DEVICE" /dev/zero >/dev/null 2>&1
cmp_status=$?
set -e

if [[ $cmp_status -eq 0 ]]; then
  echo "FAIL: device is still all zeroes after the random wipe." >&2
  exit 1
fi
if [[ $cmp_status -ne 1 ]]; then
  echo "FAIL: unable to read back the device (cmp exit code $cmp_status)." >&2
  exit 1
fi

echo "PASS: ${DEVICE} was wiped and verified successfully."
