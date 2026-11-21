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
# Local port plan per run (format section 3): connect = B + P, limited send_to =
# B + 100 + P, directed send_to = B + 200 + P, host ranges from B + 8000. The
# base B is chosen per boot from bases whose whole span (B + 1024 .. B + 8299)
# stays clear of the ports the format doc uses as examples (41024-47451,
# 52000-52299), and only if every port in that span can be bound.
PORT_BASES = (20000, 30000, 10000)
CONNECT_OFF, LIMITED_OFF, DIRECTED_OFF, HOST_OFF = 0, 100, 200, 8000
# The fixed source ports the spec tests need (T-B3-33: 1024, T-B8-19: 7250).
# Format v1.4: hosts[0].fixed_ports lists them; the bench binds one only when a
# test asks for that exact port, so they are not in hosts[0].ports. Each is
# offered when it is free at the start of the run (G7250_NO_FIXED_PORTS=1:
# never). A datagram from such a local port maps to hosts[0] with that port.
# No runner port lies there (every base puts them above 11000), and the
# reference model never binds 1024-7251 (v1.4).
HOST0_FIXED = (1024, 7250)
NO_FIXED = os.environ.get("G7250_NO_FIXED_PORTS") == "1"
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
        # the radio's source IP as its CH394 has it now (SIPR): 0.0.0.0 from power-on
        # until the image programs an address; radio_ip is the published address
        # for the bench and survives a power cycle, the source address does not
        self.src_ip = "0.0.0.0"
        self.netmask = "255.255.255.0"
        self.hosts = []
        self.pcap = Pcap(pcap_path)
        self.socks = {}       # local socket -> (kind, radio port)
        self.by_port = {}     # radio port -> connect socket
        self.stop = False
        self.stats = {"to_radio": 0, "from_radio": 0, "unroutable": 0}
        self.dhcp = DhcpServer(self._dhcp_send, lambda: self.now_us() if self.now_us else None)
        self.now_us = None
        self.base = None
        for b in PORT_BASES:
            if self._claim(b):
                self.base = b
                break
        if self.base is None:
            self.close()
            raise RuntimeError("no free local port base (tried %s): a port of every span is in use" %
                               ", ".join(str(b) for b in PORT_BASES))
        threading.Thread(target=self._accept, daemon=True).start()
        threading.Thread(target=self._udp_loop, daemon=True).start()

    def _dhcp_send(self, src_ip, dst_ip, dport, payload, broadcast):
        """A datagram from the DHCP server on the LAN into the Ethernet controller."""
        self.pcap.udp(src_ip, 67, dst_ip, dport, payload, False)
        if self.ch is None:
            return
        frame = socket.inet_aton(src_ip) + struct.pack("!HHB", 67, dport, 1 if broadcast else 0) + payload
        try:
            self.ch.sendall(struct.pack("!BH", 2, len(frame)) + frame)
        except OSError:
            pass

    def _claim(self, b):
        """Bind the bridge sockets for base b, without SO_REUSEADDR so that a port
        another process holds makes the bind fail; check the host ranges are free."""
        got = []
        try:
            for p in SPEC_PORTS:
                for kind, off in (("connect", CONNECT_OFF), ("limited", LIMITED_OFF), ("directed", DIRECTED_OFF)):
                    if kind != "connect" and p not in (1024, 7251):
                        continue
                    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                    got.append((s, kind, p))
                    s.bind(("127.0.0.1", b + off + p))
            for port in range(b + HOST_OFF, b + HOST_OFF + 300):
                t = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                try:
                    t.bind(("127.0.0.1", port))
                finally:
                    t.close()
        except OSError:
            for s, _, _ in got:
                s.close()
            return False
        self.fixed = []
        if not NO_FIXED:
            for port in HOST0_FIXED:
                t = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                try:
                    t.bind(("127.0.0.1", port))
                    self.fixed.append(port)
                except OSError:
                    pass
                finally:
                    t.close()
        for s, kind, p in got:
            self.socks[s] = (kind, p)
            if kind == "connect":
                self.by_port[p] = s
        return True

    def set_radio(self, ip, netmask="255.255.255.0"):
        if netmask == "0.0.0.0":
            netmask = "255.255.255.0"     # SUBR not written yet; updated when it is
        net = ipaddress.ip_network("%s/%s" % (ip, netmask), strict=False)
        hosts_in = []
        for h in net.hosts():             # the 20th and 21st host of the subnet (no full list for big nets)
            if str(h) == ip:
                continue
            hosts_in.append(str(h))
            if len(hosts_in) >= 21:
                break
        hosts_in = hosts_in[19:21]
        outside = "10.99.0.20" if not ipaddress.ip_address("10.99.0.20") in net else "192.0.2.20"
        hosts = []
        for i, hip in enumerate(hosts_in + [outside]):
            lo = self.base + HOST_OFF + 100 * i
            h = {"ip": hip, "in_subnet": i < 2, "bind": "127.0.0.1", "ports": [lo, lo + 99]}
            if i == 0 and getattr(self, "fixed", None):
                h["fixed_ports"] = list(self.fixed)
            hosts.append(h)
        # publish the address last: the runner waits for radio_ip
        self.hosts, self.directed, self.netmask = hosts, str(net.broadcast_address), netmask
        self.radio_ip = ip

    def endpoint(self, control_port, fixtures):
        return {
            "format": "g7250-endpoint", "version": 1, "minor": 1, "kind": "emulator",
            "radio": {"ip": self.radio_ip, "netmask": self.netmask, "mac": RADIO_MAC,
                      "connect": {str(p): "127.0.0.1:%d" % (self.base + CONNECT_OFF + p) for p in SPEC_PORTS}},
            "hosts": self.hosts,
            "broadcast": {"limited": True, "directed": self.directed,
                          "send_to": {"limited": {str(p): "127.0.0.1:%d" % (self.base + LIMITED_OFF + p) for p in (1024, 7251)},
                                      "directed": {str(p): "127.0.0.1:%d" % (self.base + DIRECTED_OFF + p) for p in (1024, 7251)}}},
            "control": {"transport": "tcp", "address": "127.0.0.1:%d" % control_port},
            "fixtures": fixtures, "time_scale": 1.0,
            "runner": {"port_base": self.base},
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
            self.src_ip = "0.0.0.0"            # a new emulator: the CH394 starts with SIPR 0.0.0.0
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
                elif t == 6 and len(p) >= 16:
                    # the CH394's emulated SEND time in front of the frame
                    self._from_radio(p[8:], t_us=struct.unpack("!Q", p[0:8])[0])
                elif t == 3 and len(p) >= 8:
                    self.src_ip = socket.inet_ntoa(p[0:4])
                    self.set_radio(socket.inet_ntoa(p[0:4]), socket.inet_ntoa(p[4:8]))

    def _from_radio(self, p, t_us=None):
        sport, = struct.unpack("!H", p[0:2])
        dip = socket.inet_ntoa(p[2:6])
        dport, = struct.unpack("!H", p[6:8])
        data = p[8:]
        self.stats["from_radio"] += 1
        self.pcap.udp(self.src_ip, sport, dip, dport, data, True)
        if dport == 67 and sport == 68:
            self.dhcp.handle(self.src_ip, dip, data, t_us=t_us)
            return
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


class DhcpServer:
    """A DHCP server on the emulated LAN (RFC 2131), for the dhcp.* fixtures.
    Answers DISCOVER with OFFER and REQUEST with ACK or NAK by the client state
    (SELECTING: options 50 + 54; INIT-REBOOT: 50 only; RENEWING/REBINDING:
    ciaddr, no 50/54); RELEASE and DECLINE drop the binding. Replies go to
    ciaddr:68 when the client has an address and the broadcast flag is clear,
    else to 255.255.255.255:68; a NAK is always broadcast. Off until started."""

    def __init__(self, send, now_us):
        self.send = send            # send(src_ip, dst_ip, dport, payload, broadcast)
        self.now_us = now_us
        self.running = False
        self.server = "192.168.73.1"
        self.address = "192.168.73.10"
        self.netmask = "255.255.255.0"
        self.router = "192.168.73.1"
        self.lease_s = 3600
        self.nak_next = 0
        self.binding = None         # address bound to the client, if any
        self.log = []

    def configure(self, args):
        for k in ("server", "address", "netmask", "router"):
            if k in args:
                socket.inet_aton(args[k])
                setattr(self, k, args[k])
        if "lease_s" in args:
            self.lease_s = int(args["lease_s"])

    @staticmethod
    def _opts(data):
        o, i = {}, 240
        while i < len(data):
            t = data[i]
            if t == 255:
                break
            if t == 0:
                i += 1
                continue
            n = data[i + 1]
            o[t] = data[i + 2:i + 2 + n]
            i += 2 + n
        return o

    def _record(self, direction, kind, **kw):
        e = {"t_wall": round(time.time(), 3), "dir": direction, "type": kind}
        try:
            e["t_us"] = self.now_us()
        except Exception:  # noqa: BLE001
            e["t_us"] = None
        exact = kw.pop("t_us_exact", None)
        e.update(kw)
        if exact is not None:
            e["t_us"] = exact               # the radio's message: its emulated SEND time in the CH394
        else:
            e["t_us_approx"] = True         # read from the emulator when the bridge handled it
        if "broadcast_flag" in e:
            e["broadcast"] = e["broadcast_flag"]        # the key format v1.4 reads (dhcp.log, T-A1-74)
        self.log.append(e)

    def handle(self, src_ip, dst_ip, data, t_us=None):
        if len(data) < 240 or data[236:240] != b"\x63\x82\x53\x63":
            return
        o = self._opts(data)
        mtype = o.get(53, b"\x00")[0]
        names = {1: "DISCOVER", 3: "REQUEST", 4: "DECLINE", 7: "RELEASE", 8: "INFORM"}
        ciaddr = socket.inet_ntoa(data[12:16])
        req = socket.inet_ntoa(o[50]) if 50 in o else None
        sid = socket.inet_ntoa(o[54]) if 54 in o else None
        bflag = (data[10] & 0x80) != 0
        self._record("from_radio", names.get(mtype, str(mtype)), src=src_ip, dst=dst_ip, ciaddr=ciaddr,
                     opt50=req, opt54=sid, broadcast_flag=bflag, running=self.running, t_us_exact=t_us)
        if not self.running:
            return
        if mtype == 1:                                   # DISCOVER
            self._reply(data, 2, self.address, ciaddr, bflag)
        elif mtype == 3:                                 # REQUEST
            if sid is not None and sid != self.server:
                self.binding = None                      # the client chose another server
                return
            if self.nak_next > 0:
                self.nak_next -= 1
                self._reply(data, 6, "0.0.0.0", ciaddr, True)
                return
            if sid is not None:                          # SELECTING
                if req == self.address:
                    self.binding = self.address
                    self._reply(data, 5, self.address, ciaddr, bflag)
                else:
                    self._reply(data, 6, "0.0.0.0", ciaddr, True)
            elif req is not None:                        # INIT-REBOOT
                if req == self.address and (self.binding in (None, req)):
                    self.binding = req
                    self._reply(data, 5, req, ciaddr, bflag)
                else:
                    self._reply(data, 6, "0.0.0.0", ciaddr, True)
            elif ciaddr != "0.0.0.0":                    # RENEWING / REBINDING
                if self.binding is None:
                    self._reply(data, 6, "0.0.0.0", ciaddr, True)
                else:
                    self.binding = self.address          # dhcp.offer may have changed the address (A1.11a)
                    self._reply(data, 5, self.address, ciaddr, False)
        elif mtype in (4, 7):                            # DECLINE, RELEASE
            self.binding = None

    def _reply(self, req, mtype, yiaddr, ciaddr, broadcast):
        pkt = bytearray(240)
        pkt[0] = 2; pkt[1] = 1; pkt[2] = 6
        pkt[4:8] = req[4:8]                              # xid
        pkt[10:12] = req[10:12]                          # flags
        pkt[12:16] = req[12:16] if mtype != 6 else b"\0\0\0\0"
        pkt[16:20] = socket.inet_aton(yiaddr)
        pkt[20:24] = socket.inet_aton(self.server)
        pkt[28:44] = req[28:44]                          # chaddr
        pkt[236:240] = b"\x63\x82\x53\x63"
        o = bytearray([53, 1, mtype, 54, 4]) + socket.inet_aton(self.server)
        if mtype in (2, 5):
            L = self.lease_s
            o += bytes([51, 4]) + struct.pack("!I", L)
            o += bytes([58, 4]) + struct.pack("!I", L // 2) + bytes([59, 4]) + struct.pack("!I", L * 7 // 8)
            o += bytes([1, 4]) + socket.inet_aton(self.netmask) + bytes([3, 4]) + socket.inet_aton(self.router)
        o += b"\xff"
        pkt += o
        bcast = broadcast or mtype == 6 or ciaddr == "0.0.0.0"
        dst = "255.255.255.255" if bcast else ciaddr
        name = {2: "OFFER", 5: "ACK", 6: "NAK"}[mtype]
        self._record("to_radio", name, src=self.server, dst=dst, yiaddr=yiaddr, lease_s=self.lease_s if mtype != 6 else None,
                     broadcast_flag=(req[10] & 0x80) != 0)          # the flags field the reply carries (copied from the request)
        self.send(self.server, dst, 68, bytes(pkt), bcast)


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
        self.cycling = False
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
            try:
                self._serve(c.makefile("r"))
            except (OSError, ValueError):
                pass    # the emulator went away (power cycle, cut): accept the next one
            # no emulator until the next one connects: asks fail at once ("emulator
            # not ready") instead of waiting their timeout on a dead connection
            if self.conn is c:
                self.conn = None

    def _serve(self, f):
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

    def ask(self, line, timeout=10.0, internal=False):
        # during a power cycle only the runner's own steps (halt, flash dump) reach the
        # emulator; a bench request gets "emulator not ready" at once, not after the
        # dump (which can take longer than the bench's socket timeout)
        if self.conn is None or (self.cycling and not internal):
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
               "net.broadcast", "net.hosts", "power.cycle", "power.cut", "reset.wdt", "link.set",
               "dhcp.server",
               # the operations of dhcp.server and net.endpoint by name too: the bench's
               # fixture() looks up the operation name (cdfe899, T-A1-66 dhcp.stop)
               "dhcp.start", "dhcp.stop", "dhcp.offer", "dhcp.nak", "dhcp.expire", "dhcp.log", "net.endpoint",
               # format v1.6: datagram times and the keyer log, emulated time
               "net.log", "key.log",
               # format v1.7: datagrams placed at exact emulated offsets; waits in emulated time
               "net.inject", "time.wait", "sock.log",
               # format v1.8
               "gpio.force", "dac.force", "time.advance", "console.log"]

    def __init__(self, agent_getter, on_power_cycle, bridge_getter=None):
        self.bridge_getter = bridge_getter
        self.srv = _listen_tcp()
        self.port = self.srv.getsockname()[1]
        self.agent_getter = agent_getter
        self.on_power_cycle = on_power_cycle
        self.boot_no = 1
        self.boot_forces = []                 # (pin, level) for the next emulator's start-up script
        self.con_entries = []                 # console.log: every line since the first power-on
        self.con_next = 0
        self.con_origin = 0
        self.con_on = True
        self.sock_on = False
        self.sock_entries = []
        self.sock_next = 0
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

    def _sock_count(self, agent):
        r = agent.ask("socklog 1000000000", timeout=30, internal=True) if agent else None
        return json.loads(r[3:]).get("count", 0) if r and r.startswith("ok ") else 0

    def sock_pull(self, boot=None):
        """Append the CH394's new OPEN/CLOSE entries (sock.log); called on reads and by
        the runner before a power cycle (sock.log reads across the reset)."""
        agent = self.agent_getter()
        if not getattr(self, "sock_on", False) or not agent:
            return
        for _ in range(100):
            r = agent.ask("socklog %d" % self.sock_next, timeout=30, internal=True)
            if not r or not r.startswith("ok "):
                return
            v = json.loads(r[3:])
            for e in v.get("entries", []):
                e["boot"] = self.boot_no
                self.sock_entries.append(e)
            if v.get("next", 0) <= self.sock_next:
                return
            self.sock_next = v["next"]

    def sock_new_boot(self, boot):
        self.boot_no = boot
        self.sock_next = 0                    # the new emulator's log starts empty
        self.con_next = 0

    def console_pull(self):
        """Append the agent's new console lines (console.log); also before a power cycle."""
        agent = self.agent_getter()
        if not agent:
            return
        for _ in range(100):
            r = agent.ask("consolelog %d" % self.con_next, timeout=30, internal=True)
            if not r or not r.startswith("ok "):
                return
            v = json.loads(r[3:])
            for e in v.get("entries", []):
                e["boot"] = self.boot_no
                self.con_entries.append(e)
            if v.get("next", 0) <= self.con_next:
                return
            self.con_next = v["next"]

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
                if not r or not (r == "ok" or r.startswith("ok ")):
                    return {"ok": False, "error": r or "emulator not ready"}
                # "ok <t_us>": the emulated time the trace started; replayed boot
                # events lie before it, live events at or after it
                v = {"file": path}
                if r.startswith("ok "):
                    v["t_us"] = int(r.split()[1])
                return {"ok": True, "value": v}
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
        if op.startswith("dhcp."):
            br = self.bridge_getter() if self.bridge_getter else None
            if br is None:
                return {"ok": False, "error": "no network bridge"}
            d = br.dhcp
            try:
                if op == "dhcp.start":
                    d.configure(args); d.running = True
                elif op == "dhcp.stop":
                    d.running = False
                elif op == "dhcp.offer":
                    d.configure(args); d.running = True
                elif op == "dhcp.nak":
                    d.nak_next = int(args.get("count", 1))
                elif op == "dhcp.expire":
                    d.binding = None             # the server forgets the lease: a renewal gets NAK
                elif op == "dhcp.log":
                    return {"ok": True, "value": {"messages": list(d.log)}}
                else:
                    return {"ok": False, "error": "fixture not offered: %s" % op}
            except (ValueError, OSError) as e:
                return {"ok": False, "error": "bad arguments: %s" % e}
            return {"ok": True, "value": {"running": d.running, "server": d.server, "address": d.address,
                                          "lease_s": d.lease_s, "nak_next": d.nak_next, "binding": d.binding}}
        if op == "net.endpoint":
            br = self.bridge_getter() if self.bridge_getter else None
            return {"ok": True, "value": br.endpoint(self.port, self.OFFERED)} if br else {"ok": False, "error": "no bridge"}
        if op == "link.set":
            sp = args.get("speed")
            if sp not in (0, 10, 100):
                return {"ok": False, "error": "speed must be 0, 10 or 100"}
            r = agent.ask("link %d" % sp) if agent else None
            return {"ok": True} if r == "ok" else {"ok": False, "error": (r or "emulator not ready").replace("err ", "", 1)}
        if op == "console.log":
            action = args.get("action")
            if action not in ("start", "read", "stop"):
                return {"ok": False, "error": "action must be start|read|stop"}
            self.console_pull()
            if action == "start":
                self.con_origin = len(self.con_entries)
                self.con_on = True
                return {"ok": True, "value": {"t_us": agent.now_us() if agent else None}}
            ents = self.con_entries[self.con_origin:]
            frm = max(0, int(args.get("from", 0)))
            page = ents[frm:frm + 5000]
            if action == "stop":
                self.con_on = False
            return {"ok": True, "value": {"t_us": agent.now_us() if agent else None, "count": len(ents), "dropped": 0,
                                          "from": frm, "next": frm + len(page), "entries": page}}
        if op == "gpio.force" and args.get("at_power_on"):
            if args.get("target", "idr") != "idr":
                return {"ok": False, "error": "bad arguments: at_power_on only for target idr"}
            lv = args.get("level")
            pin = str(args.get("pin"))
            self.boot_forces = [f for f in self.boot_forces if f[0] != pin]
            if lv is not None:
                self.boot_forces.append((pin, int(lv)))
            return {"ok": True, "value": {"pin": pin, "target": "idr", "level": lv, "at_power_on": True}}
        if op == "gpio.force":
            if not agent:
                return {"ok": False, "error": "emulator not ready"}
            target = args.get("target", "idr")
            if target == "moder":
                val = args.get("mode")
                val = "null" if val is None else str(val)
                if val not in ("input", "output", "af", "analog", "null"):
                    return {"ok": False, "error": "bad arguments: mode input|output|af|analog|null"}
            else:
                lv = args.get("level")
                val = "null" if lv is None else str(int(lv))
            dur = args.get("duration_us")
            r = agent.ask("force %s %s %s %d %s" % (args.get("pin"), target, val, int(args.get("from_us", 0) or 0),
                                                   "-" if dur is None else int(dur)), timeout=40)
            if not r or not r.startswith("ok "):
                return {"ok": False, "error": (r or "emulator not ready").replace("err ", "", 1)}
            v = {"pin": args.get("pin"), "target": target, "t_us": int(r.split()[1])}
            v["mode" if target == "moder" else "level"] = args.get("mode" if target == "moder" else "level")
            return {"ok": True, "value": v}
        if op == "dac.force":
            if not agent:
                return {"ok": False, "error": "emulator not ready"}
            ch = int(args.get("channel", 0))
            if ch not in (2, 7, 8):
                return {"ok": False, "error": "bad arguments: channel 2|7|8"}
            val = args.get("value")
            r = agent.ask("dacforce %d %d %d" % (ch, -1 if val is None else int(val), int(args.get("from_us", 0) or 0)), timeout=40)
            if not r or not r.startswith("ok "):
                return {"ok": False, "error": (r or "emulator not ready").replace("err ", "", 1)}
            return {"ok": True, "value": {"channel": ch, "value": val, "t_us": int(r.split()[1])}}
        if op == "time.advance":
            if not agent:
                return {"ok": False, "error": "emulator not ready"}
            r = agent.ask("advance %d" % int(args.get("us", 0)), timeout=3700)
            if not r or not r.startswith("ok "):
                return {"ok": False, "error": (r or "emulator not ready").replace("err ", "", 1)}
            return {"ok": True, "value": {"t_us": int(r.split()[1])}}
        if op == "sock.log":
            action = args.get("action")
            if action not in ("start", "read", "stop"):
                return {"ok": False, "error": "action must be start|read|stop"}
            if action == "start":
                if not agent:
                    return {"ok": False, "error": "emulator not ready"}
                self.sock_on = True
                self.sock_entries = []
                self.sock_next = self._sock_count(agent)     # record from now
                return {"ok": True, "value": {"t_us": agent.now_us()}}
            if not getattr(self, "sock_on", False) and action == "read":
                return {"ok": False, "error": "sock.log not started"}
            self.sock_pull()
            if action == "stop":
                self.sock_on = False
            ents = getattr(self, "sock_entries", [])
            frm = max(0, int(args.get("from", 0)))
            page = ents[frm:frm + 5000]
            return {"ok": True, "value": {"t_us": agent.now_us() if agent else None, "count": len(ents), "dropped": 0,
                                          "from": frm, "next": frm + len(page), "entries": page}}
        if op == "net.inject":
            br = self.bridge_getter() if self.bridge_getter else None
            if not agent or br is None or not br.hosts:
                return {"ok": False, "error": "emulator not ready"}
            try:
                h = br.hosts[int(args.get("host", 0))]
                offs = [int(x) for x in args["at_us"]]
                pls = [str(x).lower() for x in args["payloads"]]
                fp, tp = int(args["from_port"]), int(args["to_port"])
            except (KeyError, ValueError, IndexError, TypeError) as e:
                return {"ok": False, "error": "bad arguments: %s" % e}
            if len(offs) != len(pls) or not offs or any(o < 0 for o in offs):
                return {"ok": False, "error": "bad arguments: at_us and payloads must match, offsets >= 0"}
            r = agent.ask("inject %s %d %d %s %s" % (h["ip"], fp, tp, ",".join(map(str, offs)), ",".join(pls)), timeout=40)
            if not r or not r.startswith("ok "):
                return {"ok": False, "error": (r or "emulator not ready").replace("err ", "", 1)}
            return {"ok": True, "value": {"t_us": [int(x) for x in r[3:].split(",")]}}
        if op == "time.wait":
            if not agent:
                return {"ok": False, "error": "emulator not ready"}
            try:
                us = int(args["us"])
            except (KeyError, ValueError, TypeError) as e:
                return {"ok": False, "error": "bad arguments: %s" % e}
            r = agent.ask("wait %d" % us, timeout=620)
            if not r or not r.startswith("ok "):
                return {"ok": False, "error": (r or "emulator not ready").replace("err ", "", 1)}
            return {"ok": True, "value": {"t_us": int(r.split()[1])}}
        if op in ("net.log", "key.log"):
            if not agent:
                return {"ok": False, "error": "emulator not ready"}
            action = args.get("action")
            if action not in ("start", "read", "stop"):
                return {"ok": False, "error": "action must be start|read|stop"}
            if op == "net.log":
                cmd = "netlog %s %d" % (action, int(args.get("from", 0)))
            else:
                cmd = "keylog %s %d %d" % (action, int(args.get("from_key", 0)), int(args.get("from_hp", 0)))
            r = agent.ask(cmd, timeout=30)
            if not r or not r.startswith("ok "):
                return {"ok": False, "error": (r or "emulator not ready").replace("err ", "", 1)}
            return {"ok": True, "value": json.loads(r[3:])}
        if op == "reset.wdt":
            if not agent:
                return {"ok": False, "error": "emulator not ready"}
            def resets():
                q = agent.ask("resets")
                return int(q.split()[1]) if q and q.startswith("resets ") else None
            n0 = resets()
            r = agent.ask("wdt")
            if r != "ok":
                return {"ok": False, "error": (r or "emulator not ready").replace("err ", "", 1)}
            # return once the reset has happened (the emulator applies it at its next
            # synchronisation point, and emulated time can run slower than wall time)
            t0 = time.time()
            while n0 is not None and time.time() - t0 < 30.0:
                n = resets()
                if n is not None and n > n0:
                    break
                time.sleep(0.02)
            return {"ok": True}
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
