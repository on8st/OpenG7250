#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 ON8ST
"""Check the fault-tolerant reads of the flash (board/common/src/flash.rs) and of the RAM kept over a
reset (kept.rs), each a read_tolerant, in a built ELF:
one instance, FAULTMASK set once and cleared once, and nothing in between that calls out or
branches out of the function (a panic or a call with every fault held off would hang the core
until the watchdog). Usage: check-faultmask.py <elf>...
"""
import re
import subprocess
import sys

BRANCH = re.compile(r"^(b|bl|blx|bx|cbz|cbnz|b(eq|ne|cs|cc|hs|lo|mi|pl|vs|vc|hi|ls|ge|lt|gt|le))(\.n|\.w)?$")


def fail(msg):
    print("check-faultmask: FAIL " + msg, file=sys.stderr)
    sys.exit(1)


for elf in sys.argv[1:]:
    out = subprocess.run(["arm-none-eabi-objdump", "-d", "--no-show-raw-insn", elf], check=True, capture_output=True, text=True).stdout
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
    for what, pat in (("flash", r"flash\d*read_tolerant"), ("kept RAM", r"kept\d*read_tolerant")):
        names = [n for n in funcs if re.search(pat, n)]
        if len(names) != 1:
            fail("%s: expected one read_tolerant of this kind, found %r" % (elf, names))
        body = funcs[names[0]]
        lo, hi = body[0][0], body[-1][0]
        on = [i for i, (_, mn, ops) in enumerate(body) if mn == "cpsid" and ops.strip() == "f"]
        off = [i for i, (_, mn, ops) in enumerate(body) if mn == "cpsie" and ops.strip() == "f"]
        if len(on) != 1 or len(off) != 1 or on[0] > off[0]:
            fail("%s: FAULTMASK not set once and cleared once" % elf)
        for a, mn, ops in body[on[0]:off[0]]:
            if mn in ("bl", "blx", "bx", "svc", "udf", "bkpt") or mn.startswith(("pop", "ldm")) and "pc" in ops:
                fail("%s: %s %s at %x with FAULTMASK set" % (elf, mn, ops, a))
            if BRANCH.match(mn):
                m = re.search(r"([0-9a-f]{7,8}) <", ops)
                if not m or not (lo <= int(m.group(1), 16) <= hi):
                    fail("%s: branch out at %x with FAULTMASK set: %s %s" % (elf, a, mn, ops))
                if not (body[on[0]][0] <= int(m.group(1), 16) <= body[off[0]][0]):
                    fail("%s: branch out of the FAULTMASK window at %x: %s %s" % (elf, a, mn, ops))
        print("check-faultmask: %s: %s read_tolerant at %x, %d instructions with FAULTMASK set, no branch out" % (elf, what, lo, off[0] - on[0]))
