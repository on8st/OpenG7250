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
# Annex P parameters a test needs measured (0035: the spec names them, "TBD" is gone). A test
# that needs one gives "blocked" on the radio until the calibration set holds it.
PENDING_SYMBOLS = {
    "P_rf_off": "P-RF-3", "t_rf_off": "P-RF-3", "t_settle": "P-RF-2", "t_blank": "P-SEN-3",
    "P_swr": "P-PWR-2", "P_temp": "P-PWR-2", "T2": "P-PWR-2", "T3": "P-PWR-2", "T4": "P-PWR-2",
    "S1": "P-PWR-2", "S2": "P-PWR-2", "P_ssb_mean": "P-PWR-2", "underrun time": "P-TIM-2",
    "margin": "P-PWR-3", "tune power": "P-ATU-1", "MAC": "P-NET-2",
}
PID = re.compile(r"\bP-[A-Z0-9]+-\d\b")
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
    c: dict = {"scope": "v1.0", "target": "emulator", "also": [], "needs": [], "pending": [],
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
    for sym, pid in PENDING_SYMBOLS.items():
        if sym in desc and pid not in c["pending"]:
            c["pending"].append(pid)
    for pid in PID.findall(desc):
        if pid not in c["pending"]:
            c["pending"].append(pid)
    return c


# Explicit decisions where the rules are not enough. Each value overrides fields of classify().
# Reason for the TX tests: in r1.0 TX is locked until calibrated (ON8ST, 2026-11-20); an arm
# cannot succeed on the emulator, so every test whose subject is a transmission or an armed
# state runs on the reference model with TX unlocked (host) and on the radio with measured
# values (needs-radio). The emulator runs the T-*-Lnn locked-state tests instead.
TXR = {"target": "needs-radio", "also": ["host"]}
# The freeze rule of Annex P: TX unlocks (record tx bit 4) when every one of these is set.
CAL_SET = ([f"P-RF-{i}" for i in range(1, 6)] + [f"P-PWR-{i}" for i in range(1, 6)]
           + [f"P-SEN-{i}" for i in range(1, 6)] + ["P-TIM-1", "P-TIM-2", "P-RX-2", "P-TX-1",
                                                    "P-ATU-1", "P-ANT-1", "P-SYS-9"])  # P-SYS-9: 0050
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
    "T-A3-06": {"target": "emulator", "also": ["host", "needs-radio"], "tx_part": False,
                "needs": ["hw.observe", "hw.trace", "power.cycle"],
                "notes": ["HIS 85 safe state via hw.observe and hw.trace, from the boot event + 20 us (0045)"]},
    "T-A3-07": {"scope": "not-v1.0"}, "T-A3-42": {"scope": "not-v1.0"},
    "T-A3-15": {"target": "emulator", "also": ["host"], "needs": ["reset.wdt"]},
    "T-A3-38": {"target": "emulator", "also": ["host"], "needs": ["power.cycle", "reset.wdt"],
                "notes": ["'RESTART while armed' part needs TX unlocked: host"]},
    "T-A3-47": {"target": "emulator", "also": ["host"],
                "notes": ["unarmed drop events: runs while TX is locked"]},
    # B3
    "T-B3-09": {"target": "needs-radio", "also": ["emulator"], "needs": ["power.cycle"],
                "pending": ["P-NET-2"]},
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
    "T-B9-03": {"target": "needs-radio", "also": [], "pending": ["P-SEN-1"]},
    "T-B9-05": {"target": "needs-radio", "also": ["host"], "pending": ["P-SEN-1"]},
    # B10: whole group needs a transmission, except these
    **{t: TXR for t in [f"T-B10-{n:02d}" for n in range(1, 41)]},
    "T-B10-01": {"target": "emulator", "also": ["host"], "tx_part": False,
                 "notes": ["record tx bit 1 = 1 in r1.0 (0036): this test applies to a build "
                           "without TX only; the r1.0 frozen state is T-B10-41, T-B10-42"]},
    "T-B10-03": {"target": "emulator", "also": ["host"], "tx_part": False,
                 "notes": ["frozen TX reaches BAD_TOKEN, WATCHDOG_DISABLED, PTT_ACTIVE, "
                           "NO_MODE_CLASS, NOT_CALIBRATED and their order; the other reasons "
                           "need faults, inhibits, level 2 or an update: host"]},
    "T-B10-41": {"target": "emulator", "also": ["host"], "tx_part": False,
                 "notes": ["'with bit 4 = 1 the arm is accepted' part: reference model with a "
                           "calibration set (host)"]},
    "T-B10-42": {"target": "emulator", "also": ["host"], "tx_part": False, "needs": ["hw.observe"]},
    "T-A3-58": {"target": "emulator", "also": ["host"], "tx_part": False, "needs": ["hw.input"],
                "notes": ["needs the local inputs PH6, PH3, PH4 driven: fixture hw.input"]},
    "T-A1-63": {"target": "host", "also": ["emulator"]},
    "T-A1-66": {"target": "emulator", "also": [], "needs": ["dhcp.server", "power.cycle"], "timeout_s": 300,
                "notes": ["DHCP fixtures of the emulator (format v1.4); the reference model has no DHCP"]},
    "T-A1-67": {"target": "host", "also": ["emulator"]},
    "T-A1-68": {"target": "host", "also": ["emulator"]},
    "T-A3-59": {"target": "emulator", "also": ["host"], "tx_part": False,
                "notes": ["needs the debug console (USART1) of the runner; no fixture for it yet"]},
    "T-B4-18": {"target": "emulator", "also": ["host"], "tx_part": False},
    "T-B6-27": {"target": "emulator", "also": ["host"]},
    **{t: {"pending": ["P-RX-2"], "notes": ["DDC0 is not offered before P-RX-2 (B6.1; 0042): blocked there"]}
       for t in ["T-B5-11", "T-B6-02", "T-B6-03", "T-B6-04", "T-B6-05", "T-B6-06", "T-B6-13",
                 "T-B6-14", "T-B6-15", "T-B6-16", "T-B6-24", "T-B6-25"]},
    **{t: {"target": "needs-radio", "also": ["emulator"], "pending": ["P-RX-2"]}
       for t in ["T-B6-08", "T-B6-09", "T-B6-11", "T-B6-12", "T-B6-17"]},
    "T-A1-64": {"target": "emulator", "also": ["host"]},
    # Takeover notice (A2.4): two sockets of hosts[0] are two hosts (A2.2), so no net.hosts;
    # with TX locked the old owner is unarmed, which A2.4 step 1 covers in full
    **{t: {"target": "emulator", "also": ["host"], "needs": [], "tx_part": False,
           "notes": ["two sockets of hosts[0] are hosts A and B (A2.2); the register destination is a third socket in the mapped range"]}
       for t in ["T-A2-10", "T-A2-11", "T-A2-13", "T-A2-15", "T-A2-16", "T-A2-17", "T-B8-21"]},
    # CWX keyer (B10.5a, 0046): judged on the key.log fixture (format v1.5), on the reference
    # model with a calibration set and on the emulator once the image keys; blocked on the
    # calibration set of the freeze rule until then (Annex P)
    **{t: {"target": "emulator", "also": ["host"], "needs": ["key.log"], "pending": CAL_SET,
           "notes": ["needs TX unlocked: blocked until the calibration set is complete (tx bit 4)"]}
       for t in ["T-B10-33", "T-B10-43", "T-B10-44", "T-B10-45", "T-B10-46"]},
    "T-A1-75": {"target": "emulator", "also": ["host"],
                "notes": ["the armed part needs TX unlocked; 'outside a session while armed' is not reachable over P2"]},
    "T-A1-77": {"target": "emulator", "also": [], "needs": ["flash.log"],
                "notes": ["needs the update client (Table M-4 package) in the bench and the slot layout; flash.log v1.8"]},
    "T-A1-78": {"target": "emulator", "also": ["host"],
                "notes": ["needs the update client (Table M-4 package) in the bench and the slot size of the firmware layout"]},
    "T-B6-28": {"target": "needs-radio", "also": [],
                "notes": ["radio test with a test build, dummy loads and a signal generator under HIS 90 (0060)"]},
    "T-B5-21": {"target": "emulator", "also": ["host"], "pending": CAL_SET,
                "notes": ["needs an arm: blocked until the calibration set is complete (tx bit 4)"]},
    "T-B5-20": {"target": "emulator", "also": ["host"],
                "notes": ["the reference model has no socket buffers: it always passes there; the emulator honours the sizes of Table B2-2"]},
    "T-A1-69": {"target": "emulator", "also": ["host"], "needs": ["dhcp.server", "power.cycle"],
                "notes": ["from link up: the trace link event where offered, else counted from the power cycle (stricter)"]},
    "T-A1-76": {"target": "emulator", "also": [], "needs": ["dhcp.server", "power.cycle"],
                "notes": ["MACRAW failing: macraw.fail where offered (proposed), else a runner without MACRAW; console line not checked"]},
    "T-A1-74": {"target": "emulator", "also": ["host"], "needs": ["dhcp.server", "power.cycle"]},
    "T-B10-47": {"target": "emulator", "also": ["host"], "pending": CAL_SET,
                 "notes": ["needs TX unlocked: blocked until the calibration set is complete (tx bit 4)",
                           "tuned, failed and tune-time parts need the model-only fixture atu.sim; the stop "
                           "request is TCI (part C C5.7), not on P2"]},
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
LOCK = {"scope": "v1.0", "target": "emulator", "also": ["host"], "tx_part": False, "pending": []}
BENCH_TESTS = [
    # Kept where the spec has no test of its own; the other T1 locked tests are now spec tests:
    # L02, L06 -> T-B10-41; L03 -> T-B10-03; L04, L05, L08, L12 -> T-B10-42; L10 -> T-A1-63.
    ("T-B10-L01", "TX frozen: record `tx` reads bit 0 = 1, bit 1 = 1 (0036), bits 2 and 3 = 1, "
     "bit 4 = 0 until the calibration set is complete; `streams` bit 1 = 1; `regmap_groups` "
     "bits 0-5 set, 6-7 clear (B4.8; 0035).", ["B4.8", "B8.3", "B10", "Annex P", "0035", "0036"], []),
    ("T-B10-L07", "TX frozen: `ATU_TUNE` = 0 from the owner is accepted as a no-op, answered "
     "with (0x0400, 0) and no `WRITE_STATUS` (B8.4); `ATU_STATE` read by a write reads 5 "
     "(unknown) with `READ_ONLY` (B8.4a); a start (1) is rejected with `NOT_CALIBRATED` and not "
     "latched; from a non-owner `NOT_OWNER` (A3.9b item 1; 0032; Annex P).",
     ["A3.9b", "B8.4", "B8.4a", "Annex P"], []),
    ("T-B10-L13", "TX frozen: `TX_STATE` bit 4 (PA disabled by the host) reads 1 while General "
     "byte 58 bit [0] is 0 and 0 once the owner sets it; a same-host restart by a High Priority "
     "packet only (byte 58 bit [0] not carried over, B5.4) and a takeover clear it again (A3.6a, "
     "Table A3-4); bit 0 and the source [11:8] stay 0, also under a PTT edge (B10, B10.10; "
     "Table B8-2).", ["A3.6a", "B5.4", "B10", "B10.10", "Table B8-2"], []),
    ("T-B10-L09", "TX frozen: `TX_MODE_CLASS`, `KEYED_TIMEOUT`, `LIMIT_TX_TIME` and "
     "`LIMIT_FWD_POWER` writes are answered as in Table B8-2 (accepted values, `BAD_VALUE`, "
     "`LOOSER_LIMIT`, 0 = back to the persistent value) and are reset at the session end; none "
     "moves a pin of the RF section.", ["B8.4", "Table B8-2", "A3.6", "HIS 85"], ["hw.observe"]),
    ("T-B6-L01", "FPGA bus ([HIS] HIS 80) during a session: no command 0x05, 0x06, 0x09 or 0x0A; a "
     "0x01 frame only with the RF section in the safe state (B6.1; 0042); D0-D7 back to outputs "
     "only after STB is low. Needs hw.trace with PE0-PE10.", ["B6.1", "0042", "HIS 80"], ["hw.trace"]),
    ("T-B10-L11", "TX frozen: the HIS 85 safe state is held, traced from power-on, through idle, "
     "a session with PTT held, a takeover, a watchdog standby and a power cycle: no T/R line "
     "leaves its RX level, chain A bit 7 is never set, bias channels 2, 7, 8 are never non-zero, "
     "the ATU latches and the codec are untouched.", ["A3.6", "A3.6a", "B10", "0029", "0031",
     "0032", "HIS 85"], ["hw.observe", "power.cycle", "net.hosts"]),
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


# The firmware's emulator run sheet (Store ST-, Update UP-), as relayed by openg7250 with the
# stimulus only (the bench does not read product/firmware/): which spec tests judge the same
# situation. "covers" on a test lists the run-sheet ids it judges; UNCOVERED lists those no spec
# test covers, with the clause that should, for the spec session.
COVERS: dict[str, list[str]] = {
    "T-A1-67": ["UP-01 (63 for the running slot)", "UP-02 (the full update and the trial)", "UP-08 (write failure, status 55)"],
    "T-A1-78": ["UP-01 (63 whatever the length)"],
    "T-A1-58": ["UP-02 (the own-image install through the management commands)"],
    "T-A1-77": ["UP-02 (flash.log: sectors 6 and 7 only as store writes)"],
    "T-A1-80": ["UP-02 (no transfer after an activation)"],
    "T-A1-23": ["UP-03 (run request: reason 10; register write: BUSY)"],
    "T-B8-10": ["UP-03 (a non-owner's register write: BUSY)"],
    "T-A1-40": ["UP-03 (an arm and UPDATE_BEGIN serialise)"],
    "T-A1-24": ["UP-04"],
    "T-A1-68": ["UP-05 (network down: rollback after 60 s)", "UP-05a (reset before confirming)"],
    "T-A1-09": ["UP-05a (a trial without confirmation rolls back and logs it)"],
    "T-A1-08": ["UP-06", "UP-07 (in part: 'each step of an update')", "ST-06b (in part: the other slot after a power cut)"],
    "T-A1-34": ["UP-09"],
    "T-A1-46": ["UP-09 (after UPDATE_ABORT and after VERIFY without ACTIVATE)"],
    "T-A1-27": ["UP-10 (in part: refused armed, accepted unarmed)"],
    "T-A1-59": ["ST-06b (in part: a block or image that does not verify gets 53 or 55)"],
}
UNCOVERED = [
    ("UP-03", "CAP_REQUEST still answered during an update, and an arm after UPDATE_BEGIN rejected with MAINTENANCE: "
              "A1.8 first item (0022 §10) says both; no test asserts them (T-A1-23 could)"),
    ("UP-05", "the trial with a class 2 fault: A1.8 self-confirmation (0044) needs 'no class 2 fault'; T-A1-68 covers "
              "the network-down case only"),
    ("UP-07", "a power cut during the boot stage's trial-attempt write or the self-confirmation write: A1.8 U-5, U-6 "
              "and 0044; T-A1-08 says 'each step of an update' without naming these two writes"),
    ("UP-10", "what ROLLBACK does: A1.8 and Table M-1 give only its status rules and an empty reply; which image it "
              "starts, its precondition (a good other slot), the log and the reset are not specified"),
    ("ST-01", "a sector erase at the datasheet maximum: B2.3 'Flash work' (0055) and A3.10 (safety timing under load) "
              "allow the stall but no test checks that the processor watchdog and the safety timing hold through it"),
    ("ST-02", "an erase that never ends (beyond 10 s): none; the spec says nothing on a flash operation that does not "
              "finish"),
    ("ST-03", "a power cut during the erase of the other store sector: A1.6 (stored with a version and a check value, "
              "survives a restart) and 0061 (sectors used in turn, a compaction never erases the only copy); no test"),
    ("ST-04", "a power cut after each flash word of a store write (copy, commit): A1.6 and 0061 as ST-03; no test"),
    ("ST-05", "the bus during a long erase: if the FPGA bus of HIS 80, the bench test T-B6-L01 judges it (no spec "
              "test); if the safe-state pins, the run-wide hw.trace rules (A3.6) judge them in every test"),
    ("ST-06a", "a double ECC error in the live store sector (a record header or payload): A1.6 check value; none on "
               "an ECC read error"),
    ("ST-06c", "a double ECC error in the newest commit word: A1.6 and 0061; none"),
]


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
        if t["id"] in COVERS:
            c["covers"] = COVERS[t["id"]]
        tests.append({"id": t["id"], "origin": "spec", "part": t["part"],
                      "title": t["desc"], "reqs": reqs_of(t["id"], t["desc"]), **c})
    for tid, desc, reqs, needs in BENCH_TESTS:
        tests.append({"id": tid, "origin": "testbench", "part": "A-B", "title": desc,
                      "reqs": reqs, "needs": needs, "notes": ["TX frozen until the "
                      "calibration set is complete (Annex P; 0035, 0036)"], **LOCK})
    src_hash = hashlib.sha256(ni.read_bytes() + pc.read_bytes()).hexdigest()
    doc = {"format": "g7250-catalogue", "version": 1,
           "spec": {"branch": "spec/draft", "commit": git("log", "-1", "--format=%h", "--",
                    "product/spec"), "sha256_of_inputs": src_hash},
           "his": {"commit": "72591aa"},
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
             "`pending`: Annex P parameters the test needs measured (result `blocked` until set).", ""]
    summary: dict = {}
    for t in doc["tests"]:
        s = summary.setdefault(t["scope"], {})
        s[t["target"]] = s.get(t["target"], 0) + 1
    lines += ["| Scope | host | emulator | needs-radio |", "|---|---|---|---|"]
    for sc, d in summary.items():
        lines.append(f"| {sc} | {d.get('host', 0)} | {d.get('emulator', 0)} | {d.get('needs-radio', 0)} |")
    lines += ["", "| Id | Scope | Target | Also | tx | Needs | Pending | Requirements | Test |",
              "|---|---|---|---|---|---|---|---|---|"]
    for t in doc["tests"]:
        lines.append("| {id} | {scope} | {target} | {also} | {tx} | {needs} | {tbd} | {reqs} | {title} |".format(
            id=t["id"], scope=t["scope"], target=t["target"], also=", ".join(t["also"]),
            tx="x" if t["tx_part"] else "", needs=", ".join(t["needs"]), tbd=", ".join(t["pending"]),
            reqs=", ".join(t["reqs"]), title=t["title"].replace("|", "\\|")))
    lines += ["", "## Firmware run sheet (ST-, UP-) and the spec tests", "",
              "From the stimulus of each run-sheet check as relayed by openg7250 (the bench does not read "
              "product/firmware/). A run-sheet check is judged by the spec tests that cover it.", "",
              "| Run-sheet id | Covered by |", "|---|---|"]
    by: dict[str, list[str]] = {}
    for t in doc["tests"]:
        for cv in t.get("covers", []):
            by.setdefault(cv.split(" ")[0], []).append(f"{t['id']} {cv[len(cv.split(' ')[0]):].strip()}".strip())
    for rid in sorted(by, key=lambda x: (x[:2], x)):
        lines.append(f"| {rid} | {'; '.join(by[rid])} |")
    lines += ["", "Not covered by any spec test (for the spec session):", "", "| Run-sheet id | Clause that should cover it |", "|---|---|"]
    for rid, why in UNCOVERED:
        lines.append(f"| {rid} | {why} |")
    (OUT / "catalogue.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


if __name__ == "__main__":
    sys.exit(main())
