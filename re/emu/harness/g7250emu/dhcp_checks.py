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
        return self.boot.agent.wait_connected(120)

    def now_s(self):
        t = self.boot.agent.now_us()
        return (t or 0) / 1e6

    def wait_until(self, emu_s, wall=900):
        t0 = time.time()
        while self.now_s() < emu_s and time.time() - t0 < wall:
            time.sleep(0.2)

    def console(self):
        p = os.path.join(self.out, "console.txt")
        return open(p, errors="replace").read() if os.path.isfile(p) else ""

    def log(self, kind=None, direction="from_radio"):
        return [m for m in self.dhcp.log if m["dir"] == direction and (kind is None or m["type"] == kind)]

    def close(self):
        self.boot.close()


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
    gaps = [round(b - a, 2) for a, b in zip(disc, disc[1:])]
    addrs = RE_ADDR.findall(con)
    leased = bool(RE_LEASE.search(con))
    ok = bool(RE_NOLEASE.search(con) or (addrs and addrs[0][1] == "LinkLocal")) and leased
    return {"id": "DH-01", "set_up": "no server until %.0f s emulated, then a server (lease 600 s)" % start,
            "pass_dhcp_part": ok, "discover_times_s": disc, "gaps_s": gaps,
            "expected_gaps_s": "4, then 8, 16, 32, 64, then every 64 (each exchange waits 3 s)",
            "addresses": addrs, "lease_line": RE_LEASE.findall(con),
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
    r, t = lease_run(elf, out, 112, at_t1=lambda r: setattr(r.dhcp, "running", False))
    try:
        con = r.console()
    finally:
        r.close()
    if t is None:
        return {"id": "DH-03", "pass_dhcp_part": False, "note": "no lease taken"}
    reqs = [m for m in r.log("REQUEST") if m["t_us"] / 1e6 > t + 1]
    rel = [(round(m["t_us"] / 1e6 - t, 2), m["dst"]) for m in reqs]
    rebind = [x for x in rel if x[1] == "255.255.255.255"]
    ok = bool(rel) and abs(rel[0][0] - 60) < 3 and bool(rebind) and abs(rebind[0][0] - 105) < 3
    return {"id": "DH-03", "set_up": "lease 120 s; server silent from 55 s after the lease", "pass_dhcp_part": ok,
            "requests_after_lease_s_and_dst": rel, "expected": "60 s unicast, retries after half the remaining time (at least 60 s), 105 s broadcast"}


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
    ap.add_argument("--run-id")
    a = ap.parse_args()
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
