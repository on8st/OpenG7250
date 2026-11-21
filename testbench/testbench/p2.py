"""G7250-P2 host side, written from the spec (part B) only: packet builders and a host socket
set that sends from one source port per session (A2.2, B5.5) and demultiplexes what it receives
by the radio's sender port (B2.4a).

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import select
import socket
import struct
import time

PORT_DISC, PORT_DDC, PORT_DUC, PORT_HP, PORT_DUCIQ, PORT_DDC0, PORT_REG, PORT_MGMT = (
    1024, 1025, 1026, 1027, 1029, 1035, 7250, 7251)
LEVEL1_PORTS = (1024, 1025, 1026, 1027, 1029, 1035, 7250, 7251)
ARM_MAGIC = 0x47375458
DSP_CLOCK = 122_880_000

# Table B8-2 addresses
CAP_REQUEST, STREAM_STATUS, LINK_STATUS, WRITE_STATUS = 0x0100, 0x0101, 0x0102, 0x0103
TX_ARM_TOKEN, TX_ARM, TX_ARM_STATE, TX_STATE = 0x0200, 0x0201, 0x0202, 0x0203
TX_MODE_CLASS, KEYED_TIMEOUT, WATCHDOG_STATE = 0x0204, 0x0205, 0x0206
SAFETY_STATE, FAULT_CODE, FAULT_CLEAR = 0x0210, 0x0211, 0x0215
LIMIT_TX_TIME, LIMIT_FWD_POWER, LIMIT_SWR, LIMIT_PA_TEMP = 0x0230, 0x0231, 0x0232, 0x0233
ATU_TUNE, ATU_STATE = 0x0400, 0x0401
# Table B8-3
WR = {1: "UNKNOWN_ADDRESS", 2: "READ_ONLY", 3: "NOT_OWNER", 4: "BAD_VALUE", 5: "LOOSER_LIMIT",
      6: "BUSY", 7: "NOT_CLEARABLE", 8: "KEYED", 9: "NOT_ARMED", 10: "NOT_KEYABLE",
      11: "NOT_CALIBRATED"}
WR_CODE = {v: k for k, v in WR.items()}
ARM_REJECT = {0: "none", 2: "NOT_RUNNING", 3: "WATCHDOG_DISABLED", 4: "IN_FAULT", 5: "INHIBITED",
              6: "PTT_ACTIVE", 7: "LEVEL_NOT_ACTIVE", 8: "MAINTENANCE", 9: "NO_MODE_CLASS",
              10: "BAD_TOKEN", 11: "TX_NOT_IMPLEMENTED"}


def phase(hz: float) -> int:
    return int(round(hz * 2**32 / DSP_CLOCK)) & 0xFFFFFFFF


def discovery(pad: int = 60) -> bytes:
    b = bytearray(pad)
    b[4] = 0x02
    return bytes(b)


def general(wd: bool = True, pa: bool = False, b37: int = 0x08, b39: int = 0,
            ports: dict | None = None, reg_dest: int = 0) -> bytes:
    b = bytearray(60)
    b[4] = 0x00
    for off, val in (ports or {}).items():
        struct.pack_into(">H", b, off, val)
    struct.pack_into(">H", b, 31, reg_dest)
    b[37] = b37
    b[38] = 1 if wd else 0
    b[39] = b39
    b[58] = 1 if pa else 0
    return bytes(b)


def ddc_specific(rate: int = 192, enable: bool = True, size: int = 24) -> bytes:
    b = bytearray(1444)
    b[7] = 1 if enable else 0
    struct.pack_into(">H", b, 18, rate)
    b[22] = size
    return bytes(b)


def duc_specific(rate: int = 192, bits: int = 24, eer: bool = False) -> bytes:
    b = bytearray(60)
    b[5] = 1 if eer else 0
    struct.pack_into(">H", b, 14, rate)
    b[16] = bits
    return bytes(b)


def high_priority(run: bool = True, ptt: bool = False, ddc0_hz: float = 7.1e6,
                  duc0_hz: float = 14.2e6, drive: int = 0, cwx: int = 0, seq: int = 0) -> bytes:
    b = bytearray(1444)
    struct.pack_into(">I", b, 0, seq)
    b[4] = (1 if run else 0) | (2 if ptt else 0)
    b[5] = cwx
    struct.pack_into(">I", b, 9, phase(ddc0_hz))
    struct.pack_into(">I", b, 329, phase(duc0_hz))
    b[345] = drive
    return bytes(b)


def regs(pairs) -> bytes:
    return b"\0\0\0\0" + b"".join(struct.pack(">HI", a, d) for a, d in pairs)


def parse_regs(data: bytes) -> list[tuple[int, int]]:
    n = (len(data) - 4) // 6
    return [struct.unpack_from(">HI", data, 4 + 6 * i) for i in range(n)]


class Host:
    """One P2 host: one UDP socket (one source port) for everything of its session."""

    def __init__(self, net, idx: int = 0, port: int = 0):
        """`net`: a testbench.net.Net; `idx`: the endpoint host entry; `port`: a fixed source
        port (0 = any)."""
        self.net = net
        self.radio = net.radio_ip
        self.s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
        try:
            self.port = net.bind(self.s, idx, port)
        except BaseException:
            self.s.close()
            raise

    def close(self):
        self.s.close()

    def __enter__(self):
        return self

    def __exit__(self, *a):
        self.close()

    def send(self, port: int, data: bytes, kind: str = "unicast"):
        """Send to a radio port; kind "limited" or "directed" for a broadcast."""
        self.s.sendto(data, self.net.dest(port, kind))

    def recv(self, timeout: float = 0.5, from_port: int | None = None, until=None):
        """Datagrams (src_port, data, t) until timeout; filtered by sender port if given.
        Stops early when `until(list)` is true."""
        got = []
        end = time.monotonic() + timeout
        while True:
            left = end - time.monotonic()
            if left <= 0:
                break
            r, _, _ = select.select([self.s], [], [], left)
            if not r:
                break
            data, src = self.s.recvfrom(4096)
            p = self.net.radio_port(src)
            if from_port is None or p == from_port:
                got.append((p, data, time.monotonic()))
                if until and until(got):
                    break
        return got

    def drain(self, t: float = 0.05):
        self.recv(t)

    # --- conveniences ---
    def discover(self, timeout: float = 0.5, kind: str = "unicast"):
        self.send(PORT_DISC, discovery(), kind)
        return [d for p, d, _ in self.recv(timeout, PORT_DISC)]

    running = False

    def start(self, rate: int = 192, wd: bool = True, pa: bool = False, enable: bool = True,
              reg_dest: int = 0):
        """B5.1 order: General, DDC-specific, DUC-specific, run."""
        self.send(PORT_DISC, general(wd=wd, pa=pa, reg_dest=reg_dest))
        self.send(PORT_DDC, ddc_specific(rate, enable))
        self.send(PORT_DUC, duc_specific())
        self.send(PORT_HP, high_priority(run=True))
        self.running = True

    def keepalive(self, **kw):
        self.send(PORT_HP, high_priority(run=True, **kw))
        self.running = True

    def stop(self, settle: float = 0.05):
        """Clear run, then wait: [P2] orders nothing between ports, so a General packet of the
        next test (port 1024) could otherwise overtake this stop (port 1027) and be a takeover."""
        self.send(PORT_HP, high_priority(run=False))
        self.running = False
        if settle:
            self.recv(settle)

    def recv_alive(self, timeout: float, from_port: int | None = None, every: float = 0.1):
        """recv() while sending a High Priority keepalive every 100 ms (B5.6: the host SHOULD
        send C&C every 100 ms), so that the watchdog does not end the session."""
        got, end = [], time.monotonic() + timeout
        while (left := end - time.monotonic()) > 0:
            self.keepalive()
            got += self.recv(min(every, left), from_port)
        return got

    def write(self, pairs, timeout: float = 0.3):
        """Register write; returns all register pairs received within the timeout. While this
        host runs a session, a keepalive goes first (memory-mapped packets do not feed the
        watchdog, B5.6)."""
        if self.running:
            self.keepalive()
        self.send(PORT_REG, regs(pairs))
        out = []
        for p, d, _ in self.recv(timeout, PORT_REG):
            out += parse_regs(d)
        return out

    def cap_request(self, timeout: float = 0.5, retry: bool = True):
        """CAP_REQUEST; without an answer, one retry after 1 s (B4.4 item 4: a non-owner is
        answered at most once per second)."""
        for attempt in range(2 if retry else 1):
            if attempt:
                time.sleep(1.05 - timeout if timeout < 1.05 else 0)
            self.send(PORT_REG, regs([(CAP_REQUEST, 1)]))
            for p, d, _ in self.recv(timeout, PORT_REG):
                pr = parse_regs(d)
                if pr and pr[0] == (CAP_REQUEST, 1):
                    return pr
        return None


def record_of(pairs) -> dict[int, int]:
    return {a: v for a, v in pairs if a < 0x0100}


def value(pairs, addr):
    vals = [v for a, v in pairs if a == addr]
    return vals[-1] if vals else None


def write_status(pairs):
    """List of (address, counter, reason name) of the WRITE_STATUS pairs."""
    return [((v >> 16) & 0xFFFF, (v >> 8) & 0xFF, WR.get(v & 0xFF, v & 0xFF))
            for a, v in pairs if a == WRITE_STATUS]
