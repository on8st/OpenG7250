#!/usr/bin/env python3
"""ip.conflict raises in the fixture log (2026-11-27): after the runner self-check, the run's fixture-log.jsonl
holds T-EMU-38's two raises with their answers: t_us, and already_set false, then true (the second came
before the image cleared the bit).

  python3 tests/baseline/conflict_record_check.py [run dir, default .private/product-runs/smoke-runner]
"""
import json
import os
import sys

d = sys.argv[1] if len(sys.argv) > 1 else os.path.join(".private", "product-runs", "smoke-runner")
recs = []
for root, _, files in os.walk(d):
    if "fixture-log.jsonl" in files:
        for line in open(os.path.join(root, "fixture-log.jsonl")):
            try:
                e = json.loads(line)
            except ValueError:
                continue
            if e.get("op") == "ip.conflict" and (e.get("args") or {}).get("action") == "raise":
                recs.append(e)
vals = [((e.get("answer") or {}).get("value") or {}) for e in recs]
ok = len(vals) >= 2 and all(isinstance(v.get("t_us"), int) for v in vals[:2]) and vals[0].get("already_set") is False \
    and vals[1].get("already_set") is True and all(isinstance(e.get("boot"), int) for e in recs[:2])
print("  %s ip.conflict raises in the fixture log: %d, already_set %s" % ("ok  " if ok else "FAIL", len(vals), [v.get("already_set") for v in vals]))
print("conflict record check: %s" % ("ALL PASS" if ok else "FAILURES"))
sys.exit(0 if ok else 1)
