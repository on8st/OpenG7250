"""The C-3 writes of T-A1-86 (0072) against a fake store: one erase for a C-3 write, the cuts at
each word of a mixed write, the cuts during a C-1 compaction's erase, and the cut (at_power_on) in
the start's erase of what a write cut before its commit word left. No network.

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
import struct
import unittest
from contextlib import contextmanager

from testbench import t_flash
from testbench.core import Ctx

STORE, N, BUILT_IN = 0x080C0000, 6, 300_000  # N words per generation, the last the commit word


class C3Radio:
    def __init__(self, broken=None):
        self.tx, self.gw, self.broken = 250_000, bytes(4), broken
        self.log, self.cut, self.addr, self.appended, self.stale = [], None, STORE, 0, False
        self.pon, self.left = None, None  # a cut armed for the next power-on; what a cut write left

    def fixture(self, op, **a):
        if op == "flash.log":
            if a.get("action") == "start":
                self.log = []
                return None
            return dict(count=len(self.log), dropped=0, entries=self.log[a.get("from", 0):])
        if op == "power.cut":
            if a.get("at_power_on"):
                self.pon = a
            else:
                self.cut, self.n_left = a, a.get("after_flash_words")
            return None
        raise AssertionError(op)

    def program(self, n: int) -> int | None:
        """n words; the word the cut lands in (1-based), or None."""
        for i in range(1, n + 1):
            if self.cut and "after_flash_words" in self.cut:
                self.n_left -= 1
                if self.n_left == 0:
                    self.log.append(dict(op="program", address=self.addr, len=32, result="cut", words_written=i - 1))
                    self.cut = None
                    return i
            self.log.append(dict(op="program", address=self.addr, len=32, result="ok"))
            self.addr += 32
        return None

    def erase(self) -> bool:
        e = dict(op="erase", address=self.addr - (self.addr - 0x0800_0000) % 0x20000, len=131072, result="ok")
        self.log.append(e)
        if self.cut and "during_erase" in self.cut:
            e["result"], e["fraction_done"] = "cut", self.cut["during_erase"]
            self.cut = None
            if self.broken == "erase-cut-built-in":
                self.tx = BUILT_IN
            return True
        return False

    def power_on(self):
        """After a cut: the start check erases what a write cut before its commit word left; a cut armed
        at_power_on cuts that erase, and the power-on after it erases the sector again."""
        while self.left is not None:
            if self.pon:
                self.cut, self.pon = self.pon, None
            if self.erase():
                if self.broken == "left-cut-stricter":
                    self.tx = min(self.tx, self.left)
                continue
            self.left = None

    def restart(self):
        """The start check erases the stale sector a CHANGE_SECRET left (A1.10)."""
        if self.stale:
            self.stale = False
            self.erase()

    def set_config(self, p: bytes):
        its, i = {}, 0
        while i < len(p):
            its[p[i]] = p[i + 2:i + 2 + p[i + 1]]
            i += 2 + p[i + 1]
        tx = struct.unpack(">I", its[7])[0] if 7 in its else None
        gw = its.get(4)
        if tx is not None and self.broken != "c3-appends":  # a C-3 write always compacts
            if self.erase():
                return None
            torn = self.program(N)
            if torn is None:
                self.tx, self.gw = tx, gw if gw is not None else self.gw
                return 0
            if torn < N:  # the new generation left without its commit word
                self.left = tx
                self.power_on()
            if torn == N:  # the commit word cut: C-1 old, C-3 the stricter
                self.tx = max(self.tx, tx) if self.broken == "commit-looser" else min(self.tx, tx)
                if self.broken == "commit-c1-new" and gw is not None:
                    self.gw = gw
            return None
        self.appended += 1
        if self.appended % 5 == 0 and self.erase():  # a C-1 compaction
            return None
        if self.program(2) is not None:
            return None
        self.tx, self.gw = tx if tx is not None else self.tx, gw if gw is not None else self.gw
        return 0


class FakeU:
    def __init__(self, r):
        self.r, self.c = r, self

    def req(self, cmd, payload=b"", window=None):
        if cmd == "SET_CONFIG":
            st = self.r.set_config(payload)
            return None if st is None else dict(status=st, payload=b"")
        if cmd == "RESTART":
            self.r.restart()
            return None
        if cmd == "GET_CONFIG":
            return dict(status=0, payload=bytes([4, 4]) + self.r.gw + bytes([7, 4]) + struct.pack(">I", self.r.tx))
        return dict(status=0, payload=b"")

    def identify(self):
        return {25: bytes([2, 2, 4, 2, 6, 2])}


def run(broken=None):
    r = C3Radio(broken)
    ctx = Ctx(dict(radio=dict(ip="10.0.0.5"), fixtures=["flash.log", "power.cut"]))
    ctx.fixture = r.fixture

    @contextmanager
    def mg(ctx_, s):
        yield FakeU(r)
    from testbench import t_store
    saved_change = t_store.change_secret

    def change(ctx_, u, new, window=16.0):
        r.stale = True
        return dict(status=0)
    t_store.change_secret = change
    saved = {k: getattr(t_flash, k) for k in ("mg", "wait_reachable", "lost_lines", "log_end")}
    t_flash.mg, t_flash.wait_reachable = mg, (lambda *a, **k: 0.0)
    t_flash.lost_lines, t_flash.log_end = (lambda ctx_, u, first: 0), (lambda u: 0)
    missing = []
    try:
        t_flash.c3_writes(ctx, b"s" * 32, missing)
    finally:
        for k, v in saved.items():
            setattr(t_flash, k, v)
        t_store.change_secret = saved_change
    return [c["name"] for c in ctx.checks if not c["ok"]], missing, ctx


class C3Writes(unittest.TestCase):
    def test_correct_store(self):
        bad, missing, ctx = run()
        self.assertEqual((bad, missing), ([], []))
        self.assertEqual(sum("cut at word" in c["name"] and "left" not in c["name"] for c in ctx.checks), N + 1)
        self.assertTrue(ctx.parts)
        self.assertEqual(sum("start check's erase cut" in c["name"] and "as before" in c["name"] for c in ctx.checks), 3)
        self.assertEqual(sum("left, cut at" in c["name"] and "as before" in c["name"] for c in ctx.checks), 6)
        self.assertFalse(any("waits for" in x for x in ctx.parts))

    def test_broken_stores(self):
        for broken, seen in (("c3-appends", "one store erase"), ("commit-c1-new", "C-1 old"),
                             ("commit-looser", "the stricter"), ("erase-cut-built-in", "the C-3 item as before"),
                             ("left-cut-stricter", "left, cut at 0.1: the C-3 item and the gateway as before")):
            bad, _, _ = run(broken)
            self.assertTrue(any(seen in b for b in bad), (broken, bad[:4]))


if __name__ == "__main__":
    unittest.main()


class CutPlaced(unittest.TestCase):
    """cut_placed: a cut found by its word count, or by its torn word being the k-th of the reference."""
    REF = [STORE + 32 * i for i in range(7)] + [STORE + 0x200]  # the commit word elsewhere

    def test_by_count(self):
        log = [dict(op="program", address=STORE, len=224, result="ok"),
               dict(op="program", address=STORE + 0x200, len=32, result="cut", words_written=0)]
        self.assertEqual(t_flash.cut_placed(log, 8, self.REF), (True, ""))

    def test_by_the_torn_word(self):
        # the count disagrees (one word logged elsewhere), the torn word is the reference's 8th
        log = [dict(op="program", address=STORE, len=192, result="ok"),
               dict(op="program", address=STORE + 0x200, len=32, result="cut", words_written=0, torn=hex(STORE + 0x200))]
        self.assertEqual(t_flash.cut_placed(log, 8, self.REF)[0], True)

    def test_not_placed_says_why(self):
        log = [dict(op="program", address=STORE, len=192, result="ok"),
               dict(op="program", address=STORE + 0xC0, len=32, result="cut", words_written=0)]
        placed, why = t_flash.cut_placed(log, 8, self.REF)
        self.assertFalse(placed)
        self.assertIn("words before the cut 6", why)
        self.assertIn(hex(STORE + 0x200), why)
