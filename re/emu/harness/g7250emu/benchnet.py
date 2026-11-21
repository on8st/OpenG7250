#!/usr/bin/env python3
"""Network bridge, agent link and fixture server for product runs
(docs/PRODUCT-RUNNER.md; test list format g7250-testlist v1.1, section 3-4).

- CH394 link: the emulated Ethernet chip connects here (TCP). Frames:
  [type 1 B][len 2 B BE][payload]; type 1 radio->host: srcPort(2) dstIP(4)
  dstPort(2) data; type 2 host->radio: srcIP(4) srcPort(2) dstPort(2) flags(1)
  data (flags bit 0 = broadcast); type 3 radio->host: the address the image
  programmed (SIPR(4) SUBR(4)), sent when SIPR is complete.
- Address mapping: per radio port P a local UDP socket radio.connect[P]
  (127.0.0.1:40000+P); per host i a local port range hosts[i].ports on
  hosts[i].bind; broadcast.send_to limited 40100+P, directed 40200+P. A datagram
  the bench sends from local port p reaches the radio from hosts[i].ip:p (same
  port); a radio datagram to hosts[i].ip:p goes to bind:p from connect[srcPort].
- pcapng capture of every datagram (Ethernet/IPv4/UDP, both directions).
- Agent link: the bench agent in the emulator connects here (TCP, line
  protocol, see models/csharp/G7250BenchAgent.cs).
- Fixture server: JSON Lines on a TCP port for the test bench.
"""
from __future__ import annotations
import ipaddress
import json
import os
import queue
import select
import socket
import struct
import threading
import time

SPEC_PORTS = list(range(1024, 1036)) + [7250, 7251]
CONNECT_BASE, LIMITED_BASE, DIRECTED_BASE = 40000, 40100, 40200
HOST_PORT_BASE = 52000
RADIO_MAC = "02:00:00:00:72:50"


def _listen_tcp():
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(("127.0.0.1", 0))
    s.listen(1)
    return s


class Pcap:
    """Minimal pcapng writer, link type Ethernet."""

    def __init__(self, path):
        self.f = open(path, "wb")
        shb_body = struct.pack("<IHHq", 0x1A2B3C4D, 1, 0, -1)
        self._block(0x0A0D0D0A, shb_body)
        self._block(0x00000001, struct.pack("<HHI", 1, 0, 65535))
        self.lock = threading.Lock()

    def _block(self, btype, body):
        body += b"\0" * ((4 - len(body) % 4) % 4)
        n = 12 + len(body)
        self.f.write(struct.pack("<II", btype, n) + body + struct.pack("<I", n))

    @staticmethod
    def _csum(b):
        if len(b) % 2:
            b += b"\0"
        s = sum(struct.unpack("!%dH" % (len(b) // 2), b))
        while s >> 16:
            s = (s & 0xFFFF) + (s >> 16)
        return ~s & 0xFFFF

    def udp(self, src_ip, sport, dst_ip, dport, data, from_radio):
        mac_r = bytes.fromhex(RADIO_MAC.replace(":", ""))
        mac_h = bytes.fromhex("020000000001")
        if dst_ip.endswith(".255") or dst_ip == "255.255.255.255":
            dmac = b"\xff" * 6
        else:
            dmac = mac_h if from_radio else mac_r
        smac = mac_r if from_radio else mac_h
        udp = struct.pack("!HHHH", sport, dport, 8 + len(data), 0) + data
        ip = struct.pack("!BBHHHBBH4s4s", 0x45, 0, 20 + len(udp), 0, 0, 64, 17, 0,
                         socket.inet_aton(src_ip), socket.inet_aton(dst_ip))
        ip = ip[:10] + struct.pack("!H", self._csum(ip)) + ip[12:]
        frame = dmac + smac + b"\x08\x00" + ip + udp
        t = int(time.time() * 1e6)
        body = struct.pack("<IIIII", 0, t >> 32, t & 0xFFFFFFFF, len(frame), len(frame)) + frame
        with self.lock:
            self._block(0x00000006, body)
            self.f.flush()

    def close(self):
        self.f.close()


class Bridge:
    """CH394 link + address mapping. One per boot."""

    def __init__(self, pcap_path):
        self.ch_srv = _listen_tcp()
        self.ch_port = self.ch_srv.getsockname()[1]
        self.ch = None
        self.radio_ip = None
        self.netmask = "255.255.255.0"
        self.hosts = []
        self.pcap = Pcap(pcap_path)
        self.socks = {}       # local socket -> (kind, radio port)
        self.by_port = {}     # radio port -> connect socket
        self.stop = False
        self.stats = {"to_radio": 0, "from_radio": 0, "unroutable": 0}
        for p in SPEC_PORTS:
            for kind, base in (("connect", CONNECT_BASE), ("limited", LIMITED_BASE), ("directed", DIRECTED_BASE)):
                if kind != "connect" and p not in (1024, 7251):
                    continue
                s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                s.bind(("127.0.0.1", base + p))
                self.socks[s] = (kind, p)
                if kind == "connect":
                    self.by_port[p] = s
        threading.Thread(target=self._accept, daemon=True).start()
        threading.Thread(target=self._udp_loop, daemon=True).start()

    def set_radio(self, ip, netmask="255.255.255.0"):
        if netmask == "0.0.0.0":
            netmask = "255.255.255.0"     # SUBR not written yet; updated when it is
        self.radio_ip, self.netmask = ip, netmask
        net = ipaddress.ip_network("%s/%s" % (ip, netmask), strict=False)
        hosts_in = [str(h) for h in net.hosts() if str(h) != ip][19:21]
        outside = "10.99.0.20" if not ipaddress.ip_address("10.99.0.20") in net else "192.0.2.20"
        self.hosts = []
        for i, hip in enumerate(hosts_in + [outside]):
            lo = HOST_PORT_BASE + 100 * i
            self.hosts.append({"ip": hip, "in_subnet": i < 2, "bind": "127.0.0.1", "ports": [lo, lo + 99]})
        self.directed = str(net.broadcast_address)

    def endpoint(self, control_port, fixtures):
        return {
            "format": "g7250-endpoint", "version": 1, "minor": 1, "kind": "emulator",
            "radio": {"ip": self.radio_ip, "netmask": self.netmask, "mac": RADIO_MAC,
                      "connect": {str(p): "127.0.0.1:%d" % (CONNECT_BASE + p) for p in SPEC_PORTS}},
            "hosts": self.hosts,
            "broadcast": {"limited": True, "directed": self.directed,
                          "send_to": {"limited": {str(p): "127.0.0.1:%d" % (LIMITED_BASE + p) for p in (1024, 7251)},
                                      "directed": {str(p): "127.0.0.1:%d" % (DIRECTED_BASE + p) for p in (1024, 7251)}}},
            "control": {"transport": "tcp", "address": "127.0.0.1:%d" % control_port},
            "fixtures": fixtures, "time_scale": 1.0,
        }

    def _host_for_port(self, p):
        for h in self.hosts:
            if h["ports"][0] <= p <= h["ports"][1]:
                return h
        return self.hosts[0] if self.hosts else None

    def _accept(self):
        self.ch_srv.settimeout(1.0)
        while not self.stop:
            try:
                c, _ = self.ch_srv.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            self.ch = c
            self._ch_reader(c)

    def _ch_reader(self, c):
        buf = b""
        while not self.stop:
            try:
                d = c.recv(65536)
            except OSError:
                return
            if not d:
                return
            buf += d
            while len(buf) >= 3:
                n = struct.unpack("!H", buf[1:3])[0]
                if len(buf) < 3 + n:
                    break
                t, p = buf[0], buf[3:3 + n]
                buf = buf[3 + n:]
                if t == 1 and len(p) >= 8:
                    self._from_radio(p)
                elif t == 3 and len(p) >= 8:
                    self.set_radio(socket.inet_ntoa(p[0:4]), socket.inet_ntoa(p[4:8]))

    def _from_radio(self, p):
        sport, = struct.unpack("!H", p[0:2])
        dip = socket.inet_ntoa(p[2:6])
        dport, = struct.unpack("!H", p[6:8])
        data = p[8:]
        self.stats["from_radio"] += 1
        self.pcap.udp(self.radio_ip or "0.0.0.0", sport, dip, dport, data, True)
        bcast = dip == "255.255.255.255" or dip == getattr(self, "directed", None)
        if not bcast and dip not in [h["ip"] for h in self.hosts]:
            self.stats["unroutable"] += 1
            return
        s = self.by_port.get(sport)
        if s is None:
            s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            s.bind(("127.0.0.1", 0))
            self.by_port[sport] = s
        try:
            s.sendto(data, ("127.0.0.1", dport))
        except OSError:
            self.stats["unroutable"] += 1

    def _udp_loop(self):
        while not self.stop:
            try:
                r, _, _ = select.select(list(self.socks.keys()), [], [], 0.5)
            except (OSError, ValueError):
                return                      # sockets closed at the end of the boot
            for s in r:
                try:
                    data, (_, lport) = s.recvfrom(65536)
                except OSError:
                    continue
                kind, rport = self.socks[s]
                h = self._host_for_port(lport)
                if h is None or self.ch is None or self.radio_ip is None:
                    self.stats["unroutable"] += 1
                    continue
                bcast = kind != "connect"
                dst = self.radio_ip if not bcast else ("255.255.255.255" if kind == "limited" else self.directed)
                self.pcap.udp(h["ip"], lport, dst, rport, data, False)
                payload = socket.inet_aton(h["ip"]) + struct.pack("!HHB", lport, rport, 1 if bcast else 0) + data
                try:
                    self.ch.sendall(struct.pack("!BH", 2, len(payload)) + payload)
                    self.stats["to_radio"] += 1
                except OSError:
                    self.stats["unroutable"] += 1

    def close(self):
        self.stop = True
        for s in list(self.socks) + list(self.by_port.values()):
            try:
                s.close()
            except OSError:
                pass
        for s in (self.ch, self.ch_srv):
            try:
                s and s.close()
            except OSError:
                pass
        self.pcap.close()


class Agent:
    """Link to the bench agent inside the emulator (line protocol)."""

    def __init__(self, trace_dir):
        self.srv = _listen_tcp()
        self.port = self.srv.getsockname()[1]
        self.conn = None
        self.replies = queue.Queue()
        self.trace_dir = trace_dir
        self.trace = None
        self.traces = []
        self.closed = False
        self.on_cut = None
        self.events = 0
        self.lock = threading.Lock()
        threading.Thread(target=self._accept, daemon=True).start()

    def _accept(self):
        self.srv.settimeout(1.0)
        while True:
            try:
                c, _ = self.srv.accept()
            except socket.timeout:
                if self.closed:
                    return
                continue
            except OSError:
                return
            self.conn = c
            f = c.makefile("r")
            for line in f:
                line = line.rstrip("\n")
                if line.startswith("cut "):
                    if self.on_cut:
                        threading.Thread(target=self.on_cut, daemon=True).start()
                    continue
                if line.startswith("ev "):
                    self.events += 1
                    t = self.trace
                    if t is not None and not t.closed:
                        t.write(line[3:] + "\n")
                        t.flush()
                else:
                    self.replies.put(line)

    def wait_connected(self, timeout):
        t0 = time.time()
        while self.conn is None and time.time() - t0 < timeout:
            time.sleep(0.05)
        return self.conn is not None

    def ask(self, line, timeout=10.0):
        if self.conn is None:
            return None
        with self.lock:
            while not self.replies.empty():
                self.replies.get_nowait()
            self.conn.sendall((line + "\n").encode())
            try:
                return self.replies.get(timeout=timeout)
            except queue.Empty:
                return None

    def new_trace(self):
        """Open trace-<n>.jsonl (one file per start); returns its absolute path."""
        self.end_trace()
        path = os.path.abspath(os.path.join(self.trace_dir, "trace-%d.jsonl" % (len(self.traces) + 1)))
        self.trace = open(path, "w")
        self.traces.append(path)
        return path

    def append_event(self, ev):
        t = self.trace
        if t is not None and not t.closed:
            t.write(json.dumps(ev) + "\n")
            t.flush()

    def end_trace(self):
        if self.trace is not None and not self.trace.closed:
            self.trace.close()

    def now_us(self):
        r = self.ask("time")
        return int(r.split()[1]) if r and r.startswith("time ") else None

    def close(self):
        self.closed = True
        for s in (self.conn, self.srv):
            try:
                s and s.close()
            except OSError:
                pass
        self.end_trace()


class FixtureServer:
    """JSON Lines control socket for the test bench (format section 4)."""

    OFFERED = ["hw.observe", "hw.trace", "hw.input", "ctrl.set", "time.scale",
               "net.broadcast", "net.hosts", "power.cycle", "power.cut", "reset.wdt", "link.set"]

    def __init__(self, agent_getter, on_power_cycle):
        self.srv = _listen_tcp()
        self.port = self.srv.getsockname()[1]
        self.agent_getter = agent_getter
        self.on_power_cycle = on_power_cycle
        self.ctrl_state = "absent"
        self.trace_pins = []
        self.stop = False
        threading.Thread(target=self._accept, daemon=True).start()

    def _accept(self):
        self.srv.settimeout(1.0)
        while not self.stop:
            try:
                c, _ = self.srv.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            threading.Thread(target=self._serve, args=(c,), daemon=True).start()

    def _serve(self, c):
        f = c.makefile("rw")
        for line in f:
            try:
                req = json.loads(line)
                rep = self.handle(req.get("op"), req.get("args") or {})
            except Exception as e:  # noqa: BLE001
                rep = {"ok": False, "error": "bad request: %s" % e}
            f.write(json.dumps(rep) + "\n")
            f.flush()

    def handle(self, op, args):
        agent = self.agent_getter()
        if op == "hw.observe":
            r = agent.ask("observe") if agent else None
            if not r or not r.startswith("obs "):
                return {"ok": False, "error": "emulator not ready"}
            return {"ok": True, "value": json.loads(r[4:])}
        if op == "hw.trace":
            action = args.get("action")
            if action == "start":
                if not agent:
                    return {"ok": False, "error": "emulator not ready"}
                path = agent.new_trace()
                self.trace_pins = list(args.get("pins", []))
                r = agent.ask(("trace start " + " ".join(self.trace_pins)).strip())
                if r != "ok":
                    return {"ok": False, "error": r or "emulator not ready"}
                return {"ok": True, "value": {"file": path}}
            if action == "stop":
                r = agent.ask("trace stop") if agent else None
                if not r or not r.startswith("ok"):
                    return {"ok": False, "error": r or "emulator not ready"}
                time.sleep(0.2)
                path = agent.trace.name if agent.trace is not None else None
                agent.end_trace()
                n = sum(1 for _ in open(path)) if path else 0
                w = r.split()
                return {"ok": True, "value": {"events": n, "dropped": int(w[2]) if len(w) > 2 else 0, "file": path}}
            return {"ok": False, "error": "hw.trace needs action start|stop"}
        if op == "hw.input":
            pin, level = args.get("pin"), args.get("level")
            if level not in (0, 1):
                return {"ok": False, "error": "level must be 0 or 1"}
            r = agent.ask("input %s %d" % (pin, level)) if agent else None
            return {"ok": True} if r == "ok" else {"ok": False, "error": (r or "emulator not ready").replace("err ", "", 1)}
        if op == "ctrl.set":
            st = args.get("state")
            if st not in ("absent", "silent"):
                return {"ok": False, "error": "state must be absent|silent"}
            self.ctrl_state = st   # both: no bytes on the Main's interlink RX
            return {"ok": True}
        if op == "link.set":
            sp = args.get("speed")
            if sp not in (0, 10, 100):
                return {"ok": False, "error": "speed must be 0, 10 or 100"}
            r = agent.ask("link %d" % sp) if agent else None
            return {"ok": True} if r == "ok" else {"ok": False, "error": (r or "emulator not ready").replace("err ", "", 1)}
        if op == "reset.wdt":
            r = agent.ask("wdt") if agent else None
            return {"ok": True} if r == "ok" else {"ok": False, "error": (r or "emulator not ready").replace("err ", "", 1)}
        if op == "power.cut":
            at = args.get("at_us")
            if not isinstance(at, int) or at < 0:
                return {"ok": False, "error": "at_us must be a non-negative integer (emulated us since boot)"}
            r = agent.ask("cut %d" % at) if agent else None
            if not r or not r.startswith("ok"):
                return {"ok": False, "error": (r or "emulator not ready").replace("err ", "", 1)}
            return {"ok": True, "value": {"at_us": int(r.split()[1])}}
        if op == "power.cycle":
            self.on_power_cycle()
            return {"ok": True}
        if op == "time.advance":
            return {"ok": False, "error": "not offered: time.advance"}
        return {"ok": False, "error": "fixture not offered: %s" % op}

    def close(self):
        self.stop = True
        try:
            self.srv.close()
        except OSError:
            pass
