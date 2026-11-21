#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Check that the flash erase and program routines (board/common/src/flash.rs) run from RAM and
# never branch to flash while the bank is busy. Usage: check-ramfunc.sh <elf>
set -eu
elf="$1"
for fn in flash_erase_from_ram flash_program_from_ram; do
  dis=$(arm-none-eabi-objdump -d -j .data "$elf" | sed -n "/${fn}>:/,/^\$/p")
  if [ -z "$dis" ]; then
    echo "check-ramfunc: $fn not found in .data of $elf" >&2
    exit 1
  fi
  start=$(printf '%s\n' "$dis" | head -1 | cut -c1-8)
  case "$start" in
    2*) ;;
    *) echo "check-ramfunc: $fn is at $start, not in RAM" >&2; exit 1 ;;
  esac
  # Every branch target named in the body must be in RAM (0x2.......).
  bad=$(printf '%s\n' "$dis" | grep -E '\s(b|bl|blx|bx|b\.n|b\.w|beq|bne|bcc|bcs|bhi|bls|bge|blt|bgt|ble|bmi|bpl|cbz|cbnz)(\.n|\.w)?\s' \
        | grep -oE '\s[0-9a-f]{7,8}\s<' | grep -v -E '\s2[0-9a-f]{7}\s<' || true)
  if [ -n "$bad" ]; then
    echo "check-ramfunc: $fn branches out of RAM:" >&2
    printf '%s\n' "$bad" >&2
    exit 1
  fi
  # No call through a register either (blx rN), which could land in flash.
  if printf '%s\n' "$dis" | grep -qE '\sblx\s+r[0-9]'; then
    echo "check-ramfunc: $fn calls through a register" >&2
    exit 1
  fi
  echo "check-ramfunc: $elf: $fn at 0x$start, no branch out of RAM"
done
