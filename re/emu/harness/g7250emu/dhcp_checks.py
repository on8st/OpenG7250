#!/usr/bin/env python3
"""DHCP checks DH-01..DH-05 with a product image (docs/PRODUCT-RUNNER.md, dhcp.*).

    dhcp_checks.py <image.elf> [--checks DH-01,...] [--run-id ID]

The firmware's run sheet (product side, emulator-run.md "DHCP retry and
renewal") defines the checks. This harness boots the image with the generic
models and the network bridge, drives the bridge's DHCP server (RFC 2131) and
reads the image's own console lines and the server's message log (emulated
time). It reports what the image did; the pass criteria of the run sheet are
applied where they need only DHCP messages and the console. DH-06 (management
requests during an exchange) and the session effects need the bench's
management and P2 clients and are left to the bench.
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
sys.path.insert(0, HERE)
os.environ.setdefault("G7250_NO_FIXED_PORTS", "1")
import product_run as pr     # noqa: E402
import testlist_run          # noqa: E402

RE_LEASE = re.compile(r"net: DHCP lease (\d+) s from (\S+)")
RE_NOLEASE = re.compile(r"net: no DHCP lease; link-local")
RE_ADDR = re.compile(r"net: address (\S+) mask \S+ gateway \S+ \((\w+)\)")


class Run:
    def __init__(self, elf, out):
        os.makedirs(out, exist_ok=True)
        self.boot = testlist_run.Boot(pr, elf, out, "standin", no_address=True)
        self.dhcp = self.boot.bridge.dhcp
        self.out = out

    def ready(self):
        ok = self.boot.agent.wait_connected(120)
        if ok and POWER_CYCLE_FIRST:
            # one power cycle before the check (the radio's second boot is checked): the
            # DHCP log and the socket log start again with the new boot
            time.sleep(2.0)
            self.boot.power_cycle()
            ok = self.boot.agent.wait_connected(120)
            self.dhcp.log.clear()
        return ok

    def now_s(self):
        t = self.boot.agent.now_us()
        return (t or 0) / 1e6

    def wait_until(self, emu_s, wall=900):
        t0 = time.time()
        while self.now_s() < emu_s and time.time() - t0 < wall:
            time.sleep(0.2)

    def console(self):
        p = os.path.join(self.out, "console-boot2.txt" if POWER_CYCLE_FIRST else "console.txt")
        return open(p, errors="replace").read() if os.path.isfile(p) else ""

    def log(self, kind=None, direction="from_radio"):
        return [m for m in self.dhcp.log if m["dir"] == direction and (kind is None or m["type"] == kind)]

    def close(self):
        self.boot.close()


POWER_CYCLE_FIRST = False


def sockets(out):
    """The CH394 socket log of the boot that is checked: OPEN/CLOSE with port and t_us."""
    path = os.path.join(out, "sockets-boot2.jsonl" if POWER_CYCLE_FIRST else "sockets.jsonl")
    return [json.loads(l) for l in open(path)] if os.path.isfile(path) else []


P2_PORTS = (1024, 1025, 1026, 1027, 1029, 1035, 7250, 7251)


def borrows(socks):
    """Exchanges seen in the socket log. The power-on exchange: a socket open on
    port 68 before the first P2 socket opens. A borrow: a socket that had a P2
    port, reopened on port 68, until that socket is reopened on a P2 port.
    Returns (p2_open_s, poweron (start, end) or None, [(start, end), ...]) in s."""
    p2_open = None
    poweron = None
    out = []
    cur = {}
    for e in socks:
        if e["op"] != "open":
            continue
        t = e["t_us"] / 1e6
        n, port = e["socket"], e["port"]
        if port == 68:
            cur[n] = t
            continue
        if port in P2_PORTS:
            if n in cur:
                st = cur.pop(n)
                if p2_open is None or st < p2_open:    # began before the P2 sockets opened
                    poweron = (st, t)
                else:
                    out.append((st, t))
            if p2_open is None:
                p2_open = t
    if p2_open is None:
        for n, t in cur.items():
            poweron = (t, None)
    return p2_open, poweron, out


def times(msgs):
    return [round(m["t_us"] / 1e6, 2) for m in msgs if m.get("t_us") is not None]


def dh01(elf, out):
    r = Run(elf, out)
    try:
        r.ready()
        r.wait_until(62)
        r.dhcp.configure({"lease_s": 600}); r.dhcp.running = True
        start = r.now_s()
        r.wait_until(start + 140)
        con = r.console()
    finally:
        r.close()
    disc = times(r.log("DISCOVER"))
    addrs = RE_ADDR.findall(con)
    # the lease: the image's lease line, or an address line from DHCP (later images
    # log only the address line), or the server's ACK
    leased = bool(RE_LEASE.search(con)) or any(k == "Dhcp" for _, k in addrs) or bool(r.log("ACK", "to_radio"))
    p2_open, poweron, bw = borrows(sockets(out))
    # rule (A1.11, 0041): every borrow <= 2 s; the free gap after exchange n (n = 0:
    # the power-on exchange, before the P2 sockets) = min(4 * 2^n, 64) s +- 0.5 s,
    # counted between two exchanges while no server answers; no DISCOVER after the
    # first P2 socket opens except inside a borrow
    lengths = [round(b - a, 3) for a, b in bw]
    ends = ([poweron[1]] if poweron and poweron[1] is not None else []) + [b for a, b in bw]
    starts = [a for a, b in bw]
    gaps = [round(st - en, 3) for en, st in zip(ends, starts)]
    no_server = [g for g, en in zip(gaps, ends) if en < start]         # gaps that began before the server came up
    want = [min(4 * 2 ** n, 64) for n in range(len(no_server))]
    gaps_ok = all(abs(g - w) <= 0.5 for g, w in zip(no_server, want))
    borrow_ok = all(l <= 2.0 + 0.05 for l in lengths)
    outside = [t for t in disc if p2_open is not None and t > p2_open
               and not any(a - 0.01 <= t <= b + 0.01 for a, b in bw)]
    ll_first = bool(RE_NOLEASE.search(con) or (addrs and addrs[0][1] == "LinkLocal"))
    ok = ll_first and leased and borrow_ok and gaps_ok and not outside and p2_open is not None
    return {"id": "DH-01", "set_up": "no server until %.0f s emulated, then a server (lease 600 s)" % start,
            "rule": "borrow <= 2 s; free gap after exchange n = min(4*2^n, 64) s +-0.5 s (n = 0: power-on "
                    "exchange); no DISCOVER after the first P2 socket opens except in a borrow (A1.11, 0041)",
            "pass_dhcp_part": ok, "p2_open_s": p2_open, "poweron_exchange_s": poweron,
            "borrows_s": [(round(a, 3), round(b, 3)) for a, b in bw], "borrow_lengths_s": lengths,
            "free_gaps_s": gaps, "free_gaps_without_server_s": no_server, "expected_s": want,
            "discover_times_s": disc, "discover_outside_borrows_s": outside,
            "addresses": addrs, "lease_line": RE_LEASE.findall(con),
            "evidence": "CH394 socket log (sockets.jsonl: the management socket reopened on port 68) and the "
                        "emulated DHCP server's message log",
            "not_checked": "a running session ends on the address change (bench)"}


def lease_run(elf, out, until_s, at_t1=None):
    r = Run(elf, out)
    r.dhcp.configure({"lease_s": 120}); r.dhcp.running = True
    try:
        r.ready()
        lease_at = None
        t0 = time.time()
        while time.time() - t0 < 300 and lease_at is None:
            if r.log("REQUEST") and r.log("ACK", "to_radio"):
                lease_at = r.log("ACK", "to_radio")[0]["t_us"] / 1e6
            time.sleep(0.2)
        if lease_at is None:
            return r, None
        if at_t1:
            r.wait_until(lease_at + 55)
            at_t1(r)
        r.wait_until(lease_at + until_s)
        return r, lease_at
    except Exception:
        r.close()
        raise


def dh02(elf, out):
    r, t = lease_run(elf, out, 75)
    try:
        con = r.console()
    finally:
        r.close()
    if t is None:
        return {"id": "DH-02", "pass_dhcp_part": False, "note": "no lease taken"}
    ren = [m for m in r.log("REQUEST") if m["t_us"] / 1e6 > t + 1]
    first = ren[0] if ren else None
    ok = bool(first) and abs(first["t_us"] / 1e6 - t - 60) < 3 and first["ciaddr"] != "0.0.0.0" \
        and first["dst"] == r.dhcp.server and not first["opt50"] and not first["opt54"]
    return {"id": "DH-02", "set_up": "lease 120 s from the start", "pass_dhcp_part": ok,
            "lease_at_s": round(t, 2), "renewal": first, "addresses": RE_ADDR.findall(con),
            "not_checked": "a session goes on through the renewal (bench)"}


def dh03(elf, out):
    r, t = lease_run(elf, out, 180, at_t1=lambda r: setattr(r.dhcp, "running", False))
    try:
        con = r.console()
    finally:
        r.close()
    if t is None:
        return {"id": "DH-03", "pass_dhcp_part": False, "note": "no lease taken"}
    reqs = [m for m in r.log("REQUEST") if m["t_us"] / 1e6 > t + 1]
    rel = [(round(m["t_us"] / 1e6 - t, 2), m["dst"]) for m in reqs]
    rebind = [x for x in rel if x[1] == "255.255.255.255"]
    disc = [round(x - t, 2) for x in times(r.log("DISCOVER")) if x > t + 1]
    after_end = [d for d in disc if d >= 120 - 0.5]
    _, _, bw = borrows(sockets(out))
    bw_rel = [(round(a - t, 3), round(b - t, 3)) for a, b in bw]
    # after the lease end: DISCOVER at once, then the back-off of DH-01 (4 s free gap first)
    post = [(a, b) for a, b in bw_rel if a >= 120 - 0.5]
    first_gap = round(post[1][0] - post[0][1], 3) if len(post) >= 2 else None
    addr_lines = RE_ADDR.findall(con)
    ok = bool(rel) and abs(rel[0][0] - 60) < 3 and bool(rebind) and abs(rebind[0][0] - 105) < 3 \
        and bool(after_end) and after_end[0] - 120 < 1.0 and first_gap is not None and abs(first_gap - 4) <= 0.5
    return {"id": "DH-03", "set_up": "lease 120 s; server silent from 55 s after the lease; observed to 180 s",
            "pass_dhcp_part": ok, "observed_to_s": 180,
            "requests_after_lease_s_and_dst": rel, "rebind_broadcast_s": rebind[0][0] if rebind else None,
            "discover_after_lease_s": disc, "borrows_after_lease_s": bw_rel,
            "first_free_gap_after_lease_end_s": first_gap, "addresses": addr_lines,
            "expected": "60 s unicast REQUEST; broadcast REQUEST at T2 = 105 s (RFC 2131 4.4.5); at the lease end "
                        "(120 s) link-local and a DISCOVER at once, then the DH-01 back-off (first free gap 4 s)"}


def dh04(elf, out):
    r, t = lease_run(elf, out, 70, at_t1=lambda r: setattr(r.dhcp, "nak_next", 1))
    try:
        con = r.console()
    finally:
        r.close()
    if t is None:
        return {"id": "DH-04", "pass_dhcp_part": False, "note": "no lease taken"}
    nak = r.log("NAK", "to_radio")
    after = [m for m in r.log("DISCOVER") if nak and m["t_us"] >= nak[0]["t_us"]]
    addrs = RE_ADDR.findall(con)
    ok = bool(nak) and bool(after) and (after[0]["t_us"] - nak[0]["t_us"]) / 1e6 < 2 and any(a[1] == "LinkLocal" for a in addrs[1:])
    return {"id": "DH-04", "set_up": "lease 120 s; the renewal is NAKed", "pass_dhcp_part": ok,
            "nak_at_s": round(nak[0]["t_us"] / 1e6 - t, 2) if nak else None,
            "discover_after_nak_s": round((after[0]["t_us"] - nak[0]["t_us"]) / 1e6, 2) if after else None,
            "addresses": addrs, "not_checked": "the session ends (bench)"}


def dh05(elf, out):
    r, t = lease_run(elf, out, 70, at_t1=lambda r: r.dhcp.configure({"address": "192.168.73.11"}))
    try:
        con = r.console()
    finally:
        r.close()
    if t is None:
        return {"id": "DH-05", "pass_dhcp_part": False, "note": "no lease taken"}
    addrs = RE_ADDR.findall(con)
    ok = any(a[0] == "192.168.73.11" for a in addrs)
    return {"id": "DH-05", "set_up": "lease 120 s; the renewal ACK carries 192.168.73.11", "pass_dhcp_part": ok,
            "addresses": addrs, "not_checked": "the session ends and the new address is used by the bench"}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("elf")
    ap.add_argument("--checks", default="DH-01,DH-02,DH-03,DH-04,DH-05")
    ap.add_argument("--power-cycle-first", action="store_true",
                    help="power-cycle once before each check (the second boot is checked)")
    ap.add_argument("--run-id")
    a = ap.parse_args()
    global POWER_CYCLE_FIRST
    POWER_CYCLE_FIRST = a.power_cycle_first
    run_id = a.run_id or datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    root = os.path.join(pr.REPO, ".private", "dhcp-checks", run_id)
    os.makedirs(root, exist_ok=True)
    elf = os.path.abspath(a.elf)
    fn = {"DH-01": dh01, "DH-02": dh02, "DH-03": dh03, "DH-04": dh04, "DH-05": dh05}
    res = []
    for c in [x.strip() for x in a.checks.split(",")]:
        if c in fn:
            res.append(fn[c](elf, os.path.join(root, c)))
        elif c == "DH-06":
            res.append({"id": "DH-06", "pass_dhcp_part": None, "note": "needs the bench's management client: left to the bench"})
    json.dump(res, open(os.path.join(root, "result.json"), "w"), indent=1)
    for x in res:
        print(x["id"], x.get("pass_dhcp_part"))
    print("-> %s" % os.path.relpath(root, pr.REPO))


if __name__ == "__main__":
    main()
