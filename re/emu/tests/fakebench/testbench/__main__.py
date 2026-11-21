#!/usr/bin/env python3
"""Stand-in test bench for the emulator's own runner checks (RE-side
scaffolding, not the product test bench). Same command line and output
format as `python3 -m testbench run` (g7250-testlist v1.2), with a few tests
of the runner itself against firmware-tests/bench/net_test.c:

  T-EMU-01  UDP echo on 1024 from host 0 (address mapping, same source port)
  T-EMU-02  UDP echo from the host outside the subnet
  T-EMU-03  limited broadcast to 1024 reaches the radio (echo back)
  T-EMU-04  hw.observe: T/R lines at the levels the scaffold sets
  T-EMU-05  hw.trace start/stop with a hw.input edge on PH6, file readable
  T-EMU-06  hw.input on a pin that is not offered is refused
  T-EMU-09  power.cycle: the image boots again, the trace carries a reset event
  T-EMU-10  reset.wdt: watchdog reset, the image boots again, reset event "wdt"
  T-EMU-11  link.set: no traffic while the link is down, traffic again at 100
  T-EMU-12  power.cut: the core stops at at_us, the image boots again from the kept flash
"""
import argparse
import json
import os
import socket
import sys
import time


def addr(s):
    h, p = s.rsplit(":", 1)
    return h, int(p)


class Ctl:
    def __init__(self, ep):
        self.s = socket.create_connection(addr(ep["control"]["address"]), timeout=10)
        self.f = self.s.makefile("rw")

    def op(self, name, **args):
        self.f.write(json.dumps({"op": name, "args": args}) + "\n")
        self.f.flush()
        return json.loads(self.f.readline())


def echo(ep, host, dest, payload, wait=20.0):
    h = ep["hosts"][host]
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.bind((h["bind"], h["ports"][0] + 7))
    s.settimeout(1.0)
    t0 = time.time()
    while time.time() - t0 < wait:
        s.sendto(payload, addr(dest))
        try:
            d, src = s.recvfrom(2048)
        except socket.timeout:
            continue
        return d, src
    return None, None


def check(name, ok, detail=""):
    return {"name": name, "ok": bool(ok), "detail": detail}


def run_test(tid, ep):
    c = ep["radio"]["connect"]["1024"]
    if tid == "T-EMU-01":
        d, src = echo(ep, 0, c, b"ping-0")
        return [check("echo", d == b"ping-0", repr(d)), check("from connect[1024]", src == addr(c), str(src))]
    if tid == "T-EMU-02":
        d, src = echo(ep, 2, c, b"ping-out")
        return [check("echo outside host", d == b"ping-out", repr(d))]
    if tid == "T-EMU-03":
        d, src = echo(ep, 1, ep["broadcast"]["send_to"]["limited"]["1024"], b"bcast")
        return [check("broadcast echo", d == b"bcast", repr(d)), check("reply from connect[1024]", src == addr(c), str(src))]
    ctl = Ctl(ep)
    if tid == "T-EMU-04":
        r = ctl.op("hw.observe")
        v = r.get("value", {})
        tr = v.get("tr_lines", {})
        return [check("ok", r.get("ok"), str(r.get("error", ""))),
                check("T/R at RX levels", tr == {"PF4": 1, "PF6": 0, "PC7": 0, "PH11": 0}, json.dumps(tr)),
                check("chains output-disabled", v.get("spi1_a", {}).get("oe_PA10") == 1 and v.get("spi1_b", {}).get("oe_PH10") == 1),
                check("DAC in reset, never written", v.get("pa_dac", {}).get("reset_PB14") == 0 and
                      all(x is None for x in v.get("pa_dac", {}).get("ch", [0])))]
    if tid == "T-EMU-05":
        r1 = ctl.op("hw.trace", action="start", pins=["PH6"])
        f = (r1.get("value") or {}).get("file")
        i1 = ctl.op("hw.input", pin="PH6", level=1)
        time.sleep(0.5)
        i0 = ctl.op("hw.input", pin="PH6", level=0)
        time.sleep(0.5)
        r2 = ctl.op("hw.trace", action="stop")
        evs = [json.loads(l) for l in open(f)] if f and os.path.isfile(f) else []
        ph6 = [e for e in evs if e.get("kind") == "gpio" and e.get("pin") == "PH6"]
        return [check("start", r1.get("ok") and f, str(r1)), check("input", i1.get("ok") and i0.get("ok"), str(i1)),
                check("stop counts", (r2.get("value") or {}).get("events", 0) == len(evs), str(r2.get("value"))),
                check("PH6 edges in trace", [e["level"] for e in ph6][-2:] == [1, 0], json.dumps(ph6[-2:])),
                check("seq increasing", all(evs[i]["seq"] < evs[i + 1]["seq"] for i in range(len(evs) - 1)))]
    if tid == "T-EMU-09":
        r1 = ctl.op("hw.trace", action="start")
        f = (r1.get("value") or {}).get("file")
        pc = ctl.op("power.cycle")
        d, src = echo(ep, 0, c, b"after-cycle", wait=60.0)
        r2 = ctl.op("hw.trace", action="stop")
        evs = [json.loads(l) for l in open(f)] if f and os.path.isfile(f) else []
        resets = [e for e in evs if e.get("kind") == "reset"]
        return [check("power.cycle ok", pc.get("ok"), str(pc)), check("echo after cycle", d == b"after-cycle", repr(d)),
                check("reset event", len(resets) == 1 and resets[0].get("cause") == "power", json.dumps(resets)),
                check("seq increasing", all(evs[i]["seq"] < evs[i + 1]["seq"] for i in range(len(evs) - 1)))]
    if tid == "T-EMU-10":
        r1 = ctl.op("hw.trace", action="start")
        f = (r1.get("value") or {}).get("file")
        w = ctl.op("reset.wdt")
        d, src = echo(ep, 0, c, b"after-wdt", wait=60.0)
        ctl.op("hw.trace", action="stop")
        evs = [json.loads(l) for l in open(f)] if f and os.path.isfile(f) else []
        resets = [e for e in evs if e.get("kind") == "reset"]
        return [check("reset.wdt ok", w.get("ok"), str(w)), check("echo after reset", d == b"after-wdt", repr(d)),
                check("reset event wdt", len(resets) == 1 and resets[0].get("cause") == "wdt", json.dumps(resets))]
    if tid == "T-EMU-11":
        d0, _ = echo(ep, 0, c, b"before", wait=20.0)
        r0 = ctl.op("link.set", speed=0)
        d1, _ = echo(ep, 0, c, b"link-down", wait=3.0)
        r1 = ctl.op("link.set", speed=100)
        d2, _ = echo(ep, 0, c, b"link-up", wait=20.0)
        bad = ctl.op("link.set", speed=55)
        return [check("echo before", d0 == b"before", repr(d0)), check("link down ok", r0.get("ok"), str(r0)),
                check("no echo while down", d1 is None, repr(d1)), check("echo after link up", d2 == b"link-up", repr(d2)),
                check("bad speed refused", bad.get("ok") is False, str(bad))]
    if tid == "T-EMU-12":
        r1 = ctl.op("hw.trace", action="start")
        f = (r1.get("value") or {}).get("file")
        t = ctl.op("hw.observe")["value"]["t_us"]
        pc = ctl.op("power.cut", at_us=t + 200000)
        time.sleep(3.0)
        d, src = echo(ep, 0, c, b"after-cut", wait=60.0)
        ctl.op("hw.trace", action="stop")
        evs = [json.loads(l) for l in open(f)] if f and os.path.isfile(f) else []
        resets = [e for e in evs if e.get("kind") == "reset"]
        last_before = max([e["t_us"] for e in evs if e.get("kind") != "reset" and evs.index(e) < evs.index(resets[0])] or [0]) if resets else 0
        bad = ctl.op("power.cut", at_us=-1)
        return [check("power.cut ok", pc.get("ok") and pc["value"]["at_us"] >= t + 200000, str(pc)),
                check("image runs again (from the kept flash)", d == b"after-cut", repr(d)),
                check("one reset event, cause power", len(resets) == 1 and resets[0].get("cause") == "power", json.dumps(resets)),
                check("no events after the cut time before the reset", last_before <= t + 200000 + 100, str(last_before)),
                check("bad at_us refused", bad.get("ok") is False, str(bad))]
    if tid == "T-EMU-06":
        r = ctl.op("hw.input", pin="PA0", level=1)
        return [check("refused", r.get("ok") is False and "pin not offered" in r.get("error", ""), str(r))]
    return None


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd")
    r = sub.add_parser("run")
    r.add_argument("--endpoint", required=True)
    r.add_argument("--test", action="append", default=[])
    r.add_argument("--json", action="store_true")
    a = ap.parse_args()
    ep = json.load(open(a.endpoint))
    for tid in a.test:
        t0 = time.time()
        try:
            checks = run_test(tid, ep)
            if checks is None:
                e = {"id": tid, "status": "error", "reason": "unknown test"}
            else:
                e = {"id": tid, "status": "pass" if all(c["ok"] for c in checks) else "fail", "checks": checks}
        except Exception as ex:  # noqa: BLE001
            e = {"id": tid, "status": "error", "reason": "%s: %s" % (type(ex).__name__, ex)}
        e["duration_s"] = round(time.time() - t0, 3)
        print(json.dumps(e))
        sys.stdout.flush()


if __name__ == "__main__":
    main()
