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
           CLEAR_LOG=0x0031, UPDATE_BEGIN=0x0040, UPDATE_BLOCK=0x0041, UPDATE_VERIFY=0x0042,
           UPDATE_ACTIVATE=0x0043, UPDATE_ABORT=0x0044, UPDATE_STATUS=0x0045, ROLLBACK=0x0046,
           RETURN_TO_STOCK=0x0050, ENTER_VENDOR_RECOVERY=0x0051, GET_CALIBRATION=0x0060,
           SET_CALIBRATION=0x0061, CLEAR_CALIBRATION=0x0062)

# Annex P freeze rule: every parameter of groups P-RF, P-PWR, P-SEN, P-TIM, and P-TX-1, P-ATU-1,
# P-ANT-1 (Table P-1).
FREEZE_SET = ([f"P-RF-{i}" for i in range(1, 6)] + [f"P-PWR-{i}" for i in range(1, 6)]
              + [f"P-SEN-{i}" for i in range(1, 6)] + ["P-TIM-1", "P-TIM-2", "P-RX-2", "P-TX-1", "P-ATU-1",
                                                         "P-ANT-1", "P-SYS-9"])  # P-RX-2: 0042; P-SYS-9: 0050


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

    pace = 0  # requests per second at most from this socket (0 = unpaced; A1.10 S4 caps at 20)
    clock = None  # core.RadioClock: pacing and reply waits also in radio time (an emulator)
    why = ""  # why the last retransmitted request has no reply

    def raw(self, data: bytes, timeout: float = 0.5, kind: str = "unicast") -> list[dict]:
        self.post(data, kind)
        return self.listen(data, timeout)

    def post(self, data: bytes, kind: str = "unicast"):
        """Send one datagram, paced on the wall clock and, on an emulator, in radio time."""
        if self.pace and self.clock is not None and self.clock.emu:
            # the radio counts its cap in radio time: at most `pace` datagrams in any second of it
            rt = self.__dict__.setdefault("_sent_rt", [])
            now, w0 = self.clock.now(), time.monotonic()
            while len(rt) >= self.pace and now - rt[-self.pace] < 1.0 and time.monotonic() - w0 < 20:
                time.sleep(0.02)
                now = self.clock.now()
            rt.append(now)
            del rt[:-self.pace]
        if self.pace:
            sent = self.__dict__.setdefault("_sent", [])
            now = time.monotonic()
            sent[:] = [t for t in sent if now - t < 1.0]
            if len(sent) >= self.pace:
                time.sleep(max(0.0, 1.0 - (now - sent[0])) + 0.005)
            sent.append(time.monotonic())
        self.s.sendto(data, self.net.dest(7251, kind))

    def listen(self, data: bytes, timeout: float) -> list[dict]:
        """The reply to the request `data`, within timeout seconds of wall time; [] if none."""
        # only the reply to this request: a late reply to an earlier one (same socket) is not its
        # answer and is counted in self.stale (a framing-error reply, 16-19, may not carry the id)
        sent = parse(data)
        got, end = [], time.monotonic() + timeout
        while (left := end - time.monotonic()) > 0:
            r, _, _ = select.select([self.s], [], [], left)
            if not r:
                break
            d, src = self.s.recvfrom(4096)
            if self.net.radio_port(src) == 7251:
                rep = parse(d)
                if sent and rep and rep["id"] != sent["id"] and rep["status"] not in (16, 17, 18, 19):
                    self.stale = getattr(self, "stale", 0) + 1
                    continue
                got.append(rep or {"raw": d})
                break
        return got

    def next_id(self) -> int:
        self.rid = (self.rid + 1) & 0xFFFFFFFF
        return self.rid

    def once(self, cmd: str, data: bytes, timeout: float | None):
        """One datagram and a wait of `timeout` (0.5 s); a request that may do flash work as 0069
        tells a host: retransmitted every 500 ms for 16 s (a wait given by the caller stays)."""
        if timeout is None and cmd in FLASH_CMDS:
            return send_retransmit(self, data, FLASH_WINDOW_S, cmd)
        r = self.raw(data, 0.5 if timeout is None else timeout)
        return r[0] if r else None

    def req(self, cmd: str, payload: bytes = b"", timeout: float | None = None, rid: int | None = None):
        return self.once(cmd, build(CMD[cmd], payload, rid if rid is not None else self.next_id()), timeout)

    def challenge(self) -> bytes | None:
        r = self.req("GET_CHALLENGE")
        return r["payload"] if r and r["status"] == 0 else None

    def authed(self, cmd: str, secret: bytes, payload: bytes = b"", challenge: bytes | None = None,
               rid: int | None = None, timeout: float | None = None, mangle: bool = False):
        ch = challenge
        if ch is None and cmd in FLASH_CMDS:
            r = req_retransmit(self, "GET_CHALLENGE")
            ch = r["payload"] if r and r["status"] == 0 else None
        elif ch is None:
            ch = self.challenge()
        if ch is None:
            return None
        msg = build(CMD[cmd], payload, rid if rid is not None else self.next_id(), flags=1)
        mac = hmac.new(secret, ch + msg, hashlib.sha256).digest()
        if mangle:
            mac = bytes([mac[0] ^ 1]) + mac[1:]
        return self.once(cmd, msg + mac, timeout)


RETRANSMIT_S, RETRANSMITS = 0.5, 5  # A1.4 host retransmission: every 500 ms, up to 5 times (0041)


FLASH_WINDOW_S = 16.0  # a request that may do flash work: at least 16 s (0069)
FLASH_CMDS = {"SET_CONFIG", "SET_FIRST_SECRET", "CHANGE_SECRET", "FACTORY_RESET", "SET_CALIBRATION", "CLEAR_CALIBRATION",
              "UPDATE_BEGIN", "UPDATE_BLOCK", "UPDATE_ACTIVATE", "ROLLBACK"}


def send_retransmit(c: "Mgmt", data: bytes, window: float | None = None, what: str = "request"):
    """One request datagram sent as a host does (A1.4): the same bytes, so the same request id,
    every 500 ms, up to 5 retransmissions (a DHCP borrow of at most 2 s is covered, A1.11), or,
    with `window`, for that many seconds (0069). Each wait lasts 500 ms of wall time and, on an
    emulator, also 500 ms of radio time (an erase can run at a fifth of real time); `window` counts
    radio time. The reply, or None with the reason in c.why."""
    clock = c.clock if c.clock is not None and c.clock.emu else None
    w0, t0, sends = time.monotonic(), None, 0
    limit = window if window is not None else (1 + RETRANSMITS) * RETRANSMIT_S
    cap = limit * 12 + 60  # wall: the emulator stands still
    while True:
        r = c.raw(data, timeout=RETRANSMIT_S)
        sends += 1
        if r:
            c.why = ""
            return r[0]
        if clock is not None:  # no reply in 500 ms of wall time: look at the radio's clock
            t_send = clock.now() if t0 is None else t_send_next
            if t0 is None:
                t0 = t_send  # the first wait is not counted: never shorter than the rule
            while clock.now() - t_send < RETRANSMIT_S and time.monotonic() - w0 < cap:
                r = c.listen(data, 0.1)
                if r:
                    c.why = ""
                    return r[0]
            t_send_next = clock.now()
            spent = t_send_next - t0
        else:
            spent = time.monotonic() - w0
        if (spent >= limit if window is not None else sends > RETRANSMITS) or time.monotonic() - w0 >= cap:
            c.why = (f"no reply to {what} in {sends} sends over {spent:.1f} s of radio time, {time.monotonic() - w0:.1f} s wall"
                     f" ({getattr(c, 'stale', 0)} replies to earlier ids set aside so far)")
            return None


def req_retransmit(c: "Mgmt", cmd: str, payload: bytes = b""):
    """An unauthenticated request with A1.4 retransmission (16 s where it may do flash work)."""
    return send_retransmit(c, build(CMD[cmd], payload, c.next_id()), FLASH_WINDOW_S if cmd in FLASH_CMDS else None, cmd)


def signed(c: "Mgmt", cmd: str, secret: bytes, payload: bytes = b"") -> bytes | None:
    """The signed request datagram, with a fresh challenge (GET_CHALLENGE retransmitted per A1.4);
    None with the reason in c.why when the radio gives no challenge."""
    r = req_retransmit(c, "GET_CHALLENGE")
    if not r or r["status"] != 0:
        c.why = f"{cmd} not sent: " + (f"GET_CHALLENGE answered {r['status']}" if r else c.why)
        return None
    msg = build(CMD[cmd], payload, c.next_id(), flags=1)
    return msg + hmac.new(secret, r["payload"] + msg, hashlib.sha256).digest()


def authed_retransmit(c: "Mgmt", cmd: str, secret: bytes, payload: bytes = b"", window: float | None = None):
    """An authenticated request with A1.4 retransmission: GET_CHALLENGE retransmitted, then the
    signed request retransmitted unchanged (same id, same challenge and MAC: a retransmission the
    radio already answered is answered from its stored reply)."""
    data = signed(c, cmd, secret, payload)
    if data is None:
        return None
    if window is None and cmd in FLASH_CMDS:
        window = FLASH_WINDOW_S
    t0 = c.clock.now() if getattr(c, "timed", False) and c.clock is not None else None
    r = send_retransmit(c, data, window, cmd)
    if t0 is not None:
        c.reply_s = c.clock.now() - t0  # at most this long, radio time
    return r


def identify(ctx, c: Mgmt) -> dict[int, bytes]:
    """IDENTIFY TLVs (Annex M Table M-2), or a failed test."""
    r = ctx.got(req_retransmit(c, "IDENTIFY"), "IDENTIFY",
                f"no reply on port 7251 to the request and {RETRANSMITS} retransmissions 500 ms apart (A1.4)")
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


def parse_stats(payload: bytes) -> dict[str, int]:
    """GET_STATS reply (Table M-1): repeated name length (u8), name (ASCII), value (u64)."""
    out, i = {}, 0
    while i < len(payload):
        n = payload[i]
        name = payload[i + 1:i + 1 + n].decode("ascii", "replace")
        out[name] = int.from_bytes(payload[i + 1 + n:i + 9 + n], "big")
        i += 9 + n
    return out


def parse_log(payload: bytes) -> list[tuple[int, int, str]]:
    """READ_LOG reply (Table M-1): repeated sequence (u32), uptime ms (u32), text length (u8),
    text (ASCII)."""
    out, i = [], 0
    while i + 9 <= len(payload):
        seq = int.from_bytes(payload[i:i + 4], "big")
        up = int.from_bytes(payload[i + 4:i + 8], "big")
        n = payload[i + 8]
        out.append((seq, up, payload[i + 9:i + 9 + n].decode("ascii", "replace")))
        i += 9 + n
    return out


def stats(c: "Mgmt") -> dict[str, int] | None:
    r = c.req("GET_STATS")
    return parse_stats(r["payload"]) if r and r.get("status") == 0 else None


def read_log(c: "Mgmt", first: int = 0) -> list[tuple[int, int, str]]:
    """Every log entry from sequence `first` on, page by page (L-3)."""
    out: list = []
    while True:
        r = req_retransmit(c, "READ_LOG", first.to_bytes(4, "big"))
        if not r or r.get("status") != 0:
            return out
        page = parse_log(r["payload"])
        if not page:
            return out
        out += page
        first = page[-1][0] + 1
