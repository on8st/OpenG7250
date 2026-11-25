"""T-A1-85's 0081 cases against a fake radio: an erase that never ends (no reply, watchdog reset, 33 to a
retransmission after the restart) and one that ends late (55 within 0.5 s of its end). Each passes on a
correct fake and fails on a broken one; a runner without flash.fault end_after_us gives a part and no
pass. No network. GPL-3.0-or-later, (c) 2026 ON8ST.
"""
import unittest
from contextlib import contextmanager

from testbench import t_details, t_flash, t_update
from testbench.core import Ctx, Skip

N_LATE = 2 * 4_000_000 + 100_000


class Radio:
    def __init__(self, broken=None, late_offered=True):
        self.broken, self.late_offered = broken, late_offered
        self.fault, self.log, self.flags, self.now = None, [], 0, 1.0
        self.conf = {7: (1000).to_bytes(4, "big")}

    def fixture(self, name, /, **a):
        op = name
        if op == "flash.log":
            if a.get("action") == "start":
                self.log = []
                return None
            return dict(count=len(self.log), dropped=0, entries=self.log[a.get("from", 0):])
        if op == "flash.fault":
            if "end_after_us" in a and not self.late_offered:
                raise Skip("flash.fault: unknown key end_after_us")
            self.fault = a
            return None
        raise AssertionError(op)

    def flash_work(self, cmd):
        """The reply to a request that erases, by the armed fault."""
        f, self.fault = self.fault, None
        if f is None:
            return dict(status=0, payload=b"")
        if f.get("never_end"):
            self.flags |= 1 << t_flash.WDT_RESET_AT_START
            if self.broken == "conf-lost":
                self.conf = {}
            return dict(status=55 if self.broken == "reply-55" else 33, payload=b"")
        t0 = self.now * 1e6
        n = f["end_after_us"]
        self.log.append(dict(op="erase", address=0x080E0000, len=131072, t_us=t0, duration_us=n, result="late"))
        self.flags |= 1 << t_flash.INTERNAL_ERROR
        self.now += n / 1e6 + (0.9 if self.broken == "slow-55" else 0.1)
        return dict(status=55, payload=b"")


class U:
    def __init__(self, r):
        self.r, self.c = r, self
        self.why, self.timed, self.reply_s, self.clock = "", False, None, self

    def now(self):
        return self.r.now

    def req(self, cmd, payload=b"", window=None):
        if cmd in ("SET_CONFIG", "UPDATE_BEGIN"):
            t0 = self.r.now
            r = self.r.flash_work(cmd)
            self.reply_s = self.r.now - t0
            return r
        return dict(status=0, payload=b"")

    def post(self, cmd, payload=b""):
        return (cmd, payload)

    def collect(self, data, what=""):
        return self.req(*data)

    def identify(self):
        return {23: b"\x01", 10: b"\x00", 25: bytes([2, 2, 4, 2, 6, 2]), 4: bytes([1, 0, 0, 1])}


def run(fn, radio):
    ctx = Ctx(dict(radio=dict(ip="127.0.0.1"), fixtures=["flash.log", "flash.fault"]))
    ctx.fixture = radio.fixture

    @contextmanager
    def mg(ctx_, s):
        yield U(radio)
    saved = {k: getattr(t_flash, k) for k in ("mg", "fault_flags", "wait_reachable", "items")}
    saved_u = {k: getattr(t_update, k) for k in ("restart", "facts", "rx_works")}
    t_flash.mg, t_flash.wait_reachable = mg, (lambda *a, **k: 0.0)
    t_flash.fault_flags = lambda ctx_: radio.flags
    t_flash.items = lambda u: dict(radio.conf)
    t_update.restart = lambda ctx_, u: setattr(radio, "flags", 0)
    t_update.facts = lambda ctx_, u: (1, 2, (1, 0, 1), ((2, 2), (4, 2), (6, 2)))
    t_update.rx_works = lambda ctx_: True
    saved["stream_reason10"] = t_flash.stream_reason10
    t_flash.stream_reason10 = lambda ctx_: getattr(radio, "reason10", True)
    try:
        fn(ctx, b"s" * 32)
    finally:
        for k, v in saved.items():
            setattr(t_flash, k, v)
        for k, v in saved_u.items():
            setattr(t_update, k, v)
    return [c["name"] for c in ctx.checks if not c["ok"]], ctx


class NeverEnds(unittest.TestCase):
    def test_correct(self):
        bad, ctx = run(t_flash.never_ends, Radio())
        self.assertEqual(bad, [])
        self.assertEqual(sum("gets 33" in c["name"] for c in ctx.checks), 2)

    def test_broken(self):
        for broken, seen in (("reply-55", "gets 33"), ("conf-lost", "the store as before")):
            bad, _ = run(t_flash.never_ends, Radio(broken))
            self.assertTrue(any(seen in b for b in bad), (broken, bad))


class LateEnd(unittest.TestCase):
    def test_correct(self):
        bad, ctx = run(t_flash.late_end, Radio())
        self.assertEqual(bad, [])
        self.assertEqual(sum("within 0.5 s of the erase's end" in c["name"] for c in ctx.checks), 2)
        # RX after the SET_CONFIG; after the update the update state (reason 10, spec 68c0ff5)
        self.assertEqual(sum(c["name"].endswith("RX goes on") for c in ctx.checks), 1)
        self.assertEqual(sum("a run request gets reason 10" in c["name"] for c in ctx.checks), 1)
        self.assertFalse(ctx.no_pass)

    def test_update_state_not_kept_fails(self):
        r = Radio()
        r.reason10 = False
        bad, _ = run(t_flash.late_end, r)
        self.assertTrue(any("reason 10" in b for b in bad), bad)

    def test_slow_reply_fails(self):
        bad, _ = run(t_flash.late_end, Radio("slow-55"))
        self.assertTrue(any("within 0.5 s" in b for b in bad), bad)

    def test_not_offered_is_a_part(self):
        bad, ctx = run(t_flash.late_end, Radio(late_offered=False))
        self.assertEqual(bad, [])
        self.assertTrue(ctx.no_pass)
        self.assertTrue(any("end_after_us" in p for p in ctx.parts), ctx.parts)

    def test_fixture_shape(self):
        r = Radio()
        run(t_flash.late_end, r)
        self.assertEqual(r.log[0]["duration_us"], N_LATE)


if __name__ == "__main__":
    unittest.main()
