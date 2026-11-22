#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 ON8ST
"""The RAM kept over a reset (the persist block, the log ring) lies at one fixed address in every
image, in NOLOAD sections of its own, above the RAM of the image (data, bss, stack): the boot stage
hands over to the application there (a mismatch lost the handover once), and the start code must
never clear it. Usage: check-kept-ram.py <elf>..."""
import re
import subprocess
import sys

WANT = {".persist": (0x2401FF80, 128), ".logring": (0x2401E700, 6272)}
SYMBOLS = {"persist6REGION": 0x2401FF80, "logmem3LOG": 0x2401E700}
RAM_END = 0x2401E700


def run(*a):
    return subprocess.run(a, check=True, capture_output=True, text=True).stdout


bad = []
for elf in sys.argv[1:]:
    sections = {}
    lines = run("arm-none-eabi-objdump", "-h", elf).splitlines()
    for i, l in enumerate(lines):
        m = re.match(r"\s*\d+\s+(\S+)\s+([0-9a-f]{8})\s+([0-9a-f]{8})", l)
        if m:
            sections[m[1]] = (int(m[3], 16), int(m[2], 16), lines[i + 1])
    for name, (at, room) in WANT.items():
        s = sections.get(name)
        if s is None or s[0] != at or s[1] > room or s[1] == 0 or "LOAD" in s[2].replace("NOLOAD", ""):
            bad.append(f"{elf}: {name} at {at:#x}, at most {room} bytes, not loaded: found {s}")
    for name, (at, size, _) in sections.items():
        if 0x24000000 <= at < 0x24020000 and name not in WANT and at + size > RAM_END:
            bad.append(f"{elf}: {name} reaches into the kept RAM")
    syms = run("arm-none-eabi-nm", elf)
    for s, at in SYMBOLS.items():
        found = [int(l.split()[0], 16) for l in syms.splitlines() if l.endswith(s)]
        if found != [at]:
            bad.append(f"{elf}: {s} at {at:#x}: found {[hex(f) for f in found]}")
    for s in ("_stack_start",):
        found = [int(l.split()[0], 16) for l in syms.splitlines() if l.split()[-1] == s]
        if found != [RAM_END]:
            bad.append(f"{elf}: {s}: {found}")
    free = [int(l.split()[0], 16) for l in syms.splitlines() if l.split()[-1] == "_stack_end"]
    print(f"check-kept-ram: {elf}: kept RAM at its fixed addresses, {RAM_END - free[0]} bytes for the stack")
if bad:
    print("\n".join(bad))
    sys.exit(1)
