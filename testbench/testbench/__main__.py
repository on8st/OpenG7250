"""Test bench command line (testbench/docs/testlist-format.md).

  python3 -m testbench list [--target emulator] [--scope v1.0] [-o tests.json]
  python3 -m testbench run --endpoint ep.json (--test ID ... | --tests tests.json) [--json] [-o result.json]
      [--param key=value ...]

Run from the testbench/ folder (or with testbench/ on PYTHONPATH).
GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import subprocess
import sys
from pathlib import Path

from . import core
from . import t_a2, t_atu, t_cw, t_mgmt, t_p2, t_tx_locked  # noqa: F401  (register tests)

HERE = Path(__file__).resolve().parent
CATALOGUE = HERE.parent / "catalogue" / "catalogue.json"


def git_commit() -> str:
    try:
        return subprocess.run(["git", "-C", str(HERE), "log", "-1", "--format=%h"], capture_output=True,
                              text=True, check=True).stdout.strip()
    except Exception:
        return "unknown"


def load_catalogue() -> dict:
    return json.loads(CATALOGUE.read_text(encoding="utf-8"))


def cmd_list(a) -> int:
    cat = load_catalogue()
    tests = []
    for t in cat["tests"]:
        if a.target and t["target"] != a.target and not (a.include_also and a.target in t.get("also", [])):
            continue
        if a.scope and t["scope"] != a.scope:
            continue
        if a.implemented and t["id"] not in core.REGISTRY:
            continue
        e = {k: t[k] for k in ("id", "reqs", "target", "scope")}
        e["title"] = t["title"]
        if t.get("needs"):
            e["needs"] = t["needs"]
        if t.get("pending"):
            e["params"] = {"pending": t["pending"]}
        if t.get("timeout_s"):
            e["timeout_s"] = t["timeout_s"]
        tests.append(e)
    doc = {"format": "g7250-testlist", "version": 1, "minor": 6, "spec": cat["spec"], "his": cat["his"],
           "catalogue": {"commit": git_commit(),
                         "sha256": hashlib.sha256(CATALOGUE.read_bytes()).hexdigest()},
           "defaults": {"timeout_s": 30}, "boot": a.boot, "tests": tests}
    text = json.dumps(doc, indent=1, ensure_ascii=False) + "\n"
    if a.o:
        Path(a.o).write_text(text, encoding="utf-8")
        print(f"{len(tests)} tests -> {a.o}", file=sys.stderr)
    else:
        sys.stdout.write(text)
    return 0


def cmd_run(a) -> int:
    endpoint = json.loads(Path(a.endpoint).read_text(encoding="utf-8"))
    params = dict(p.split("=", 1) for p in a.param)
    for k, v in list(params.items()):
        if v.isdigit():
            params[k] = int(v)
    ids: list[str] = list(a.test)
    tl_hash = None
    per_test_params: dict[str, dict] = {}
    if a.tests:
        raw = Path(a.tests).read_bytes()
        tl_hash = hashlib.sha256(raw).hexdigest()
        tl = json.loads(raw)
        for t in tl["tests"]:
            ids.append(t["id"])
            per_test_params[t["id"]] = t.get("params", {})
    started = datetime.datetime.now(datetime.UTC).isoformat(timespec="seconds")
    results, memo = [], {}
    for tid in ids:
        fn = core.REGISTRY.get(tid)
        if fn is not None and fn in memo:
            r = dict(memo[fn], id=tid)
            r["reason"] = (r.get("reason", "") + f" (shared with {memo[fn]['id']})").strip()
        else:
            ctx = core.Ctx(endpoint, {**per_test_params.get(tid, {}), **params})
            r = core.run_one(tid, ctx)
            if fn is not None:
                memo[fn] = r
        results.append(r)
        if a.json:
            print(json.dumps(r, ensure_ascii=False), flush=True)
        else:
            print(f"{r['status']:8} {tid}  {r.get('reason', '')}", flush=True)
            for c in r.get("checks", []):
                if not c["ok"]:
                    print(f"         FAIL {c['name']}: {c['detail']}", flush=True)
    if a.o:
        summary = {s: sum(1 for r in results if r["status"] == s)
                   for s in ("pass", "fail", "skip", "blocked", "error", "not_run")}
        doc = {"format": "g7250-testresult", "version": 1, "minor": 6, "testlist": {"sha256": tl_hash},
               "subject": {"kind": endpoint.get("kind"), "model": endpoint.get("model")},
               "runner": {"name": "testbench", "version": 1}, "testbench": {"commit": git_commit()},
               "started": started,
               "finished": datetime.datetime.now(datetime.UTC).isoformat(timespec="seconds"),
               "results": results, "summary": summary}
        Path(a.o).write_text(json.dumps(doc, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
        print(json.dumps(summary), file=sys.stderr)
    return 0


def main() -> int:
    p = argparse.ArgumentParser(prog="testbench")
    sub = p.add_subparsers(dest="cmd", required=True)
    l = sub.add_parser("list")
    l.add_argument("--target", choices=["host", "emulator", "needs-radio"])
    l.add_argument("--include-also", action="store_true", help="also tests whose `also` names the target")
    l.add_argument("--scope", default="v1.0")
    l.add_argument("--implemented", action="store_true", help="only tests the bench implements")
    l.add_argument("--boot", choices=["per-test", "per-list"], default="per-test",
                   help="per-list only for lists whose tests restore what they change")
    l.add_argument("-o")
    r = sub.add_parser("run")
    r.add_argument("--endpoint", required=True)
    r.add_argument("--test", action="append", default=[])
    r.add_argument("--tests")
    r.add_argument("--json", action="store_true")
    r.add_argument("--param", action="append", default=[])
    r.add_argument("-o")
    a = p.parse_args()
    return cmd_list(a) if a.cmd == "list" else cmd_run(a)


if __name__ == "__main__":
    sys.exit(main())
