"""Tests of part B: discovery (B3), record (B4), session (B5), streams (B6), fields (B7),
register map (B8). Each test leaves the radio without a session. GPL-3.0-or-later, (c) 2026 ON8ST."""
from __future__ import annotations

import socket
import struct
import time

from .core import Ctx, Skip, test
from .p2 import *  # noqa: F401,F403


def host(ctx: Ctx) -> Host:
    return Host(ctx.radio_ip, ctx.host_ip)


def idle(h: Host):
    h.stop()
    h.drain(0.05)


# ---------------- B3 discovery ----------------
@test("T-B3-02")
def t_b3_02(ctx):
    with host(ctx) as h:
        ctx.eq("one reply to a unicast discovery", len(h.discover(1.0)), 1)


@test("T-B3-04")
def t_b3_04(ctx):
    with host(ctx) as h:
        h.send(PORT_DISC, discovery())
        got = h.recv(0.5)
        ctx.check("reply from port 1024", got and all(p == 1024 for p, _, _ in got), str([p for p, _, _ in got]))


@test("T-B3-05", "T-B3-10", "T-B3-11", "T-B3-13", "T-B3-16", "T-B3-24")
def t_b3_reply_bytes(ctx):
    with host(ctx) as h:
        r = h.discover()
        if not ctx.check("a reply", len(r) == 1):
            return
        d = r[0]
        ctx.eq("payload 60 bytes (B3.3)", len(d), 60)
        ctx.eq("bytes 0-3 zero", d[0:4], b"\0\0\0\0")
        ctx.eq("byte 11 board type 200", d[11], 200)
        ctx.eq("byte 12 protocol 44", d[12], 44)
        ctx.eq("bytes 14-19 zero", d[14:20], bytes(6))
        ctx.eq("bytes 24-59 zero", d[24:60], bytes(36))
        ctx.eq("byte 22 endian modes 0", d[22], 0)
        ctx.check("byte 4 not 0xFE/0xFF", d[4] in (2, 3), hex(d[4]))
        ctx.check("byte 11 not 254/255", d[11] not in (254, 255))


@test("T-B3-06")
def t_b3_06(ctx):
    with host(ctx) as h:
        r = h.discover()
        ctx.eq("byte 4 idle", r[0][4] if r else None, 2)


@test("T-B3-07", "T-B3-22")
def t_b3_07(ctx):
    with host(ctx) as a, host(ctx) as b:
        a.start()
        time.sleep(0.05)
        r = b.discover()
        ctx.eq("byte 4 for another host (port) while running", r[0][4] if r else None, 3)
        a.keepalive()
        r = a.discover()
        ctx.eq("byte 4 for the owner", r[0][4] if r else None, 2)
        idle(a)


@test("T-B3-08")
def t_b3_08(ctx):
    with host(ctx) as a:
        a.start()
        time.sleep(0.05)
        r = a.discover()
        ctx.eq("byte 4 for the owner on the same port", r[0][4] if r else None, 2)
        idle(a)


@test("T-B3-12", "T-B3-30")
def t_b3_12(ctx):
    with host(ctx) as h:
        d = h.discover()[0]
        rec = record_of(h.cap_request() or [])
        fw = rec.get(0x08)
        if not ctx.check("record read", fw is not None):
            return
        maj, mino = fw >> 24, (fw >> 16) & 0xFF
        ctx.eq("byte 13 = major*10+minor (capped)", d[13], min(maj * 10 + mino, 255))
        ctx.eq("byte 12 = p2_version", d[12], rec.get(0x09))


@test("T-B3-14")
def t_b3_14(ctx):
    with host(ctx) as h:
        d = h.discover()[0]
        rec = record_of(h.cap_request() or [])
        ctx.eq("byte 20 = num_ddc", d[20], rec.get(0x0E))
        ctx.eq("one DDC in this version (0004)", d[20], 1)


@test("T-B3-17")
def t_b3_17(ctx):
    with host(ctx) as h:
        h.send(PORT_DISC, general(b39=0x01))
        r = h.discover()
        ctx.check("answered, Big-Endian layout", len(r) == 1 and r[0][11] == 200)


@test("T-B3-18")
def t_b3_18(ctx):
    with host(ctx) as h:
        b = bytearray(b"\x11" * 60)
        b[4] = 2
        h.send(PORT_DISC, bytes(b))
        r = h.recv(0.5, PORT_DISC)
        ctx.eq("answered with non-zero bytes 5-59", len(r), 1)
        h.send(PORT_DISC, bytes(b[:5]))
        ctx.eq("answered with a 5-byte request", len(h.recv(0.5, PORT_DISC)), 1)


@test("T-B3-20", "T-B11-03")
def t_b3_20(ctx):
    with host(ctx) as h:
        for cmd in (0x04, 0x05, 0x07, 0x7F):
            b = bytearray(264)
            b[4] = cmd
            h.send(PORT_DISC, bytes(b))
        ctx.eq("no reply to Erase/Program/unknown", h.recv(0.5), [])
        ctx.eq("still answers discovery", len(h.discover()), 1)


@test("T-B3-21")
def t_b3_21(ctx):
    """Firmware rule: nothing answers outside Table B2-1 (B11.9). Over UDP we can only check
    that no datagram comes back; ICMP is not visible to an unprivileged socket."""
    with host(ctx) as h:
        for p in (1028, 1030, 1036, 1100, 7249, 7252, 50001):
            h.send(p, b"\0" * 64)
        ctx.eq("no datagram from closed ports", h.recv(0.5), [])


@test("T-B3-26")
def t_b3_26(ctx):
    with host(ctx) as h:
        b = bytearray(60)
        b[4] = 0x03
        b[5:11] = bytes.fromhex("0affffffff01")
        b[11:15] = bytes([127, 0, 0, 99])
        h.send(PORT_DISC, bytes(b))
        ctx.eq("Set IP with another MAC: no reply", h.recv(0.5), [])
        ctx.eq("address unchanged", len(h.discover()), 1)


@test("T-B3-31")
def t_b3_31(ctx):
    with host(ctx) as h:
        for _ in range(100):
            h.send(PORT_DISC, discovery())
        got = h.recv(1.0, PORT_DISC)
        ctx.check("at most 10 replies per second per source", 1 <= len(got) <= 10, f"{len(got)} replies")


@test("T-B3-33")
def t_b3_33(ctx):
    try:
        h = Host(ctx.radio_ip, ctx.host_ip, 1024)
    except OSError as e:
        raise Skip(f"cannot bind source port 1024 here: {e}")
    with h:
        h.send(PORT_DISC, discovery())
        ctx.eq("no reply to a discovery from source port 1024", h.recv(0.5), [])


# ---------------- B4 record ----------------
@test("T-B4-01", "T-B4-02", "T-B4-03", "T-B4-12")
def t_b4_record(ctx):
    with host(ctx) as h:
        pr = h.cap_request()
        if not ctx.check("record answered in one packet", pr is not None):
            return
        rec = record_of(pr[1:])
        ctx.eq("echo first", pr[0], (CAP_REQUEST, 1))
        ctx.eq("cap_magic", rec.get(0), 0x47373235)
        ctx.eq("cap_version 1.1", rec.get(1), 0x00010001)
        ctx.eq("cap_length 207", rec.get(2), 207)
        ctx.eq("cap_length = registers sent", len(rec), rec.get(2))
        ctx.eq("regmap_version 1.0", rec.get(3), 0x00010000)
        name = b"".join(struct.pack(">I", rec.get(i, 0)) for i in range(4, 8)).rstrip(b"\0")
        ctx.eq("product_name", name, b"G7250-P2")
        ctx.eq("mgmt", rec.get(0x52), 0x1C530001)
        ctx.eq("dsp_clock_hz", rec.get(0x12), 122_880_000)
        ctx.eq("link 100", rec.get(0x54) & 0xFFFF, 100)


@test("T-B4-04", "T-B4-13")
def t_b4_04(ctx):
    with host(ctx) as h, host(ctx) as other:
        pr = h.cap_request()
        ctx.check("answered to its source", pr is not None)
        d = other.discover()
        ctx.eq("no session started (discovery byte 4)", d[0][4] if d else None, 2)
        h.send(PORT_REG, regs([(KEYED_TIMEOUT, 200)]))
        ws = write_status(sum((parse_regs(x) for _, x, _ in h.recv(0.3, PORT_REG)), []))
        ctx.check("still not the owner", any(r == "NOT_OWNER" for _, _, r in ws), str(ws))


@test("T-B4-06")
def t_b4_06(ctx):
    with host(ctx) as h:
        h.start()
        time.sleep(0.05)
        h.drain(0.1)
        ans = h.write([(0x0000, 0x12345678), (0x0020, 0xFFFFFFFF)])
        ctx.eq("magic answered unchanged", value(ans, 0x0000), 0x47373235)
        ws = write_status(ans)
        ctx.check("READ_ONLY for both", [r for _, _, r in ws] == ["READ_ONLY", "READ_ONLY"], str(ws))
        idle(h)


@test("T-B4-07")
def t_b4_07(ctx):
    with host(ctx) as h:
        h.start()
        got = h.recv(0.5, PORT_REG)
        recs = [d for _, d, _ in got if any(a == 0 and v == 0x47373235 for a, v in parse_regs(d))]
        ctx.eq("unasked record once at session start", len(recs), 1)
        h.send(PORT_DISC, general())
        got = h.recv(0.3, PORT_REG)
        recs = [d for _, d, _ in got if any(a == 0 and v == 0x47373235 for a, v in parse_regs(d))]
        ctx.eq("not again on the next General", len(recs), 0)
        idle(h)


@test("T-B4-10")
def t_b4_10(ctx):
    with host(ctx) as h:
        rec = record_of(h.cap_request()[1:])
        n_rx = rec[0x13]
        ctx.check("unused rx_range pairs zero", all(rec[0x14 + i] == 0 for i in range(2 * n_rx, 8)))
        n_tx = rec[0x21]
        ctx.check("unused tx_band entries zero", all(rec[0x22 + i] == 0 for i in range(3 * n_tx, 48)))
        ctx.eq("ddc_rates bits 4, 5 zero (no 768/1536)", rec[0x10] & 0x30, 0)
        ctx.eq("streams bit 2 DDC audio zero", rec[0x0C] & 0x4, 0)


@test("T-B4-11")
def t_b4_11(ctx):
    with host(ctx) as h:
        ans = h.write([(CAP_REQUEST, 2)], 0.5)
        ctx.check("no record", value(ans, 0x0000) is None)
        ctx.check("BAD_VALUE", any(r == "BAD_VALUE" for _, _, r in write_status(ans)), str(write_status(ans)))


@test("T-B4-17")
def t_b4_17(ctx):
    with host(ctx) as h:
        ctx.check("first answered", h.cap_request() is not None)
        time.sleep(0.2)
        ctx.check("repeat after 200 ms dropped", h.cap_request(0.3) is None)
        time.sleep(0.6)
        ctx.check("repeat after 1 s answered", h.cap_request() is not None)


# ---------------- B5 session ----------------
@test("T-B5-01")
def t_b5_01(ctx):
    with host(ctx) as h:
        h.start()
        time.sleep(0.2)
        h.drain(0.01)
        got = h.recv(1.6)
        last = max((t for p, _, t in got if p in (PORT_DDC0, PORT_DDC)), default=None)
        first = min((t for p, _, t in got), default=None)
        ctx.check("streams stop after the watchdog", last is not None and last - first < 1.2,
                  f"last stream packet {None if last is None else round(last - first, 3)} s")
        d = h.discover()
        ctx.eq("standby: discovery byte 4", d[0][4] if d else None, 2)


@test("T-B5-02")
def t_b5_02(ctx):
    with host(ctx) as h:
        h.start()
        for _ in range(15):
            time.sleep(0.1)
            h.send(PORT_REG, regs([(KEYED_TIMEOUT, 500)]))
        h.drain(0.05)
        got = h.recv(0.3, PORT_DDC)
        ctx.eq("memory-mapped packets alone do not keep the session", len(got), 0)


@test("T-B5-05")
def t_b5_05(ctx):
    with host(ctx) as a, host(ctx) as a2:
        a.start()
        time.sleep(0.05)
        a2.keepalive()
        time.sleep(0.05)
        d = a.discover()
        ctx.eq("old port is no longer the owner", d[0][4] if d else None, 3)
        idle(a2)


@test("T-B5-10")
def t_b5_10(ctx):
    with host(ctx) as h:
        h.start()
        for _ in range(3):
            time.sleep(0.9)
            h.keepalive()
        h.drain(0.01)
        ctx.check("session kept with C&C every 900 ms", len(h.recv(0.2, PORT_DDC)) > 0)
        t0 = time.monotonic()
        got = h.recv(1.5, PORT_DDC)
        last = max((t for _, _, t in got), default=t0)
        ctx.check("standby within 1100 ms of the last C&C", last - t0 <= 1.15, f"{round(last - t0, 3)} s")


@test("T-B5-11")
def t_b5_11(ctx):
    with host(ctx) as h:
        h.send(PORT_DDC, ddc_specific(192))
        h.keepalive()
        got = h.recv(0.3, PORT_DDC0)
        ctx.check("DDC0 to the sender without a discovery", len(got) > 0, f"{len(got)} packets")
        idle(h)


# ---------------- B6 streams ----------------
@test("T-B6-01")
def t_b6_01(ctx):
    with host(ctx) as h:
        h.send(PORT_DISC, general())
        h.send(PORT_DDC, ddc_specific(192))
        ctx.eq("no DDC0 before run", h.recv(0.3, PORT_DDC0), [])


@test("T-B6-02", "T-B6-03", "T-B6-04", "T-B6-05", "T-B6-25")
def t_b6_stream(ctx):
    with host(ctx) as h, host(ctx) as disc:
        disc.discover()
        h.start(192)
        got = h.recv(0.6)
        ddc = [(d, t) for p, d, t in got if p == PORT_DDC0]
        if not ctx.check("DDC0 packets from port 1035 to the session socket", len(ddc) > 50, f"{len(ddc)}"):
            idle(h)
            return
        ctx.check("every payload 1444 bytes", all(len(d) == 1444 for d, _ in ddc))
        ctx.check("bytes 12-13 = 24, 14-15 = 238",
                  all(struct.unpack_from(">HH", d, 12) == (24, 238) for d, _ in ddc))
        seqs = [struct.unpack_from(">I", d)[0] for d, _ in ddc]
        ctx.eq("sequence starts at 0", seqs[0], 0)
        ctx.check("increments by one", all(b - a == 1 for a, b in zip(seqs, seqs[1:])))
        ports = {p for p, _, _ in got}
        ctx.check("only ports of B2.4a", ports <= {1025, 1035, 7250}, str(sorted(ports)))
        idle(h)


@test("T-B6-06")
def t_b6_06(ctx):
    with host(ctx) as h:
        h.start()
        h.recv(0.2)
        h.stop()
        h.drain(0.1)
        h.start()
        got = h.recv(0.2, PORT_DDC0)
        ctx.eq("sequence 0 after stop and run", struct.unpack_from(">I", got[0][1])[0] if got else None, 0)
        idle(h)


@test("T-B6-08")
def t_b6_08(ctx):
    with host(ctx) as h:
        h.start(48)
        h.recv_alive(0.2)
        got = h.recv_alive(1.0, PORT_DDC0)
        want = 48000 / 238
        ctx.check("packet rate = rate/238 (±10 %)", abs(len(got) - want) < 0.1 * want, f"{len(got)} in 1 s, want {want:.0f}")
        idle(h)


def _stream_status(h: Host, timeout=0.3):
    """Read STREAM_STATUS: a push comes only on change, so read it by writing it (B8.4a)."""
    h.recv(0.05)
    return value(h.write([(STREAM_STATUS, 0)], timeout), STREAM_STATUS)


@test("T-B6-10")
def t_b6_10(ctx):
    with host(ctx) as h:
        h.start(192)
        h.recv(0.1)
        for rate in (768, 1536, 100):
            h.send(PORT_DDC, ddc_specific(rate))
            ss = _stream_status(h)
            h.drain(0.05)
            ctx.check(f"rate {rate}: DDC0 stops", len(h.recv(0.15, PORT_DDC0)) == 0)
            ctx.eq(f"rate {rate}: reason 3", None if ss is None else ss & 0x7F, 3)
            h.keepalive()
        h.send(PORT_DDC, ddc_specific(96))
        h.drain(0.05)
        ctx.check("valid rate resumes", len(h.recv(0.2, PORT_DDC0)) > 0)
        idle(h)


@test("T-B6-13")
def t_b6_13(ctx):
    with host(ctx) as h:
        h.start(192)
        h.recv(0.1)
        h.send(PORT_DDC, ddc_specific(192, enable=False))
        h.drain(0.05)
        ctx.eq("enable bit clear stops DDC0", len(h.recv(0.2, PORT_DDC0)), 0)
        h.send(PORT_DDC, ddc_specific(192))
        ctx.check("set resumes", len(h.recv(0.2, PORT_DDC0)) > 0)
        idle(h)


@test("T-B6-18", "T-B6-19", "T-B6-20", "T-B7-04")
def t_b6_refusals(ctx):
    cases = [("sample size 16", lambda h: h.send(PORT_DDC, ddc_specific(192, size=16)), 4),
             ("byte 39 little-endian", lambda h: h.send(PORT_DISC, general(b39=0x02)), 5),
             ("byte 37 time stamp", lambda h: h.send(PORT_DISC, general(b37=0x09)), 6),
             ("byte 37 VNA", lambda h: h.send(PORT_DISC, general(b37=0x0C)), 11)]
    for name, act, reason in cases:
        with host(ctx) as h:
            h.start(192)
            h.recv(0.1)
            act(h)
            ss = _stream_status(h)
            h.drain(0.05)
            ctx.check(f"{name}: DDC0 stops", len(h.recv(0.15, PORT_DDC0)) == 0)
            ctx.eq(f"{name}: reason", None if ss is None else ss & 0x7F, reason)
            idle(h)


@test("T-B6-26")
def t_b6_26(ctx):
    with host(ctx) as h:
        h.send(PORT_DISC, general())
        h.keepalive()
        got = h.recv(0.3)
        ctx.check("status packets run", any(p == PORT_DDC for p, _, _ in got))
        ctx.check("no DDC0 without a DDC-specific packet", not any(p == PORT_DDC0 for p, _, _ in got))
        h.send(PORT_DDC, ddc_specific(192))
        ctx.check("DDC-specific starts DDC0", len(h.recv(0.2, PORT_DDC0)) > 0)
        idle(h)


@test("T-B7-03")
def t_b7_03(ctx):
    with host(ctx) as h:
        h.send(PORT_DISC, general(ports={17: 2000}))
        h.send(PORT_DDC, ddc_specific(192))
        h.keepalive()
        got = h.recv(0.4)
        ctx.check("DDC0 still from 1035", any(p == PORT_DDC0 for p, _, _ in got))
        ls = None
        for p, d, _ in got:
            if p == PORT_REG:
                ls = value(parse_regs(d), LINK_STATUS) or ls
        ans = h.write([(LINK_STATUS, 0)])
        ls = value(ans, LINK_STATUS)
        ctx.check("LINK_STATUS bit 17 set", ls is not None and ls & (1 << 17), hex(ls or 0))
        idle(h)


# ---------------- B8 register map ----------------
@test("T-B8-04")
def t_b8_04(ctx):
    with host(ctx) as h:
        h.start()
        h.drain(0.2)
        t0 = time.monotonic()
        h.send(PORT_REG, regs([(KEYED_TIMEOUT, 300), (TX_MODE_CLASS, 2), (KEYED_TIMEOUT, 200)]))
        got = h.recv(0.3, PORT_REG)
        pairs = sum((parse_regs(d) for _, d, _ in got), [])
        ctx.eq("answered in write order", [p for p in pairs if p[0] in (KEYED_TIMEOUT, TX_MODE_CLASS)],
               [(KEYED_TIMEOUT, 300), (TX_MODE_CLASS, 2), (KEYED_TIMEOUT, 200)])
        ctx.check("within 5 ms (+ host jitter)", got and got[0][2] - t0 < 0.02, f"{(got[0][2] - t0) * 1000:.1f} ms" if got else "")
        idle(h)


@test("T-B8-05")
def t_b8_05(ctx):
    with host(ctx) as h:
        h.start()
        h.drain(0.2)
        ans = h.write([(0x0F00, 1), (TX_STATE, 1), (KEYED_TIMEOUT, 50), (LIMIT_TX_TIME, 999_999)])
        ws = [(a, r) for a, _, r in write_status(ans)]
        ctx.eq("reasons", ws, [(0x0F00, "UNKNOWN_ADDRESS"), (TX_STATE, "READ_ONLY"),
                               (KEYED_TIMEOUT, "BAD_VALUE"), (LIMIT_TX_TIME, "LOOSER_LIMIT")])
        ctx.eq("value in force for KEYED_TIMEOUT", value(ans, KEYED_TIMEOUT), 500)
        cnt = [c for _, c, _ in write_status(ans)]
        ctx.check("rejection counter increments", all((b - a) % 256 == 1 for a, b in zip(cnt, cnt[1:])), str(cnt))
        idle(h)


@test("T-B8-06", "T-B8-11")
def t_b8_06(ctx):
    with host(ctx) as a, host(ctx) as b:
        a.start()
        a.drain(0.2)
        b.send(PORT_REG, regs([(KEYED_TIMEOUT, 200)]))
        b.send(PORT_REG, regs([(KEYED_TIMEOUT, 200)]))
        got = b.recv(0.08, PORT_REG)
        ans = sum((parse_regs(d) for _, d, _ in got), [])
        ctx.check("NOT_OWNER to the writer", any(r == "NOT_OWNER" for _, _, r in write_status(ans)))
        ctx.eq("one answer for two writes within 100 ms", len(got), 1)
        d = b.discover()
        ctx.eq("no takeover", d[0][4] if d else None, 3)
        a.keepalive()
        ans = a.write([(KEYED_TIMEOUT, 300)])
        ctx.eq("owner's value unaffected", value(ans, KEYED_TIMEOUT), 300)
        idle(a)


@test("T-B8-07")
def t_b8_07(ctx):
    with host(ctx) as h:
        h.start()
        h.drain(0.2)
        h.send(PORT_REG, regs([(0xFFFF, 5), (0x0000, 0)]) + b"\x02\x05\x00")
        ctx.eq("padding and a trailing incomplete pair not answered", h.recv(0.2, PORT_REG), [])
        idle(h)


@test("T-B8-08")
def t_b8_08(ctx):
    with host(ctx) as h, host(ctx) as dest:
        h.start(reg_dest=dest.port)
        got_dest = dest.recv(0.4, PORT_REG)
        ctx.check("register packets go to General bytes 31-32", len(got_dest) > 0)
        idle(h)


@test("T-B8-19")
def t_b8_19(ctx):
    try:
        h7 = Host(ctx.radio_ip, ctx.host_ip, 7250)
    except OSError as e:
        raise Skip(f"cannot bind source port 7250 here: {e}")
    with h7:
        h7.send(PORT_REG, regs([(KEYED_TIMEOUT, 200)]))
        h7.send(PORT_REG, regs([(CAP_REQUEST, 1)]))
        ctx.eq("no answer to a non-owner from port 7250", h7.recv(0.3), [])
