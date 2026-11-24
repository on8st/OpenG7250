#!/usr/bin/env python3
"""flash_dump_check on made-up runs (no emulator): a program, then a cut erase of the same sector in one
boot. The words the cut erase kept above its fraction must count as logged (until 2026-11-24 the check
called them "not logged", the fw-adb8f45 boot 3 and 4 mismatch); a word the cut erase lost above its
fraction, or a changed word without a log entry, must still make the run DIFFERENT.

  python3 tests/baseline/dump_check_case.py [checker.py]     (default: harness/g7250emu/flash_dump_check.py)
"""
import importlib.util
import json
import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
path = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "..", "..", "harness", "g7250emu", "flash_dump_check.py")
spec = importlib.util.spec_from_file_location("fdc", path)
F = importlib.util.module_from_spec(spec)
spec.loader.exec_module(F)

SEC6 = 0x080C0000
WORDS = [SEC6 + 32 * i for i in range(2, 11)]       # programmed: 0x080C0040 .. 0x080C0140
FRACTION = 0.000976                                 # 128 KiB: edge at word 3, tolerance 1 word


def run(dump_change=None, extra_word=None):
    """A run of one boot: erase sector 6, program WORDS, a cut erase of sector 6 at FRACTION; its dump."""
    d = tempfile.mkdtemp(prefix="dumpcase-")
    log = [{"op": "erase", "address": hex(SEC6), "len": 131072, "result": "ok", "t_us": 100, "boot": 1}]
    log += [{"op": "program", "address": hex(w), "len": 32, "words": 1, "result": "ok", "t_us": 200 + i, "boot": 1}
            for i, w in enumerate(WORDS)]
    log += [{"op": "erase", "address": hex(SEC6), "len": 131072, "result": "cut", "fraction_done": FRACTION,
             "t_us": 900, "boot": 1}]
    with open(os.path.join(d, "flash-log.jsonl"), "w") as f:
        f.write("".join(json.dumps(e) + "\n" for e in log))
    img = bytearray(b"\xff" * 0x100000)
    for w in WORDS:
        if w >= SEC6 + 32 * 3:                     # the cut erase reached words 0..2 only
            img[w - 0x08000000:w - 0x08000000 + 32] = bytes([w & 0xFF]) * 32
    if dump_change is not None:
        img[dump_change - 0x08000000:dump_change - 0x08000000 + 32] = b"\xff" * 32
    if extra_word is not None:
        img[extra_word - 0x08000000:extra_word - 0x08000000 + 32] = b"\x5a" * 32
    open(os.path.join(d, "flash-1.bin"), "wb").write(bytes(img))
    rows, _, _ = F.check(d)
    r = rows[0]
    return not (r[1] is None or r[2] or r[3] or r[5]), r


fails = 0
for name, want, kw in (
        ("program, then a cut erase of the same sector: dumps equal the log", True, {}),
        ("a programmed word above the fraction lost by the cut erase: DIFFERENT", False, {"dump_change": SEC6 + 0x100}),
        ("a changed word above the fraction without a log entry: DIFFERENT", False, {"extra_word": SEC6 + 0x400})):
    equal, r = run(**kw)
    good = equal == want
    fails += not good
    print("  %s %s (not logged %s, logged but unchanged %s)" % ("ok  " if good else "FAIL", name,
          [hex(w) for w in r[2]], [hex(w) for w in r[3]]))
print("dump check case: %s" % ("ALL PASS" if not fails else "FAILURES"))
sys.exit(1 if fails else 0)
