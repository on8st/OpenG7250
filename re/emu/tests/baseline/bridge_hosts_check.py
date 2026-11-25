#!/usr/bin/env python3
"""The bridge's host addresses are fixed for the boot (no emulator): a lease that moves the radio below .20 in the
same subnet keeps the hosts' source addresses (T-A1-100 on fw-481c8ab saw host 0 move from .20 to .21), a SUBR of
0.0.0.0 changes nothing, and they are worked out again only when the radio moves to another subnet or onto a
host's own address.

  python3 tests/baseline/bridge_hosts_check.py
"""
import os
import sys
import tempfile

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "harness", "g7250emu"))
import benchnet  # noqa: E402

d = tempfile.mkdtemp(prefix="bridgehosts-")
b = benchnet.Bridge(os.path.join(d, "network.pcapng"))
fails = 0


def hosts():
    return [h["ip"] for h in b.hosts]


def check(name, ok, detail):
    global fails
    fails += not ok
    print("  %s %s (%s)" % ("ok  " if ok else "FAIL", name, detail))


try:
    b.set_radio("0.0.0.0", "255.255.255.0")
    b.set_radio("169.254.109.173", "255.255.0.0")
    ll = hosts()
    check("link-local /16: the hosts are the 20th and 21st of the subnet", ll[:2] == ["169.254.0.20", "169.254.0.21"], ll)
    b.set_radio("169.254.109.173", "0.0.0.0")
    check("SUBR 0.0.0.0 (a DHCP borrow): the hosts stay", hosts() == ll and b.radio_ip == "169.254.109.173", hosts())
    b.set_radio("192.168.73.50", "255.255.255.0")
    st = hosts()
    check("a static address in another subnet: the hosts are worked out again", st[:2] == ["192.168.73.20", "192.168.73.21"], st)
    b.set_radio("192.168.73.10", "255.255.255.0")
    check("a lease that moves the radio below .20 in the same subnet: the hosts keep their addresses",
          hosts() == st and b.radio_ip == "192.168.73.10", hosts())
    ep = b.endpoint(0, [])
    check("the endpoint gives the same hosts and the new radio address",
          [h["ip"] for h in ep["hosts"]] == st and ep["radio"]["ip"] == "192.168.73.10", (ep["radio"]["ip"], [h["ip"] for h in ep["hosts"]]))
    b.set_radio("192.168.73.20", "255.255.255.0")
    check("the radio on host 0's own address: worked out again, no host on the radio's address",
          "192.168.73.20" not in hosts()[:2] and hosts()[:2] == ["192.168.73.21", "192.168.73.22"], hosts())
finally:
    b.close()
print("bridge hosts check: %s" % ("ALL PASS" if not fails else "FAILURES"))
sys.exit(1 if fails else 0)
