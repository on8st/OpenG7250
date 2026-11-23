#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 ON8ST
"""Every processor fault vector of an image points to the handler of A3.6 "Unexpected stop of the
firmware" (0075 point 6; board/common/src/stop.rs): the vector table entries of HardFault (3),
MemManage (4), BusFault (5) and UsageFault (6) hold the address of that vector's entry in stop.rs,
and each entry sets the main stack pointer from a literal that is the top of the stack (the value
of vector 0, `_stack_start`), calls g7250_fault_pins (the pins of A3.6; its time is bounded by
check-safe-margin.py), puts its vector number in r0 and branches to fault_entry, with nothing
before it. Usage: check-fault-vectors.py <elf>...
"""
import re
import subprocess
import sys

VECTORS = {3: "HardFault", 4: "MemoryManagement", 5: "BusFault", 6: "UsageFault"}


def run(*a):
    return subprocess.run(a, check=True, capture_output=True, text=True).stdout


def symbols(elf):
    out = {}
    for line in run("arm-none-eabi-nm", elf).splitlines():
        p = line.split()
        if len(p) == 3:
            out.setdefault(p[2], int(p[0], 16))
    return out


def vector_table(elf):
    words = []
    for line in run("arm-none-eabi-objdump", "-s", "-j", ".vector_table", elf).splitlines():
        m = re.match(r"^\s*([0-9a-f]{7,8})\s+((?:[0-9a-f]{8}\s?){1,4})", line)
        if m:
            for w in m.group(2).split():
                words.append(int.from_bytes(bytes.fromhex(w), "little"))
    return words


def word_at(elf, addr):
    out = run("arm-none-eabi-objdump", "-s", f"--start-address={addr}", f"--stop-address={addr + 4}", elf)
    for line in out.splitlines():
        m = re.match(r"^\s*([0-9a-f]{7,8})\s+([0-9a-f]{8})", line)
        if m and int(m.group(1), 16) == addr:
            return int.from_bytes(bytes.fromhex(m.group(2)), "little")
    return None


def body(elf, name, addr):
    text = run("arm-none-eabi-objdump", "-d", "--no-show-raw-insn", f"--start-address={addr}", f"--stop-address={addr + 20}", elf)
    ins = []
    for line in text.splitlines():
        m = re.match(r"^\s+[0-9a-f]+:\s+(\S+)\s*(.*)$", line)
        if m:
            ins.append((m.group(1), m.group(2)))
    return ins


def check(elf):
    syms = symbols(elf)
    vt = vector_table(elf)
    entry = [a for n, a in syms.items() if "fault_entry" in n]
    if len(entry) != 1:
        return [f"{elf}: fault_entry not found once ({len(entry)})"]
    bad = []
    for i, name in VECTORS.items():
        if name not in syms:
            bad.append(f"{elf}: {name} not defined")
            continue
        if i >= len(vt) or vt[i] & ~1 != syms[name] or vt[i] & 1 != 1:
            got = f"{vt[i]:#x}" if i < len(vt) else "missing"
            bad.append(f"{elf}: vector {i} is {got}, not {name} at {syms[name]:#x}")
            continue
        ins = body(elf, name, syms[name])
        ops = [o for o, _ in ins]
        want = ["ldr", "msr", "bl", "movs", "b.w"]
        if ops[:5] != want or "msp" not in ins[1][1].lower() or "<g7250_fault_pins>" not in ins[2][1] or f"#{i}" not in ins[3][1].replace(" ", "") or f"{entry[0]:x}" not in ins[4][1]:
            bad.append(f"{elf}: {name}: {ins[:5]} is not: stack pointer set, the pins, r0 = {i}, branch to fault_entry")
            continue
        # The stack pointer comes from a literal that is the top of the stack (the reset value).
        lit = re.search(r"@ \(([0-9a-f]+)", ins[0][1])
        sp = word_at(elf, int(lit.group(1), 16)) if lit else None
        if sp is None or sp != syms.get("_stack_start") or sp != vt[0]:
            bad.append(f"{elf}: {name}: stack literal {sp} is not _stack_start {syms.get('_stack_start')} (vector 0 {vt[0]:#x})")
            continue
        print(f"check-fault-vectors: {elf}: vector {i} -> {name}: stack set, pins, r0 = {i}, b fault_entry")
    return bad


bad = [b for e in sys.argv[1:] for b in check(e)]
for b in bad:
    print("check-fault-vectors: FAIL " + b)
sys.exit(1 if bad or not sys.argv[1:] else 0)
