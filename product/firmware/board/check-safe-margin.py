#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 ON8ST
"""Check that the boot stage reaches the safe state within 20 us of its first instruction
(spec A3.6; decision 0045). Usage: check-safe-margin.py <boot ELF>

The path is fixed by construction: the reset handler (cortex-m-rt) sets the stack pointer and
the vector table and calls __pre_init, into which g7250_board::safe::apply_at_reset is inlined.
This script checks that, in the built ELF:
  - the first call of Reset is to __pre_init, and Reset does nothing else before it (no RAM
    initialisation, no other call);
  - __pre_init is straight-line (no branch, no call) to its return and loads nothing through a
    literal pool (no RAM address can come from one; .data and .bss are not set up yet);
and bounds the time of the whole path with a worst-case model at the reset clock:
  - CPU at 64 MHz (HSI, the reset clock; RM0468), no cache, no prefetch credit;
  - every 32-byte flash word of code fetched once at 16 cycles (15 wait states, the largest
    FLASH_ACR LATENCY value, plus one);
  - 1 cycle per instruction on top;
  - 16 cycles per load or store (peripheral or stack), none overlapped.
The emulator trace (T-A3-06) is the measurement; this is the build-time guard against a change
that makes the path longer.
"""
import re
import subprocess
import sys

LIMIT_US = 20.0
MHZ = 64
FETCH = 16
ACCESS = 16
OBJDUMP = "arm-none-eabi-objdump"

BRANCH = re.compile(r"^(b|bl|blx|bx|cbz|cbnz|b(eq|ne|cs|cc|hs|lo|mi|pl|vs|vc|hi|ls|ge|lt|gt|le))(\.n|\.w)?$")


def fail(msg):
    print("check-safe-margin: FAIL " + msg, file=sys.stderr)
    sys.exit(1)


def functions(elf):
    out = subprocess.run([OBJDUMP, "-d", "--no-show-raw-insn", elf], check=True, capture_output=True, text=True).stdout
    funcs, cur = {}, None
    for line in out.splitlines():
        m = re.match(r"^([0-9a-f]{8}) <(.+)>:$", line)
        if m:
            cur = m.group(2)
            funcs[cur] = []
            continue
        m = re.match(r"^\s+([0-9a-f]+):\s+(\S+)\s*(.*)$", line)
        if m and cur:
            funcs[cur].append((int(m.group(1), 16), m.group(2), m.group(3)))
    return funcs


def find(funcs, pattern):
    names = [n for n in funcs if re.search(pattern, n)]
    if len(names) != 1:
        fail("expected one function matching %r, found %r" % (pattern, names))
    return names[0], funcs[names[0]]


def target(ops):
    m = re.search(r"<([^>+]+)", ops)
    return m.group(1) if m else None


def main():
    elf = sys.argv[1]
    funcs = functions(elf)
    _, reset = find(funcs, r"^Reset$")
    pre_name, pre = find(funcs, r"^__pre_init$")

    # Reset up to its first call.
    path = []
    for ins in reset:
        path.append(ins)
        if BRANCH.match(ins[1]):
            if ins[1] != "bl" or target(ins[2]) != pre_name:
                fail("Reset branches to %s before __pre_init" % ins[2])
            break
    else:
        fail("Reset never calls __pre_init")
    if len(path) > 8:
        fail("Reset runs %d instructions before __pre_init" % len(path))

    # __pre_init (apply_at_reset inlined): straight line to its return.
    for ins in pre:
        path.append(ins)
        mn, ops = ins[1], ins[2]
        if (mn.startswith("pop") and "pc" in ops) or (mn == "bx" and "lr" in ops):
            break
        if BRANCH.match(mn):
            fail("__pre_init is not straight-line: %s %s" % (mn, ops))
        if "[pc" in ops:
            m = re.search(r"@ \(([0-9a-f]+)", ops)
            fail("__pre_init loads from a literal pool at %s" % (m.group(1) if m else "?"))
    else:
        fail("__pre_init has no return")

    n = len(path)
    words = len({a // 32 for a, _, _ in path})
    access = 0
    for _, mn, ops in path:
        if mn.startswith(("ldr", "str", "ldm", "stm", "push", "pop")):
            regs = re.search(r"\{([^}]*)\}", ops)
            access += len(regs.group(1).split(",")) if regs and mn.startswith(("ldm", "stm", "push", "pop")) else 1
    cycles = words * FETCH + n + access * ACCESS
    us = cycles / MHZ
    print("check-safe-margin: %s: safe state %d instructions, %d flash words, %d accesses: bound %d cycles = %.1f us at %d MHz (limit %.0f us)"
          % (elf, n, words, access, cycles, us, MHZ, LIMIT_US))
    if us > LIMIT_US:
        fail("bound %.1f us exceeds %.0f us (0045)" % (us, LIMIT_US))


main()
