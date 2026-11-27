#!/usr/bin/env python3
"""arp.peer in the bridge (format v1.6), no emulator: a stand-in chip on the CH394 link sends the image's ARP
frames (type 10) and reads what the peers answer (type 11, or 12 while ip.conflict auto is on).

  python3 tests/baseline/arp_peer_check.py
"""
import os
import socket
import struct
import sys
import tempfile
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "harness", "g7250emu"))
import benchnet  # noqa: E402

d = tempfile.mkdtemp(prefix="arppeer-")
b = benchnet.Bridge(os.path.join(d, "network.pcapng"))
fails = 0
RMAC = bytes.fromhex("020000007250")


def check(name, ok, detail):
    global fails
    fails += not ok
    print("  %s %s (%s)" % ("ok  " if ok else "FAIL", name, detail))


chip = socket.create_connection(("127.0.0.1", b.ch_port), timeout=5)
buf = b""


def frames(timeout=1.0):
    """Every host-to-chip frame within timeout: [(type, payload)]."""
    global buf
    out = []
    chip.settimeout(0.1)
    t0 = time.time()
    while time.time() - t0 < timeout:
        while len(buf) >= 3 and len(buf) >= 3 + struct.unpack("!H", buf[1:3])[0]:
            n = struct.unpack("!H", buf[1:3])[0]
            out.append((buf[0], buf[3:3 + n]))
            buf = buf[3 + n:]
        try:
            x = chip.recv(65536)
        except socket.timeout:
            continue
        if not x:
            break
        buf += x
    return out


def radio(op, spa, tpa, dst=b"\xff" * 6):
    f = benchnet.Bridge.arp_frame(op, dst, RMAC, spa, b"\0" * 6, tpa)
    p = struct.pack("!Q", 1) + f
    chip.sendall(struct.pack("!BH", 10, len(p)) + p)


def arp(p):
    f = p[8:]
    return (f[0:6].hex(), struct.unpack("!H", f[20:22])[0], f[22:28].hex(), socket.inet_ntoa(f[28:32]), f[32:38].hex(), socket.inet_ntoa(f[38:42]))


try:
    time.sleep(0.2)
    P = "169.254.1.2"
    b.peers[P] = {"mac": b.peer_mac(P), "answer_probe": False, "defend": False}
    radio(1, "0.0.0.0", P)
    check("a probe, answer_probe false: no answer", frames() == [], "")
    b.peers[P]["answer_probe"] = True
    radio(1, "0.0.0.0", P)
    f = frames()
    check("a probe, answer_probe true: one ARP reply (type 11) to the radio's MAC, sender the peer, target 0.0.0.0",
          len(f) == 1 and f[0][0] == 11 and arp(f[0][1]) == ("020000007250", 2, "020000009902", P, "020000007250", "0.0.0.0"), [(t, arp(p)) for t, p in f])
    radio(1, "169.254.9.9", P)
    f = frames()
    check("an ordinary request for the peer's address: a reply to its sender", len(f) == 1 and arp(f[0][1])[1] == 2 and arp(f[0][1])[5] == "169.254.9.9",
          [arp(p) for _, p in f])
    radio(1, P, P)
    check("the radio announces the peer's address, defend false: no answer", frames() == [], "")
    b.peers[P]["defend"] = True
    radio(1, P, P)
    f = frames()
    check("defend true: the peer announces it back (broadcast, sender and target its address)",
          len(f) == 1 and arp(f[0][1]) == ("ffffffffffff", 1, "020000009902", P, "000000000000", P), [arp(p) for _, p in f])
    radio(2, P, "169.254.9.9", dst=bytes.fromhex("020000009902"))
    f = frames()
    check("the radio uses the peer's address in a reply: defended too", len(f) == 1 and arp(f[0][1])[1] == 1, [arp(p) for _, p in f])
    radio(1, "0.0.0.0", "169.254.7.7")
    check("a probe of an address no peer holds: no answer", frames() == [], "")
    b.conflict_auto = True
    radio(1, "0.0.0.0", P)
    f = frames()
    check("ip.conflict auto on: the peers' frames go as type 12", len(f) == 1 and f[0][0] == 12, [t for t, _ in f])
    b.conflict_auto = False
    t0 = time.time()
    b.peer_announce(P, b.peers[P]["mac"], 2)
    f = frames(3.5)
    check("announce count 2 (no radio clock here: 2 s of host time apart): two announcements",
          len(f) == 2 and all(arp(p)[1] == 1 and arp(p)[3] == P for _, p in f), [arp(p) for _, p in f])
    check("a MAC given is used; a bad one refused", b.peer_mac(P, "02:aa:bb:cc:dd:ee") == bytes.fromhex("02aabbccddee"), "")
    try:
        b.peer_mac(P, "02:aa")
        check("a five-byte MAC refused", False, "accepted")
    except ValueError:
        check("a five-byte MAC refused", True, "")
    radio(1, "0.0.0.0", P)
    raw = open(os.path.join(d, "network.pcapng"), "rb").read()
    check("the peers' frames and the radio's are in the capture", raw.count(b"\x08\x06\x00\x01\x08\x00\x06\x04") >= 10, "")
finally:
    try:
        chip.close()
    except OSError:
        pass
    b.close()
print("arp peer check: %s" % ("ALL PASS" if not fails else "FAILURES"))
sys.exit(1 if fails else 0)
