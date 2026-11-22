"""The management client against a fake radio on a socket pair (no network, no ports): reply
matching on a fast stream, A1.4 retransmission, the 16 s window of 0069 in radio time, and what
the bench says when a request has no reply. GPL-3.0-or-later, (c) 2026 ON8ST."""
import hashlib
import json
import socket
import struct
import threading
import time
import unittest

from testbench import core, mgmt
from testbench.mgmt import CMD, MAGIC, Mgmt, parse
from testbench.t_a2 import limited_floor
from testbench.update import Updater, block_payload, header, image, parse_package

NAME = {v: k for k, v in CMD.items()}


def reply(req: bytes, status: int = 0, payload: bytes = b"") -> bytes:
    h = parse(req)
    return struct.pack(">IBBBBIHHH", MAGIC, 1, 0, 2, 0, h["id"], h["cmd"], status, len(payload)) + payload


class FakeSock:
    """sendto() hands the datagram to the radio; what the radio sends is read back here."""

    def __init__(self, radio):
        self.a, self.b = socket.socketpair(socket.AF_UNIX, socket.SOCK_DGRAM)
        self.radio = radio

    def fileno(self):
        return self.a.fileno()

    def sendto(self, d, addr):
        self.radio(d, self.b.send)

    def recvfrom(self, n):
        return self.a.recv(n), ("radio", 7251)

    def close(self):
        self.a.close()
        self.b.close()

    __del__ = close


class FakeNet:
    def dest(self, p, kind):
        return ("radio", p)

    def radio_port(self, src):
        return src[1]


class SlowClock:
    """Radio time at `rate` of wall time, as an emulator during an erase."""
    emu = True

    def __init__(self, rate):
        self.rate, self.t0 = rate, time.monotonic()

    def now(self):
        return (time.monotonic() - self.t0) * self.rate


def client(radio, clock=None):
    m = object.__new__(Mgmt)
    m.net, m.rid, m.s = FakeNet(), 100, FakeSock(radio)
    m.clock = clock
    return m


class FastStream(unittest.TestCase):
    def test_275_blocks_with_duplicates_and_strays(self):
        """A radio that answers at once, sends every reply twice and a stray one before it: no block
        may be reported unanswered and none executed with another's reply."""
        got = []

        def radio(d, send):
            h = parse(d)
            if NAME[h["cmd"]] == "GET_CHALLENGE":
                p = b"c" * 16
            else:
                n = struct.unpack(">I", d[18:22])[0]
                got.append(n)
                p = struct.pack(">II", n, n + 1)
            send(reply(d, 0, b"stray")[:8] + struct.pack(">I", 7) + reply(d)[12:])  # an earlier id
            send(reply(d, 0, p))
            send(reply(d, 0, p))
        c = client(radio)
        u = Updater(c, b"s" * 32)
        c.pace = 0  # the test is of the matching, not the pacing
        self.assertEqual(u.blocks(image(275 * 512)), (0, 275))
        self.assertEqual(got, list(range(275)))
        self.assertEqual(c.why, "")

    def test_block_reply_belongs_to_its_request(self):
        c = client(lambda d, send: send(reply(d, 0, b"x" * 16)))
        u = Updater(c, b"s" * 32)
        c.pace = 0
        for k in range(50):
            r = u.req("UPDATE_BLOCK", block_payload(k, b"d" * 512))
            self.assertEqual(r["id"], c.rid)


class Windows(unittest.TestCase):
    def radio(self, after_sends, seen):
        def r(d, send):
            h = parse(d)
            if NAME[h["cmd"]] == "GET_CHALLENGE":
                return send(reply(d, 0, b"c" * 16))
            seen.append(d)
            if len(seen) == after_sends:
                send(reply(d, 0))
        return r

    def test_flash_request_goes_on_after_5_retransmissions(self):
        seen = []
        c = client(self.radio(8, seen))
        r = mgmt.authed_retransmit(c, "UPDATE_BLOCK", b"s" * 32, b"p")
        self.assertEqual(r and r["status"], 0)
        self.assertEqual(len(set(seen)), 1, "a retransmission is the same bytes")

    def test_other_request_stops_after_5(self):
        seen = []
        c = client(self.radio(99, seen))
        self.assertIsNone(mgmt.authed_retransmit(c, "UPDATE_VERIFY", b"s" * 32))
        self.assertEqual(len(seen), 6)
        self.assertIn("no reply to UPDATE_VERIFY in 6 sends", c.why)

    def test_window_counts_radio_time(self):
        """At a fifth of real time a 1 s window lasts 5 s of wall time: a reply at 3 s is taken."""
        t0 = time.monotonic()

        def radio(d, send):
            if time.monotonic() - t0 > 3:
                send(reply(d, 0))
        c = client(radio, SlowClock(0.2))
        r = mgmt.send_retransmit(c, mgmt.build(CMD["UPDATE_BLOCK"], b"p", 5), window=1.0)
        self.assertEqual(r and r["status"], 0)

    def test_retransmissions_500_ms_of_radio_time_apart(self):
        times, clk = [], SlowClock(0.25)
        c = client(lambda d, send: times.append(clk.now()), clk)
        self.assertIsNone(mgmt.send_retransmit(c, mgmt.build(CMD["UPDATE_BLOCK"], b"p", 5), window=1.5))
        self.assertTrue(all(b - a >= 0.45 for a, b in zip(times, times[1:])), times)
        self.assertGreaterEqual(times[-1] - times[0], 1.0)

    def test_reply_during_the_slow_wait_is_taken(self):
        def radio(d, send):
            threading.Timer(0.9, lambda: send(reply(d, 55))).start()  # after the 0.5 s wall wait
        c = client(radio, SlowClock(0.1))
        r = mgmt.send_retransmit(c, mgmt.build(CMD["UPDATE_BLOCK"], b"p", 5), window=16)
        self.assertEqual(r and r["status"], 55)

    def test_refused_challenge_is_named(self):
        c = client(lambda d, send: send(reply(d, 34)))
        self.assertIsNone(mgmt.authed_retransmit(c, "UPDATE_BLOCK", b"s" * 32))
        self.assertEqual(c.why, "UPDATE_BLOCK not sent: GET_CHALLENGE answered 34")


class Pacing(unittest.TestCase):
    def test_under_20_per_second_of_radio_time(self):
        clk, times = SlowClock(0.3), []

        def radio(d, send):
            times.append(clk.now())
            send(reply(d, 0))
        c = client(radio, clk)
        c.pace = 18
        for k in range(45):
            c.req("IDENTIFY")
        worst = max(len([t for t in times if a <= t < a + 1.0]) for a in times)
        self.assertLessEqual(worst, 18)
        self.assertGreater(times[-1] - times[0], 1.5)


class Fixture(unittest.TestCase):
    def test_op_as_an_argument(self):
        """flash.fault has its own `op` argument."""
        srv = socket.socketpair()
        seen = {}

        def serve():
            f = srv[1].makefile("rw")
            seen.update(json.loads(f.readline()))
            f.write(json.dumps({"ok": True, "value": 1}) + "\n")
            f.flush()
        threading.Thread(target=serve, daemon=True).start()
        ctx = core.Ctx(dict(radio=dict(ip="127.0.0.1"), fixtures=["flash.fault"], control=dict(address="x:1")))
        real = core.socket.create_connection
        core.socket.create_connection = lambda *a, **k: srv[0]
        try:
            self.assertEqual(ctx.fixture("flash.fault", op="erase", flag="PGSERR"), 1)
        finally:
            core.socket.create_connection = real
        self.assertEqual(seen, {"op": "flash.fault", "args": {"op": "erase", "flag": "PGSERR"}})


class Clock(unittest.TestCase):
    def test_counts_across_a_power_cycle(self):
        class Ctx:
            endpoint = dict(fixtures=["hw.observe"])
            ts = [5_000_000, 6_000_000, 200_000, 700_000]

            def fixture(self, op):
                return dict(t_us=self.ts.pop(0))
        k = core.RadioClock(Ctx())
        a = k.now()
        self.assertAlmostEqual(k.now() - a, 1.0)
        self.assertAlmostEqual(k.now() - a, 1.2)
        self.assertAlmostEqual(k.now() - a, 1.7)

    def test_wall_clock_on_the_model(self):
        class Ctx:
            endpoint = dict(fixtures=["hw.observe"], model={})
        self.assertFalse(core.RadioClock(Ctx()).emu)


class NoticeFloor(unittest.TestCase):
    def test_the_fw_e518331_case(self):
        w = list(range(0, 1_310_001, 10_000))  # 1.310 s of writes
        self.assertEqual(limited_floor(w), 11)  # a 110 ms limiter lets 12 through, less one for the edges
        self.assertLessEqual(limited_floor(w), 12)  # the radio's 12 answers pass; the old bound was 13

    def test_never_more_than_the_span_allows(self):
        for step in (5_000, 30_000, 80_000, 130_000):
            w = list(range(0, 3_000_000, step))
            sent, last = 0, None  # a correct radio with exactly 100 ms
            for t in w:
                if last is None or t - last >= 100_000:
                    sent, last = sent + 1, t
            self.assertLessEqual(limited_floor(w), sent, step)

    def test_a_radio_that_shares_one_limit_fails(self):
        w = list(range(0, 3_000_000, 10_000))
        self.assertGreater(limited_floor(w), 15)  # half the rate (one limit for both) is below it


class PackageFile(unittest.TestCase):
    def test_annex_m(self):
        img = image(700)
        h = header(len(img), hashlib.sha256(img).digest(), version=(0, 2, 0), slot=2)
        self.assertEqual(parse_package(h + img)[1], img)
        for bad in (h + img + b"\0", (h + img)[:-1], h[:50] + b"\0\1" + h[52:] + img, b"X" + h[1:] + img,
                    h + img[:-1] + bytes([img[-1] ^ 1]), h[:40]):
            with self.assertRaises(ValueError):
                parse_package(bad)


if __name__ == "__main__":
    unittest.main()
