#!/usr/bin/env python3
"""List the datagrams of a product run's network capture (docs/PRODUCT-RUNNER.md).

    pcap_list.py <test-dir> [--bytes 4,7,37,39] [--head 16]

One line per datagram in time order: seconds since the first datagram (wall
clock of the capture; emulated and wall time ran at about 1.00 in these runs),
direction, source and destination ip:port, length, the radio-side port, the
requested byte offsets (0-based, of the UDP payload) and the first bytes as
hex. No protocol decoding: the image's and the bench's packets are given as
bytes, for the side that knows the protocol to read.
"""
from __future__ import annotations
import argparse
import json
import os
import struct


def read(path):
    f = open(path, "rb").read()
    i, out = 0, []
    while i + 12 <= len(f):
        bt, bl = struct.unpack_from("<II", f, i)
        if bt == 6:
            th, tl, cl = struct.unpack_from("<III", f, i + 12)
            fr = f[i + 28:i + 28 + cl]
            ip = fr[14:34]
            src = ".".join(map(str, ip[12:16]))
            dst = ".".join(map(str, ip[16:20]))
            sp, dp, ln = struct.unpack_from("!HHH", fr, 34)
            out.append(((th << 32 | tl) / 1e6, src, sp, dst, dp, fr[42:42 + ln - 8]))
        i += bl
    return out


DHCP_TYPES = {1: "DISCOVER", 2: "OFFER", 3: "REQUEST", 4: "DECLINE", 5: "ACK", 6: "NAK", 7: "RELEASE", 8: "INFORM"}


def dhcp_list(testdir):
    import socket as _s
    pk = read(os.path.join(testdir, "network.pcapng"))
    rows = [p for p in pk if p[2] in (67, 68) or p[4] in (67, 68)]
    if not rows:
        print("(no DHCP messages)")
        return
    t0 = rows[0][0]
    for t, s, sp, d, dp, data in rows:
        if len(data) < 240 or data[236:240] != b"\x63\x82\x53\x63":
            continue
        opts, i = {}, 240
        while i < len(data) and data[i] != 255:
            if data[i] == 0:
                i += 1
                continue
            opts[data[i]] = data[i + 2:i + 2 + data[i + 1]]
            i += 2 + data[i + 1]
        typ = DHCP_TYPES.get(opts.get(53, b"\x00")[0], "?")
        f = lambda k: _s.inet_ntoa(opts[k]) if k in opts and len(opts[k]) == 4 else "-"
        lease = int.from_bytes(opts[51], "big") if 51 in opts else "-"
        bflag = "B" if data[10] & 0x80 else "-"
        print("%8.3f  %-8s %s:%d -> %s:%d  ciaddr %s  yiaddr %s  opt50 %s  opt54 %s  lease %s  flag %s" %
              (t - t0, typ, s, sp, d, dp, _s.inet_ntoa(data[12:16]), _s.inet_ntoa(data[16:20]),
               f(50), f(54), lease, bflag))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("testdir")
    ap.add_argument("--bytes", default="")
    ap.add_argument("--head", type=int, default=16)
    ap.add_argument("--dhcp", action="store_true",
                    help="list only the DHCP messages (ports 67/68), decoded: type, ciaddr, yiaddr, options 50, 51, 54")
    a = ap.parse_args()
    if a.dhcp:
        return dhcp_list(a.testdir)
    ep = json.load(open(os.path.join(a.testdir, "endpoint.json")))
    radio = ep["radio"]["ip"]
    offs = [int(x) for x in a.bytes.split(",") if x.strip()]
    pk = read(os.path.join(a.testdir, "network.pcapng"))
    if not pk:
        print("(no datagrams)")
        return
    t0 = pk[0][0]
    for t, s, sp, d, dp, data in pk:
        if sp in (67, 68) or dp in (67, 68):
            continue                                    # DHCP of the image, not part of the test
        frm_radio = s == radio
        rport = sp if frm_radio else dp
        dirn = "radio->host" if frm_radio else "host->radio"
        b = " ".join("b%d=%s" % (o, ("%02X" % data[o]) if o < len(data) else "--") for o in offs)
        print("%8.3f  %-11s %s:%d -> %s:%d  len %4d  radio port %d  %s  %s" %
              (t - t0, dirn, s, sp, d, dp, len(data), rport, b, data[:a.head].hex()))


if __name__ == "__main__":
    main()
