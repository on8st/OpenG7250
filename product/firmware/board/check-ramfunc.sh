#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Check that the flash-erase routine of the application runs from RAM and never branches to
# flash while the bank is being erased (nv.rs, erase_sector_from_ram). Usage: check-ramfunc.sh <elf>
set -eu
elf="$1"
dis=$(arm-none-eabi-objdump -d -j .data "$elf" | sed -n '/erase_sector_from_ram>:/,/^$/p')
if [ -z "$dis" ]; then
  echo "check-ramfunc: erase_sector_from_ram not found in .data of $elf" >&2
  exit 1
fi
start=$(printf '%s\n' "$dis" | head -1 | cut -c1-8)
case "$start" in
  24*) ;;
  *) echo "check-ramfunc: erase_sector_from_ram is at $start, not in AXI SRAM" >&2; exit 1 ;;
esac
# Every branch target named in the body must be in RAM (0x24......).
bad=$(printf '%s\n' "$dis" | grep -E '\s(b|bl|blx|bx|b\.n|b\.w|beq|bne|bcc|bcs|bhi|bls|bge|blt|bgt|ble|bmi|bpl|cbz|cbnz)(\.n|\.w)?\s' \
      | grep -oE '\s[0-9a-f]{7,8}\s<' | grep -v -E '\s24[0-9a-f]{6}\s<' || true)
if [ -n "$bad" ]; then
  echo "check-ramfunc: erase_sector_from_ram branches out of RAM:" >&2
  printf '%s\n' "$bad" >&2
  exit 1
fi
# No call through a register either (blx rN), which could land in flash.
if printf '%s\n' "$dis" | grep -qE '\sblx\s+r[0-9]'; then
  echo "check-ramfunc: erase_sector_from_ram calls through a register" >&2
  exit 1
fi
echo "check-ramfunc: $elf: erase_sector_from_ram at 0x$start, no branch out of RAM"
