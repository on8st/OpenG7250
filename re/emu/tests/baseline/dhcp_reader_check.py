#!/usr/bin/env python3
"""The bridge's reader thread never waits for the agent (2026-11-27), no emulator: a stand-in chip on the
CH394 link sends a DHCP DISCOVER and a status datagram behind it (type 6, with their emulated SEND times)
while the agent's clock blocks for 5 s, as during a long agent request (time.wait). The datagram must
reach the host at once, the DISCOVER be answered at once, and dhcp.log carry the DISCOVER's exact time
and the OFFER's approximate one, with the clock never asked.

Before the fix the DHCP server asked the agent for the time of every message in the reader thread, so
every radio datagram behind a DHCP message waited for the agent (T-A1-100: 8.4 s, then a burst).

  python3 tests/baseline/dhcp_reader_check.py
"""
import os
import socket
import struct
import sys
import tempfile
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "harness", "g7250emu"))
import benchnet  # noqa: E402

d = tempfile.mkdtemp(prefix="dhcpreader-")
b = benchnet.Bridge(os.path.join(d, "network.pcapng"))
fails = 0
asked = []


def check(name, ok, detail):
    global fails
    fails += not ok
    print("  %s %s (%s)" % ("ok  " if ok else "FAIL", name, detail))


def slow_clock():
    asked.append(time.time())
    time.sleep(5.0)          # the agent's lock held by a long request
    return 123


b.now_us = slow_clock
b.set_radio("192.168.73.50", "255.255.255.0")
host = b.hosts[0]
hs = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
hs.bind(("127.0.0.1", host["ports"][0]))
hs.settimeout(3.0)
chip = socket.create_connection(("127.0.0.1", b.ch_port), timeout=5)


def send6(t_us, sport, dip, dport, data):
    p = struct.pack("!Q", t_us) + struct.pack("!H", sport) + socket.inet_aton(dip) + struct.pack("!H", dport) + data
    chip.sendall(struct.pack("!BH", 6, len(p)) + p)


def discover():
    pkt = bytearray(240)
    pkt[0], pkt[1], pkt[2] = 1, 1, 6
    pkt[4:8] = b"\x12\x34\x56\x78"
    pkt[10] = 0x80
    pkt[28:34] = bytes.fromhex("020000007250")
    pkt[236:240] = b"\x63\x82\x53\x63"
    return bytes(pkt) + bytes([53, 1, 1, 255])


try:
    time.sleep(0.2)
    b.dhcp.running = True
    chip.settimeout(0.1)
    t0 = time.time()
    send6(47438559, 68, "255.255.255.255", 67, discover())
    send6(47454810, 1025, host["ip"], host["ports"][0], b"status-after-the-discover")
    try:
        data, _ = hs.recvfrom(2048)
        dt = time.time() - t0
    except socket.timeout:
        data, dt = None, None
    check("a status datagram behind a DISCOVER reaches the host at once (agent clock blocked 5 s)",
          data == b"status-after-the-discover" and dt is not None and dt < 1.0, "after %s s" % (None if dt is None else round(dt, 3)))
    got, buf, t1 = [], b"", time.time()
    while time.time() - t1 < 1.0 and not got:
        try:
            buf += chip.recv(65536)
        except socket.timeout:
            continue
        while len(buf) >= 3 and len(buf) >= 3 + struct.unpack("!H", buf[1:3])[0]:
            n = struct.unpack("!H", buf[1:3])[0]
            got.append(buf[0])
            buf = buf[3 + n:]
    check("the DISCOVER answered at once (a frame to the chip within 1 s)", bool(got), "frames %s" % got)
    log = b.dhcp.log
    kinds = [(e["type"], e.get("t_us"), e.get("t_us_approx", False)) for e in log]
    check("dhcp.log: the DISCOVER with its exact SEND time, not approximate", len(log) >= 1 and kinds[0] == ("DISCOVER", 47438559, False), kinds)
    check("dhcp.log: the OFFER with the DISCOVER's time, marked approximate", len(log) >= 2 and kinds[1] == ("OFFER", 47438559, True), kinds)
    check("the reader thread never asked the agent's clock", asked == [], "%d asks" % len(asked))
finally:
    for s in (chip, hs):
        try:
            s.close()
        except OSError:
            pass
    b.close()
print("dhcp reader check: %s" % ("ALL PASS" if not fails else "FAILURES"))
sys.exit(1 if fails else 0)
