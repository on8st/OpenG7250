#!/usr/bin/env python3
"""Text report of a product run for the coordinator (docs/PRODUCT-RUNNER.md).

    result_report.py <run-dir> [--lines 60] [--grep REGEX]

Gives only what may go to the product side: the summary, each test's id,
status and reason (for a fail: the first failing checks), the "not modelled"
errors separately, and the image's own console output (first N lines of the
first booted test). Nothing from the emulator's internals.
"""
from __future__ import annotations
import argparse
import json
import os
import re


def report(run, lines=60, grep=None):
    r = json.load(open(os.path.join(run, "result.json")))
    out = ["product run %s: image sha256 %s..., boot %s, testbench %s" % (
        os.path.basename(run.rstrip("/")), r["subject"]["elf_sha256"][:16],
        r.get("environment", {}).get("boot", "?"), r.get("testbench", {}).get("commit") or "?")]
    out.append("SUMMARY: " + ", ".join("%s %d" % kv for kv in r["summary"].items()))
    for n in r.get("notes", []):
        out.append("RUN NOTE: " + n)
    cr = r.get("emulator_crash_reports", "not recorded")
    out.append("EMULATOR CRASH REPORTS in the run's window: %s" % (
        "not readable" if cr is None else cr if isinstance(cr, str) else "%d%s" % (len(cr), " " + ", ".join(cr) if cr else "")))
    sp = [(e["id"], e["emulator"]["ecc_spans_not_covered"]) for e in r["results"] if e.get("emulator", {}).get("ecc_spans_not_covered")]
    out.append("NOT COVERED, reads of a marked (ECC) word while its watchpoint is not in (PG set until 1 us after, first us of flash.ecc): %s" % (
        "no such span in this run" if not sp else "; ".join("%s: %d spans, at most %d instructions" % (i, s["spans"], s["max_instructions"]) for i, s in sp)))
    out.append("")
    out.append("PER TEST (id status reason):")
    nm = []
    for e in r["results"]:
        reason = e.get("reason") or ""
        if e["status"] == "fail":
            bad = [c for c in e.get("checks", []) if not c.get("ok")]
            reason = "; ".join("%s: %s" % (c["name"], str(c.get("detail", ""))[:80]) for c in bad[:2])
        if reason.startswith("not modelled"):
            nm.append((e["id"], reason))
        out.append("  %s %s %s" % (e["id"], e["status"], reason.replace("\n", " ")[:160]))
    out.append("")
    out.append("NOT MODELLED (environment limits, not verdicts on the firmware):")
    out += ["  %s: %s" % x for x in nm] or ["  none"]
    booted = [e for e in r["results"] if "artifacts" in e]
    out.append("")
    if booted:
        con = open(os.path.join(run, booted[0]["artifacts"]["console"]), errors="replace").read().splitlines()
        out.append("IMAGE CONSOLE (USART1, test %s, first %d lines):" % (booted[0]["id"], lines))
        out += ["  " + l for l in con[:lines]]
        if grep:
            hits = [l for l in con if re.search(grep, l)]
            out.append("")
            out.append("CONSOLE LINES MATCHING %r:" % grep)
            out += ["  " + l for l in hits] or ["  none"]
    else:
        out.append("IMAGE CONSOLE: no test booted the image")
    return "\n".join(out) + "\n"


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("run")
    ap.add_argument("--lines", type=int, default=60)
    ap.add_argument("--grep")
    a = ap.parse_args()
    print(report(a.run, a.lines, a.grep), end="")
