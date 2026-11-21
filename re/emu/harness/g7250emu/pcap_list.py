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


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("testdir")
    ap.add_argument("--bytes", default="")
    ap.add_argument("--head", type=int, default=16)
    a = ap.parse_args()
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
