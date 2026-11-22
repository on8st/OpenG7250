"""Compare the flash dumps of a product run with its flash log (docs/FLASH-model.md).

  python3 harness/g7250emu/flash_dump_check.py <run dir> [test id ...]

At every power cycle or cut the runner dumps the flash (flash-N.bin, the end of
boot N). Per boot, each 32-byte word of sectors 4-7 (slot B and the store) that
changed against the previous dump must be in flash-log.jsonl, and each logged word
must have changed. A cut or ECC result is evidence only when both counts are 0: a
model that misses programmed words counts the words of a cut wrongly and marks the
wrong word torn. Boot 1 is compared with an erased bank, so slot A (loaded from the
ELF) and the boot stage are left out.

Limits: the last boot has no dump and is NOT checked; a word programmed with its old
content (or all ones) cannot be seen in a dump; a word programmed and then erased in
the same boot is left out; after an erase in a boot, words that kept their old content
in an interrupted erase are left out; so are the words slot.corrupt changed. flash.fill calls (fixture-log.jsonl) are replayed in
their order: a filled word that no later entry of the boot touches must hold the fill's data
in the dump (its own count); an image write between the pull before a fill and the fill itself
counts as after it.
"""
import json
import os
import sys

LO, HI = 0x80000, 0x100000
STORE = range(0x080C0000, 0x08100000)
BASE = 0x08000000
FF = b"\xff" * 32


def fill_words(x):
    """{word address: 32 bytes} of a flash.fill record."""
    a, n, hx = x["args"]["address"], x["args"]["len"], x["args"].get("data")
    a = int(a, 0) if isinstance(a, str) else a
    raw = bytes.fromhex(hx) if hx is not None else b"\xff"
    full = raw * (n // len(raw))
    return {a + i: full[i:i + 32] for i in range(0, n, 32)}


def check(d):
    log = [json.loads(l) for l in open(os.path.join(d, "flash-log.jsonl")) if l.strip()]
    fx = os.path.join(d, "fixture-log.jsonl")
    fills = [x for x in map(json.loads, open(fx)) if x["op"] == "flash.fill" and x["answer"].get("ok")] if os.path.isfile(fx) else []
    prev, n, rows = None, 1, []
    while os.path.isfile(os.path.join(d, "flash-%d.bin" % n)):
        cur = open(os.path.join(d, "flash-%d.bin" % n), "rb").read()[LO:HI]
        if len(cur) != HI - LO:
            rows.append((n, None, None, None, "dump too short"))
            break
        base = prev if prev is not None else b"\xff" * (HI - LO)
        mine = [(i, 1, e) for i, e in enumerate(log) if e.get("boot") == n]
        mine += [(x["after"], 0, {"op": "fill", "words": fill_words(x)}) for x in fills if x["boot"] == n]
        mine = [e for _, _, e in sorted(mine, key=lambda t: t[:2])]
        filled, maybe = {}, set()
        changed = {BASE + LO + i for i in range(0, HI - LO, 32) if cur[i:i + 32] != base[i:i + 32] and cur[i:i + 32] != FF}
        logged, erased_later = set(), set()
        for e in mine:
            if e["op"] == "fill":
                logged -= set(e["words"])
                erased_later -= set(e["words"])
                filled.update(e["words"])
                continue
            if e["op"] in ("erase", "bank_erase", "program"):
                fa = int(e.get("address", "0x08000000"), 16)
                for w in range(fa, fa + e.get("len", 0x100000), 32):
                    if e.get("result") == "cut" and e["op"] != "program":
                        maybe.add(w)               # an interrupted erase: erased, or the fill's data
                    else:
                        filled.pop(w, None)
            if e["op"] in ("erase", "bank_erase"):
                a = int(e.get("address", "0x08000000"), 16)
                size = e.get("len", 0x100000)
                gone = {w for w in logged if a <= w < a + size}
                logged -= gone
                erased_later |= gone
            elif e["op"] == "program":
                a = int(e["address"], 16)
                if a >= BASE + LO:
                    logged |= set(range(a, a + e["len"], 32))
        # slot.corrupt changes one byte of the array directly (entries "corrupt", "restore")
        poked = {int(e["address"], 16) & ~31 for e in mine if e["op"] in ("corrupt", "restore")}
        changed -= poked - logged
        # a word written back with the same content after an erase in this boot shows no change
        same = {w for w in logged if w not in changed and any(e["op"] in ("erase", "bank_erase") for e in mine)
                and cur[w - BASE - LO:w - BASE - LO + 32] != FF}
        wrong = [w for w, v in filled.items() if cur[w - BASE - LO:w - BASE - LO + 32] not in (v, FF if w in maybe else v)]
        changed -= set(filled)
        rows.append((n, len(changed), sorted(changed - logged) + sorted(wrong), sorted(logged - changed - same),
                     "; ".join("%s cut at %s" % (e["op"], e.get("torn") or e.get("address")) for e in mine if e.get("result") == "cut")))
        prev, n = cur, n + 1
    return rows, n, max([e.get("boot", 1) for e in log] or [1])


if __name__ == "__main__":
    run = sys.argv[1]
    tests = sys.argv[2:] or sorted(x for x in os.listdir(run) if os.path.isfile(os.path.join(run, x, "flash-1.bin")))
    for t in tests:
        d = os.path.join(run, t)
        if not os.path.isfile(os.path.join(d, "flash-log.jsonl")):
            print("%s: no flash log: not checked" % t)
            continue
        rows, n, last = check(d)
        if not rows:
            print("%s: no dump (no power cycle): not checked" % t)
            continue
        bad = any(r[1] is None or r[2] or r[3] for r in rows)
        print("%s: %s" % (t, "DIFFERENT: no cut or ECC result of this test is evidence" if bad else "dumps equal the log"))
        for b, c, miss, extra, cut in rows:
            if c is None:
                print("   boot %d: %s" % (b, cut))
                continue
            print("   boot %d: %d words changed; not logged %d %s; logged but unchanged %d %s%s"
                  % (b, c, len(miss), [hex(x) for x in miss[:4]], len(extra), [hex(x) for x in extra[:4]], "; " + cut if cut else ""))
        print("   boot %d and later: no dump, not checked" % n)
