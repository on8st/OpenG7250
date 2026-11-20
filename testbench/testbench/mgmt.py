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
           RETURN_TO_STOCK=0x0050, ENTER_VENDOR_RECOVERY=0x0051)


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
    def __init__(self, radio_ip: str, bind_ip: str = "0.0.0.0", port: int = 0):
        self.radio = radio_ip
        self.s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
        self.s.bind((bind_ip, port))
        self.port = self.s.getsockname()[1]
        self.rid = int(time.time() * 1000) & 0xFFFF

    def close(self):
        self.s.close()

    def __enter__(self):
        return self

    def __exit__(self, *a):
        self.close()

    def raw(self, data: bytes, timeout: float = 0.5, ip: str | None = None) -> list[dict]:
        self.s.sendto(data, (ip or self.radio, 7251))
        got, end = [], time.monotonic() + timeout
        while (left := end - time.monotonic()) > 0:
            r, _, _ = select.select([self.s], [], [], left)
            if not r:
                break
            d, (src, p) = self.s.recvfrom(4096)
            if p == 7251:
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
