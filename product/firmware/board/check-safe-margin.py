#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 ON8ST
"""Check that the boot stage reaches the safe state within 20 us of its first instruction
(spec A3.6; decision 0045), and that every image reaches it within 20 us of a processor fault
(A3.6 "Unexpected stop of the firmware"; 0075). Usage:
check-safe-margin.py --reset <boot ELF> --fault <ELF>...

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
The fault path (board/common/src/stop.rs): each fault vector's entry (HardFault, MemManage,
BusFault, UsageFault) sets the stack pointer and calls g7250_fault_pins first, which is
straight-line to its return in the same way as __pre_init; the bound adds the exception entry,
12 cycles and 8 stacked words at 16 cycles each.
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


def straight(name, body, path):
    """`body` straight-line to its return, appended to `path`; no branch, no literal load."""
    for ins in body:
        path.append(ins)
        mn, ops = ins[1], ins[2]
        if (mn.startswith("pop") and "pc" in ops) or (mn == "bx" and "lr" in ops):
            return
        if BRANCH.match(mn):
            fail("%s is not straight-line: %s %s" % (name, mn, ops))
        if "[pc" in ops:
            m = re.search(r"@ \(([0-9a-f]+)", ops)
            fail("%s loads from a literal pool at %s" % (name, m.group(1) if m else "?"))
    fail("%s has no return" % name)


def bound(elf, what, path, extra=0):
    n = len(path)
    words = len({a // 32 for a, _, _ in path})
    access = 0
    for _, mn, ops in path:
        if mn.startswith(("ldr", "str", "ldm", "stm", "push", "pop")):
            regs = re.search(r"\{([^}]*)\}", ops)
            access += len(regs.group(1).split(",")) if regs and mn.startswith(("ldm", "stm", "push", "pop")) else 1
    cycles = words * FETCH + n + access * ACCESS + extra
    us = cycles / MHZ
    print("check-safe-margin: %s: %s %d instructions, %d flash words, %d accesses: bound %d cycles = %.1f us at %d MHz (limit %.0f us)"
          % (elf, what, n, words, access, cycles, us, MHZ, LIMIT_US))
    if us > LIMIT_US:
        fail("%s: %s bound %.1f us exceeds %.0f us" % (elf, what, us, LIMIT_US))


def fault(elf):
    funcs = functions(elf)
    pins_name, pins = find(funcs, r"^g7250_fault_pins$")
    for vector in ("HardFault", "MemoryManagement", "BusFault", "UsageFault"):
        _, entry = find(funcs, "^%s$" % vector)
        path = []
        for ins in entry:
            path.append(ins)
            if BRANCH.match(ins[1]):
                if ins[1] != "bl" or target(ins[2]) != pins_name:
                    fail("%s: %s branches to %s before the pins" % (elf, vector, ins[2]))
                break
        else:
            fail("%s: %s never calls the pins" % (elf, vector))
        if len(path) > 3:
            fail("%s: %s runs %d instructions before the pins" % (elf, vector, len(path)))
        straight(pins_name, pins, path)
        # Exception entry: 12 cycles and the 8 words stacked (ARMv7-M), each a RAM access.
        bound(elf, "fault path (%s)" % vector, path, extra=12 + 8 * ACCESS)


def reset_path(elf):
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
    straight("__pre_init", pre, path)
    bound(elf, "safe state", path)


def main():
    args = sys.argv[1:]
    if args[:1] != ["--reset"] or "--fault" not in args:
        fail("usage: check-safe-margin.py --reset <boot ELF> --fault <ELF>...")
    i = args.index("--fault")
    for elf in args[1:i]:
        reset_path(elf)
    if not args[i + 1:]:
        fail("no ELF after --fault")
    for elf in args[i + 1:]:
        fault(elf)


main()
