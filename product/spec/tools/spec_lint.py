#!/usr/bin/env python3
"""Lint for the G7250-P2 spec (rc1 definition of done, checks 1, 2, 4, 5).

Checks over spec/*.md:
  L1  no "pending 0xxx" left
  L2  every "TBD" carries a HIS id (HIS-nn) or "G6" on the same line
  L3  every referenced decision number (0001..0099 in decision context) exists in decisions/
  L4  every referenced HIS-nn exists in open-questions-his.md
  L5  every paragraph or table row with MUST/SHOULD/MAY (upper case, RFC 2119) cites a source:
      [KEY], a decision number, or a section reference "Bn"/"An"/"Cn"
  L6  conformance test ids (T-...) are defined once across the spec (a table row whose first
      cell is the id); citations elsewhere are references
  L7  register names used in the spec exist in spec/register-map.yaml (if that file exists)
  L8  spec/register-map.yaml matches the spec tables (tools/regmap.py --check)
  L9  no "TBD" in the spec (v1.0 freeze, 0035); every pending-hardware id P-... used is defined
      in Annex P, Table P-1, and every id defined there is used elsewhere; a value given only
      as "per HIS-nn" (without a P-id) is flagged
  L10 spec/traceability-auto.md is up to date (tools/trace.py --check)

Run from product/spec:  python3 tools/spec_lint.py    (exit 1 on any finding)
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SPEC = sorted((ROOT / "spec").glob("*.md"))
DECISIONS = {p.name[:4] for p in (ROOT / "decisions").glob("[0-9][0-9][0-9][0-9]-*.md")}
HIS_IDS = set(re.findall(r"### (HIS-\d\d)", (ROOT / "open-questions-his.md").read_text()))
REGMAP = ROOT / "spec" / "register-map.yaml"

RE_NORM = re.compile(r"\b(MUST|SHOULD|MAY|REQUIRED|SHALL)\b")
RE_CITE = re.compile(r"§ ?\d|\[[A-Z][A-Za-z0-9.\-]+\]|\b00[0-9]{2}\b|\b[ABC]\d{1,2}(\.\d+)?\b|\bHIS-\d\d\b|RFC ?\d{3,4}")
RE_DEC = re.compile(r"(?<![\d.x])(00[0-9]{2})(?![\d.x])")
RE_HIS = re.compile(r"\bHIS-(\d\d)\b")
RE_TEST = re.compile(r"\b(T-[A-Z0-9]+(?:-[A-Z0-9]+)*-\d{2,3})\b")
RE_REG = re.compile(r"`([A-Z][A-Z0-9]*_[A-Z0-9_]+)`")


def blocks(text):
    """Paragraphs; table rows count as their own block."""
    out, cur = [], []
    for line in text.splitlines():
        if line.startswith("|"):
            if cur:
                out.append(" ".join(cur)); cur = []
            out.append(line)
        elif not line.strip():
            if cur:
                out.append(" ".join(cur)); cur = []
        else:
            cur.append(line)
    if cur:
        out.append(" ".join(cur))
    return out


def main():
    findings = []
    tests = {}
    regnames = None
    if REGMAP.exists():
        regnames = set(re.findall(r"^\s*-?\s*name:\s*([A-Z][A-Z0-9_]+)", REGMAP.read_text(), re.M))
        # TCI names defined in part C are valid references too (e.g. in the AetherSDR annex)
        partc = ROOT / "spec" / "part-c-tci-profile.md"
        if partc.exists():
            regnames |= set(RE_REG.findall(partc.read_text()))
    for path in SPEC:
        text = path.read_text()
        rel = path.relative_to(ROOT)
        for i, line in enumerate(text.splitlines(), 1):
            if re.search(r"pending 00\d\d", line, re.I):
                findings.append(f"L1 {rel}:{i}: 'pending' left: {line.strip()[:90]}")
            if "TBD" in line and not re.search(r"HIS-\d\d|\bG6\b", line):
                findings.append(f"L2 {rel}:{i}: TBD without HIS id: {line.strip()[:90]}")
            for d in RE_DEC.findall(line):
                if d not in DECISIONS and not re.search(r"0x|port|byte|\d{4,}", line[:0]):
                    # ignore obvious non-decision numbers such as ports 0035 do not occur; keep strict
                    findings.append(f"L3 {rel}:{i}: decision {d} does not exist")
            for h in RE_HIS.findall(line):
                if f"HIS-{h}" not in HIS_IDS:
                    findings.append(f"L4 {rel}:{i}: HIS-{h} not in open-questions-his.md")
            # A test is defined by a table row whose first cell is its id; an id cited
            # elsewhere (another row, a list item, the changelog) is a reference, not a definition.
            m = re.match(r"^\| (T-[A-Z0-9]+(?:-[A-Z0-9]+)*-\d{2,3}) \|", line)
            if m:
                t = m.group(1)
                if t in tests:
                    findings.append(f"L6 {rel}:{i}: test id {t} also at {tests[t][0]}:{tests[t][1]}")
                tests.setdefault(t, (rel, i))
            if regnames is not None and path.name != "part-c-tci-profile.md":
                for r in RE_REG.findall(line):
                    if r not in regnames:
                        findings.append(f"L7 {rel}:{i}: register {r} not in register-map.yaml")
        for b in blocks(text):
            if RE_NORM.search(b) and not RE_CITE.search(b) and not b.lstrip().startswith("#"):
                if "RFC 2119" in b or "BCP 14" in b:
                    continue
                findings.append(f"L5 {rel}: normative text without a source: {b.strip()[:100]}")
    # L9: v1.0 freeze (0035): no TBD; P-ids defined and used
    ni = (ROOT / "spec" / "network-interface.md").read_text()
    defined = set(re.findall(r"^\| (P-[A-Z0-9]+-\d+) \|", ni, re.M))
    used = set()
    for path in SPEC:
        for i, line in enumerate(path.read_text().splitlines(), 1):
            if "TBD" in line and "\"TBD\"" not in line and path.name != "CHANGELOG.md":
                findings.append(f"L9 {path.relative_to(ROOT)}:{i}: TBD after the v1.0 freeze: {line.strip()[:80]}")
            if re.search(r"\bper HIS-\d\d", line) and not re.search(r"\bP-[A-Z0-9]+-\d+\b", line) and path.name != "CHANGELOG.md":
                findings.append(f"L9 {path.relative_to(ROOT)}:{i}: value 'per HIS-nn' without a P-id: {line.strip()[:80]}")
            if re.match(r"^\| P-[A-Z0-9]+-\d+ \|", line):
                continue
            used |= set(re.findall(r"\bP-[A-Z0-9]+-\d+\b", line))
    for u in sorted(used - defined):
        findings.append(f"L9 P-id {u} used but not defined in Annex P")
    for d in sorted(defined - used):
        findings.append(f"L9 P-id {d} defined in Annex P but not used")
    # L8: the generated register map must match the spec tables (tools/regmap.py --check)
    import subprocess
    if REGMAP.exists():
        rc = subprocess.run([sys.executable, str(ROOT / "tools" / "regmap.py"), "--check"]).returncode
        if rc:
            findings.append("L8 spec/register-map.yaml is out of date: run python3 tools/regmap.py")
    # L10: generated traceability file up to date
    rc = subprocess.run([sys.executable, str(ROOT / "tools" / "trace.py"), "--check"], capture_output=True).returncode
    if rc:
        findings.append("L10 spec/traceability-auto.md is out of date or a decision is not traced: run python3 tools/trace.py")
    for f in findings:
        print(f)
    print(f"\n{len(findings)} finding(s) in {len(SPEC)} file(s); {len(tests)} test id(s)")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
