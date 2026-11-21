"""Tests of part B: discovery (B3), record (B4), session (B5), streams (B6), fields (B7),
register map (B8). Each test leaves the radio without a session. GPL-3.0-or-later, (c) 2026 ON8ST."""
from __future__ import annotations

import socket
import struct
import time

from .core import Ctx, Skip, test
from .p2 import *  # noqa: F401,F403


def host(ctx: Ctx) -> Host:
    return Host(ctx.net)


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
        if not ctx.check("a reply", len(r) == 1, f"{len(r)} replies on port 1024 within 0.5 s"):
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
        ctx.check("byte 11 not 254/255", d[11] not in (254, 255), f"byte 11 = {d[11]}")


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
        d = need_discovery(ctx, h)
        rec = need_record(ctx, h)
        fw = rec.get(0x08)
        if not ctx.check("record read", fw is not None, "fw_version (0x08) absent" if fw is None else hex(fw)):
            return
        maj, mino = fw >> 24, (fw >> 16) & 0xFF
        ctx.eq("byte 13 = major*10+minor (capped)", d[13], min(maj * 10 + mino, 255))
        ctx.eq("byte 12 = p2_version", d[12], rec.get(0x09))


@test("T-B3-14")
def t_b3_14(ctx):
    with host(ctx) as h:
        d = need_discovery(ctx, h)
        ctx.eq("byte 20: one DDC in this version (0004; Table B3-1)", d[20], 1)
        rec = need_record(ctx, h)
        ctx.eq("byte 20 = record num_ddc (B4.8)", d[20], rec[0x0E])


@test("T-B3-17")
def t_b3_17(ctx):
    with host(ctx) as h:
        h.send(PORT_DISC, general(b39=0x01))
        r = h.discover()
        ctx.check("answered, Big-Endian layout", len(r) == 1 and r[0][11] == 200, f"{len(r)} replies" + (f", byte 11 = {r[0][11]}" if r else ""))


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
        ctx.eq("no reply to Erase/Program/unknown (datagrams)", len(h.recv(0.5)), 0)
        ctx.eq("still answers discovery", len(h.discover()), 1)


@test("T-B3-21")
def t_b3_21(ctx):
    """Firmware rule: nothing answers outside Table B2-1 (B11.9). Over UDP we can only check
    that no datagram comes back; ICMP is not visible to an unprivileged socket."""
    closed = [1028, 1030, 1036, 1100, 7249, 7252, 50001]
    if ctx.net.connect:  # v1.3 mapping: only ports the runner maps are reachable
        closed = [p for p in closed if p in ctx.net.connect]
        if not closed:
            raise Skip("radio.connect maps no closed port; to test B11.9 the runner can map "
                       "closed ports too (for example 1028)")
    with host(ctx) as h:
        for p in closed:
            h.send(p, b"\0" * 64)
        got = h.recv(0.5)
        ctx.eq(f"no datagram from closed ports {closed}", [(p, len(d)) for p, d, _ in got], [])


@test("T-B3-26")
def t_b3_26(ctx):
    with host(ctx) as h:
        b = bytearray(60)
        b[4] = 0x03
        b[5:11] = bytes.fromhex("0affffffff01")
        b[11:15] = bytes([127, 0, 0, 99])
        h.send(PORT_DISC, bytes(b))
        ctx.eq("Set IP with another MAC: no reply (datagrams)", len(h.recv(0.5)), 0)
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
        h = Host(ctx.net, port=1024)
    except OSError as e:
        raise Skip(f"cannot bind source port 1024 here: {e}")
    with h:
        h.send(PORT_DISC, discovery())
        ctx.eq("no reply to a discovery from source port 1024 (datagrams)", len(h.recv(0.5)), 0)
    with host(ctx) as h2:  # liveness: the same request from another port is answered
        ctx.eq("a discovery from another port is answered", len(h2.discover()), 1)


# ---------------- B4 record ----------------
@test("T-B4-01", "T-B4-02", "T-B4-03", "T-B4-12")
def t_b4_record(ctx):
    with host(ctx) as h:
        pr = h.cap_request()
        if not ctx.check("record answered in one packet", pr is not None, "no answer to CAP_REQUEST (with one retry after 1 s)"):
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
        ctx.check("answered to its source", pr is not None, "no answer to CAP_REQUEST at the request's source" if pr is None else "answered")
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
        # by register: the record may come in parts (0059), only the first holds register 0
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
        rec = need_record(ctx, h)
        n_rx = rec[0x13]
        ctx.all_of("unused rx_range pairs zero", [(hex(0x14 + i), rec[0x14 + i]) for i in range(2 * n_rx, 8)], lambda x: x[1] == 0)
        n_tx = rec[0x21]
        ctx.all_of("unused tx_band entries zero", [(hex(0x22 + i), rec[0x22 + i]) for i in range(3 * n_tx, 48)], lambda x: x[1] == 0)
        ctx.eq("ddc_rates bits 4, 5 zero (no 768/1536)", rec[0x10] & 0x30, 0)
        ctx.eq("streams bit 2 DDC audio zero", rec[0x0C] & 0x4, 0)


@test("T-B4-11")
def t_b4_11(ctx):
    with host(ctx) as h:
        ans = h.write([(CAP_REQUEST, 2)], 0.5)
        ctx.check("no record", value(ans, 0x0000) is None, "record register 0x00 in the answer" if value(ans, 0x0000) is not None else "no record")
        ctx.check("BAD_VALUE", any(r == "BAD_VALUE" for _, _, r in write_status(ans)), str(write_status(ans)))


@test("T-B4-17")
def t_b4_17(ctx):
    with host(ctx) as h:
        ctx.check("first answered", h.cap_request(retry=False) is not None, "no answer within 0.5 s")
        time.sleep(0.2)
        ctx.check("repeat after 200 ms dropped", h.cap_request(0.3, retry=False) is None, "answered")
        time.sleep(0.6)
        ctx.check("repeat after 1 s answered", h.cap_request(retry=False) is not None, "no answer within 0.5 s")


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
        # liveness first: a radio that answers nothing would pass the negative check below
        if not ctx.at_least("session started (status packets)", len(h.recv(0.2, PORT_DDC)), what="status packets in 0.2 s"):
            return
        for _ in range(15):
            time.sleep(0.1)
            h.send(PORT_REG, regs([(KEYED_TIMEOUT, 500)]))
        h.drain(0.05)
        got = h.recv(0.3, PORT_DDC)
        ctx.eq("memory-mapped packets alone do not keep the session (status packets after 1.5 s)", len(got), 0)


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
        ctx.at_least("session kept with C&C every 900 ms", len(h.recv(0.2, PORT_DDC)), what="status packets in 0.2 s")
        t0 = time.monotonic()
        got = h.recv(1.5, PORT_DDC)
        last = max((t for _, _, t in got), default=t0)
        ctx.check("standby within 1100 ms of the last C&C", last - t0 <= 1.15, f"{round(last - t0, 3)} s")


@test("T-B5-11")
def t_b5_11(ctx):
    with host(ctx) as h:
        need_ddc0(ctx, h)
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
        ctx.eq("no DDC0 before run (packets in 0.3 s)", len(h.recv(0.3, PORT_DDC0)), 0)
        h.keepalive()  # liveness: the same set-up with run starts the session
        if ddc0_offered(ctx, h):
            ctx.at_least("DDC0 after run", len(h.recv(0.3, PORT_DDC0)), what="DDC0 packets in 0.3 s")
        else:  # before P-RX-2 the session runs without DDC0 (B6.1)
            ss = _stream_status(h)
            ctx.eq("after run, before P-RX-2: DDC0 reason 2 (not offered)", None if ss is None else ss & 0x7F, 2)
        idle(h)


@test("T-B6-02", "T-B6-03", "T-B6-04", "T-B6-05", "T-B6-25")
def t_b6_stream(ctx):
    with host(ctx) as h, host(ctx) as disc:
        need_ddc0(ctx, h)
        disc.discover()
        h.start(192)
        got = h.recv(0.6)
        ddc = [(d, t) for p, d, t in got if p == PORT_DDC0]
        if not ctx.check("DDC0 packets from port 1035 to the session socket", len(ddc) > 50, f"{len(ddc)}"):
            idle(h)
            return
        ctx.all_of("every payload 1444 bytes", [len(d) for d, _ in ddc], lambda n: n == 1444)
        ctx.all_of("bytes 12-13 = 24, 14-15 = 238", [struct.unpack_from(">HH", d, 12) for d, _ in ddc],
                   lambda x: x == (24, 238))
        seqs = [struct.unpack_from(">I", d)[0] for d, _ in ddc]
        ctx.eq("sequence starts at 0", seqs[0], 0)
        ctx.all_of("increments by one", list(zip(seqs, seqs[1:])), lambda x: x[1] - x[0] == 1)
        ports = {p for p, _, _ in got}
        ctx.check("only ports of B2.4a", ports <= {1025, 1035, 7250}, str(sorted(ports)))
        idle(h)


@test("T-B6-06")
def t_b6_06(ctx):
    with host(ctx) as h:
        need_ddc0(ctx, h)
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
        need_ddc0(ctx, h)
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
        rec = need_record(ctx, h)
        offered = bool(rec[0x0C] & 1)
        want = 3 if offered else 2  # before P-RX-2 reason 2 comes before 3 (Table B8-3 order; 0042)
        mask = rec.get(0x10, 0)
        bits = {48: 0, 96: 1, 192: 2, 384: 3}
        rates = [r for r, b in bits.items() if not mask >> b & 1] + [768, 1536, 100]
        for rate in rates:
            h.send(PORT_DDC, ddc_specific(rate))
            ss = _stream_status(h)
            h.drain(0.05)
            ctx.eq(f"rate {rate}: DDC0 stops (packets in 0.15 s)", len(h.recv(0.15, PORT_DDC0)), 0)
            ctx.eq(f"rate {rate}: reason {want}", None if ss is None else ss & 0x7F, want)
            h.keepalive()
        if offered:
            h.send(PORT_DDC, ddc_specific(96))
            h.drain(0.05)
            ctx.at_least("valid rate resumes", len(h.recv(0.2, PORT_DDC0)), what="DDC0 packets in 0.2 s")
        else:
            ctx.notes.append("before P-RX-2: reason 2 checked; the resume part needs P-RX-2")
        idle(h)


@test("T-B6-13")
def t_b6_13(ctx):
    with host(ctx) as h:
        need_ddc0(ctx, h)
        h.start(192)
        h.recv(0.1)
        h.send(PORT_DDC, ddc_specific(192, enable=False))
        h.drain(0.05)
        ctx.eq("enable bit clear stops DDC0", len(h.recv(0.2, PORT_DDC0)), 0)
        h.send(PORT_DDC, ddc_specific(192))
        ctx.at_least("set resumes", len(h.recv(0.2, PORT_DDC0)), what="DDC0 packets in 0.2 s")
        idle(h)


@test("T-B6-18", "T-B6-19", "T-B6-20", "T-B7-04")
def t_b6_refusals(ctx):
    # before P-RX-2 the sample size gives reason 2 (2 before 4), the General-packet refusals keep
    # their reasons (they come before 2; Table B8-3 order, 0042)
    with host(ctx) as h0:
        size_reason = 4 if ddc0_offered(ctx, h0) else 2
    cases = [("sample size 16", lambda h: h.send(PORT_DDC, ddc_specific(192, size=16)), size_reason),
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
            ctx.eq(f"{name}: DDC0 stops (packets in 0.15 s)", len(h.recv(0.15, PORT_DDC0)), 0)
            ctx.eq(f"{name}: reason", None if ss is None else ss & 0x7F, reason)
            idle(h)


@test("T-B6-26")
def t_b6_26(ctx):
    with host(ctx) as h:
        h.send(PORT_DISC, general())
        h.keepalive()
        got = h.recv(0.3)
        ctx.at_least("status packets run", sum(p == PORT_DDC for p, _, _ in got), what="status packets in 0.3 s")
        ctx.check("no DDC0 without a DDC-specific packet", not any(p == PORT_DDC0 for p, _, _ in got), f"{sum(p == PORT_DDC0 for p, _, _ in got)} DDC0 packets")
        ss = _stream_status(h)
        ctx.eq("run without a DDC-specific packet: DDC0 reason 1", None if ss is None else ss & 0x7F, 1)
        h.send(PORT_DDC, ddc_specific(192))
        if ddc0_offered(ctx, h):
            ctx.at_least("DDC-specific starts DDC0", len(h.recv(0.2, PORT_DDC0)), what="DDC0 packets in 0.2 s")
        else:
            ctx.notes.append("before P-RX-2: the start of DDC0 needs P-RX-2 (B6.1)")
        idle(h)


@test("T-B7-03")
def t_b7_03(ctx):
    with host(ctx) as h:
        h.send(PORT_DISC, general(ports={17: 2000}))
        h.send(PORT_DDC, ddc_specific(192))
        h.keepalive()
        got = h.recv(0.4)
        if ddc0_offered(ctx, h):
            ctx.at_least("DDC0 still from 1035", sum(p == PORT_DDC0 for p, _, _ in got), what="DDC0 packets from 1035 in 0.4 s")
        else:
            ctx.notes.append("before P-RX-2: the DDC0 port part needs P-RX-2 (B6.1)")
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
        ctx.check("NOT_OWNER to the writer", any(r == "NOT_OWNER" for _, _, r in write_status(ans)), f"WRITE_STATUS seen: {write_status(ans)}")
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
        ctx.eq("padding and a trailing incomplete pair not answered (register packets)", len(h.recv(0.2, PORT_REG)), 0)
        # liveness: a real pair in the same session is answered
        ctx.eq("a real write is answered", value(h.write([(KEYED_TIMEOUT, 400)]), KEYED_TIMEOUT), 400)
        idle(h)


@test("T-B8-08")
def t_b8_08(ctx):
    with host(ctx) as h, host(ctx) as dest:
        h.start(reg_dest=dest.port)
        got_dest = dest.recv(0.4, PORT_REG)
        ctx.check("register packets go to General bytes 31-32", len(got_dest) > 0, f"{len(got_dest)} register packets at port {dest.port} within 0.4 s")
        idle(h)


@test("T-B8-19")
def t_b8_19(ctx):
    try:
        h7 = Host(ctx.net, port=7250)
    except OSError as e:
        raise Skip(f"cannot bind source port 7250 here: {e}")
    with h7:
        h7.send(PORT_REG, regs([(KEYED_TIMEOUT, 200)]))
        h7.send(PORT_REG, regs([(CAP_REQUEST, 1)]))
        ctx.eq("no answer to a non-owner from port 7250", h7.recv(0.3), [])


@test("T-B6-27")
def t_b6_27(ctx):
    """Before P-RX-2: streams bit 0 = 0, DDC0 enable gets reason 2 and no packets, the rest answers;
    with P-RX-2 set DDC0 runs (B6.1; 0042)."""
    with host(ctx) as h:
        streams = need_record(ctx, h)[0x0C]
        offered = bool(streams & 1)
        ctx.notes.append(f"record streams = {streams:#x}")
        h.start(192)
        got = h.recv(0.4)
        ss = _stream_status(h)
        n = sum(p == PORT_DDC0 for p, _, _ in got)
        if not offered:
            ctx.eq("DDC0 enabled: STREAM_STATUS DDC0 reason 2, not running", None if ss is None else ss & 0xFF, 2)
            ctx.eq("no DDC0 packets", n, 0)
            ctx.eq("discovery answered", len(h.discover()), 1)
        else:
            ctx.notes.append("P-RX-2 set here: checked that DDC0 runs")
            ctx.at_least("with P-RX-2 set, DDC0 runs", n, what="DDC0 packets in 0.4 s")
            ctx.check("STREAM_STATUS DDC0 running", ss is not None and ss & 0x80, hex(ss or 0))
        idle(h)
    from .mgmt import Mgmt, identify
    with Mgmt(ctx.net) as c:
        ctx.eq("management channel answers (IDENTIFY model)", identify(ctx, c).get(1), b"G7250")


def running_seen_by(obs: Host, timeout: float = 0.3) -> int | None:
    """Discovery byte 4 as another host sees it: 3 while a session runs, 2 idle (B3.3)."""
    r = obs.discover(timeout)
    return r[0][4] if r else None


def status_seqs(h: Host, t: float):
    return [struct.unpack_from(">I", d, 0)[0] for p, d, _ in h.recv(t) if p == PORT_DDC]


def restarted(last_before: int, after: list[int]) -> bool:
    """The status sequence restarts (drops to 0 or 1) at some point after last_before; a packet
    sent before the stop may still come first (the 20 packets take about 22 ms)."""
    prev = last_before
    for x in after:
        if x < prev and x <= 1:
            return True
        prev = x
    return False


def time_scale(ctx) -> float:
    """Emulated seconds per wall second, from the endpoint (1.0 if not given): the bench's waits
    and host spacing are stretched by 1 / time_scale so that they hold in the radio's time."""
    ts = float(ctx.endpoint.get("time_scale", 1.0) or 1.0)
    return ts if 0 < ts <= 1.0 else 1.0


class Burst:
    """A named burst: its id and host wall-clock window (epoch seconds) go into the detail of
    every timing check that judges it, so a runner can line it up with its own capture."""

    def __init__(self, name: str):
        self.name, self.t0, self.t1 = name, None, None

    def sent(self):
        now = time.time()
        self.t0 = now if self.t0 is None else self.t0
        self.t1 = now

    def tag(self) -> str:
        return f"[burst {self.name}, host wall {self.t0:.6f}-{self.t1:.6f}]"


@test("T-B5-20")
def t_b5_20(ctx):
    """B2.3 "High Priority bursts" (0053, 0054): two packets back to back at wire speed with no
    third within 2 ms, and a sequence of 20 packets 1 ms apart, are all processed. Precondition
    (0055): no management write, store compaction or update in progress; the test sends none.
    Waits and host spacing are in the radio's time: stretched by 1 / time_scale.
    What it can see from outside: the stop and the last packet of each burst. "No loss in the
    burst" for every packet is judged from the net.log "dropped" entries (v1.6) where offered,
    else noted as not judged (a received-packet counter is asked of the spec session)."""
    ts = time_scale(ctx)

    def W(sec: float) -> float:
        return sec / ts

    def eq(name: str, got, want, b: Burst):
        return ctx.check(f"{name} [burst {b.name}]", got == want, f"got {got!r}, want {want!r} {b.tag()}")

    netlog = "net.log" in ctx.endpoint.get("fixtures", [])
    if not netlog:
        ctx.notes.append("'no loss in the burst' not judged: no net.log (v1.6) to show drops")

    from .mgmt import Mgmt, stats
    mg = Mgmt(ctx.net)
    hp_before = {}

    def hp_rx():
        st = stats(mg)
        return None if st is None else st.get("hp_rx")

    def log_start():
        hp_before["n"] = hp_rx()  # GET_STATS on port 7251: not a High Priority packet
        if netlog:
            ctx.fixture("net.log", action="start")

    def no_loss(b: Burst, n_sent: int):
        """0054: no datagram to radio port 1027 lost in the burst: GET_STATS hp_rx rose by exactly
        the number sent (0058; no other High Priority packet was sent since log_start), and no
        net.log "dropped" entry where offered."""
        a, z = hp_before.get("n"), hp_rx()
        if a is None or z is None:
            ctx.notes.append(f"hp_rx not judged for burst {b.name}: GET_STATS has no hp_rx")
        else:
            ctx.check(f"no loss in the burst: GET_STATS hp_rx +{n_sent} (0058) [burst {b.name}]", z - a == n_sent,
                      f"hp_rx {a} -> {z} (+{z - a}), sent {n_sent} {b.tag()}")
        if not netlog:
            return
        entries, n = [], 0
        for _ in range(1000):
            r = ctx.fixture("net.log", action="read", **{"from": len(entries)}) or {}
            entries += r.get("entries", [])
            n = r.get("count", 0)
            if len(entries) >= n:
                break
        drops = [e for e in entries if e.get("dir") == "dropped" and e.get("radio_port") == PORT_HP]
        ctx.check(f"no loss in the burst: no High Priority datagram dropped (0054) [burst {b.name}]", not drops,
                  (f"{len(drops)} dropped, first {drops[0]}" if drops else "none dropped") + f" {b.tag()}")

    def send(b: Burst, pkt: bytes):
        h.s.sendto(pkt, ctx.net.dest(PORT_HP))
        b.sent()

    if ts < 1.0:
        ctx.notes.append(f"time_scale {ts}: waits and spacing stretched by {1 / ts:.1f}")
    with Host(ctx.net) as h, Host(ctx.net) as obs:
        # running, then run = 1 and run = 0 back to back: the session ends
        h.start()
        keep_until = time.monotonic() + W(0.3)
        while time.monotonic() < keep_until:
            h.keepalive()
            h.recv(W(0.05))
        time.sleep(W(0.005))  # no packet within 2 ms before the pair either
        log_start()
        b1 = Burst("pair-1 run1-run0")
        send(b1, high_priority(run=True))
        send(b1, high_priority(run=False))
        h.running = False
        h.recv(W(0.2))  # nothing sent for 200 ms: no third packet within 2 ms
        eq("run = 1 then run = 0 back to back: the session ends (discovery byte 4 for another host)", running_seen_by(obs, W(0.3)), 2, b1)
        eq("and no High Priority status packet follows", len(status_seqs(h, W(0.3))), 0, b1)
        no_loss(b1, 2)
        # idle, then run = 0 and run = 1 back to back: a session starts
        log_start()
        b2 = Burst("pair-2 run0-run1 idle")
        send(b2, high_priority(run=False))
        send(b2, high_priority(run=True))
        h.running = True
        h.recv(W(0.1))
        eq("run = 0 then run = 1 back to back: a session runs", running_seen_by(obs, W(0.3)), 3, b2)
        no_loss(b2, 2)
        before = status_seqs(h, W(0.3))
        h.keepalive()
        before += status_seqs(h, W(0.2))
        # running, run = 0 then run = 1 back to back (the pair of 0054 item 1): the stop resets the
        # counters (B2.7), so the status sequence restarts; this separates a lost stop in the 20
        # below from counters that a stop with a run right after does not reset
        time.sleep(W(0.005))
        log_start()
        b3 = Burst("pair-3 run0-run1 running")
        send(b3, high_priority(run=False))
        send(b3, high_priority(run=True))
        pair_after = status_seqs(h, W(0.2))
        no_loss(b3, 2)
        ctx.check(f"running, run = 0 then run = 1 back to back: the status sequence restarts (B2.7) [burst {b3.name}]",
                  bool(before) and bool(pair_after) and restarted(before[-1], pair_after),
                  f"before {before[-3:]}, after {pair_after[:3]} {b3.tag()}")
        h.keepalive()
        before = status_seqs(h, W(0.3))
        h.keepalive()
        before += status_seqs(h, W(0.2))
        # 20 packets at least 1 ms apart: nine run, a stop, ten run. The stop resets the outgoing
        # counters (B2.7), so the status sequence restarts near 0 only if no packet was lost
        seq_ = [high_priority(run=True)] * 9 + [high_priority(run=False)] + [high_priority(run=True)] * 10
        log_start()
        b4 = Burst("seq20 1ms")
        gaps, last = [], None
        for pkt in seq_:
            while last is not None and time.monotonic() - last < W(0.0011):
                pass  # busy wait: at least 1 ms (in the radio's time) between packets (0054)
            now = time.monotonic()
            if last is not None:
                gaps.append(now - last)
            send(b4, pkt)
            last = now
        ctx.check(f"20 packets sent at least 1 ms apart (host side, radio time) [burst {b4.name}]", min(gaps) >= W(0.001),
                  f"gaps {min(gaps) * 1000:.2f}-{max(gaps) * 1000:.2f} ms wall {b4.tag()}")
        after = status_seqs(h, W(0.25))
        no_loss(b4, 20)
        h.keepalive()
        eq("20 packets 1 ms apart: the session runs at the end (the last packet processed)", running_seen_by(obs, W(0.3)), 3, b4)
        ctx.check(f"20 packets 1 ms apart: the stop inside them was processed (status sequence restarted, B2.7) [burst {b4.name}]",
                  bool(after) and bool(before) and restarted(before[-1], after),
                  f"status sequence before {before[-3:]}, after {after[:3]} {b4.tag()}")
        h.stop()
    # information, not judged (the spec sets no figure): the firmware's longest busy-loop pass
    with mg as c:
        st = stats(c)
    v = (st or {}).get("net_busy_pass_max_us")
    ctx.check("information (not judged): GET_STATS net_busy_pass_max_us after the bursts", True,
              f"{v} us" if v is not None else ("GET_STATS has no net_busy_pass_max_us" if st is not None else "no GET_STATS answer"))
    # information, not judged: the whole GET_STATS answer after the last burst (every counter,
    # the firmware's step_*_us pass timers included), name=value sorted by name
    ctx.check("information (not judged): GET_STATS after the last burst, all counters", True,
              ", ".join(f"{k}={st[k]}" for k in sorted(st)) if st else "no GET_STATS answer")


@test("T-B4-19")
def t_b4_19(ctx):
    """B4.4 items 1, 2 (0059): the unasked record in one to three parts, collected by register
    address; the CAP_REQUEST answer in one packet."""
    win = 0.05 / time_scale(ctx)
    with host(ctx) as h:
        h.start()
        got = h.recv(0.4, PORT_REG)
        parts = []  # (t, seq, pairs) of the packets that carry record registers
        for _, d, t in got:
            pairs = parse_regs(d)
            rec_pairs = [(a, v) for a, v in pairs if a < 0x0100]
            if rec_pairs:
                parts.append((t, struct.unpack_from(">I", d, 0)[0], rec_pairs, len(rec_pairs) == len(pairs)))
        if not ctx.check("the unasked record arrives at the session start", bool(parts), f"{len(got)} register packets"):
            return
        ctx.check("in one to three packets (B4.4 item 2)", 1 <= len(parts) <= 3, f"{len(parts)} parts")
        ctx.check("each part holds record registers only", all(p[3] for p in parts), str([len(p[2]) for p in parts]))
        addrs = [a for p in parts for a, _ in p[2]]
        ctx.check("ascending register address order, within and across the parts", addrs == sorted(addrs) and len(set(addrs)) == len(addrs),
                  f"{len(addrs)} registers, first {addrs[:3]}, last {addrs[-3:]}")
        # increasing numbers of the session counter; a gap only where the bench saw another
        # register push to this destination take the number in between (0059 as clarified)
        seqs = [p[1] for p in parts]
        others = {struct.unpack_from(">I", d, 0)[0] for _, d, _ in got} - set(seqs)
        gaps_ok = all(seqs[i + 1] > seqs[i] and all(n in others for n in range(seqs[i] + 1, seqs[i + 1]))
                      for i in range(len(seqs) - 1))
        ctx.check("increasing port 7250 sequence numbers, gaps only for other pushes seen in between (B2.7, 0051, 0059)",
                  gaps_ok, f"parts {seqs}, other register packets {sorted(others)[:8]}")
        span = parts[-1][0] - parts[0][0]
        ctx.check(f"all parts within 50 ms of the first ({win * 1000:.0f} ms on the host clock)", span <= win, f"{span * 1000:.1f} ms")
        unasked = dict(p for part in parts for p in part[2])
        n = unasked.get(2)
        ctx.check("together every record register exactly once (cap_length registers, 0 to cap_length - 1)",
                  n is not None and sorted(unasked) == list(range(n)), f"cap_length {n}, {len(unasked)} registers")
        # the CAP_REQUEST answer in the same session: one packet, the same values
        h.send(PORT_REG, regs([(CAP_REQUEST, 1)]))
        ans = [d for _, d, _ in h.recv(0.3, PORT_REG) if any(a == 0 for a, _ in parse_regs(d))]
        ctx.eq("the CAP_REQUEST answer is one packet (B4.4 item 1)", len(ans), 1)
        if ans:
            answer = {a: v for a, v in parse_regs(ans[0]) if a < 0x0100}
            diff = [a for a in set(answer) | set(unasked) if answer.get(a) != unasked.get(a)]
            ctx.check("the unasked record has the values of the CAP_REQUEST answer", not diff,
                      f"differ at {sorted(diff)[:5]}" if diff else f"{len(answer)} registers equal")
        idle(h)
