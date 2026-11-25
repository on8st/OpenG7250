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
import re
import socket
import sys
import time

from .erasewait import erases, line, wait_erase


def addr(s):
    h, p = s.rsplit(":", 1)
    return h, int(p)


class Ctl:
    def __init__(self, ep):
        # 30 s per fixture call, as the test bench (a power.cycle waits for the emulator's start); a call over 10 s
        # is written to the result by the runner
        self.s = socket.create_connection(addr(ep["control"]["address"]), timeout=30)
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


EP_PATH = ""
NOTES = []      # lines for the result's reason, also of a test that passes


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
        # wait in emulated time, not wall time: the cut comes 200 ms of emulated time later
        t_end = time.time() + 60.0
        while time.time() < t_end:
            o = ctl.op("hw.observe")
            if o.get("ok") and o["value"]["t_us"] < t:
                break                       # the new emulator after the cut counts from 0
            time.sleep(0.2)
        time.sleep(0.5)
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
    if tid == "T-EMU-13":
        # supply-fault evidence: RST# (PB5) stays low, no network answer; the
        # power cycle at the end makes the runner dump the flash for comparison
        r1 = ctl.op("hw.trace", action="start", pins=["PB5"])
        f = (r1.get("value") or {}).get("file")
        time.sleep(3.5)
        d, _ = echo(ep, 0, c, b"discovery?", wait=4.0)
        ctl.op("hw.trace", action="stop")
        evs = [json.loads(l) for l in open(f)] if f and os.path.isfile(f) else []
        pb5 = [e["level"] for e in evs if e.get("kind") == "gpio" and e.get("pin") == "PB5"]
        pc = ctl.op("power.cycle")
        return [check("RST# (PB5) held low", pb5 != [] and all(l == 0 for l in pb5), json.dumps(pb5[:8])),
                check("no network answer", d is None, repr(d)),
                check("power cycle for the flash dump", pc.get("ok"), str(pc))]
    if tid == "T-EMU-14":
        # FPGA bus pins PE0-PE10 never driven by the image (bus gate closed)
        pins = ["PE%d" % i for i in range(11)]
        r1 = ctl.op("hw.trace", action="start", pins=pins)
        f = (r1.get("value") or {}).get("file")
        time.sleep(10.0)
        ctl.op("hw.trace", action="stop")
        evs = [json.loads(l) for l in open(f)] if f and os.path.isfile(f) else []
        pe = [e for e in evs if e.get("kind") == "gpio" and e.get("pin") in pins]
        driven = [e for e in pe if e.get("level") in (0, 1) or e.get("mode") == "output"]
        seen = sorted({e["pin"] for e in pe}, key=lambda x: int(x[2:]))
        return [check("all PE0-PE10 seen in the trace", len(seen) == 11, ",".join(seen)),
                check("none driven (level 0/1 or mode output)", not driven, json.dumps(driven[:4])),
                check("modes", True, json.dumps(sorted({(e["pin"], e.get("mode"), str(e.get("level"))) for e in pe})[:11]))]
    if tid == "T-EMU-15":
        # clock check: TIM2 toggles PB0 and SysTick toggles PB1 every 50 ms of the image's clock
        r1 = ctl.op("hw.trace", action="start", pins=["PB0", "PB1", "PB2"])
        f = (r1.get("value") or {}).get("file")
        time.sleep(3.0)
        ctl.op("hw.trace", action="stop")
        evs = [json.loads(l) for l in open(f)] if f and os.path.isfile(f) else []
        def periods(pin):
            t = [e["t_us"] for e in evs if e.get("kind") == "gpio" and e.get("pin") == pin and e.get("level") in (0, 1)]
            d = sorted(b - a for a, b in zip(t[1:], t[2:]))
            return d[len(d) // 2] / 1000.0 if d else None
        p0, p1, p2 = periods("PB0"), periods("PB1"), periods("PB2")
        return [check("TIM2 period 50 ms (emulated)", p0 is not None and abs(p0 - 50) < 1.0, "median %s ms" % p0),
                check("TIM5 compare period 50 ms (emulated)", p2 is not None and abs(p2 - 50) < 1.0, "median %s ms" % p2),
                check("SysTick period 50 ms (emulated)", p1 is not None and abs(p1 - 50) < 1.0, "median %s ms" % p1)]
    if tid == "T-EMU-16":
        # boot event and the boot phase: a trace started after power-on begins with the
        # last boot event and the pin changes of the first 10 ms (since_boot_us)
        r1 = ctl.op("hw.trace", action="start")
        f = (r1.get("value") or {}).get("file")
        time.sleep(0.5)
        ctl.op("hw.trace", action="stop")
        evs = [json.loads(l) for l in open(f)] if f and os.path.isfile(f) else []
        boots = [e for e in evs if e.get("kind") == "boot"]
        watched = ["PF4", "PF6", "PC7", "PH11", "PA10", "PH10", "PB14", "PB10", "PD12", "PB5", "PD5", "PH9",
                   "PA11", "PH12", "PD13", "PG2", "PG5"]
        last = {}
        for e in evs:
            if e.get("kind") == "gpio" and "since_boot_us" in e:
                last[e["pin"]] = e
        lines = []
        for p in watched:
            e = last.get(p)
            lines.append("%s %s" % (p, "since_boot_us %.3f level %s mode %s pull %s" % (
                e["since_boot_us"], e["level"], e["mode"], e["pull"]) if e else "no change in the boot phase"))
        return [check("trace starts with the boot event", bool(evs) and evs[0].get("kind") == "boot",
                      str(boots[:1])),
                check("boot phase per watched pin", True, "; ".join(lines))]
    if tid == "T-EMU-17":
        # format v1.6: net.log (datagrams in emulated time), key.log (empty without a keyer),
        # the link event in the trace (replayed at the start)
        r1 = ctl.op("hw.trace", action="start")
        f = (r1.get("value") or {}).get("file")
        n0 = ctl.op("net.log", action="start")
        k0 = ctl.op("key.log", action="start")
        d, _ = echo(ep, 0, c, b"netlog-1", wait=20.0)
        n1 = ctl.op("net.log", action="read", **{"from": 0})
        k1 = ctl.op("key.log", action="stop")
        n2 = ctl.op("net.log", action="stop")
        ctl.op("hw.trace", action="stop")
        evs = [json.loads(l) for l in open(f)] if f and os.path.isfile(f) else []
        ents = (n1.get("value") or {}).get("entries", [])
        mine = [e for e in ents if e.get("head", "").startswith(b"netlog-1".hex())]
        to_r = [e for e in mine if e["dir"] == "to_radio"]
        fr_r = [e for e in mine if e["dir"] == "from_radio"]
        link = [e for e in evs if e.get("kind") == "link"]
        return [check("echo", d == b"netlog-1", repr(d)),
                check("net.log start t_us", n0.get("ok") and isinstance((n0.get("value") or {}).get("t_us"), int), str(n0)),
                check("datagram to and from the radio", bool(to_r) and bool(fr_r), json.dumps(mine[:2])),
                check("answer after arrival, emulated", bool(to_r) and bool(fr_r) and fr_r[0]["t_us"] >= to_r[0]["t_us"],
                      "%s -> %s" % (to_r[0]["t_us"] if to_r else None, fr_r[0]["t_us"] if fr_r else None)),
                check("radio port 1024", bool(to_r) and to_r[0]["radio_port"] == 1024, json.dumps(to_r[:1])),
                check("paging fields", all(k in (n1.get("value") or {}) for k in ("count", "dropped", "next")), str(list((n1.get("value") or {}).keys()))),
                check("net.log stop", n2.get("ok"), str(n2)[:200]),
                check("key.log start/stop, no key changes", k0.get("ok") and k1.get("ok") and (k1.get("value") or {}).get("count_key") == 0,
                      str(k1)[:200]),
                check("link event in the trace", bool(link) and link[-1].get("up") == 1, json.dumps(link[-1:]))]
    if tid == "T-EMU-18":
        # format v1.7: net.inject places datagrams at exact emulated offsets through the
        # CH394 receive path; time.wait advances in emulated time
        h = ep["hosts"][0]
        so = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        so.bind((h["bind"], h["ports"][0] + 11))
        so.settimeout(5.0)
        n0 = ctl.op("net.log", action="start")
        pl = [b"inj-%d" % k for k in range(3)]
        r = ctl.op("net.inject", host=0, from_port=h["ports"][0] + 11, to_port=1024, at_us=[0, 121, 1000],
                   payloads=[x.hex() for x in pl])
        got = []
        for _ in range(3):
            try:
                d, _ = so.recvfrom(2048)
                got.append(d)
            except socket.timeout:
                break
        w0 = ctl.op("time.wait", us=0)
        w = ctl.op("time.wait", us=5000)
        n1 = ctl.op("net.log", action="stop")
        ents = [e for e in (n1.get("value") or {}).get("entries", []) if e.get("injected")]
        ts = (r.get("value") or {}).get("t_us", [])
        return [check("inject ok, three placing times", r.get("ok") and len(ts) == 3, str(r)[:200]),
                check("offsets kept exactly", len(ts) == 3 and ts[1] - ts[0] == 121 and ts[2] - ts[0] == 1000, str(ts)),
                check("injected entries in net.log at those times", [e["t_us"] for e in ents] == ts, json.dumps(ents)[:300]),
                check("through the receive path to port 1024", all(e["radio_port"] == 1024 and e["dir"] == "to_radio" for e in ents)),
                check("echoes reach the bench socket", sorted(got) == sorted(pl), repr(got)),
                check("time.wait 5000 us", w.get("ok") and w0.get("ok") and w["value"]["t_us"] - w0["value"]["t_us"] >= 5000,
                      "%s -> %s" % (w0.get("value"), w.get("value")))]
    if tid == "T-EMU-19":
        # sock.log (format v1.7): the CH394's OPEN/CLOSE in emulated time, read across a power cycle
        r0 = ctl.op("sock.log", action="start")
        pc = ctl.op("power.cycle")
        d, _ = echo(ep, 0, c, b"after-cycle", wait=60.0)
        r1 = ctl.op("sock.log", action="stop")
        ents = (r1.get("value") or {}).get("entries", [])
        last = max([e.get("boot", 0) for e in ents] or [0])
        b2 = [e for e in ents if e.get("boot") == last]     # sock.log started before the cycle
        opens = [e for e in b2 if e.get("action") == "OPEN"]
        return [check("start", r0.get("ok"), str(r0)), check("power.cycle", pc.get("ok") and d == b"after-cycle", repr(d)),
                check("entries of the boot after the cycle", bool(b2), json.dumps(ents[:4])),
                check("an OPEN of port 1024 after the cycle", any(e.get("port") == 1024 for e in opens), json.dumps(opens[:3])),
                check("fields", all(set(("t_us", "socket", "action", "port", "mode", "boot")) <= set(e) for e in ents) and bool(ents))]
    if tid == "T-EMU-20":
        # format v1.8: gpio.force (exact start and release in emulated time), dac.force, time.advance
        r1 = ctl.op("hw.trace", action="start", pins=["PF4"])
        f = (r1.get("value") or {}).get("file")
        m = ctl.op("gpio.force", pin="PF4", target="moder", mode="output", from_us=1000)
        o = ctl.op("gpio.force", pin="PF4", target="odr", level=1, from_us=2000, duration_us=5000)
        ctl.op("time.wait", us=20000)
        dc = ctl.op("dac.force", channel=2, value=100)
        ctl.op("time.wait", us=1000)
        ob = ctl.op("hw.observe")
        a0 = ctl.op("time.wait", us=0)
        t0 = time.time()
        av = ctl.op("time.advance", us=200000)     # this busy-polling image runs far below real time
        wall = time.time() - t0
        ctl.op("hw.trace", action="stop")
        evs = [json.loads(l) for l in open(f)] if f and os.path.isfile(f) else []
        fo = [e for e in evs if e.get("kind") in ("force", "release") and e.get("target") == "odr"]
        span = fo[1]["t_us"] - fo[0]["t_us"] if len(fo) >= 2 else None
        ch = ((ob.get("value") or {}).get("pa_dac") or {}).get("ch", [])
        return [check("gpio.force moder and odr accepted", m.get("ok") and o.get("ok"), str(m) + str(o)),
                check("odr force released exactly 5000 us later (trace)", span == 5000, json.dumps(fo)),
                check("dac.force channel 2 = 100 in hw.observe", dc.get("ok") and len(ch) >= 2 and ch[1] == 100, str(ch)),
                check("time.advance 200 ms of radio time", av.get("ok") and av["value"]["t_us"] - a0["value"]["t_us"] >= 200000,
                      "%s -> %s in %.1f s wall" % (a0.get("value"), av.get("value"), wall))]
    if tid == "T-EMU-21":
        # console.log from power-on (no start), across a power cycle; gpio.force at_power_on
        r0 = ctl.op("console.log", action="read")
        first = (r0.get("value") or {}).get("entries", [])
        f = ctl.op("gpio.force", pin="PF4", target="idr", level=1, at_power_on=True)
        pc = ctl.op("power.cycle")
        d, _ = echo(ep, 0, c, b"after-cycle", wait=60.0)
        r1 = ctl.op("console.log", action="read")
        ents = (r1.get("value") or {}).get("entries", [])
        resc = os.path.join(os.path.dirname(os.path.abspath(EP_PATH)), "run.resc")
        script = open(resc).read() if os.path.isfile(resc) else ""
        ctl.op("gpio.force", pin="PF4", target="idr", level=None, at_power_on=True)
        return [check("lines from power-on without a start", any(e.get("line", "").startswith("bench ") for e in first), json.dumps(first[:2])),
                check("fields t_first_us <= t_us, line, boot", bool(first) and all(e["t_first_us"] <= e["t_us"] and "boot" in e for e in first)),
                check("lines of the second boot after the power cycle", pc.get("ok") and d == b"after-cycle" and
                      any(e.get("boot", 1) > first[0].get("boot", 1) for e in ents) if first else False, json.dumps(ents[-2:])),
                check("at_power_on force in the new boot's start-up script", f.get("ok") and 'ForceIdrAtBoot "PF4" 1' in script, str(f)),
                check("endpoint usart1_timed false", ep.get("usart1_timed") is False)]
    if tid == "T-EMU-22":
        # format v1.8 flash fixtures without flash work by the image: flash.timing, flash.log,
        # flash.ecc, slot.corrupt (the erased state 0xFF outside the image), ch394.silent
        ft = ep.get("flash_timing") or {}
        tm = ctl.op("flash.timing", erase_us=ft.get("t_erase_max_us"))
        fl0 = ctl.op("flash.log", action="start")
        ec = ctl.op("flash.ecc", address=0x080C0040, type="double")
        sc = ctl.op("slot.corrupt", slot=2, offset=0x1000)
        sr = ctl.op("slot.corrupt", slot=2, restore=True)
        sc1 = ctl.op("slot.corrupt", slot=2, offset=0x1000, xor=0x80)
        fl = ctl.op("flash.log", action="read")
        ents = (fl.get("value") or {}).get("entries", [])
        si = ctl.op("ch394.silent", boots=1)
        pc = ctl.op("power.cycle")
        d1, _ = echo(ep, 0, c, b"silent?", wait=8.0)
        pc2 = ctl.op("power.cycle")
        d2, _ = echo(ep, 0, c, b"after-silent", wait=60.0)
        fl2 = ctl.op("flash.log", action="read")
        sc2 = ctl.op("slot.corrupt", slot=2, restore=True)
        resc = os.path.join(os.path.dirname(os.path.abspath(EP_PATH)), "run.resc")
        script = open(resc).read() if os.path.isfile(resc) else ""
        v = sc.get("value") or {}
        return [check("endpoint flash_timing with t_erase_max_us and source", ft.get("t_erase_max_us") == 4000000 and bool(ft.get("source")), json.dumps(ft)),
                check("flash.timing erase_us accepted", tm.get("ok") and (tm.get("value") or {}).get("erase_us") == 4000000, str(tm)),
                check("flash.ecc accepted", ec.get("ok"), str(ec)),
                check("slot.corrupt: erased byte 0xFF XOR 1 = 0xFE at 0x08081000", sc.get("ok") and v.get("old") == 255 and v.get("new") == 254
                      and v.get("address") == "0x08081000", str(sc)),
                check("slot.corrupt restore", sr.get("ok") and (sr.get("value") or {}).get("restored") == 255, str(sr)),
                check("flash.log has the corrupt entries", sum(1 for e in ents if e.get("op") == "corrupt") >= 2 and all("boot" in e for e in ents), json.dumps(ents[:3])),
                check("ch394.silent: no answer in the next boot", si.get("ok") and pc.get("ok") and d1 is None, repr(d1)),
                check("the boot after it answers", pc2.get("ok") and d2 == b"after-silent", repr(d2)),
                check("kept over the power cycles: ECC word and erase time in the start-up script",
                      'AddEccText "0x080C0040" true' in script and "SetEraseUs 4000000" in script, script[-400:]),
                check("the corrupted byte survives power cycles and is restored", sc2.get("ok") and (sc2.get("value") or {}).get("restored") == 255, str(sc2)),
                check("flash.log read after the cycles", fl2.get("ok"), str(fl2)[:200])]
    if tid == "T-EMU-23":
        # format v1.8: flash.fault on real flash work (net_test "F" commands), a watchdog reset that
        # ends a never-ending erase, RCC_RSR, power.cut replaced by a later one, AXI SRAM kept over
        # resets and pseudo-random (not zero, not kept) at power-on
        h = ep["hosts"][0]
        sk = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sk.bind((h["bind"], h["ports"][0] + 8))

        def cmd(payload, wait=20.0, again=False):
            """One datagram, sent once (again: every 2 s, for waiting until the image is back)."""
            sk.setblocking(False)
            try:
                while True:
                    sk.recv(2048)
            except OSError:
                pass
            sk.settimeout(2.0)
            t0, sent = time.time(), False
            while time.time() - t0 < wait:
                if again or not sent:
                    sk.sendto(payload, addr(c))
                    sent = True
                try:
                    d = sk.recv(2048)
                except socket.timeout:
                    continue
                if d[:2] == payload[:2]:
                    return d
            return None

        def words(d):
            return (int.from_bytes(d[2:6], "big"), int.from_bytes(d[6:10], "big")) if d and len(d) >= 10 else (None, None)

        def log():
            return (ctl.op("flash.log", action="read").get("value") or {}).get("entries", [])
        FF, PGSERR, ERRS = 0xFFFFFFFF, 1 << 18, 0x1FE << 16
        out = []
        ctl.op("flash.timing", erase_us=200000)
        r0 = words(cmd(b"FR", again=True, wait=60.0))
        out.append(check("power-on: RCC_RSR 0x00E80000, 0 after RMVF", r0 == (0x00E80000, 0), str(r0)))
        m0 = words(cmd(b"FM"))
        out.append(check("AXI SRAM at power-on is neither 0 nor the marker", None not in m0 and all(x not in (0, 0x4B455054, 0x4B455055) for x in m0), str(m0)))
        ctl.op("flash.log", action="start")
        e1 = words(cmd(b"FE"))
        out.append(check("erase without a fault: no error flag, word erased", e1[0] is not None and not e1[0] & ERRS and e1[1] == FF, str(e1)))
        p0 = words(cmd(b"FP\x00"))
        out.append(check("program without a fault: content, no error flag", p0[0] is not None and not p0[0] & ERRS and p0[1] == 0xA5000000, str(p0)))
        f1 = ctl.op("flash.fault", op="program", after=1, flag="PGSERR")
        p1, p2, p3 = words(cmd(b"FP\x01")), words(cmd(b"FP\x02")), words(cmd(b"FP\x03"))
        out.append(check("flash.fault program after 1: the second word has PGSERR and stays erased, the others are written",
                         f1.get("ok") and not (p1[0] or 0) & ERRS and p1[1] == 0xA5000100 and (p2[0] or 0) & PGSERR and p2[1] == FF
                         and not (p3[0] or 0) & ERRS and p3[1] == 0xA5000300, str((p1, p2, p3))))
        f2 = ctl.op("flash.fault", op="erase", flag="PGSERR")
        e2 = words(cmd(b"FE"))
        e3 = words(cmd(b"FP\x04"))
        out.append(check("flash.fault erase: PGSERR, the sector keeps its content; the flag is cleared by CCR1",
                         f2.get("ok") and (e2[0] or 0) & PGSERR and e2[1] == 0xA5000000 and not (e3[0] or 0) & ERRS, str((e2, e3))))
        bad = [ctl.op("flash.fault", op="program", never_end=True), ctl.op("flash.fault", op="program", flag="PGSERR", never_end=True),
               ctl.op("flash.fault", op="erase"), ctl.op("flash.fault", op="erase", flag="NOFLAG"), ctl.op("flash.fault", op="mass", flag="PGSERR")]
        out.append(check("refused: never_end with program, no flag, unknown flag, unknown op", all(b.get("ok") is False for b in bad), str(bad)))
        L = log()
        ops = [(x["op"], x["result"]) for x in L]
        out.append(check("flash.log results", ops.count(("program", "error:PGSERR")) == 1 and ops.count(("erase", "error:PGSERR")) == 1
                         and ops.count(("erase", "ok")) == 1, str(ops)))
        # a never-ending erase (no flag), ended by the watchdog
        m1 = words(cmd(b"FM"))
        f3 = ctl.op("flash.fault", op="erase", never_end=True)
        w = cmd(b"FW")
        sk.sendto(b"FE", addr(c))                  # the watchdog (0.5 s) ends it: look at once
        during, t0 = [], time.time()
        while not during and time.time() - t0 < 5.0:
            L = log()
            during = [x for x in L if x["result"] == "running"]
            if any(x["op"] == "erase" and x["result"] == "reset" for x in L):
                break
            time.sleep(0.02)
        sk.settimeout(0.05)
        try:
            n = sk.recv(2048)
        except OSError:
            n = None
        back = words(cmd(b"FR", again=True, wait=90.0))
        ends = [x for x in log() if x["op"] == "erase"]
        out.append(check("never_end without a flag accepted; no answer during it; listed running with t_end_us null",
                         f3.get("ok") and w is not None and n is None and len(during) == 1 and during[0].get("t_end_us", 0) is None, str((f3, w, n, during))))
        out.append(check("the watchdog reset ends it: the image runs again, RCC_RSR IWDG1RSTF and PINRSTF only", back[0] == (1 << 26 | 1 << 22), str(back)))
        out.append(check("flash.log: that erase has result reset", ends and ends[-1]["result"] == "reset", str(ends[-1:])))
        out.append(check("flash.log: an erase a reset ends has no instruction count (null: Renode's count starts again at the reset)",
                         ends and "instructions" in ends[-1] and ends[-1]["instructions"] is None, str(ends[-1:])))
        m2 = words(cmd(b"FM"))
        out.append(check("AXI SRAM kept over the watchdog reset", m2 == (0x4B455054, 0x4B455055), "%s then %s" % (m1, m2)))
        e4 = words(cmd(b"FE"))
        out.append(check("the next erase works (BSY cleared by the reset, sector erased)", e4[0] is not None and not e4[0] & ERRS and e4[1] == FF, str(e4)))
        cmd(b"FX", wait=1.0)
        sw = words(cmd(b"FR", again=True, wait=60.0))
        m3 = words(cmd(b"FM"))
        out.append(check("software reset: RCC_RSR SFT1RSTF and PINRSTF only; AXI SRAM kept", sw[0] == (1 << 24 | 1 << 22) and m3 == (0x4B455054, 0x4B455055), str((sw, m3))))
        # a later power.cut replaces one still armed
        k1 = ctl.op("power.cut", after_flash_words=3)
        k2 = ctl.op("power.cut", during_erase=0.5)
        cmd(b"FP\x05"), cmd(b"FP\x06"), cmd(b"FP\x07")
        alive = cmd(b"FP\x08")
        cmd(b"FE", wait=1.0)
        m4 = words(cmd(b"FM", again=True, wait=90.0))
        cuts = [x for x in log() if x.get("result") == "cut"]
        out.append(check("power.cut replaced: four words programmed without a cut, then one cut at half of the erase",
                         k1.get("ok") and k2.get("ok") and alive is not None and len(cuts) == 1 and cuts[0]["op"] == "erase"
                         and 0.45 <= cuts[0].get("fraction_done", 0) <= 0.55, str(cuts)))
        out.append(check("AXI SRAM not kept over the power loss: the power-on pattern again", m4 == m0, "%s, at first power-on %s" % (m4, m0)))
        tn = ctl.op("flash.timing", erase_us=None)
        out.append(check("flash.timing erase_us null: back to the datasheet model", tn.get("ok") and (tn.get("value") or {}).get("erase_us", 0) is None, str(tn)))
        e5 = words(cmd(b"FE", wait=40.0))
        L2 = [x for x in log() if x["op"] == "erase" and x["result"] == "ok"]
        # the end is stamped at the emulator's next synchronisation point (100 us)
        out.append(check("that erase takes 1.1 s (x64), to one synchronisation quantum", e5[0] is not None and L2
                         and 1100000 <= L2[-1]["t_end_us"] - L2[-1]["t_us"] <= 1100200, str(L2[-1:])))
        return out
    if tid == "T-EMU-24":
        # own timers: the image (timmono_test) reads TIM12 and its tick-less time without a pause,
        # writes timer registers at any moment and checks every interrupt entry of the run
        ctl.op("console.log", action="start")
        rows, t0 = [], time.time()
        while len(rows) < 4 and time.time() - t0 < 150.0:
            time.sleep(2.0)
            ents = (ctl.op("console.log", action="read").get("value") or {}).get("entries", [])
            rows = [(e["t_us"], {m.group(1): int(m.group(2), 16) for m in re.finditer(r"(\w+) 0x([0-9A-Fa-f]+)", e.get("text", e.get("line", "")))})
                    for e in ents if "timmono t " in e.get("text", e.get("line", ""))]
        if len(rows) < 2:
            return [check("timmono lines on the console", False, "%d lines" % len(rows))]
        (ta, va), (tb, vb) = rows[0], rows[-1]
        drift = (vb["t"] - va["t"]) - (tb - ta)          # both in us
        span = tb - ta
        return [check("time and counter never step backwards", vb["time_back"] == 0 and vb["cnt_back"] == 0, str(vb)),
                check("no interrupt entry with CNT before its event (over the whole run)", vb["early"] == 0 and vb["upd"] + vb["half"] > 100, str(vb)),
                check("every boundary taken once (two per 65536 us)", abs((vb["upd"] + vb["half"]) - vb["t"] // 32768) <= 2, str(vb)),
                check("the counter keeps emulated time (a register write costs nothing): under 20 ppm or 60 us over the run",
                      abs(drift) <= max(60, span * 20e-6), "drift %d us over %d us" % (drift, span))]
    if tid == "T-EMU-25":
        # flash.log entries end at a reset and at a pause; an erase clears ECC marks (also over a
        # power cycle); net.endpoint gives the whole endpoint; net.log keeps the order of the
        # sends; a masked erase: CYCCNT, TIM5 and TIM12 agree
        h = ep["hosts"][0]
        sk = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sk.bind((h["bind"], h["ports"][0] + 9))

        def cmd(payload, wait=20.0, again=False):
            sk.setblocking(False)
            try:
                while True:
                    sk.recv(2048)
            except OSError:
                pass
            sk.settimeout(2.0)
            t0, sent = time.time(), False
            while time.time() - t0 < wait:
                if again or not sent:
                    sk.sendto(payload, addr(c))
                    sent = True
                try:
                    d = sk.recv(2048)
                except socket.timeout:
                    continue
                if d[:2] == payload[:2]:
                    return d
            return None

        def words(d):
            return tuple(int.from_bytes(d[i:i + 4], "big") for i in range(2, len(d) - 3, 4)) if d else ()

        def log():
            return (ctl.op("flash.log", action="read").get("value") or {}).get("entries", [])

        def restart(out):
            # a new emulator: the next test starts with an image in step and no fault armed (there is no other way to drop one)
            ctl.op("flash.timing", erase_us=None)
            ctl.op("power.cycle")
            cmd(b"FR", again=True, wait=90.0)
            return out

        def answer(head, s):
            sk.settimeout(s)
            try:
                d = sk.recv(2048)
            except socket.timeout:
                return None
            return d if head and d[:2] == head else None
        DBECC, out = 1 << 26, []
        ctl.op("flash.timing", erase_us=200000)
        cmd(b"FR", again=True, wait=60.0)
        # net.endpoint is whole
        n = (ctl.op("net.endpoint").get("value") or {})
        out.append(check("net.endpoint: minor, flash_timing, usart1_timed, time_scale as in the endpoint",
                         all(k in n and n[k] == ep.get(k) for k in ("minor", "flash_timing", "usart1_timed", "time_scale")),
                         str({k: n.get(k) for k in ("minor", "usart1_timed", "time_scale")})))
        # flash.log: back-to-back words are one entry; a pause and a reset end it
        ctl.op("flash.log", action="start")
        cmd(b"FE")
        q = words(cmd(b"FQ\x00"))
        time.sleep(0.3)
        cmd(b"FP\x02")
        cmd(b"FX", wait=1.0)
        cmd(b"FR", again=True, wait=60.0)
        cmd(b"FP\x03")
        pg = [(int(x["address"], 16) - 0x080E0000) // 32 for x in log() if x["op"] == "program"]
        pw = [x["words"] for x in log() if x["op"] == "program"]
        out.append(check("two words back to back: one entry; a word after a pause: its own; a word after a reset: its own",
                         pg == [0, 2, 3] and pw == [2, 1, 1], "first words %s, word counts %s, FQ %s" % (pg, pw, q)))
        lt = [x for x in log() if x["op"] == "program"]
        out.append(check("t_last_us in the program entries", all(x.get("t_last_us", -1) >= x["t_us"] for x in lt), str(lt[:1])))
        # an erase clears ECC marks, also in what a power cycle carries over
        cmd(b"FE"); cmd(b"FP\x03"); cmd(b"FP\x05")
        ctl.op("flash.ecc", address=0x080E0000 + 32 * 5, type="double")
        d1 = words(cmd(b"FD\x05"))
        ctl.op("power.cycle")
        cmd(b"FR", again=True, wait=90.0)
        d2 = words(cmd(b"FD\x05"))
        cmd(b"FE"); cmd(b"FP\x05")
        d3 = words(cmd(b"FD\x05"))
        ctl.op("power.cycle")
        cmd(b"FR", again=True, wait=90.0)
        d4 = words(cmd(b"FD\x05"))
        out.append(check("flash.ecc: the double error shows, also after a power cycle",
                         d1 and d1[0] & DBECC and d2 and d2[0] & DBECC, str((d1, d2))))
        out.append(check("erase and program again: no error, also after a power cycle (the mark is not carried over)",
                         d3 and not d3[0] & DBECC and d3[1] == 0xA5000500 and d4 and not d4[0] & DBECC, str((d3, d4))))
        # net.log keeps the order of the sends
        ctl.op("net.log", action="start")
        sk.settimeout(3.0)
        sent = [bytes([0x61 + i]) * (8 + i) for i in range(6)]
        for x in sent:
            sk.sendto(x, addr(c))
        got, t0 = [], time.time()
        while len(got) < len(sent) and time.time() - t0 < 15.0:
            try:
                got.append(sk.recv(2048))
            except socket.timeout:
                break
        ents = (ctl.op("net.log", action="read").get("value") or {}).get("entries", [])
        tx = [(e["t_us"], e["head"]) for e in ents if e.get("dir") == "from_radio" and e.get("radio_port") == 1024]
        # the radio's SEND order is the order it received the datagrams in (one loop, socket 0)
        rx = [e["head"] for e in ents if e.get("dir") == "to_radio" and e.get("radio_port") == 1024]
        out.append(check("net.log: sends in order, t_us never going back, in the order received", len(tx) == len(sent)
                         and [x[0] for x in tx] == sorted(x[0] for x in tx) and [x[1] for x in tx] == rx[:len(tx)], "%d sends, %d received back" % (len(tx), len(got))))
        # masked erases: cycles (64 MHz) against TIM5 and TIM12
        for us, kind in ((4000000, "4 s"), (8000000, "8 s"), (4000000, "4 s with PGSERR")):
            ctl.op("flash.timing", erase_us=us)
            if "PGSERR" in kind:
                ctl.op("flash.fault", op="erase", flag="PGSERR")
            while answer(b"FC", 0.01) is not None:      # nothing old on the socket
                pass
            er = wait_erase(lambda: sk.sendto(b"FC", addr(c)), lambda s: answer(b"FC", s), log)
            if line(kind, er):
                NOTES.append(line(kind, er))
            out.append(check("masked erase %s: it ended in flash.log, the image answered" % kind, er["ok"],
                             "%s; host %.1f s, load %.1f to %.1f" % (er["why"], er["host_s"], er["load_start"], er["load_max"])))
            if not er["ok"]:
                # out of step with the image: nothing after this would be what it says it is
                return restart(out)
            w = words(er["answer"])
            ok = len(w) == 6
            cyc, t5, t12, worst = (w[1] / 64.0, w[2], w[3], w[4]) if ok else (0, 0, 0, 0)
            out.append(check("masked erase %s: cycles/64, TIM5 and TIM12 within 20 us of each other, of the length and between any two reads" % kind,
                             ok and abs(cyc - t5) <= 20 and abs(t12 - t5) <= 20 and abs(t5 - us) <= 20 and worst <= 20 and bool(w[0] & (1 << 18)) == ("PGSERR" in kind),
                             "cycles/64 %.1f, TIM5 %d, TIM12 %d, worst between reads %d us, polls %d" % (cyc, t5, t12, worst, w[5] if ok else 0)))
        # a failing erase that never ends, ended by the watchdog: the figures up to the last poll
        # only with no erase running: an armed fault takes whatever erase comes next
        open_ = [x for x in erases(log()) if x.get("result") == "running"]
        wdt = None
        if not open_:
            # the fault before the watchdog: its 0.5 s would run out on a loaded host otherwise
            ctl.op("flash.fault", op="erase", never_end=True)
            wdt = cmd(b"FW")
        out.append(check("no erase runs before never_end is armed; the watchdog command is answered", not open_ and wdt is not None,
                         "running %s, FW %s" % (open_, wdt)))
        if open_ or wdt is None:
            return restart(out)
        sk.sendto(b"FC", addr(c))
        cmd(b"FR", again=True, wait=90.0)
        w = words(cmd(b"FV"))
        ok = len(w) == 6
        cyc, t5, t12, worst = (w[1] / 64.0, w[2], w[3], w[4]) if ok else (0, 0, 0, 0)
        out.append(check("a never-ending erase ended by the watchdog: the figures at the last poll agree within 20 us, 0.2 to 0.7 s in",
                         ok and abs(cyc - t5) <= 20 and abs(t12 - t5) <= 20 and worst <= 20 and 200000 <= t5 <= 700000,
                         "cycles/64 %.1f, TIM5 %d, TIM12 %d, worst %d us" % (cyc, t5, t12, worst)))
        ctl.op("flash.timing", erase_us=None)
        return out
    if tid == "T-EMU-26":
        # ECC marks: a program, then a read of a marked word at once; a mark added while the
        # watchpoints are lifted; the first read after a power cut is covered. (These sequences pass
        # on the layer of 5960f2a too: the crash reproducer is ecc-crash.json, m1_smoke [10].)
        h = ep["hosts"][0]
        sk = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sk.bind((h["bind"], h["ports"][0] + 10))

        def cmd(payload, wait=20.0, again=False):
            sk.setblocking(False)
            try:
                while True:
                    sk.recv(2048)
            except OSError:
                pass
            sk.settimeout(2.0)
            t0, sent = time.time(), False
            while time.time() - t0 < wait:
                if again or not sent:
                    sk.sendto(payload, addr(c))
                    sent = True
                try:
                    d = sk.recv(2048)
                except socket.timeout:
                    continue
                if d[:2] == payload[:2]:
                    return d
            return None

        def words(d):
            return tuple(int.from_bytes(d[i:i + 4], "big") for i in range(2, len(d) - 3, 4)) if d else ()
        DBECC, out = 1 << 26, []
        ctl.op("flash.timing", erase_us=200000)
        cmd(b"FR", again=True, wait=60.0)
        cmd(b"FE"); cmd(b"FP\x05")
        ctl.op("flash.ecc", address=0x080C0040, type="double")     # a mark elsewhere, carried on
        ctl.op("flash.ecc", address=0x080E0000 + 32 * 5, type="double")
        d0 = words(cmd(b"FD\x05"))
        out.append(check("control: the marked word shows the double error", d0 and d0[0] & DBECC, str(d0)))
        # flash.ecc answers when the mark is in force (in_force): two marks back to back, then a read
        # of the second one, fresh words, all marks erased every 4 pairs
        force, hit = [], []
        for k in range(24):
            for w in (40 + 2 * (k % 4), 41 + 2 * (k % 4)):
                force.append((ctl.op("flash.ecc", address=0x080E0000 + 32 * w, type="double").get("value") or {}).get("in_force"))
            hit.append(bool((words(cmd(b"FD" + bytes([41 + 2 * (k % 4)]))) or (0,))[0] & DBECC))
            if k % 4 == 3:
                cmd(b"FE")
        cmd(b"FP\x05")
        ctl.op("flash.ecc", address=0x080E0000 + 32 * 5, type="double")
        out.append(check("flash.ecc: in force in each answer, the read at once after the second of two shows the error, 24 pairs",
                         force == [True] * 48 and hit == [True] * 24, "in force %d of 48, shown %d of 24" % (sum(x is True for x in force), sum(hit))))
        for n in range(3):
            # words 6, 8 and 10 are programmed next to the marked words 5, 7 and 9
            if n:
                cmd(b"FP" + bytes([5 + 2 * n]))
                ctl.op("flash.ecc", address=0x080E0000 + 32 * (5 + 2 * n), type="double")
            g = words(cmd(b"FG" + bytes([5 + 2 * n])))
            out.append(check("program, then read of the marked word at once: the error shows some us later (%d)" % (n + 1),
                             len(g) == 2 and g[1] & DBECC, "first read (inside the microsecond) SR1 %s, second %s" % tuple("0x%08X" % x for x in g) if len(g) == 2 else "no answer"))
        # a mark carried over a power cut, programs, a mark added while the watchpoints are lifted,
        # then a command that writes FLASH_CCR1 and reads the marked word
        ctl.op("power.cut", during_erase=0.5)
        cmd(b"FE", wait=1.0)
        cmd(b"FR", again=True, wait=90.0)
        alive = []
        for n in range(4):
            cmd(b"FE"); cmd(b"FQ\x00"); time.sleep(0.3); cmd(b"FP\x02")
            cmd(b"FX", wait=1.0)
            cmd(b"FR", again=True, wait=60.0)
            cmd(b"FE"); cmd(b"FP\x03"); cmd(b"FP\x05")
            ctl.op("flash.ecc", address=0x080E0000 + 32 * 5, type="double")
            g = words(cmd(b"FD\x05", wait=10.0))
            alive.append(bool(g and g[0] & DBECC))
        out.append(check("a mark added while the watchpoints are lifted, then read by a command at once: the error shows (4 times)",
                         alive == [True] * 4, str(alive)))
        # a power cut that tears word 9 (fresh sector), read by the first code of the next start
        cmd(b"FE")
        k = ctl.op("power.cut", after_flash_words=1)
        cmd(b"FP\x09", wait=1.0)
        cmd(b"FR", again=True, wait=90.0)
        f = words(cmd(b"FS"))
        cuts = [x for x in (ctl.op("flash.log", action="read").get("value") or {}).get("entries", []) if x.get("result") == "cut"]
        out.append(check("a torn word of a power cut: the first read of the next start shows the double error",
                         k.get("ok") and len(f) == 2 and f[1] & DBECC and any(x.get("torn") == "0x080E0120" for x in cuts), "%s %s" % (f, cuts[-1:])))
        ctl.op("flash.timing", erase_us=None)
        return out
    if tid == "T-EMU-27":
        # format v1.9: flash.dump, flash.fill, reset.flags, log.corrupt; a cut armed over a reset
        h = ep["hosts"][0]
        sk = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sk.bind((h["bind"], h["ports"][0] + 11))

        def cmd(payload, wait=20.0, again=False):
            sk.setblocking(False)
            try:
                while True:
                    sk.recv(2048)
            except OSError:
                pass
            sk.settimeout(2.0)
            t0, sent = time.time(), False
            while time.time() - t0 < wait:
                if again or not sent:
                    sk.sendto(payload, addr(c))
                    sent = True
                try:
                    d = sk.recv(2048)
                except socket.timeout:
                    continue
                if d[:2] == payload[:2]:
                    return d
            return None

        def words(d):
            return tuple(int.from_bytes(d[i:i + 4], "big") for i in range(2, len(d) - 3, 4)) if d else ()

        def dump(a, n):
            r = ctl.op("flash.dump", address=a, len=n)
            return bytes.fromhex((r.get("value") or {}).get("hex", "")) if r.get("ok") else r

        def log():
            return (ctl.op("flash.log", action="read").get("value") or {}).get("entries", [])
        S7, DBECC, out = 0x080E0000, 1 << 26, []
        ctl.op("flash.timing", erase_us=200000)
        cmd(b"FR", again=True, wait=60.0)
        ctl.op("flash.log", action="start")
        cmd(b"FE"); cmd(b"FP\x05")
        # flash.dump: what is there, and no ECC event from a dump of a marked word
        ctl.op("flash.ecc", address=S7 + 32 * 5, type="double")
        n0 = len([x for x in log() if x["op"] == "ecc"])
        d = dump(S7 + 32 * 5, 32)
        n1 = len([x for x in log() if x["op"] == "ecc"])
        w = words(cmd(b"FD\x05"))
        out.append(check("flash.dump: the programmed word; a dump of a marked word gives no ECC entry, a read by the image does",
                         d == b"".join((0xA5000500 + i).to_bytes(4, "little") for i in range(8)) and n0 == n1 and w and w[0] & DBECC, str((d, n0, n1, w))[:200]))
        bad = [ctl.op("flash.dump", address=0x08010000, len=32), ctl.op("flash.dump", address=S7, len=32769),
               ctl.op("flash.dump", address=0x080FFFE0, len=64), ctl.op("flash.dump", address=S7, len=0)]
        big = dump(S7, 32768)
        out.append(check("flash.dump refused: below 0x08020000, over 32768, past the bank, empty; 32768 given", all(b.get("ok") is False for b in bad)
                         and isinstance(big, bytes) and len(big) == 32768, str(bad)[:200]))
        # flash.fill
        ctl.op("flash.ecc", address=S7 + 32 * 7, type="double")
        blk = bytes(range(64)) * 1
        f1 = ctl.op("flash.fill", address=S7, len=131072, data=blk.hex())
        ok1 = dump(S7, 4096) == blk * 64 and dump(S7 + 0x1FFC0, 64) == blk
        f2 = ctl.op("flash.fill", address=S7 + 0x1000, len=4096)
        ok2 = dump(S7 + 0x1000, 4096) == b"\xff" * 4096 and dump(S7 + 0x2000, 64) == blk
        bd = [ctl.op("flash.fill", address=0x080BFFE0, len=32), ctl.op("flash.fill", address=0x080FFFE0, len=64),
              ctl.op("flash.fill", address=S7 + 4, len=32), ctl.op("flash.fill", address=S7, len=40),
              ctl.op("flash.fill", address=S7, len=64, data="aabbcc"), ctl.op("flash.fill", address=S7, len=32, data="xyz")]
        out.append(check("flash.fill: data repeated, erased without data, refused outside the store sectors, misaligned, data not dividing len, not hex",
                         f1.get("ok") and ok1 and f2.get("ok") and ok2 and all(b.get("ok") is False for b in bd), str((f1, f2, ok1, ok2, bd))[:300]))
        e7 = words(cmd(b"FD\x07"))
        ctl.op("power.cycle")
        cmd(b"FR", again=True, wait=90.0)
        e7b = words(cmd(b"FD\x07"))
        kept = dump(S7, 64) == blk and dump(S7 + 0x1000, 64) == b"\xff" * 64
        out.append(check("flash.fill: an earlier mark is gone, the content is kept over a power cycle, no mark comes back",
                         e7 and not e7[0] & DBECC and e7b and not e7b[0] & DBECC and kept, str((e7, e7b, kept))))
        # a fill erases no entry of flash.log, and the next program entry is the image's own word
        cmd(b"FE"); ctl.op("flash.fill", address=S7, len=4096, data=bytes(32).hex())
        ctl.op("flash.fill", address=S7, len=4096)
        ctl.op("flash.log", action="start")
        cmd(b"FP\x09")
        L = log()
        out.append(check("flash.fill leaves no flash.log entry; the next program is one word at its own address",
                         [(x["op"], x.get("address"), x.get("words")) for x in L] == [("program", "0x080E0120", 1)], str(L)[:300]))
        # refused while an erase of the image runs
        ctl.op("flash.timing", erase_us=2000000)
        sk.sendto(b"FE", addr(c))
        fr, t0 = None, time.time()
        while time.time() - t0 < 10.0:
            if [x for x in log() if x["result"] == "running"]:
                fr = ctl.op("flash.fill", address=S7, len=32)
                break
            time.sleep(0.02)
        cmd(b"FR", again=True, wait=60.0)
        ctl.op("flash.timing", erase_us=200000)
        out.append(check("flash.fill refused while an erase runs", fr is not None and fr.get("ok") is False and "flash operation" in fr.get("error", ""), str(fr)))
        # reset.flags
        m0 = words(cmd(b"FM"))
        m1 = words(cmd(b"FM"))
        res = []
        for flags, want in (([], 0), (["software", "power-on"], 1 << 24 | 1 << 23), (["watchdog", "pin", "brown-out", "window-watchdog", "low-power"], 1 << 26 | 1 << 22 | 1 << 21 | 1 << 28 | 1 << 30)):
            rf = ctl.op("reset.flags", flags=flags)
            cmd(b"FX", wait=1.0)
            v = words(cmd(b"FR", again=True, wait=60.0))
            res.append((flags, rf.get("ok"), v[0] if v else None, v and v[0] == want))
        m2 = words(cmd(b"FM"))
        unk = ctl.op("reset.flags", flags=["cpu"])
        out.append(check("reset.flags: the next reset shows exactly the named flags, RAM kept, the reset after it its own, an unknown name refused",
                         all(x[1] and x[3] for x in res) and m2 == (0x4B455054, 0x4B455055) and unk.get("ok") is False, str((res, m1, m2, unk))[:300]))
        cmd(b"FX", wait=1.0)
        own = words(cmd(b"FR", again=True, wait=60.0))
        out.append(check("the reset after it has its own cause again", own and own[0] == (1 << 24 | 1 << 22), str(own)))
        # log.corrupt: on the ring the image wrote, at the next MCU reset, once; dropped by a power cycle
        def again():
            cmd(b"FX", wait=1.0)
            cmd(b"FR", again=True, wait=60.0)
        offered = "log.corrupt" in ep.get("fixtures", [])
        n0 = words(cmd(b"FL\x03"))
        lc = ctl.op("log.corrupt", seq=3)
        n1 = words(cmd(b"FK\x03"))
        no = ctl.op("log.corrupt", seq=4)
        again()
        n2 = words(cmd(b"FK\x03"))
        again()
        n3 = words(cmd(b"FK\x03"))
        E0, E1 = (3, 0x41424344), (3, 0x41424345)
        out.append(check("log.corrupt (offered with the runner's ring setting): nothing changes while the image runs; after the next reset one text bit of the entry is changed, its number kept; not again at the reset after it; an entry not there refused; the address not in the endpoint",
                         offered and (n0, n1, n2, n3) == (E0, E0, E1, E1) and lc.get("ok") and no.get("ok") is False
                         and "2401e700" not in json.dumps(ep).lower(), str((offered, n0, lc, n1, n2, n3, no))[:300]))
        cmd(b"FL\x03")
        lo = ctl.op("log.corrupt", seq=3)
        w0 = words(cmd(b"FL\x43"))
        again()
        w1 = words(cmd(b"FK\x03"))
        out.append(check("log.corrupt: an entry the image overwrote before the reset is not changed", lo.get("ok") and w0 == w1 == (67, 0x41424344), str((lo, w0, w1))))
        cmd(b"FL\x03")
        lp = ctl.op("log.corrupt", seq=3)
        pw = ctl.op("power.cycle")
        cmd(b"FR", again=True, wait=90.0)
        cmd(b"FL\x03")
        again()
        p1 = words(cmd(b"FK\x03"))
        out.append(check("log.corrupt: a power cycle drops it", lp.get("ok") and pw.get("ok") and p1 == E0, str((lp, pw, p1))))
        # a cut armed before a reset hits the first erase after it, and flash.log names its sector
        ctl.op("flash.log", action="start")
        ctl.op("power.cut", during_erase=0.5)
        cmd(b"FX", wait=1.0)
        cmd(b"FR", again=True, wait=60.0)
        cmd(b"FE", wait=1.0)
        cmd(b"FR", again=True, wait=90.0)
        cuts = [x for x in log() if x.get("result") == "cut"]
        out.append(check("a cut armed before a software reset hits the first erase after it; its entry names the sector",
                         len(cuts) == 1 and cuts[0]["op"] == "erase" and cuts[0]["sector"] == 7 and 0.45 <= cuts[0]["fraction_done"] <= 0.55, str(cuts)))
        ctl.op("flash.timing", erase_us=None)
        return out
    if tid == "T-EMU-28":
        # the state at a cut is the state at that instant, also when the runner is slow after it
        # (run with G7250_TEST_CUT_DELAY_S; without it the checks say little)
        h = ep["hosts"][0]
        sk = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sk.bind((h["bind"], h["ports"][0] + 13))

        def drain():
            sk.setblocking(False)
            n = 0
            try:
                while True:
                    sk.recv(2048)
                    n += 1
            except OSError:
                return n

        def cmd(payload, wait=20.0, again=False):
            drain()
            sk.settimeout(2.0)
            t0, sent = time.time(), False
            while time.time() - t0 < wait:
                if again or not sent:
                    sk.sendto(payload, addr(c))
                    sent = True
                try:
                    d = sk.recv(2048)
                except socket.timeout:
                    continue
                if d[:2] == payload[:2]:
                    return d
            return None
        S7, N, out = 0x080E0000, 131072, []
        old = b"".join((0x5A000000 + i).to_bytes(4, "little") for i in range(N // 4))

        def dump():
            b = b""
            for o in range(0, N, 32768):
                r = ctl.op("flash.dump", address=S7 + o, len=32768)
                b += bytes.fromhex((r.get("value") or {}).get("hex", ""))
            return b

        def prepare(erase_us):
            ctl.op("flash.timing", erase_us=erase_us)
            cmd(b"FR", again=True, wait=90.0)
            f = ctl.op("flash.fill", address=S7, len=4096, data=old[:4096].hex())
            for o in range(4096, N, 4096):
                ctl.op("flash.fill", address=S7 + o, len=4096, data=old[o:o + 4096].hex())
            ctl.op("flash.log", action="start")
            return f.get("ok") and dump() == old

        def verdict(name, ready, lo, hi, quiet=True):
            e = [x for x in (ctl.op("flash.log", action="read").get("value") or {}).get("entries", []) if x.get("op") == "erase"]
            b = dump()
            f = e[0].get("fraction_done", -1) if e else -1
            edge = int(N * f) & ~31 if f >= 0 else -1
            near = any(b[:k] == b"\xff" * k and b[k:] == old[k:] for k in (edge - 32, edge, edge + 32) if 0 <= k <= N)
            kept = next((i for i in range(0, N, 32) if b[i:i + 32] != b"\xff" * 32), N)
            out.append(check(name, ready and len(e) == 1 and e[0]["result"] == "cut" and lo <= f <= hi and near and quiet,
                             "ready %s, entries %s, first word not 0xFF at fraction %.4f, recorded %.4f, quiet %s" % (
                                 ready, [(x.get("result"), x.get("fraction_done")) for x in e], kept / N, f, quiet)))
        # 1. a cut during an erase, the runner slow after it
        ok = prepare(200000)
        ctl.op("power.cut", during_erase=0.5)
        cmd(b"FE", wait=1.0)
        cmd(b"FR", again=True, wait=90.0)
        verdict("a cut at half of an erase, the runner slow after it: the entry is cut, the sector is erased to that word and kept beyond it", ok, 0.49, 0.51)
        # 2. the watchdog running over the cut (0.5 s, the cut at 0.2 s of the erase)
        ok = prepare(400000)
        ctl.op("power.cut", during_erase=0.5)
        fw = cmd(b"FW")
        cmd(b"FE", wait=1.0)
        cmd(b"FR", again=True, wait=90.0)
        verdict("the watchdog runs over a cut: no reset after it, the entry is cut, the content as at the cut", ok and fw is not None, 0.49, 0.51)
        # 3. a timed cut while the core is stalled by a fetch during the erase: the core does not run again
        ok = prepare(2000000)
        t = (ctl.op("hw.observe").get("value") or {}).get("t_us")
        drain()
        k = ctl.op("power.cut", at_us=(t or 0) + 500000)
        sk.sendto(b"FE", addr(c))
        sk.settimeout(8.0)
        try:
            late = sk.recv(2048)
        except socket.timeout:
            late = None
        cmd(b"FR", again=True, wait=90.0)
        verdict("a timed cut during a fetch stall: no answer of the image after it, the entry is cut, the content as at the cut",
                ok and t is not None and k.get("ok"), 0.1, 0.4, quiet=late is None)
        # 4. a power cycle during an erase: the moment of its halt (emulated time runs fast while the core is
        # stalled, so the erase is long)
        ok = prepare(40000000)
        drain()
        sk.sendto(b"FE", addr(c))
        time.sleep(0.3)
        p = ctl.op("power.cycle")
        n = drain()
        cmd(b"FR", again=True, wait=90.0)
        verdict("a power cycle during an erase: the entry is cut at the halt, the content as then", ok and p.get("ok"), 0.005, 0.95, quiet=n == 0)
        # 5. a cut after a flash word: the next word, programmed back to back by the image, is not programmed
        ctl.op("flash.timing", erase_us=200000)
        cmd(b"FR", again=True, wait=90.0)
        cmd(b"FE")
        ctl.op("flash.log", action="start")
        ctl.op("power.cut", after_flash_words=1)
        cmd(b"FQ\x00", wait=1.0)
        cmd(b"FR", again=True, wait=90.0)
        e = [x for x in (ctl.op("flash.log", action="read").get("value") or {}).get("entries", []) if x.get("op") == "program"]
        b = dump()
        out.append(check("a cut after the first of two words written back to back: the second is not programmed, the log has one word",
                         len(e) == 1 and e[0]["result"] == "cut" and e[0]["words"] == 1 and e[0].get("torn") == "0x080E0000" and b[32:64] == b"\xff" * 32,
                         "%s second word %s" % (e, b[32:40].hex())))
        ctl.op("flash.timing", erase_us=None)
        return out
    if tid == "T-EMU-29":
        # cpu.fault (0075, way (a)): an undefined instruction at a chosen emulated time, in thread mode,
        # in a chosen interrupt handler and with UsageFault enabled; the image keeps the frame it saw
        h = ep["hosts"][0]
        sk = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sk.bind((h["bind"], h["ports"][0] + 14))

        def cmd(payload, wait=20.0, again=False):
            sk.setblocking(False)
            try:
                while True:
                    sk.recv(2048)
            except OSError:
                pass
            sk.settimeout(1.0)
            t0, sent = time.time(), False
            while time.time() - t0 < wait:
                if again or not sent:
                    sk.sendto(payload, addr(c))
                    sent = True
                try:
                    d = sk.recv(2048)
                except socket.timeout:
                    continue
                if d[:2] == payload[:2]:
                    return d
            return None

        def kept():
            d = cmd(b"FF", again=True, wait=60.0)
            return None if d is None or len(d) < 34 else [int.from_bytes(d[2 + 4 * i:6 + 4 * i], "big") for i in range(8)]

        def after_fault(n0, wait=60.0):
            t0 = time.time()
            while time.time() - t0 < wait:
                k = kept()
                if k and k[0] != n0:
                    return k
                time.sleep(0.5)
            return None

        def read():
            return ctl.op("cpu.fault", read=True).get("value") or {}

        out, STUB = [], 0x0FFF0000
        k0 = kept()
        n0 = k0[0] if k0 else None
        # 1. thread mode
        a = ctl.op("cpu.fault", in_us=20000, mode="thread")
        k = after_fault(n0)
        r = read()
        e, inj = r.get("entry") or {}, r.get("injected") or {}
        out.append(check("thread mode: armed, injected in thread mode, HardFault entered (HFSR FORCED, CFSR UNDEFINSTR), stacked PC the stub, stacked IPSR 0",
                         a.get("ok") and r.get("state") == "entered" and inj.get("ipsr") == 0 and e.get("exception") == 3
                         and int(e.get("hfsr", "0"), 16) >> 30 & 1 and int(e.get("cfsr", "0"), 16) == 0x10000
                         and int(e.get("stacked_pc", "0"), 16) == STUB and e.get("stacked_ipsr") == 0, "%s %s" % (a, r)))
        out.append(check("thread mode: the vector entered within 20 us of emulated time of the injection",
                         e.get("t_us") is not None and inj.get("t_us") is not None and 0 <= e["t_us"] - inj["t_us"] <= 20,
                         "injected %s, entry %s" % (inj.get("t_us"), e.get("t_us"))))
        out.append(check("thread mode: the image's own handler saw the same (IPSR 3, CFSR 0x10000, HFSR FORCED, stacked PC the stub, IPSR 0, EXC_RETURN thread MSP)",
                         k is not None and k[7] == 0xFA0170FF and k[1] == 0x10000 and k[2] >> 30 & 1 and k[3] == 3 and k[4] == STUB
                         and k[5] == 0 and k[6] == 0xFFFFFFF9, str([hex(x) for x in k] if k else None)))
        rsr = cmd(b"FR", again=True, wait=30.0)
        out.append(check("thread mode: the image's reset after it is a software reset (RCC_RSR SFTRSTF)",
                         rsr is not None and int.from_bytes(rsr[2:6], "big") >> 24 & 1, rsr[2:6].hex() if rsr else None))
        # 2. in the handler of exception 44 (TIM2), pended by the image; the mode is sampled every 10 us
        n0 = k[0] if k else n0
        a = ctl.op("cpu.fault", in_us=0, mode="handler", exception=44, within_us=3000000)
        cmd(b"FH\x32", wait=1.0)
        k = after_fault(n0)
        r = read()
        e, inj = r.get("entry") or {}, r.get("injected") or {}
        out.append(check("handler mode, exception 44: injected there, HardFault entered, stacked IPSR 44, the image saw it as well (EXC_RETURN handler)",
                         a.get("ok") and r.get("state") == "entered" and inj.get("ipsr") == 44 and e.get("exception") == 3
                         and e.get("stacked_ipsr") == 44 and k is not None and k[5] == 44 and k[3] == 3 and k[6] == 0xFFFFFFF1,
                         "%s %s" % (r, [hex(x) for x in k] if k else None)))
        # 3. UsageFault enabled by the image: the UDF enters UsageFault, not HardFault
        n0 = k[0] if k else n0
        u = cmd(b"FU", again=True, wait=30.0)
        a = ctl.op("cpu.fault", in_us=20000, mode="thread")
        k = after_fault(n0)
        r = read()
        e = r.get("entry") or {}
        out.append(check("with USGFAULTENA set: UsageFault (exception 6) entered, CFSR UNDEFINSTR, HFSR not FORCED; the image saw IPSR 6",
                         u is not None and r.get("state") == "entered" and e.get("exception") == 6 and int(e.get("cfsr", "0"), 16) == 0x10000
                         and not int(e.get("hfsr", "0"), 16) >> 30 & 1 and k is not None and k[3] == 6, "%s %s" % (r, [hex(x) for x in k] if k else None)))
        # 4. a handler that does not run within the window: missed, no fault
        n0 = k[0] if k else n0
        a = ctl.op("cpu.fault", in_us=0, mode="handler", exception=44, within_us=20000)
        w = ctl.op("time.wait", us=200000)
        r = read()
        k = kept()
        out.append(check("a handler that does not run within within_us: missed, no fault taken",
                         a.get("ok") and r.get("state") == "missed" and r.get("samples", 0) >= 1000 and k is not None and k[0] == n0,
                         "%s %s wait %s" % (r, k[0] if k else None, w)))
        # 5. a reset before the time: dropped
        a = ctl.op("cpu.fault", in_us=5000000, mode="any")
        cmd(b"FX", wait=1.0)
        time.sleep(3.0)
        cmd(b"FR", again=True, wait=30.0)
        r = read()
        out.append(check("an MCU reset before the fault's time: dropped, no fault taken",
                         a.get("ok") and r.get("state") == "dropped" and (kept() or [None])[0] == n0, str(r)))
        # 6. refusals
        b1 = ctl.op("cpu.fault", in_us=10, mode="thread", exception=44)
        b2 = ctl.op("cpu.fault", mode="any")
        out.append(check("refused: an exception number with mode thread, and neither at_us nor in_us",
                         b1.get("ok") is False and b2.get("ok") is False, "%s %s" % (b1, b2)))
        return out
    if tid == "T-EMU-30":
        # the diagnostic flash read log (off by default; run with G7250_FLASH_READLOG=0x080E0000-0x080E0FFF):
        # reads of the range with their values, SR1 as the core reads it, CCR1 and CR1 writes, CR1 at
        # power-on and after a reset, the watchpoints lifted while PG is set
        h = ep["hosts"][0]
        sk = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sk.bind((h["bind"], h["ports"][0] + 15))

        def cmd(payload, wait=20.0, again=False):
            sk.setblocking(False)
            try:
                while True:
                    sk.recv(2048)
            except OSError:
                pass
            sk.settimeout(1.0)
            t0, sent = time.time(), False
            while time.time() - t0 < wait:
                if again or not sent:
                    sk.sendto(payload, addr(c))
                    sent = True
                try:
                    d = sk.recv(2048)
                except socket.timeout:
                    continue
                if d[:2] == payload[:2]:
                    return d
            return None

        def words(d):
            return tuple(int.from_bytes(d[i:i + 4], "big") for i in range(2, len(d) - 3, 4)) if d else ()

        def rl(n):
            p = os.path.join(os.path.dirname(os.path.abspath(EP_PATH)), "flash-reads-%d.jsonl" % n)
            try:
                return [json.loads(x) for x in open(p) if x.strip()]
            except OSError:
                return []

        DBECC, S7 = 1 << 26, 0x080E0000
        cmd(b"FR", again=True, wait=60.0)
        cmd(b"FE", wait=60.0)
        cmd(b"FP\x03")
        cmd(b"FP\x05")
        d3 = words(cmd(b"FD\x03"))
        ctl.op("flash.ecc", address=S7 + 32 * 5, type="double")
        d5 = words(cmd(b"FD\x05"))
        bb = words(cmd(b"FB\x05\x0b"))               # a byte read inside the marked word
        sk.sendto(b"FX", addr(c))                      # software reset, no reply
        time.sleep(1.0)
        cmd(b"FR", again=True, wait=60.0)
        ab = words(cmd(b"FB\x05\x0b"))               # the marked word after the reset, before any program
        ctl.op("power.cycle")
        cmd(b"FR", again=True, wait=90.0)
        # run with G7250_FLASH_READLOG_BOOTS=1-2: a third power-on, outside the window, writes no log
        ctl.op("power.cycle")
        b3 = cmd(b"FR", again=True, wait=90.0)
        tdir = os.path.dirname(os.path.abspath(EP_PATH))
        r1, r2 = rl(1), rl(2)

        def first(es, i0, f):
            for i in range(i0, len(es)):
                if f(es[i]):
                    return i
            return -1
        i3 = first(r1, 0, lambda e: e.get("op") == "read" and e.get("address") == "0x%08X" % (S7 + 96) and e.get("width") == 4)
        s3 = first(r1, i3 + 1 if i3 >= 0 else len(r1), lambda e: e.get("op") == "sr1")
        c3 = [e for e in r1[:i3 if i3 >= 0 else 0] if e.get("op") == "ccr1"]
        i5 = first(r1, 0, lambda e: e.get("op") == "read" and e.get("address") == "0x%08X" % (S7 + 160) and e.get("width") == 4)
        s5 = first(r1, i5 + 1 if i5 >= 0 else len(r1), lambda e: e.get("op") == "sr1")
        ib = first(r1, 0, lambda e: e.get("op") == "read" and e.get("address") == "0x%08X" % (S7 + 160 + 11) and e.get("width") == 1)
        sb = first(r1, ib + 1 if ib >= 0 else len(r1), lambda e: e.get("op") == "sr1")
        un = [i for i, e in enumerate(r1) if e.get("op") == "unwatched"]
        wa = [i for i, e in enumerate(r1) if e.get("op") == "watched"]
        quiet = all(not any(r1[k].get("op") == "read" for k in range(u, w)) for u, w in zip(un, wa))
        rs = first(r1, 0, lambda e: e.get("op") == "reset")
        rw = first(r1, rs, lambda e: e.get("op") == "watched") if rs >= 0 else -1
        ra = first(r1, rs, lambda e: e.get("op") == "read" and e.get("address") == "0x%08X" % (S7 + 160 + 11)) if rs >= 0 else -1
        sa = first(r1, ra + 1, lambda e: e.get("op") == "sr1") if ra >= 0 else -1
        pg = first(r1, rs, lambda e: e.get("op") == "unwatched" and e.get("why") != "MCU reset") if rs >= 0 else -1
        return [check("the log starts with its range, and CR1 is read 1 us after the power-on (both power-ons)",
                      bool(r1) and r1[0].get("range") == "0x080E0000-0x080E0FFF"
                      and any(e.get("op") == "cr1" and e["t_us"] <= 2 for e in r1[:8])
                      and bool(r2) and any(e.get("op") == "cr1" and e["t_us"] <= 2 for e in r2[:8]), json.dumps(r1[:5] + r2[:5])),
                check("a 32-bit read of word 3 with the value the image got (FD 3)",
                      i3 >= 0 and len(d3) >= 2 and int(r1[i3]["value"], 16) == d3[1], "%s %s" % (r1[i3] if i3 >= 0 else None, d3)),
                check("the SR1 read after it, with the value the image got, and a CCR1 write before it",
                      s3 >= 0 and int(r1[s3]["value"], 16) == d3[0] and bool(c3) and int(c3[-1]["value"], 16) & DBECC,
                      "%s %s %s" % (r1[s3] if s3 >= 0 else None, c3[-1:], d3)),
                check("the marked word 5: its read with ecc_mark, the SR1 after it with DBECCERR as the image got it",
                      i5 >= 0 and r1[i5].get("ecc_mark") is True and s5 >= 0 and r1[s5].get("dbeccerr") is True
                      and len(d5) >= 1 and int(r1[s5]["value"], 16) == d5[0] and d5[0] & DBECC,
                      "%s %s %s" % (r1[i5] if i5 >= 0 else None, r1[s5] if s5 >= 0 else None, d5)),
                check("a byte read at offset 0x0B of the marked word 5 (FB 5 11): width 1, ecc_mark, the value the image got, then SR1 with DBECCERR",
                      ib >= 0 and r1[ib].get("ecc_mark") is True and len(bb) >= 2 and int(r1[ib]["value"], 16) == bb[1]
                      and sb >= 0 and r1[sb].get("dbeccerr") is True and int(r1[sb]["value"], 16) == bb[0] and bb[0] & DBECC,
                      "%s %s %s" % (r1[ib] if ib >= 0 else None, r1[sb] if sb >= 0 else None, bb)),
                check("while PG is set the reads are not watched: an unwatched and a watched line per program, no read between",
                      len(un) >= 2 and len(wa) >= 2 and quiet, "unwatched %d, watched %d, quiet %s" % (len(un), len(wa), quiet)),
                check("CR1 writes logged (PG for the programs)",
                      any(e.get("op") == "cr1_write" and int(e["value"], 16) & 2 for e in r1), str([e for e in r1 if e.get("op") == "cr1_write"][:2])),
                check("the software reset: a reset line and CR1 read after it",
                      rs >= 0 and first(r1, rs, lambda e: e.get("op") == "cr1") > rs, str(r1[rs:rs + 3] if rs >= 0 else None)),
                check("after the software reset the watchpoints are back at once (watched, MCU reset): the byte read of marked word 5 before any program is logged with ecc_mark, then SR1 with DBECCERR",
                      rw >= 0 and r1[rw].get("why") == "MCU reset" and ra >= 0 and (pg < 0 or ra < pg) and r1[ra].get("ecc_mark") is True
                      and sa >= 0 and r1[sa].get("dbeccerr") is True and len(ab) >= 1 and ab[0] & DBECC,
                      "%s %s %s %s" % (r1[rw] if rw >= 0 else None, r1[ra] if ra >= 0 else None, r1[sa] if sa >= 0 else None, ab)),
                check("the boot window 1-2: power-on 3 runs (its console and an answer) and writes no flash-reads-3.jsonl",
                      b3 is not None and os.path.exists(os.path.join(tdir, "console-boot3.txt"))
                      and not os.path.exists(os.path.join(tdir, "flash-reads-3.jsonl")) and bool(r2),
                      "answer %s, files %s" % (b3 is not None, sorted(f for f in os.listdir(tdir) if f.startswith("flash-reads"))))]
    if tid == "T-EMU-31":
        # power.cut at_power_on: a cut armed for the first erase (during_erase) or the k-th programmed
        # word (after_flash_words) after the next power-on; used once; a cut of the running boot stays;
        # a later plain power.cut does not drop it; refusals. The fixture log (outcome per cut, the
        # boot it fired in) is checked by m1_smoke step [14] after the run.
        h = ep["hosts"][0]
        sk = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sk.bind((h["bind"], h["ports"][0] + 16))

        def cmd(payload, wait=20.0, again=False):
            sk.setblocking(False)
            try:
                while True:
                    sk.recv(2048)
            except OSError:
                pass
            sk.settimeout(1.0)
            t0, sent = time.time(), False
            while time.time() - t0 < wait:
                if again or not sent:
                    sk.sendto(payload, addr(c))
                    sent = True
                try:
                    d = sk.recv(2048)
                except socket.timeout:
                    continue
                if d[:2] == payload[:2]:
                    return d
            return None

        def up():
            return cmd(b"FR", again=True, wait=90.0) is not None

        def log():
            return [x for x in (ctl.op("flash.log", action="read").get("value") or {}).get("entries", []) if x.get("op") in ("erase", "program")]

        def new(n0):
            return log()[n0:]

        out = []
        ctl.op("flash.timing", erase_us=200000)
        up()
        ctl.op("flash.log", action="start")
        # 1. during_erase at the next power-on: an erase in this boot is not cut; after power.cycle it is
        r1 = ctl.op("power.cut", during_erase=0.5, at_power_on=True)
        n0 = len(log())
        cmd(b"FE", wait=30.0)
        e_now = new(n0)
        ctl.op("power.cycle")
        a = up()
        n1 = len(log())
        cmd(b"FE", wait=1.0)
        b = up()
        e_next = new(n1)
        out.append(check("during_erase at_power_on: the reply; this boot's erase completes; the first erase after power.cycle is cut at 0.5, in the next boot",
                         r1 == {"ok": True, "value": {"during_erase": 0.5, "at_power_on": True}}
                         and [x.get("result") for x in e_now] == ["ok"] and a and b
                         and len(e_next) == 1 and e_next[0].get("result") == "cut" and 0.49 <= e_next[0].get("fraction_done", -1) <= 0.51
                         and e_next[0].get("boot") == (e_now[0].get("boot") or 0) + 1,
                         "%s; now %s; next %s" % (r1, [(x.get("result"), x.get("boot")) for x in e_now],
                                                  [(x.get("result"), x.get("fraction_done"), x.get("boot")) for x in e_next])))
        # 2. used once: the start after that cut erases in full
        n2 = len(log())
        cmd(b"FE", wait=30.0)
        e2 = new(n2)
        out.append(check("used once: the erase after the cut's restart completes", [x.get("result") for x in e2] == ["ok"], str(e2)))
        # 3. after_flash_words at the next power-on: the 2nd programmed word of the next boot is cut
        r3 = ctl.op("power.cut", after_flash_words=2, at_power_on=True)
        n3 = len(log())
        cmd(b"FP\x01")
        p_now = new(n3)
        ctl.op("power.cycle")
        up()
        n4 = len(log())
        q2 = cmd(b"FP\x02")
        l2 = log()[n4:]                           # a log read closes the program entry: two entries at any load
        cmd(b"FP\x03", wait=1.0)
        c3 = up()
        p_next = new(n4)
        out.append(check("after_flash_words 2 at_power_on: this boot's program is not cut; in the next boot the first programmed word ok, the second cut",
                         r3 == {"ok": True, "value": {"after_flash_words": 2, "at_power_on": True}}
                         and [x.get("result") for x in p_now] == ["ok"] and c3
                         and [(x.get("op"), x.get("result")) for x in p_next] == [("program", "ok"), ("program", "cut")],
                         "%s; now %s; next %s; FP2 reply %r, read after it %s" % (r3, [(x.get("op"), x.get("result")) for x in p_now], [(x.get("op"), x.get("address"), x.get("result"), x.get("t_us")) for x in p_next], q2, [(x.get("op"), x.get("address"), x.get("t_us")) for x in l2])))
        # 4. not fired: armed, then two power cycles without an erase in the armed boot; then an erase completes
        ctl.op("power.cut", during_erase=0.5, at_power_on=True)
        ctl.op("power.cycle")
        up()
        ctl.op("power.cycle")
        up()
        n5 = len(log())
        cmd(b"FE", wait=30.0)
        e5 = new(n5)
        out.append(check("not fired in its boot: dropped at the power cycle after it, the next boot's erase completes",
                         [x.get("result") for x in e5] == ["ok"], str(e5)))
        # 5. a plain cut of the running boot stays and fires first; a later plain cut does not drop the at_power_on one
        ra = ctl.op("power.cut", during_erase=0.3, at_power_on=True)
        rp = ctl.op("power.cut", during_erase=0.7)
        n6 = len(log())
        cmd(b"FE", wait=1.0)
        up()
        cmd(b"FE", wait=1.0)
        up()
        e6 = new(n6)
        out.append(check("a plain cut in this boot fires at 0.7, then the at_power_on one at 0.3 in the start after that cut",
                         ra.get("ok") and rp.get("ok") and [x.get("result") for x in e6] == ["cut", "cut"]
                         and 0.69 <= e6[0].get("fraction_done", -1) <= 0.71 and 0.29 <= e6[1].get("fraction_done", -1) <= 0.31
                         and e6[1].get("boot") == e6[0].get("boot") + 1,
                         str([(x.get("result"), x.get("fraction_done"), x.get("boot")) for x in e6])))
        # 6. a second at_power_on replaces the first
        ctl.op("power.cut", during_erase=0.2, at_power_on=True)
        ctl.op("power.cut", during_erase=0.6, at_power_on=True)
        ctl.op("power.cycle")
        up()
        n7 = len(log())
        cmd(b"FE", wait=1.0)
        up()
        e7 = new(n7)
        out.append(check("a second at_power_on replaces the first: the cut is at 0.6",
                         len(e7) >= 1 and e7[0].get("result") == "cut" and 0.59 <= e7[0].get("fraction_done", -1) <= 0.61,
                         str([(x.get("result"), x.get("fraction_done")) for x in e7])))
        # 7. refusals
        bad = [ctl.op("power.cut", at_us=1000, at_power_on=True),
               ctl.op("power.cut", at_address=0x080E0000, during_erase=0.5, at_power_on=True),
               ctl.op("power.cut", during_erase=1.5, at_power_on=True),
               ctl.op("power.cut", during_erase=-0.1, at_power_on=True),
               ctl.op("power.cut", after_flash_words=0, at_power_on=True),
               ctl.op("power.cut", during_erase=0.5, after_flash_words=2, at_power_on=True),
               ctl.op("power.cut", at_power_on=True),
               ctl.op("power.cut", during_erase=0.5, at_power_on=False)]
        out.append(check("refused: with at_us, with at_address, f outside 0..1, k 0, both keys, neither, at_power_on false",
                         all(x.get("ok") is False for x in bad), str([x.get("error") for x in bad])))
        # the end: one armed and never used (the fixture log says: not armed)
        ctl.op("power.cut", during_erase=0.5, at_power_on=True)
        return out
    if tid in ("T-EMU-34", "T-EMU-35"):
        h = ep["hosts"][0]
        sk = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sk.bind((h["bind"], h["ports"][0] + (19 if tid == "T-EMU-34" else 20)))

        def cmd(payload, wait=20.0, again=False):
            sk.setblocking(False)
            try:
                while True:
                    sk.recv(2048)
            except OSError:
                pass
            sk.settimeout(1.0)
            t0, sent = time.time(), False
            while time.time() - t0 < wait:
                if again or not sent:
                    sk.sendto(payload, addr(c))
                    sent = True
                try:
                    d = sk.recv(2048)
                except socket.timeout:
                    continue
                if d[:2] == payload[:2]:
                    return d
            return None

        def words(d):
            return tuple(int.from_bytes(d[i:i + 4], "big") for i in range(2, len(d) - 3, 4)) if d else ()

        cmd(b"FR", again=True, wait=60.0)
    if tid == "T-EMU-34":
        # flash.fault {op erase, end_after_us N} (decision 0081): the erase runs N us of emulated time, then
        # ends with no error flag; flash.log names it "late" with duration_us; refused with a flag, with
        # never_end, for N < 1 and for op program; used once
        ERRS, FF, N = 0x1FE << 16, 0xFFFFFFFF, 300000
        bad = [ctl.op("flash.fault", op="erase", end_after_us=N, flag="PGSERR"),
               ctl.op("flash.fault", op="erase", end_after_us=N, never_end=True),
               ctl.op("flash.fault", op="erase", end_after_us=0),
               ctl.op("flash.fault", op="erase", end_after_us=-5),
               ctl.op("flash.fault", op="program", end_after_us=N),
               ctl.op("flash.fault", op="erase", end_after_us="300000")]
        ctl.op("flash.log", action="start")
        cmd(b"FP\x03")                                  # a word to erase
        f = ctl.op("flash.fault", op="erase", end_after_us=N)
        e1 = words(cmd(b"FE", wait=60.0))
        e2 = words(cmd(b"FE", wait=60.0))               # the fault is used once
        ers = [x for x in (ctl.op("flash.log", action="read").get("value") or {}).get("entries", []) if x.get("op") == "erase"]
        late = ers[-2] if len(ers) >= 2 else {}
        nxt = ers[-1] if ers else {}
        return [check("refused: with flag, with never_end, N 0 and -5, op program, N not an integer", all(b.get("ok") is False for b in bad), str(bad)),
                check("accepted; the erase answers with no error flag and the sector erased", f.get("ok") and len(e1) >= 2 and not e1[0] & ERRS and e1[1] == FF,
                      "%s %s" % (f, e1)),
                check("flash.log: result late, duration_us N, and it ran N us of emulated time (within 100 us, as a normal erase's entry)",
                      late.get("result") == "late" and late.get("duration_us") == N and late.get("t_end_us") is not None
                      and abs(late["t_end_us"] - late["t_us"] - N) <= 100, str(late)),
                check("the next erase is a normal one (the fault used once): result ok", nxt.get("result") == "ok" and len(e2) >= 2 and not e2[0] & ERRS,
                      "%s %s" % (nxt, e2))]
    if tid == "T-EMU-35":
        # cpu.fault in a handler that runs with FAULTMASK set (net_test FN): a fault the core cannot take
        # (LOCKUP on the chip). The stub is UDF everywhere with a branch-to-self after the first: whatever
        # Renode does with the UDF there, the core stays in the stub or enters a fault vector, never runs off
        # it. The state names it (lockup or entered); FAULTMASK, BASEPRI, PRIMASK and the running priority at
        # the injection are recorded; a watchdog reset brings the image back and the state stays
        a = ctl.op("cpu.fault", in_us=0, mode="handler", exception=44, within_us=3000000)
        cmd(b"FN\x32", wait=1.0)
        time.sleep(2.0)
        r = ctl.op("cpu.fault", read=True).get("value") or {}
        inj = r.get("injected") or {}
        w = ctl.op("reset.wdt")
        back = cmd(b"FR", again=True, wait=60.0)
        r2 = ctl.op("cpu.fault", read=True).get("value") or {}
        ints = all(isinstance(inj.get(k), int) for k in ("faultmask", "basepri", "primask", "priority"))
        return [check("armed and injected in exception 44", a.get("ok") and inj.get("ipsr") == 44, "%s %s" % (a, r)),
                check("FAULTMASK 1 at the injection; BASEPRI, PRIMASK and the running priority recorded as numbers",
                      ints and inj.get("faultmask") == 1, str(inj)),
                check("the state names what happened: lockup (no entry) or entered (an entry)",
                      (r.get("state") == "lockup" and r.get("entry") is None) or (r.get("state") == "entered" and r.get("entry") is not None),
                      "state %s entry %s" % (r.get("state"), r.get("entry"))),
                check("no abort: after a watchdog reset the image answers again, and the state is kept",
                      w.get("ok") and back is not None and r2.get("state") == r.get("state"), "reset %s, answer %s, state %s" % (w, back is not None, r2.get("state")))]
    if tid == "T-EMU-33":
        # an ECC mark set before an MCU reset (software reset, SYSRESETREQ) is in force at once after
        # it: a byte read of the marked word shows DBECCERR, with no program before the reset and with
        # one right before it, and before any program after it
        h = ep["hosts"][0]
        sk = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sk.bind((h["bind"], h["ports"][0] + 18))

        def cmd(payload, wait=20.0, again=False):
            sk.setblocking(False)
            try:
                while True:
                    sk.recv(2048)
            except OSError:
                pass
            sk.settimeout(1.0)
            t0, sent = time.time(), False
            while time.time() - t0 < wait:
                if again or not sent:
                    sk.sendto(payload, addr(c))
                    sent = True
                try:
                    d = sk.recv(2048)
                except socket.timeout:
                    continue
                if d[:2] == payload[:2]:
                    return d
            return None

        def words(d):
            return tuple(int.from_bytes(d[i:i + 4], "big") for i in range(2, len(d) - 3, 4)) if d else ()

        def reset():
            sk.sendto(b"FX", addr(c))                  # software reset, no reply
            time.sleep(1.0)
            return words(cmd(b"FR", again=True, wait=60.0))

        DBECC, S7 = 1 << 26, 0x080E0000
        cmd(b"FR", again=True, wait=60.0)
        cmd(b"FE", wait=60.0)
        cmd(b"FP\x03")
        cmd(b"FP\x05")
        cmd(b"FP\x09")
        ec = ctl.op("flash.ecc", address=S7 + 32 * 5, type="double")
        e9 = ctl.op("flash.ecc", address=S7 + 32 * 9, type="double")   # read by the first code of each start (FS)
        b0 = words(cmd(b"FB\x05\x0b"))
        ra = reset()                                   # no program before it
        fa = words(cmd(b"FS"))
        ba = words(cmd(b"FB\x05\x0b"))
        da = words(cmd(b"FD\x05"))
        cmd(b"FP\x07")                                 # a program right before the reset
        rb = reset()
        fb = words(cmd(b"FS"))
        bb = words(cmd(b"FB\x05\x0b"))
        db = words(cmd(b"FD\x05"))
        sft = 1 << 24                                  # RCC_RSR SFTRSTF
        dbl = lambda w: bool(w) and bool(w[0] & DBECC)
        return [check("flash.ecc accepted, and a byte read of the marked word before any reset shows DBECCERR",
                      ec.get("ok") and dbl(b0), "%s %s" % (ec, b0)),
                check("software reset with no program before it: RSR shows it, then a byte read and a word read of the marked word show DBECCERR",
                      bool(ra) and bool(ra[0] & sft) and dbl(ba) and dbl(da), "RSR %s, FB %s, FD %s" % (ra, ba, da)),
                check("a program right before the software reset: then a byte read and a word read of the marked word show DBECCERR",
                      bool(rb) and bool(rb[0] & sft) and dbl(bb) and dbl(db), "RSR %s, FB %s, FD %s" % (rb, bb, db)),
                check("no read slips through at the reset: the first code of each start (a few instructions after it) reads marked word 9 and gets DBECCERR",
                      e9.get("ok") and len(fa) == 2 and bool(fa[1] & DBECC) and len(fb) == 2 and bool(fb[1] & DBECC),
                      "FS after the 1st reset %s, after the 2nd %s" % (fa, fb))]
    if tid == "T-EMU-32":
        # one change gives one event: PH6 driven by hw.input (the agent's socket thread) while the
        # image changes its pull (net_test FT, seen by the 50 us sampler on the emulation thread).
        # No two events in a row with the same state; every level change and every pull change
        # is in the trace once.
        h = ep["hosts"][0]
        sk = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sk.bind((h["bind"], h["ports"][0] + 17))
        pairs, spin, n_in = 100, 20, 60
        ctl.op("hw.input", pin="PH6", level=0)
        r1 = ctl.op("hw.trace", action="start", pins=["PH6"])
        f = (r1.get("value") or {}).get("file")
        sk.sendto(b"FT" + bytes([pairs, spin]), addr(c))
        ins = []
        for i in range(n_in):
            ins.append(ctl.op("hw.input", pin="PH6", level=1 - i % 2))
            time.sleep(0.01)
        sk.settimeout(1.0)
        rep, t0 = None, time.time()
        while rep is None and time.time() - t0 < 90:
            try:
                d = sk.recv(2048)
            except socket.timeout:
                continue
            if d[:2] == b"FT":
                rep = d
        time.sleep(0.3)
        r2 = ctl.op("hw.trace", action="stop")
        evs = [json.loads(x) for x in open(f)] if f and os.path.isfile(f) else []
        ph6 = [e for e in evs if e.get("kind") == "gpio" and e.get("pin") == "PH6"]
        keys = [(e.get("level"), e.get("mode"), e.get("pull")) for e in ph6]
        dup = [(ph6[i - 1].get("seq"), ph6[i].get("seq")) for i in range(1, len(keys)) if keys[i] == keys[i - 1]]
        lv = sum(1 for i in range(1, len(keys)) if keys[i][0] != keys[i - 1][0])
        pu = sum(1 for i in range(1, len(keys)) if keys[i][2] != keys[i - 1][2])
        changes = int.from_bytes(rep[2:6], "big") if rep else None
        t_pull = [ph6[i]["t_us"] for i in range(1, len(keys)) if keys[i][2] != keys[i - 1][2]]
        t_lv = [ph6[i]["t_us"] for i in range(1, len(keys)) if keys[i][0] != keys[i - 1][0]]
        both = sum(1 for t in t_lv if t_pull and t_pull[0] <= t <= t_pull[-1])
        return [check("start, inputs, FT reply, stop", r1.get("ok") and f and all(x.get("ok") for x in ins) and rep is not None
                      and (r2.get("value") or {}).get("events", 0) == len(evs), "%s; FT %r; %s" % (r1, rep, r2.get("value"))),
                check("both paths at once: level changes between the first and the last pull change", both >= 5,
                      "%d of %d level changes inside the pull changes (%s to %s us)" % (both, len(t_lv), t_pull[:1], t_pull[-1:])),
                check("one event per change: no two PH6 events in a row with the same state", not dup and len(keys) > 1,
                      "%d events; repeated: %s" % (len(keys), dup[:10])),
                check("no level change dropped: %d hw.input changes, each once in the trace" % n_in, lv == n_in, "%d level changes" % lv),
                check("no pull change dropped: the image's changes, each once in the trace", changes is not None and pu == changes,
                      "%s pull changes by the image, %d in the trace" % (changes, pu))]
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
    global EP_PATH
    EP_PATH = a.endpoint
    ep = json.load(open(a.endpoint))
    for tid in a.test:
        t0 = time.time()
        del NOTES[:]
        try:
            checks = run_test(tid, ep)
            if checks is None:
                e = {"id": tid, "status": "error", "reason": "unknown test"}
            else:
                e = {"id": tid, "status": "pass" if all(c["ok"] for c in checks) else "fail", "checks": checks}
        except Exception as ex:  # noqa: BLE001
            e = {"id": tid, "status": "error", "reason": "%s: %s" % (type(ex).__name__, ex)}
        if NOTES:
            e["reason"] = "; ".join(NOTES + ([e["reason"]] if e.get("reason") else []))
        e["duration_s"] = round(time.time() - t0, 3)
        print(json.dumps(e))
        sys.stdout.flush()


if __name__ == "__main__":
    main()
