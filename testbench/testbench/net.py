"""Address mapping between the test bench and the endpoint (testlist-format v1.1 section 3).

Without mapping fields the bench talks to `radio.ip` on the spec's ports and binds the host
addresses itself. An emulator (or any runner that relays datagrams) can instead give:

- `radio.connect`: {radio port: "addr:port"}: where the bench sends a datagram meant for that
  radio port; datagrams from that address are read as coming from that radio port;
- `hosts[i].bind` and `hosts[i].ports` [lo, hi]: the local address and port range the bench
  binds for host i; the radio sees `hosts[i].ip` with the same port;
- `broadcast.send_to`: {"limited"|"directed": {radio port: "addr:port"}}.

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import socket

from .core import Skip


def _addr(s: str) -> tuple[str, int]:
    h, p = s.rsplit(":", 1)
    return h, int(p)


class Net:
    def __init__(self, ep: dict):
        self.ep = ep
        self.radio_ip = ep["radio"]["ip"]
        self.connect = {int(k): _addr(v) for k, v in ep["radio"].get("connect", {}).items()}
        self.back = {v: k for k, v in self.connect.items()}
        self.hosts = ep.get("hosts") or [{"ip": "0.0.0.0", "in_subnet": True}]
        bc = ep.get("broadcast", {})
        self.bc = bc
        self.send_to = {kind: {int(k): _addr(v) for k, v in m.items()}
                        for kind, m in bc.get("send_to", {}).items()}

    def host_index(self, in_subnet: bool = True, n: int = 0) -> int:
        """Index of the n-th host entry inside (or outside) the subnet; Skip if none."""
        idx = [i for i, h in enumerate(self.hosts) if h.get("in_subnet", True) == in_subnet]
        if len(idx) <= n:
            raise Skip("fixture:net.hosts (" + ("in" if in_subnet else "outside") + " subnet)")
        return idx[n]

    def bind(self, s: socket.socket, idx: int = 0, port: int = 0) -> int:
        h = self.hosts[idx]
        ip = h.get("bind", h["ip"])
        lo_hi = h.get("ports")
        if port or not lo_hi:
            s.bind((ip, port))
            return s.getsockname()[1]
        for p in range(lo_hi[0], lo_hi[1] + 1):
            try:
                s.bind((ip, p))
                return p
            except OSError:
                continue
        raise Skip(f"no free port in {lo_hi} for host {idx}")

    def dest(self, port: int, kind: str = "unicast") -> tuple[str, int]:
        if kind == "unicast":
            return self.connect.get(port, (self.radio_ip, port))
        if kind in self.send_to and port in self.send_to[kind]:
            return self.send_to[kind][port]
        if kind == "limited" and self.bc.get("limited"):
            return ("255.255.255.255", port)
        if kind == "directed" and self.bc.get("directed"):
            return (self.bc["directed"], port)
        raise Skip(f"fixture:net.broadcast ({kind})")

    def radio_port(self, src: tuple[str, int]) -> int:
        """The radio port a received datagram comes from."""
        return self.back.get((src[0], src[1]), src[1])
