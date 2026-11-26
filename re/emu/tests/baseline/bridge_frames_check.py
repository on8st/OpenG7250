#!/usr/bin/env python3
"""The bridge's host-to-chip frames carry the destination IP (type 8), the capture records it, and after
the radio moves address radio.former {ip, connect} names the address before the last move, with a
connect map of its own at B + 300 + P (format v1.10). No emulator: a stand-in chip on the CH394 link.

  python3 tests/baseline/bridge_frames_check.py
"""
import os
import socket
import struct
import sys
import tempfile
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "harness", "g7250emu"))
import benchnet  # noqa: E402

d = tempfile.mkdtemp(prefix="bridgeframes-")
cap = os.path.join(d, "network.pcapng")
b = benchnet.Bridge(cap)
fails = 0


def check(name, ok, detail):
    global fails
    fails += not ok
    print("  %s %s (%s)" % ("ok  " if ok else "FAIL", name, detail))


def addr(s):
    h, p = s.rsplit(":", 1)
    return h, int(p)


chip = socket.create_connection(("127.0.0.1", b.ch_port), timeout=5)
buf = b""


def frame(timeout=3.0):
    """The next host-to-chip frame: (type, payload), or (None, None)."""
    global buf
    chip.settimeout(0.2)
    t0 = time.time()
    while time.time() - t0 < timeout:
        if len(buf) >= 3:
            n = struct.unpack("!H", buf[1:3])[0]
            if len(buf) >= 3 + n:
                t, p, buf = buf[0], buf[3:3 + n], buf[3 + n:]
                return t, p
        try:
            x = chip.recv(65536)
        except socket.timeout:
            continue
        if not x:
            break
        buf += x
    return None, None


def report_address(ip, mask="255.255.255.0"):
    """The chip reports SIPR and SUBR (type 3), as when the image writes them."""
    p = socket.inet_aton(ip) + socket.inet_aton(mask)
    chip.sendall(struct.pack("!BH", 3, len(p)) + p)
    t0 = time.time()
    while b.radio_ip != ip and time.time() - t0 < 3:
        time.sleep(0.01)


def parse(p):
    """A type 8 payload: capture time(8) srcIP(4) dstIP(4) srcPort(2) dstPort(2) flags(1) data."""
    src, dst = socket.inet_ntoa(p[8:12]), socket.inet_ntoa(p[12:16])
    sport, dport, flags = struct.unpack("!HHB", p[16:21])
    return src, dst, sport, dport, flags, p[21:]


def captured():
    """(src, dst, dport, data) of every datagram in the capture."""
    out = []
    raw = open(cap, "rb").read()
    i = 0
    while i + 12 <= len(raw):
        btype, n = struct.unpack("<II", raw[i:i + 8])
        if btype == 6:
            fr = raw[i + 28:i + 28 + struct.unpack("<I", raw[i + 20:i + 24])[0]]
            ip = fr[14:34]
            out.append((socket.inet_ntoa(ip[12:16]), socket.inet_ntoa(ip[16:20]), struct.unpack("!H", fr[36:38])[0], fr[42:]))
        i += n
    return out


try:
    time.sleep(0.2)
    report_address("192.168.73.10")
    ep = b.endpoint(0, [])
    check("before any move: no radio.former", "former" not in ep["radio"], sorted(ep["radio"]))
    h = ep["hosts"][0]
    sk = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sk.bind((h["bind"], h["ports"][0] + 3))
    sk.sendto(b"to-current", addr(ep["radio"]["connect"]["1024"]))
    t, p = frame()
    f = parse(p) if t == 8 else None
    check("a datagram to connect[1024]: frame type 8 with source host 0, destination the radio's address, port 1024",
          f is not None and f[0] == h["ip"] and f[1] == "192.168.73.10" and f[3] == 1024 and f[4] == 0 and f[5] == b"to-current",
          (t, f))
    sk.sendto(b"to-limited", addr(ep["broadcast"]["send_to"]["limited"]["1024"]))
    t, p = frame()
    f = parse(p) if t == 8 else None
    check("a limited broadcast: destination 255.255.255.255, flags bit 0", f is not None and f[1] == "255.255.255.255" and f[4] == 1, (t, f))

    report_address("192.168.73.50")
    ep2 = b.endpoint(0, [])
    fo = ep2["radio"].get("former") or {}
    fc = fo.get("connect") or {}
    others = set(ep2["radio"]["connect"].values()) | set(ep2["broadcast"]["send_to"]["limited"].values()) \
        | set(ep2["broadcast"]["send_to"]["directed"].values())
    lo, hi = h["ports"]
    check("after a move (.10 to .50): radio.ip .50, radio.former.ip .10", ep2["radio"]["ip"] == "192.168.73.50" and fo.get("ip") == "192.168.73.10",
          (ep2["radio"]["ip"], fo.get("ip")))
    check("former.connect: every spec port at B + 300 + P, none a connect, send_to or host port",
          sorted(fc) == sorted(ep2["radio"]["connect"]) and all(addr(v)[1] == b.base + 300 + int(k) for k, v in fc.items())
          and not set(fc.values()) & others and not any(b.base + 8000 <= addr(v)[1] < b.base + 8300 for v in fc.values()), fc)
    sk.sendto(b"to-former", addr(fc["1024"]))
    t, p = frame()
    f = parse(p) if t == 8 else None
    check("a datagram to former.connect[1024]: destination the old address .10, port 1024, unicast",
          f is not None and f[1] == "192.168.73.10" and f[3] == 1024 and f[4] == 0 and f[5] == b"to-former", (t, f))
    sk.sendto(b"to-new", addr(ep2["radio"]["connect"]["1024"]))
    t, p = frame()
    f = parse(p) if t == 8 else None
    check("a datagram to connect[1024] after the move: destination the new address .50", f is not None and f[1] == "192.168.73.50", (t, f))
    report_address("192.168.73.50", "255.255.0.0")
    check("a SUBR change with the same SIPR is no move: former stays .10", (b.endpoint(0, []).get("radio", {}).get("former") or {}).get("ip") == "192.168.73.10",
          b.former_ip)
    report_address("192.168.73.60")
    check("a second move (.50 to .60): radio.former.ip .50", (b.endpoint(0, []).get("radio", {}).get("former") or {}).get("ip") == "192.168.73.50",
          b.former_ip)
    time.sleep(0.2)
    caps = [(s, dd, dp, x) for s, dd, dp, x in captured() if x.startswith(b"to-")]
    check("the capture records each datagram's destination as the frame gives it",
          [(dd, x) for _, dd, _, x in caps] == [("192.168.73.10", b"to-current"), ("255.255.255.255", b"to-limited"),
                                               ("192.168.73.10", b"to-former"), ("192.168.73.50", b"to-new")], caps)
finally:
    try:
        chip.close()
    except OSError:
        pass
    b.close()
print("bridge frames check: %s" % ("ALL PASS" if not fails else "FAILURES"))
sys.exit(1 if fails else 0)
