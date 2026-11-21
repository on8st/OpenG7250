"""The flash tests of t_flash (T-A1-85, T-A1-86, T-A1-87) run in-process against a fake radio
with the store semantics of A1.6 "Store writes" (0065) and the v1.8 fixture shapes: each must
pass on a correct fake and fail on a broken one. No network.

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
import struct
import unittest
from contextlib import contextmanager

from testbench import t_flash
from testbench.core import Ctx

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
            self.cut = a
            return None
        if op == "flash.fault":
            self.fault = a
            return None
        if op == "flash.ecc":
            self.ecc.add(a["address"])
            return None
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
            if self.cut and self.cut.get("after_flash_words") == i + 1:
                # the cut lands during the n-th word: n - 1 complete, the n-th torn (v1.8); a
                # torn commit word leaves the old record in force
                self.log.append(dict(op="program", address=addr, len=32, result="cut", words_written=i))
                self.cut = None
                if self.broken == "mix":
                    self.value = v // 2  # a mix of old and new
                return None
            self.log.append(dict(op="program", address=addr, len=32, result="ok"))
        if self.cut and "after_flash_words" in self.cut:
            self.cut = None  # the write ended before the cut point
        self.prev, self.value, self.newest = self.value, v, words
        return 0


class FakeU:
    def __init__(self, radio):
        self.r = radio
        self.c = self

    def req(self, cmd, payload=b""):
        if cmd == "SET_CONFIG":
            st = self.r.write(struct.unpack(">I", payload[2:6])[0])
            return None if st is None else dict(status=st, payload=b"")
        if cmd == "GET_CONFIG":
            return dict(status=0, payload=bytes([7, 4]) + struct.pack(">I", self.r.value))
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

    def test_a1_86_fails_on_a_mixed_value(self):
        self.assertTrue(run(t_flash.t_a1_86, FakeRadio(broken="mix")))

    def test_a1_87_passes_and_fails(self):
        self.assertEqual(run(t_flash.t_a1_87, FakeRadio()), [])
        self.assertTrue(run(t_flash.t_a1_87, FakeRadio(broken="ecc")))


if __name__ == "__main__":
    unittest.main()
