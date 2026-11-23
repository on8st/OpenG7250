"""The flash tests of t_flash (T-A1-85, T-A1-86, T-A1-87) run in-process against a fake radio
with the store semantics of A1.6 "Store writes" (0065) and the v1.8 fixture shapes: each must
pass on a correct fake and fail on a broken one. No network.

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
import struct
import unittest
from contextlib import contextmanager

from testbench import t_flash
from testbench.core import Ctx, RunnerFault

WORDS = 4  # flash words per store write; the last one is the commit word
STORE = 0x080C0000


class FakeRadio:
    def __init__(self, broken=None):
        self.value = 200_000  # the stored C-3 TX time
        self.prev = None  # the value of the record before the newest (for ECC fall-back)
        self.log, self.cut, self.fault, self.ecc = [], None, None, set()
        self.writes, self.next_addr, self.flags = 0, STORE, 0
        self.newest = []  # addresses of the newest record's words
        self.broken = broken

    # --- fixtures ---
    def fixture(self, op, **a):
        if op == "flash.log":
            if a.get("action") == "start":
                self.log = []
                return None
            n = a.get("from", 0)
            return dict(count=len(self.log), dropped=0, entries=self.log[n:])
        if op == "power.cut":
            self.cut, self.left = a, a.get("after_flash_words")
            if self.broken == "stale" and "during_erase" in a:
                self.cut = None  # the erase cut is lost
            return None
        if op == "flash.fault":
            self.fault = a
            return None
        if op == "flash.ecc":
            if self.broken == "ecc-late":
                return dict(in_force=False)
            self.ecc.add(a["address"])
            return dict(in_force=True)
        if op == "power.cycle":
            self.boot()
            return None
        raise AssertionError(op)

    def boot(self):
        """At start the store opens: a record with an unreadable word is skipped (0065 item 9)."""
        if self.newest and any(w in self.ecc for w in self.newest) and self.prev is not None:
            self.log.append(dict(op="ecc", type="double", address=sorted(self.ecc)[0], boot=2))
            if self.broken != "ecc":
                self.value = self.prev

    # --- a store write ---
    def write(self, v):
        self.writes += 1
        if self.fault and self.fault.get("op") == "program":
            self.fault = None
            self.flags |= 1 << 19
            self.log.append(dict(op="program", address=self.next_addr, len=32, result="error:PGSERR"))
            return 55
        if self.writes % 10 == 0:  # a compaction erases the other sector
            e = dict(op="erase", sector=7, address=0x080E0000, len=131072, t_us=0, t_end_us=1, result="ok")
            if self.cut and "during_erase" in self.cut:
                e["result"], e["fraction_done"] = "cut", self.cut["during_erase"]
                self.cut = None
                self.log.append(e)
                return None  # power lost: the old value stays
            self.log.append(e)
        words = []
        for i in range(WORDS):
            addr = self.next_addr
            self.next_addr += 32
            if addr in self.ecc:  # later writes skip the word
                addr = self.next_addr
                self.next_addr += 32
            words.append(addr)
            if self.cut and "after_flash_words" in self.cut and self.broken != "nocut":
                self.left -= 1
            if self.cut and "after_flash_words" in self.cut and self.left == 0 and self.broken != "nocut":
                # the cut lands during the n-th word: n - 1 complete, the n-th torn (v1.8); a
                # torn commit word leaves the old record in force
                self.log.append(dict(op="program", address=addr, len=32, result="cut", words_written=i))
                self.cut = None
                if self.broken == "mix":
                    self.value = v // 2  # a mix of old and new
                return None
            self.log.append(dict(op="program", address=addr, len=32, result="ok"))
        self.prev, self.value, self.newest = self.value, v, words
        return 0


class FakeU:
    def __init__(self, radio):
        self.r = radio
        self.c = self

    def req(self, cmd, payload=b"", window=None):
        if cmd == "SET_CONFIG":
            assert payload[:3] == bytes([11, 5, 10]), payload  # the plain item (0072)
            st = self.r.write(int.from_bytes(payload[3:6], "big"))
            return None if st is None else dict(status=st, payload=b"")
        if cmd == "GET_CONFIG":
            return dict(status=0, payload=bytes([11, 5, 10]) + (self.r.value & 0xFFFFFF).to_bytes(3, "big") + bytes([32]))
        return dict(status=0, payload=b"")

    def identify(self):
        return {25: bytes([2, 2, 4, 2, 6, 2]), 23: b"\x01", 16: struct.pack(">I", 1000)}


class FakeCtx(Ctx):
    def __init__(self, radio):
        super().__init__(dict(radio=dict(ip="127.0.0.1"),
                              fixtures=["flash.log", "flash.fault", "flash.ecc", "power.cut", "power.cycle"]))
        self.r = radio

    def fixture(self, op, **a):
        return self.r.fixture(op, **a)


def run(test_fn, radio):
    ctx = FakeCtx(radio)
    from testbench import t_update
    saved = {k: getattr(t_flash, k) for k in ("mg", "secret", "fault_flags", "wait_reachable")}
    saved_restart = t_update.restart
    t_update.restart = lambda ctx_, u: radio.fixture("power.cycle")

    @contextmanager
    def mg(ctx_, s):
        yield FakeU(radio)

    t_flash.mg = mg
    t_flash.secret = lambda ctx_: b"s" * 32
    t_flash.fault_flags = lambda ctx_: radio.flags
    t_flash.wait_reachable = lambda ctx_, *a, **k: 0.0
    try:
        test_fn(ctx)
    finally:
        for k, v in saved.items():
            setattr(t_flash, k, v)
        t_update.restart = saved_restart
    return [c for c in ctx.checks if not c["ok"]]


class FlashTestsTest(unittest.TestCase):
    def test_a1_86_passes_on_a_correct_store(self):
        self.assertEqual(run(t_flash.t_a1_86, FakeRadio()), [])

    def test_a1_86_never_passes_without_its_cuts(self):
        """A planned cut that does not fire, or is lost, is an error, not a pass (fw-e518331)."""
        for broken in ("nocut", "stale"):
            with self.assertRaises(RunnerFault):
                run(t_flash.t_a1_86, FakeRadio(broken=broken))

    def test_a1_86_fails_on_a_mixed_value(self):
        self.assertTrue(run(t_flash.t_a1_86, FakeRadio(broken="mix")))

    def test_a1_87_passes_and_fails(self):
        notes = []
        def keep(ctx):
            t_flash.t_a1_87(ctx)
            notes.extend(ctx.notes)
        self.assertEqual(run(keep, FakeRadio()), [])
        self.assertEqual(notes.count(t_flash.ECC_LIMIT), 1)  # two errors set, said once
        self.assertTrue(run(t_flash.t_a1_87, FakeRadio(broken="ecc")))

    def test_a1_87_mark_not_in_force_is_not_judged(self):
        kept = []
        def keep(ctx):
            kept.append(ctx)
            t_flash.t_a1_87(ctx)
        self.assertEqual(run(keep, FakeRadio(broken="ecc-late")), [])
        self.assertTrue(kept[0].no_pass)
        self.assertEqual(len(kept[0].parts), 1)  # the first case not judged, the second not run


class CompactionTest(unittest.TestCase):
    """T-B5-22: the SET_CONFIG loop and the erase the force is placed in."""

    def test_erase_found_after_an_unanswered_write(self):
        """A write that gets no reply while its erase runs: the erase is found, not 'no erase'."""
        r = FakeRadio()

        class U(FakeU):
            def req(self, cmd, payload=b"", window=None):
                got = super().req(cmd, payload, window)
                return None if cmd == "SET_CONFIG" and r.writes == 10 else got
        u = U(r)
        u.why = "no reply to SET_CONFIG"
        ctx = FakeCtx(r)
        self.assertEqual(t_flash.compaction_writes(ctx, u, 200_000, STORE, STORE + 2 * 131072), (200_010, 10))
        self.assertIn("1 store erases", ctx.__dict__["_compaction"])

    def test_a_refused_write_is_named(self):
        r = FakeRadio()
        r.fault = dict(op="program")
        ctx = FakeCtx(r)
        self.assertIsNone(t_flash.compaction_writes(ctx, FakeU(r), 200_000, STORE, STORE + 2 * 131072))
        self.assertIn("answered 55", ctx.__dict__["_compaction"])

    def post(self, erase_at):
        r = FakeRadio()
        ctx = FakeCtx(r)

        class U:
            why, sent = "", []
            c = property(lambda self: self)

            def post(self, cmd, payload):
                self.sent.append(cmd)
                if len(self.sent) == erase_at:
                    r.log.append(dict(op="erase", sector=4, address=0x08080000, t_us=10, t_end_us=1_100_010,
                                      result="running"))  # t_end_us: the predicted end
                return b"req%d" % len(self.sent)

            def listen(self, data, timeout):
                return [] if len(self.sent) == erase_at else [dict(status=0)]

            def collect(self, data, what):
                raise AssertionError("waited for a reply")
        u = U()
        u.sent = []
        return t_flash.post_until_erase(ctx, u, b"h" * 53, b"i" * 2148), u.sent

    def test_erase_in_begin(self):
        (er, data), sent = self.post(1)
        self.assertEqual((er["sector"], data, sent), (4, b"req1", ["UPDATE_BEGIN"]))

    def test_erase_in_the_first_block(self):
        (er, data), sent = self.post(2)
        self.assertEqual((data, sent), (b"req2", ["UPDATE_BEGIN", "UPDATE_BLOCK"]))

    def test_no_erase(self):
        (er, data), sent = self.post(99)
        self.assertEqual((er, data, len(sent)), (None, None, 6))


if __name__ == "__main__":
    unittest.main()
