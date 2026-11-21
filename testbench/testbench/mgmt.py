"""Management channel client (spec A1.4, A1.10; payloads provisional as in
product/refmodel/docs/mgmt-payloads.md). GPL-3.0-or-later, (c) 2026 ON8ST."""
from __future__ import annotations

import hashlib
import hmac
import select
import socket
import struct
import time

MAGIC = 0x47374D47
CMD = dict(IDENTIFY=0x0001, GET_STATS=0x0002, GET_CHALLENGE=0x0010, SET_FIRST_SECRET=0x0011,
           CHANGE_SECRET=0x0012, END_WRITE_SESSION=0x0013, GET_CONFIG=0x0020, SET_CONFIG=0x0021,
           FACTORY_RESET=0x0022, SWITCH_LEVEL=0x0023, RESTART=0x0024, READ_LOG=0x0030,
           CLEAR_LOG=0x0031, UPDATE_BEGIN=0x0040, UPDATE_STATUS=0x0045, ROLLBACK=0x0046,
           RETURN_TO_STOCK=0x0050, ENTER_VENDOR_RECOVERY=0x0051, GET_CALIBRATION=0x0060,
           SET_CALIBRATION=0x0061, CLEAR_CALIBRATION=0x0062)

# Annex P freeze rule: every parameter of groups P-RF, P-PWR, P-SEN, P-TIM, and P-TX-1, P-ATU-1,
# P-ANT-1 (Table P-1).
FREEZE_SET = ([f"P-RF-{i}" for i in range(1, 6)] + [f"P-PWR-{i}" for i in range(1, 6)]
              + [f"P-SEN-{i}" for i in range(1, 6)] + ["P-TIM-1", "P-TIM-2", "P-RX-2", "P-TX-1", "P-ATU-1",
                                                         "P-ANT-1"])  # P-RX-2 joined with 0042


def calibration_payload(entries: dict[str, bytes], date: int = 0, source: str = "testbench") -> bytes:
    """SET_CALIBRATION payload (Annex M Tables M-1, M-5)."""
    b = struct.pack(">IB", date, len(source)) + source.encode()
    for k, v in entries.items():
        b += struct.pack(">B", len(k)) + k.encode() + struct.pack(">H", len(v)) + v
    return b


def config_tlv(*items: tuple[int, bytes]) -> bytes:
    """SET_CONFIG payload (Annex M Table M-3)."""
    return b"".join(bytes([t, len(v)]) + v for t, v in items)


def build(cmd: int, payload: bytes = b"", rid: int = 1, major: int = 1, minor: int = 0,
          typ: int = 1, flags: int = 0, declared_n: int | None = None) -> bytes:
    n = len(payload) if declared_n is None else declared_n
    return struct.pack(">IBBBBIHHH", MAGIC, major, minor, typ, flags, rid, cmd, 0, n) + payload


def parse(d: bytes) -> dict | None:
    if len(d) < 18 or struct.unpack_from(">I", d)[0] != MAGIC:
        return None
    magic, ma, mi, typ, flags, rid, cmd, status, n = struct.unpack_from(">IBBBBIHHH", d)
    return dict(major=ma, minor=mi, type=typ, flags=flags, id=rid, cmd=cmd, status=status,
                payload=d[18:18 + n], raw=d)


def tlv(payload: bytes) -> dict[int, bytes]:
    out, i = {}, 0
    while i + 2 <= len(payload):
        t, ln = payload[i], payload[i + 1]
        out[t] = payload[i + 2:i + 2 + ln]
        i += 2 + ln
    return out


class Mgmt:
    def __init__(self, net, idx: int = 0, port: int = 0):
        self.net = net
        self.radio = net.radio_ip
        self.s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
        try:
            self.port = net.bind(self.s, idx, port)
        except BaseException:
            self.s.close()
            raise
        self.rid = int(time.time() * 1000) & 0xFFFF

    def close(self):
        self.s.close()

    def __enter__(self):
        return self

    def __exit__(self, *a):
        self.close()

    def raw(self, data: bytes, timeout: float = 0.5, kind: str = "unicast") -> list[dict]:
        self.s.sendto(data, self.net.dest(7251, kind))
        got, end = [], time.monotonic() + timeout
        while (left := end - time.monotonic()) > 0:
            r, _, _ = select.select([self.s], [], [], left)
            if not r:
                break
            d, src = self.s.recvfrom(4096)
            if self.net.radio_port(src) == 7251:
                got.append(parse(d) or {"raw": d})
                break
        return got

    def next_id(self) -> int:
        self.rid = (self.rid + 1) & 0xFFFFFFFF
        return self.rid

    def req(self, cmd: str, payload: bytes = b"", timeout: float = 0.5, rid: int | None = None):
        r = self.raw(build(CMD[cmd], payload, rid if rid is not None else self.next_id()), timeout)
        return r[0] if r else None

    def challenge(self) -> bytes | None:
        r = self.req("GET_CHALLENGE")
        return r["payload"] if r and r["status"] == 0 else None

    def authed(self, cmd: str, secret: bytes, payload: bytes = b"", challenge: bytes | None = None,
               rid: int | None = None, timeout: float = 0.5, mangle: bool = False):
        ch = challenge if challenge is not None else self.challenge()
        if ch is None:
            return None
        msg = build(CMD[cmd], payload, rid if rid is not None else self.next_id(), flags=1)
        mac = hmac.new(secret, ch + msg, hashlib.sha256).digest()
        if mangle:
            mac = bytes([mac[0] ^ 1]) + mac[1:]
        r = self.raw(msg + mac, timeout)
        return r[0] if r else None


def identify(ctx, c: Mgmt) -> dict[int, bytes]:
    """IDENTIFY TLVs (Annex M Table M-2), or a failed test."""
    r = ctx.got(c.req("IDENTIFY"), "IDENTIFY", "no reply on port 7251 within 0.5 s")
    if r.get("status") != 0:
        from .core import Missing
        raise Missing(f"IDENTIFY: status {r.get('status')}")
    return tlv(r["payload"])


def parse_calibration(payload: bytes):
    """GET_CALIBRATION reply (Annex M Table M-1, M-5): (set id, date, source, {id: value})."""
    set_id = payload[:32]
    date, sl = struct.unpack_from(">IB", payload, 32)
    source = payload[37:37 + sl].decode("ascii", "replace")
    i, entries = 37 + sl, {}
    while i < len(payload):
        il = payload[i]
        k = payload[i + 1:i + 1 + il].decode("ascii", "replace")
        vl = struct.unpack_from(">H", payload, i + 1 + il)[0]
        entries[k] = payload[i + 3 + il:i + 3 + il + vl]
        i += 3 + il + vl
    return set_id, date, source, entries


class SavedCalibration:
    """Save the radio's calibration set at the start of a test and put it back at the end, so a
    test that writes or clears it leaves the radio as it found it (with the secret in force then)."""

    def __init__(self, ctx, c: Mgmt):
        self.ctx, self.c = ctx, c
        g = ctx.got(c.req("GET_CALIBRATION"), "GET_CALIBRATION", "no reply on port 7251 within 0.5 s")
        self.set_id, self.date, self.source, self.entries = parse_calibration(g["payload"])

    @property
    def empty(self) -> bool:
        return self.set_id == bytes(32)

    def restore(self, secret: bytes):
        import time
        for _ in range(4):  # 57 while the test's P2 session still runs: it ends by its watchdog in 1 s
            if self.empty:
                r = self.c.authed("CLEAR_CALIBRATION", secret)
            else:
                r = self.c.authed("SET_CALIBRATION", secret, calibration_payload(self.entries, self.date, self.source))
            if not r or r.get("status") != 57:
                break
            time.sleep(0.6)
        if not r or r.get("status") != 0:
            self.ctx.notes.append(f"calibration set not restored: {None if not r else r.get('status')}")
        self.c.authed("END_WRITE_SESSION", secret)
