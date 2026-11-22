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
            w = words(cmd(b"FC", wait=120.0))
            ok = len(w) == 6
            cyc, t5, t12, worst = (w[1] / 64.0, w[2], w[3], w[4]) if ok else (0, 0, 0, 0)
            out.append(check("masked erase %s: cycles/64, TIM5 and TIM12 within 20 us of each other, of the length and between any two reads" % kind,
                             ok and abs(cyc - t5) <= 20 and abs(t12 - t5) <= 20 and abs(t5 - us) <= 20 and worst <= 20 and bool(w[0] & (1 << 18)) == ("PGSERR" in kind),
                             "cycles/64 %.1f, TIM5 %d, TIM12 %d, worst between reads %d us, polls %d" % (cyc, t5, t12, worst, w[5] if ok else 0)))
        # a failing erase that never ends, ended by the watchdog: the figures up to the last poll
        ctl.op("flash.fault", op="erase", never_end=True)
        cmd(b"FW")
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
        # log.corrupt: on the ring the image wrote
        offered = "log.corrupt" in ep.get("fixtures", [])
        n0 = words(cmd(b"FL\x03"))
        lc = ctl.op("log.corrupt", seq=3)
        n1 = words(cmd(b"FK\x03"))
        no = ctl.op("log.corrupt", seq=4)
        out.append(check("log.corrupt (offered with the runner's ring setting): one text bit of the entry changed, its number kept; an entry not there refused; the address not in the endpoint",
                         offered and n0 == (3, 0x41424344) and lc.get("ok") and n1 == (3, 0x41424345) and no.get("ok") is False
                         and "2401e700" not in json.dumps(ep).lower(), str((offered, n0, lc, n1, no))[:200]))
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
