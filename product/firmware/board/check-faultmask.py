#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 ON8ST
"""Check the fault-tolerant reads of the flash (board/common/src/flash.rs) and of the RAM kept over a
reset (kept.rs), each a read_tolerant, in a built ELF:
one instance, FAULTMASK set once and cleared once, and in between, in this order:
the load of SCB CCR, the store of that value with BFHFNMIGN set, DSB, ISB, the read itself (at least
one load), DSB, the store of the CCR value as it was, DSB, ISB. Every branch there stays inside the
read (after the first ISB, up to the DSB that ends it), so no path goes round a barrier or a store,
and nothing calls out or branches out of the function (a panic or a call with every fault held off
would hang the core until the watchdog). No load from memory but these and the stack's is outside
the read. Usage: check-faultmask.py <elf>...
"""
import re
import subprocess
import sys

BRANCH = re.compile(r"^(b|bl|blx|bx|cbz|cbnz|b(eq|ne|cs|cc|hs|lo|mi|pl|vs|vc|hi|ls|ge|lt|gt|le))(\.n|\.w)?$")
LOAD = re.compile(r"^(ldr|ldrb|ldrh|ldrsb|ldrsh|ldrd|ldm|ldmia|ldmdb|vldr|vldm\w*|ldrex\w*|lda\w*)(eq|ne|cs|cc|mi|pl|ge|lt|gt|le|hi|ls)?(\.w|\.n)?$")
STORE = re.compile(r"^(str|strb|strh|strd|stm|stmia|stmdb|vstr|vstm\w*|strex\w*|stl\w*)(eq|ne|cs|cc|mi|pl|ge|lt|gt|le|hi|ls)?(\.w|\.n)?$")
NODEST = re.compile(r"^(str|stm|vstr|vstm|stl|cmp|cmn|tst|teq|push|dsb|isb|dmb|cps|it|nop|b$|b\.|beq|bne|bcs|bcc|bmi|bpl|bhi|bls|bge|blt|bgt|ble|cbz|cbnz|udf|bx)")
REG = r"(?:r\d+|ip|lr|sl|fp)"
CCR = 0xE000ED14
BFHFNMIGN = 0x100
# Registers a load outside the read may address: the fault status and the flash interface.
DEVICES = (range(0xE0000000, 0xE0100000), range(0x52002000, 0x52002400))


class Bad(Exception):
    pass


def parse(text):
    funcs, cur = {}, None
    for line in text.splitlines():
        m = re.match(r"^([0-9a-f]{8}) <(.+)>:$", line)
        if m:
            cur = m.group(2)
            funcs[cur] = []
            continue
        m = re.match(r"^\s+([0-9a-f]+):\s+(\S+)\s*(.*)$", line)
        if m and cur:
            ops = re.split(r"\s*[@;]", m.group(3))[0] if "<" not in m.group(3) else m.group(3)
            funcs[cur].append((int(m.group(1), 16), m.group(2), ops.strip()))
    return funcs


def imm(s):
    m = re.search(r"#(-?(?:0x[0-9a-f]+|\d+))", s)
    return int(m.group(1), 0) if m else None


def operands(ops):
    return [o.strip() for o in re.split(r",(?![^\[]*\])(?![^{]*})", ops)]


def trace(body):
    """For every instruction the address a load or store uses (None if not known), the constant each
    register holds before it, and where each register was last written (straight on, in address order:
    a register written anywhere in a skipped or repeated part is simply not known after it)."""
    const, written, out = {}, {}, []
    for i, (_, mn, ops) in enumerate(body):
        addr, base = None, None
        m = re.search(r"\[(%s|sp)(?:,\s*#(-?\d+))?\](!?)(?:,\s*#(-?\d+))?" % REG, ops)
        if m and (LOAD.match(mn) or STORE.match(mn)):
            base = m.group(1)
            if base in const and not m.group(3):
                addr = const[base] + int(m.group(2) or 0)
        out.append((addr, base, dict(const), dict(written)))
        dests = []
        o = operands(ops)
        if mn.startswith(("pop", "ldm")):
            dests = re.findall(REG, ops)
        elif mn.startswith("ldrd"):
            dests = o[:2]
        elif not NODEST.match(mn) and o and re.fullmatch(REG, o[0]):
            dests = [o[0]]
        if m and base and (m.group(3) or m.group(4)) and (LOAD.match(mn) or STORE.match(mn)):
            dests.append(base)
        for d in dests:
            old = const.pop(d, None)
            written[d] = i
            v = imm(ops)
            if mn.startswith("movt") and old is not None and v is not None:
                const[d] = (old & 0xFFFF) | v << 16
            elif re.match(r"^(mov|movw|movs)(\.w|\.n)?$", mn) and v is not None and len(o) == 2:
                const[d] = v & 0xFFFFFFFF
    return out


def check(body):
    """The checks for one read_tolerant; returns the number of instructions with FAULTMASK set."""
    lo, hi = body[0][0], body[-1][0]
    on = [i for i, (_, mn, ops) in enumerate(body) if mn == "cpsid" and ops == "f"]
    off = [i for i, (_, mn, ops) in enumerate(body) if mn == "cpsie" and ops == "f"]
    if len(on) != 1 or len(off) != 1 or on[0] > off[0]:
        raise Bad("FAULTMASK not set once and cleared once")
    on, off = on[0], off[0]
    info = trace(body)
    win = range(on + 1, off)

    def at(i):
        return "%s %s at %x" % (body[i][1], body[i][2], body[i][0])

    for i in range(on, off):
        mn, ops = body[i][1], body[i][2]
        if mn in ("bl", "blx", "bx", "svc", "udf", "bkpt") or mn.startswith(("pop", "ldm")) and "pc" in ops:
            raise Bad("%s with FAULTMASK set" % at(i))

    # The stores to CCR: BFHFNMIGN set from the value just loaded, then that value again.
    stores = [i for i in range(len(body)) if STORE.match(body[i][1]) and info[i][0] == CCR]
    loads = [i for i in range(len(body)) if LOAD.match(body[i][1]) and info[i][0] == CCR]
    if len(loads) != 1 or loads[0] not in win:
        raise Bad("CCR not loaded once with FAULTMASK set")
    if len(stores) != 2 or any(i not in win for i in stores):
        raise Bad("CCR not stored twice (BFHFNMIGN set, then as it was) with FAULTMASK set")
    load, (put, back) = loads[0], stores
    if not load < put < back:
        raise Bad("CCR stored before it was loaded")
    was = operands(body[load][2])[0]
    val, old = operands(body[put][2])[0], operands(body[back][2])[0]
    w = info[put][3].get(val)
    if w is None or not load < w < put or not re.match(r"^orr(s)?(\.w)?$", body[w][1]):
        raise Bad("the value of %s is not made by an OR after the load" % at(put))
    o = operands(body[w][2])
    src = o[1] if len(o) == 3 else o[0]
    if src != was or info[w][3].get(src) != load or imm(body[w][2]) != BFHFNMIGN:
        raise Bad("%s is not the CCR loaded with BFHFNMIGN set" % at(w))
    if old != was or info[back][3].get(old) != load:
        raise Bad("%s does not put back the CCR as loaded" % at(back))
    for i in (load, put, back):
        if re.search(r"(eq|ne|cs|cc|mi|pl|ge|lt|gt|le|hi|ls)(\.w|\.n)?$", body[i][1]) and body[i][1] not in ("ldr", "str"):
            raise Bad("%s is conditional" % at(i))

    def first(mn, a, b):
        for i in range(a, b):
            if body[i][1] == mn:
                return i
        return None

    # DSB, ISB after the first store, before any read; DSB, ISB after the second, before CPSIE.
    d1 = first("dsb", put + 1, back)
    s1 = first("isb", (d1 or put) + 1, back)
    d2 = None
    for i in range(back - 1, put, -1):
        if body[i][1] == "dsb":
            d2 = i
            break
    d3 = first("dsb", back + 1, off)
    s3 = first("isb", (d3 or back) + 1, off)
    if d1 is None or s1 is None:
        raise Bad("no DSB then ISB after BFHFNMIGN is set")
    if d2 is None or d2 <= s1:
        raise Bad("no DSB between the read and the store that puts CCR back")
    if d3 is None or s3 is None:
        raise Bad("no DSB then ISB after CCR is put back")

    # The read, and no other read of memory.
    reads = 0
    for i in range(len(body)):
        mn, ops = body[i][1], body[i][2]
        if not LOAD.match(mn) and not mn.startswith("tbb") and not mn.startswith("tbh"):
            continue
        a, base = info[i][0], info[i][1]
        if s1 < i < d2:
            if a is None and base is not None and base != "sp":
                reads += 1
            continue
        if i == load or base == "sp" or a is not None and any(a in d for d in DEVICES):
            continue
        raise Bad("%s reads memory outside the read" % at(i))
    if reads == 0:
        raise Bad("no load between the barriers")

    # Branches with FAULTMASK set, and into the window from outside it: only inside the read.
    inside = range(body[s1][0] + 1, body[d2][0] + 1)
    for i in range(len(body)):
        mn, ops = body[i][1], body[i][2]
        if not BRANCH.match(mn):
            continue
        m = re.search(r"([0-9a-f]{7,8}) <", ops)
        to = int(m.group(1), 16) if m else None
        if on <= i < off:
            if to is None or not lo <= to <= hi:
                raise Bad("branch out with FAULTMASK set: %s" % at(i))
            if not (s1 < i < d2 and to in inside):
                raise Bad("branch round a step of the FAULTMASK window: %s" % at(i))
        elif to is not None and body[on][0] <= to <= body[off][0]:
            # Data after the last return can decode as a branch; only code before it counts.
            ret = [k for k in range(len(body)) if body[k][1].startswith(("pop", "ldm")) and "pc" in body[k][2] or body[k][1] == "bx" and body[k][2] == "lr"]
            if not ret or i < ret[-1]:
                raise Bad("branch into the FAULTMASK window: %s" % at(i))
    return off - on


KINDS = (("flash", r"flash\d*read_tolerant"), ("kept RAM", r"kept\d*read_tolerant"))


def main(elfs):
    for elf in elfs:
        out = subprocess.run(["arm-none-eabi-objdump", "-d", "--no-show-raw-insn", elf], check=True, capture_output=True, text=True).stdout
        funcs = parse(out)
        for what, pat in KINDS:
            names = [n for n in funcs if re.search(pat, n)]
            try:
                if len(names) != 1:
                    raise Bad("expected one read_tolerant of this kind, found %r" % names)
                n = check(funcs[names[0]])
            except Bad as e:
                print("check-faultmask: FAIL %s: %s: %s" % (elf, what, e), file=sys.stderr)
                sys.exit(1)
            print("check-faultmask: %s: %s read_tolerant at %x, %d instructions with FAULTMASK set: CCR, barriers and read in order, no branch out" % (elf, what, funcs[names[0]][0][0], n))


if __name__ == "__main__":
    main(sys.argv[1:])
