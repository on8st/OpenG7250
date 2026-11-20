#!/usr/bin/env python3
"""Build the test catalogue from the spec.

Inputs (read-only): product/spec/spec/network-interface.md (Annex X3) and
product/spec/spec/part-c-tci-profile.md (C9). Output: testbench/catalogue/catalogue.json
and catalogue.md. Classification = rules below + the explicit OVERRIDES table; every test the
bench adds itself is in BENCH_TESTS. Run from anywhere: python3 testbench/catalogue/build.py

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = ROOT / "product/spec/spec"
OUT = Path(__file__).resolve().parent

ROW = re.compile(r"^\| (T-[A-C]\d{1,2}-\d{2}) \| (.*) \|\s*$")

# --- requirement references -------------------------------------------------------------
SEC = re.compile(r"(?<![\w.])([ABC]\d{1,2}(?:\.\d{1,2}[a-z]?){0,2})(?![\w])")
TABLE = re.compile(r"Table ([ABC]\d{1,2}-\d)")
DEC = re.compile(r"\b(00[0-3]\d)(?: (§\d+(?: item \d+)?))?")
HISREF = re.compile(r"\bHIS-\d{2}\b")


def reqs_of(tid: str, desc: str) -> list[str]:
    group = tid.split("-")[1]
    out: list[str] = [group]
    for m in SEC.finditer(desc):
        s = m.group(1)
        if s[0] == group[0] or s.startswith(("A", "B", "C")):
            out.append(s)
    out += ["Table " + t for t in TABLE.findall(desc)]
    for d, para in DEC.findall(desc):
        out.append(f"{d} {para}".strip())
    seen, res = set(), []
    for r in out:
        if r not in seen:
            seen.add(r)
            res.append(r)
    return res


# --- classification rules ---------------------------------------------------------------
TBD_SYMBOLS = {
    "P_rf_off": ["HIS-07", "HIS-12"], "t_rf_off": ["HIS-07", "HIS-12"],
    "t_settle": ["HIS-07"], "t_blank": ["HIS-11", "HIS-12"], "P_swr": ["HIS-12"],
    "P_temp": ["HIS-12"], "T2": ["HIS-11", "HIS-12"], "T3": ["HIS-11", "HIS-12"],
    "T4": ["HIS-11", "HIS-12"], "S1": ["HIS-11", "HIS-12"], "S2": ["HIS-11", "HIS-12"],
    "underrun time": ["HIS-18"], "P_ssb_mean": ["HIS-11", "HIS-12"], "margin": ["HIS-12"],
    "tune power": ["HIS-08", "HIS-12"], "MAC": ["HIS-15"],
}
NEEDS_RULES = [
    (r"10 Mbit|100 to 10|link loss|link renegotiat|100 Mbit", "link.set"),
    (r"power cycle|power loss|power-on|power on\b", "power.cycle"),
    (r"processor watchdog reset", "reset.wdt"),
    (r"\b10 min\b|\b300 s\b|\b60 s\b|\b5 min\b|for 10 s\b|1000 times", "time.scale"),
    (r"host B|two hosts|second host|another host|non-owner|other host|from 200 sources|20 sources|old owner", "net.hosts"),
    (r"outside the (radio's )?subnet|outside the subnet", "net.outside"),
    (r"(?i)broadcast", "net.broadcast"),
    (r"forward power|SWR|temperature|supply|under-voltage|over-voltage|detector|sensor", "sensor.set"),
    (r"controller", "ctrl.set"),
    (r"DHCP", "dhcp.server"),
    (r"relay|SEND|T/R|safe state|state of A3\.6", "hw.observe"),
]
RADIO = re.compile(
    r"test tone|full-scale|Packet rate|without loss|line rate|flood|Known supply and power"
    r"|ICMP|stable across power cycles|what the chip does|accuracy|at full power|at full drive")
TX_PRECOND = re.compile(
    r"while (the owner is )?(armed|keyed|transmitting)|armed and keyed|keyed at|\bkeys\b|keys once"
    r"|\bkeyed\b|with arm\b|PTT 0→1|a PTT|PTT bit|CWX|tune cycle|tunes within|ATU_TUNE"
    r"|\bKEY\b|break-in|hang delay|TX request|nothing keys|carrier|drive|DUC underflow"
    r"|arm write|an arm\b|arm is|the arm\b|TX_ARM\b|token")
TX_ANY = re.compile(r"\barm|\bkey|PTT|CWX|TX|transmit|ATU|DUC", re.I)


def classify(tid: str, desc: str) -> dict:
    group = tid.split("-")[1]
    c: dict = {"scope": "v1.0", "target": "emulator", "also": [], "needs": [], "tbd": [],
               "tx_part": False, "notes": []}
    # scope
    if desc.startswith("Not in v1.0"):
        c["scope"] = "not-v1.0"
        c["notes"].append("0030: local PTT and KEY not used in v1.0")
    elif "after G6" in desc:
        c["scope"] = "after-G6"
    # target
    if group.startswith("C"):
        if desc.startswith("(L2)"):
            c["scope"], c["target"] = "after-G6", "host"
            c["notes"].append("level 2 (firmware TCI server), after G6; refmodel level 2 first")
        else:
            c["target"] = "host"
            c["notes"].append("level 1: AetherSDR's TCI server vs a TCI test client "
                              "(0013, 0014); the firmware part is level 2, after G6")
        if "reference model" in desc:
            c["needs"].append("sensor.set")
        return c
    if "reference model" in desc:
        c["target"] = "host"
        c["also"] = ["emulator"]
        c["notes"].append("spec marks it reference model; emulator only with the fixtures")
    if RADIO.search(desc):
        c["target"], c["also"] = "needs-radio", ["host"]
    if group in ("A3", "B10") or TX_PRECOND.search(desc):
        c["tx_part"] = True
    for pat, need in NEEDS_RULES:
        if re.search(pat, desc) and need not in c["needs"]:
            c["needs"].append(need)
    for sym, his in TBD_SYMBOLS.items():
        if sym in desc:
            for h in his:
                if h not in c["tbd"]:
                    c["tbd"].append(h)
    return c


# Explicit decisions where the rules are not enough. Each value overrides fields of classify().
# Reason for the TX tests: in r1.0 TX is locked until calibrated (ON8ST, 2026-11-20); an arm
# cannot succeed on the emulator, so every test whose subject is a transmission or an armed
# state runs on the reference model with TX unlocked (host) and on the radio with measured
# values (needs-radio). The emulator runs the T-*-Lnn locked-state tests instead.
TXR = {"target": "needs-radio", "also": ["host"]}
OVERRIDES: dict[str, dict] = {
    # A1: update and reset tests whose subject is the armed / keyed case
    **{t: TXR for t in ["T-A1-02", "T-A1-05", "T-A1-27", "T-A1-29", "T-A1-30", "T-A1-33",
                        "T-A1-38", "T-A1-40", "T-A1-52"]},
    "T-A1-25": {"target": "needs-radio", "also": ["host", "emulator"],
                "notes": ["status 57 part runs on the emulator; status 50 part needs an arm"]},
    "T-A1-24": {"target": "needs-radio", "also": ["host", "emulator"],
                "notes": ["the RX and MAINTENANCE parts run on the emulator once trial boot is "
                          "observable; 'accepted after confirmation' needs TX unlocked"]},
    "T-A1-01": {"needs": ["net.broadcast"]},
    "T-A1-08": {"target": "host", "also": ["emulator"], "needs": ["power.cycle"]},
    "T-A1-18": {"scope": "after-G6"},
    "T-A1-45": {"scope": "after-G6", "target": "host"},
    "T-A1-57": {"target": "emulator", "also": ["host", "needs-radio"], "needs": ["ctrl.set"]},
    "T-A1-58": {"target": "emulator", "also": ["host"], "needs": ["ctrl.set"]},
    # A2: takeover tests are RX-side on the emulator; their TX parts ride on host/radio
    "T-A2-09": {"target": "emulator", "also": ["host"], "tx_part": True,
                "notes": ["'neither can stay armed' needs TX unlocked: host only"]},
    "T-A2-14": {**TXR, "notes": ["KEY part not in v1.0 (0030)"]},
    # A3: whole group needs a transmission, except these
    **{t: TXR for t in [f"T-A3-{n:02d}" for n in range(1, 58)]},
    "T-A3-03": {"target": "emulator", "also": ["host"], "tx_part": False,
                "notes": ["LIMIT_* writes need register group 2 only, no arm"]},
    "T-A3-06": {"target": "emulator", "also": ["host", "needs-radio"], "needs": ["hw.observe",
                "power.cycle"], "notes": ["HIS 85 safe state via hw.observe"]},
    "T-A3-07": {"scope": "not-v1.0"}, "T-A3-42": {"scope": "not-v1.0"},
    "T-A3-15": {"target": "emulator", "also": ["host"], "needs": ["reset.wdt"]},
    "T-A3-38": {"target": "emulator", "also": ["host"], "needs": ["power.cycle", "reset.wdt"],
                "notes": ["'RESTART while armed' part needs TX unlocked: host"]},
    "T-A3-47": {"target": "emulator", "also": ["host"],
                "notes": ["unarmed drop events: runs while TX is locked"]},
    # B3
    "T-B3-09": {"target": "needs-radio", "also": ["emulator"], "needs": ["power.cycle"],
                "tbd": ["HIS-15"]},
    "T-B3-15": {"target": "emulator", "also": ["needs-radio"],
                "notes": ["tuning check on the emulator through the FPGA frame (HIS 80)"]},
    "T-B3-25": {"tx_part": True, "notes": ["'works while armed' part needs TX unlocked"]},
    "T-B3-27": TXR, "T-B3-28": TXR,
    # B4/B5
    "T-B5-04": TXR, "T-B5-07": {"tx_part": True},
    "T-B5-13": {"target": "emulator", "also": ["host", "needs-radio"], "tx_part": True},
    "T-B5-14": {"tx_part": True}, "T-B5-17": {"tx_part": True},
    # B6
    "T-B6-08": {"target": "needs-radio", "also": ["emulator"]},
    "T-B6-09": {"target": "needs-radio", "also": ["emulator"]},
    "T-B6-11": {"target": "needs-radio", "also": ["emulator"],
                "notes": ["emulator only if it can inject an ADC test tone"]},
    "T-B6-12": {"target": "needs-radio", "also": ["emulator"]},
    "T-B6-07": {"target": "host"},
    "T-B6-21": {"notes": ["before-M2 part in v1.0; the M2 part when M2 is in scope"]},
    "T-B6-22": {"scope": "from-M2"},
    # B7/B8
    "T-B7-05": TXR, "T-B7-06": {"tx_part": True},
    "T-B8-02": {"tx_part": True}, "T-B8-09": {"tx_part": True},
    "T-B8-12": TXR, "T-B8-13": {"target": "host", "also": ["needs-radio"]},
    "T-B8-14": {"target": "host", "also": ["needs-radio"]},
    "T-B8-16": TXR, "T-B8-18": {"tx_part": True},
    "T-B8-20": TXR,
    # B9
    "T-B9-01": {"tx_part": True, "notes": ["1 ms while transmitting: radio"]},
    "T-B9-03": {"target": "needs-radio", "also": [], "tbd": ["HIS-11"]},
    "T-B9-05": {"target": "needs-radio", "also": ["host"], "tbd": ["HIS-11"]},
    # B10: whole group needs a transmission, except these
    **{t: TXR for t in [f"T-B10-{n:02d}" for n in range(1, 41)]},
    "T-B10-01": {"target": "emulator", "also": ["host"], "tx_part": False,
                 "notes": ["the locked state; see also T-B10-L01..L12"]},
    "T-B10-08": {"target": "emulator", "also": ["host"]},
    "T-B10-16": {"target": "emulator", "also": ["host"],
                 "notes": ["write range and session-end reset run locked; the unkey timing "
                           "part needs TX unlocked"]},
    "T-B10-18": {"target": "emulator", "also": ["host"], "notes": ["read back via STREAM_STATUS"]},
    "T-B10-19": {"target": "emulator", "also": ["host"]},
    "T-B10-15": {"scope": "not-v1.0"}, "T-B10-37": {"scope": "not-v1.0"},
    # B11
    "T-B11-02": TXR, "T-B11-10": TXR, "T-B11-11": TXR, "T-B11-14": TXR, "T-B11-15": TXR,
    "T-B11-16": TXR,
    "T-B11-13": {"target": "host", "also": ["needs-radio"]},
    "T-B11-05": {"target": "host", "also": ["emulator", "needs-radio"],
                 "notes": ["needs a forced overrun"]},
    "T-B11-06": {"target": "needs-radio", "also": ["host"], "tx_part": True},
    "T-B11-08": {"target": "needs-radio", "also": ["emulator"],
                 "notes": ["records the CH394Q's ICMP behaviour; firmware silence also on the emulator"]},
    "T-B11-09": {"target": "emulator", "also": ["needs-radio"]},
}

# --- tests the bench adds: the TX-locked state (ON8ST, 2026-11-20) ----------------------
# "locked" = TX not yet calibrated (r1.0 until the measured values of HIS-11/HIS-12 are set).
# The expected reject reason for an arm is the spec as written (TX_NOT_IMPLEMENTED, B10.2 item
# 3) until finding SF-01 is decided; the test takes it as a parameter.
LOCK = {"scope": "v1.0", "target": "emulator", "also": ["host"], "tx_part": False, "tbd": []}
BENCH_TESTS = [
    ("T-B10-L01", "Locked: record `tx` reads bit 0 = 1, bit 1 = 0, bits 2 and 3 = 1; "
     "`regmap_groups` bit 2 is set (group 2 present in r1.0).", ["B4.8", "B8.3", "B10"], []),
    ("T-B10-L02", "Locked: a correct arm write (`token XOR ARM_MAGIC`) with every other B10.2 "
     "item 3 precondition met is rejected: `TX_ARM_STATE` stays state 0, generation unchanged, "
     "reject reason = the locked reason (spec: 11 `TX_NOT_IMPLEMENTED`; SF-01).",
     ["B10.2", "Table B8-3", "SF-01"], []),
    ("T-B10-L03", "Locked: with other preconditions also failing (no mode class, watchdog bit "
     "clear, run clear) the arm reject reason is the one of the agreed precedence (SF-02).",
     ["B10.2", "SF-02"], []),
    ("T-B10-L04", "Locked: PTT 0→1 (High Priority byte 4 bit [1]) with PA enable set, DUC0 "
     "inside a transmit range and drive 255 does not key: `TX_STATE` bit 0 stays 0, no "
     "`TX_STATE` push shows a key, and the HIS 85 safe state holds throughout (T/R lines at RX "
     "levels, chain A bit 7 clear in every frame, bias channels 2, 7, 8 at 0).",
     ["A3.1a", "B10.1", "A3.6", "HIS 85"], ["hw.observe"]),
    ("T-B10-L05", "Locked: CWX (byte 5 bit [0]) with dot and dash bits, break-in set, keyer "
     "speed 20, does not key; the safe state holds.", ["B10.5a", "A3.1a", "HIS 85"],
     ["hw.observe"]),
    ("T-B10-L06", "Locked: `ATU_TUNE` = 1 from the owner, armed or not, gets `WRITE_STATUS` "
     "reason 11 `NOT_CALIBRATED`, `ATU_STATE` 0, no tune starts, and the ATU latches PD13, "
     "PG2, PG5 are never written (0032; precedence SF-03).",
     ["A3.9b", "B8.4", "Table B8-3", "0032", "HIS 85"], ["hw.observe"]),
    ("T-B10-L07", "Locked: `ATU_TUNE` = 0 from the owner is accepted as a no-op; from a "
     "non-owner `NOT_OWNER`; during an update `BUSY` (A3.9b item 1).", ["A3.9b", "B8.4"], []),
    ("T-B10-L08", "Locked: DUC I&Q from the owner at 192 ksps is discarded or held without "
     "effect; `STREAM_STATUS` DUC0 reads not running with the agreed reason (SF-04); no RF "
     "path change.", ["B10.7", "B8", "SF-04"], ["hw.observe"]),
    ("T-B10-L09", "Locked: `TX_MODE_CLASS`, `KEYED_TIMEOUT` and the `LIMIT_<name>` writes are "
     "answered as in Table B8-2 (accepted values, `BAD_VALUE`, `LOOSER_LIMIT`); none moves a "
     "pin of the RF section.", ["B8.4", "Table B8-2", "HIS 85"], ["hw.observe"]),
    ("T-B10-L10", "Locked: no management request unlocks TX or changes record `tx` bit 1 within "
     "a session (A1.9, B4.7); the unlock path is SF-05.", ["A1.9", "B4.7", "SF-05"], []),
    ("T-B10-L11", "Locked: the HIS 85 safe state is held, traced from power-on, through idle, "
     "a session with PTT held 10 s, a takeover, a watchdog standby, `RESTART` and an update "
     "begin/abort: no T/R line leaves its RX level, chain A bit 7 is never set, bias channels "
     "2, 7, 8 are never non-zero, PB14 is low until the first DAC write, the ATU latches and the "
     "codec are untouched.", ["A3.6", "A3.6a", "0029", "0031", "0032", "HIS 85"],
     ["hw.observe", "power.cycle", "net.hosts"]),
    ("T-B10-L12", "Locked: High Priority status byte 4 bits [0]–[2] are 0 and status bytes "
     "14–15, 22–23 carry no RF (0030; B9.2).", ["B9.2", "0030"], []),
]


def parse(path: Path, part: str) -> list[dict]:
    tests = []
    for line in path.read_text(encoding="utf-8").splitlines():
        m = ROW.match(line)
        if m:
            tid, desc = m.groups()
            tests.append({"id": tid, "part": part, "desc": desc})
    return tests


def git(*args: str) -> str:
    try:
        return subprocess.run(["git", "-C", str(ROOT), *args], check=True,
                              capture_output=True, text=True).stdout.strip()
    except Exception:
        return "unknown"


def main() -> int:
    ni = SPEC / "network-interface.md"
    pc = SPEC / "part-c-tci-profile.md"
    raw = parse(ni, "A-B") + parse(pc, "C")
    seen = set()
    for t in raw:
        if t["id"] in seen:
            print(f"duplicate test id {t['id']}", file=sys.stderr)
            return 1
        seen.add(t["id"])
    tests = []
    for t in raw:
        c = classify(t["id"], t["desc"])
        ov = OVERRIDES.get(t["id"], {})
        for k, v in ov.items():
            if k == "notes":
                c["notes"] = c["notes"] + v
            else:
                c[k] = v
        tests.append({"id": t["id"], "origin": "spec", "part": t["part"],
                      "title": t["desc"], "reqs": reqs_of(t["id"], t["desc"]), **c})
    for tid, desc, reqs, needs in BENCH_TESTS:
        tests.append({"id": tid, "origin": "testbench", "part": "A-B", "title": desc,
                      "reqs": reqs, "needs": needs, "notes": ["TX locked until calibrated "
                      "(ON8ST, 2026-11-20)"], **LOCK})
    src_hash = hashlib.sha256(ni.read_bytes() + pc.read_bytes()).hexdigest()
    doc = {"format": "g7250-catalogue", "version": 1,
           "spec": {"branch": "spec/draft", "commit": git("log", "-1", "--format=%h", "--",
                    "product/spec"), "sha256_of_inputs": src_hash},
           "his": {"commit": "9d10e72"},
           "targets": ["host", "emulator", "needs-radio"],
           "tests": tests}
    (OUT / "catalogue.json").write_text(json.dumps(doc, indent=1, ensure_ascii=False) + "\n",
                                        encoding="utf-8")
    write_md(doc)
    counts: dict = {}
    for t in tests:
        key = (t["scope"], t["target"])
        counts[key] = counts.get(key, 0) + 1
    for k in sorted(counts):
        print(f"{k[0]:10} {k[1]:12} {counts[k]}")
    print(f"total {len(tests)}")
    return 0


def write_md(doc: dict) -> None:
    lines = ["# Test catalogue (generated by `build.py`; do not edit by hand)", "",
             f"Spec `{doc['spec']['branch']}` at `{doc['spec']['commit']}`, HIS `{doc['his']['commit']}`. "
             "Target: the minimum environment (`host` = reference model or host software only; "
             "`emulator` = firmware in the emulator, also the reference model; `needs-radio`). "
             "`also`: other environments where the test, or a part of it, runs. `tx`: the test "
             "has a part that needs TX unlocked (host refmodel, radio after calibration). "
             "`tbd`: HIS ids of values the test needs that are still TBD (result `blocked`).", ""]
    summary: dict = {}
    for t in doc["tests"]:
        s = summary.setdefault(t["scope"], {})
        s[t["target"]] = s.get(t["target"], 0) + 1
    lines += ["| Scope | host | emulator | needs-radio |", "|---|---|---|---|"]
    for sc, d in summary.items():
        lines.append(f"| {sc} | {d.get('host', 0)} | {d.get('emulator', 0)} | {d.get('needs-radio', 0)} |")
    lines += ["", "| Id | Scope | Target | Also | tx | Needs | TBD | Requirements | Test |",
              "|---|---|---|---|---|---|---|---|---|"]
    for t in doc["tests"]:
        lines.append("| {id} | {scope} | {target} | {also} | {tx} | {needs} | {tbd} | {reqs} | {title} |".format(
            id=t["id"], scope=t["scope"], target=t["target"], also=", ".join(t["also"]),
            tx="x" if t["tx_part"] else "", needs=", ".join(t["needs"]), tbd=", ".join(t["tbd"]),
            reqs=", ".join(t["reqs"]), title=t["title"].replace("|", "\\|")))
    (OUT / "catalogue.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


if __name__ == "__main__":
    sys.exit(main())
