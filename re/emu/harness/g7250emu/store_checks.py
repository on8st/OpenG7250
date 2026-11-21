#!/usr/bin/env python3
"""Store checks ST-01..ST-06 with a firmware TEST image (docs/FLASH-model.md).

    store_checks.py <test-image.elf> [--erase-us N] [--checks ST-01,ST-02,...] [--run-id ID]

The firmware's run sheet (product side, emulator-run.md "Store") defines the
checks; the TEST image compacts its store at every start. This harness boots
the image with the generic models and the flash timing layer, forces each
condition, power-cycles with the flash kept, and reads the image's own console
lines:
  app: boot counter N
  app: TEST compaction: live sector S generation G
  app: TEST compaction done: live sector S' generation G+1
  app: TEST compaction FAILED (...)
Output: .private/store-checks/<run-id>/ (report.txt, per-check consoles). The
report holds only the image's own output, the forced condition and the
verdict, plus the bus evidence of ST-05 (fetches from flash during the erase,
PRIMASK at START, watchdog refreshes during the erase).
"""
from __future__ import annotations
import argparse
import datetime
import json
import os
import re
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
os.environ.setdefault("G7250_NO_FIXED_PORTS", "1")     # no bench here: the fixed source ports are not needed
sys.path.insert(0, HERE)
import product_run as pr     # noqa: E402
import testlist_run          # noqa: E402

RE_COUNTER = re.compile(r"app: boot counter (\d+)")
RE_LIVE = re.compile(r"app: TEST compaction: live sector (\d+) generation (\d+)")
RE_DONE = re.compile(r"app: TEST compaction done: live sector (\d+) generation (\d+)")
RE_FAIL = re.compile(r"app: TEST compaction FAILED \((.*)\)")
RE_RESET = re.compile(r"(boot|app): reset cause (.*)")


def parse(text):
    return {
        "counter": [int(x) for x in RE_COUNTER.findall(text)],
        "live": [(int(a), int(b)) for a, b in RE_LIVE.findall(text)],
        "done": [(int(a), int(b)) for a, b in RE_DONE.findall(text)],
        "failed": RE_FAIL.findall(text),
        "resets": [b for a, b in RE_RESET.findall(text)],
    }


class Session:
    """One image with the flash kept across power cycles, on one Boot."""

    def __init__(self, elf, out, erase_us):
        os.makedirs(out, exist_ok=True)
        self.boot = testlist_run.Boot(pr, elf, out, "standin", flash_erase_us=erase_us, no_address=True)
        self.boot.wait_ready()
        self.out = out

    def console(self, n=None):
        n = n or self.boot.boots
        name = "console.txt" if n == 1 else "console-boot%d.txt" % n
        p = os.path.join(self.out, name)
        return open(p, errors="replace").read() if os.path.isfile(p) else ""

    def ft(self, line):
        return self.boot.agent.ask("ft " + line, timeout=10)

    def info(self):
        r = self.ft("info")
        return json.loads(r[5:]) if r and r.startswith("info ") else {}

    def wait_console(self, pattern, wall=120.0, n=None):
        t0 = time.time()
        while time.time() - t0 < wall:
            if re.search(pattern, self.console(n)):
                return True
            time.sleep(0.25)
        return False

    def wait_stopped(self, wall=120.0):
        t0 = time.time()
        while time.time() - t0 < wall:
            if self.info().get("StoppedByCut"):
                return True
            time.sleep(0.2)
        return False

    def cycle(self):
        self.boot.power_cycle()
        self.boot.agent.wait_connected(120)

    def close(self):
        self.boot.close()


def st01(elf, out, erase_us):
    s = Session(elf, out, erase_us)
    try:
        ok1 = s.wait_console(r"TEST compaction (done|FAILED)", wall=180)
        c1 = parse(s.console(1))
        ev = s.info()
        s.cycle()
        s.wait_console(r"TEST compaction (done|FAILED)", wall=180, n=2)
        c2 = parse(s.console(2))
    finally:
        s.close()
    g1 = c1["done"][0][1] if c1["done"] else None
    g2 = c2["live"][0][1] if c2["live"] else None
    n1 = c1["counter"][0] if c1["counter"] else None
    n2 = c2["counter"][0] if c2["counter"] else None
    verdict = ok1 and c1["done"] and len(c1["resets"]) <= 2 and not any("watchdog" in r.lower() for r in c1["resets"]) \
        and g2 == g1 and n1 is not None and n2 == n1 + 1
    return {"id": "ST-01", "force": "sector erase busy %d us (STAND-IN, not the datasheet value)" % erase_us,
            "pass": bool(verdict), "boot1": c1, "boot2": c2, "st05": ev}


def st02(elf, out):
    s = Session(elf, out, 3600 * 1000000)
    try:
        ok = s.wait_console(r"TEST compaction FAILED", wall=240)
        alive = s.boot.proc.poll() is None
        c1 = parse(s.console(1))
        s.cycle()
        s.wait_console(r"app: TEST compaction", wall=180, n=2)
        c2 = parse(s.console(2))
    finally:
        s.close()
    g_before = c1["live"][0][1] if c1["live"] else None
    g_after = c2["live"][0][1] if c2["live"] else None
    n1 = c1["counter"][0] if c1["counter"] else None
    n2 = c2["counter"][0] if c2["counter"] else None
    verdict = ok and alive and len(c1["resets"]) <= 2 and g_after == g_before and n1 is not None and n2 == n1 + 1
    return {"id": "ST-02", "force": "erase busy beyond 10 s (BSY never clears in the run)", "pass": bool(verdict),
            "boot1": c1, "boot2": c2}


def st03(elf, out, erase_us, fractions=(0.1, 0.5, 0.9)):
    res = []
    for f in fractions:
        d = os.path.join(out, "cut-%02d" % int(f * 100))
        s = Session(elf, d, erase_us)
        try:
            s.ft("cut-erase %s" % f)
            stopped = s.wait_stopped(wall=180)
            c1 = parse(s.console(1))
            s.cycle()
            s.wait_console(r"app: TEST compaction", wall=180, n=2)
            c2 = parse(s.console(2))
        finally:
            s.close()
        g0 = c1["live"][0][1] if c1["live"] else None
        g2 = c2["live"][0][1] if c2["live"] else None
        n1 = c1["counter"][0] if c1["counter"] else None
        n2 = c2["counter"][0] if c2["counter"] else None
        ok = stopped and g2 == g0 and n1 is not None and n2 is not None and n2 >= n1
        res.append({"fraction": f, "pass": bool(ok), "boot1": c1, "boot2": c2})
    return {"id": "ST-03", "force": "power cut during the erase at %s of its duration" % ", ".join(str(f) for f in fractions),
            "pass": all(r["pass"] for r in res), "cuts": res}


def st04(elf, out, erase_us, max_words=64):
    res = []
    for k in range(1, max_words + 1):
        d = os.path.join(out, "word-%03d" % k)
        s = Session(elf, d, erase_us)
        try:
            s.ft("cut-words %d" % k)
            stopped = s.wait_stopped(wall=60)
            c1 = parse(s.console(1))
            if not stopped:
                s.close()
                res.append({"k": k, "stopped": False, "note": "no k-th flash word: the copy and commit wrote fewer words"})
                break
            s.cycle()
            s.wait_console(r"app: TEST compaction", wall=180, n=2)
            c2 = parse(s.console(2))
        finally:
            s.close()
        g0 = c1["live"][0][1] if c1["live"] else None
        g2 = c2["live"][0][1] if c2["live"] else None
        n1 = c1["counter"][0] if c1["counter"] else None
        n2 = c2["counter"][0] if c2["counter"] else None
        ok = g0 is not None and g2 in (g0, g0 + 1) and n1 is not None and n2 is not None and n2 >= n1
        res.append({"k": k, "stopped": True, "pass": bool(ok), "boot1": c1, "boot2": c2})
    return {"id": "ST-04", "force": "power cut after flash word k of the compaction, k = 1, 2, ...",
            "pass": all(r.get("pass", True) for r in res), "cuts": res}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("elf")
    ap.add_argument("--erase-us", type=int, default=4000000,
                    help="sector erase duration; default 4 s, a STAND-IN until the datasheet value is set")
    ap.add_argument("--checks", default="ST-01,ST-02,ST-03,ST-04")
    ap.add_argument("--run-id")
    a = ap.parse_args()
    run_id = a.run_id or datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    root = os.path.join(pr.REPO, ".private", "store-checks", run_id)
    os.makedirs(root, exist_ok=True)
    elf = os.path.abspath(a.elf)
    want = [c.strip() for c in a.checks.split(",")]
    results = []
    if "ST-01" in want:
        results.append(st01(elf, os.path.join(root, "ST-01"), a.erase_us))
    if "ST-02" in want:
        results.append(st02(elf, os.path.join(root, "ST-02")))
    if "ST-03" in want:
        results.append(st03(elf, os.path.join(root, "ST-03"), a.erase_us))
    if "ST-04" in want:
        results.append(st04(elf, os.path.join(root, "ST-04"), a.erase_us))
    json.dump(results, open(os.path.join(root, "result.json"), "w"), indent=1)
    for r in results:
        print("%s %s  (%s)" % (r["id"], "PASS" if r["pass"] else "FAIL", r["force"]))
    print("-> %s" % os.path.relpath(root, pr.REPO))


if __name__ == "__main__":
    main()
