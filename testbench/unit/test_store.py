"""T-A1-92 (the old secret in flash; 0069) against a fake two-sector store: it must pass on a
correct one and fail on each broken one. No network. GPL-3.0-or-later, (c) 2026 ON8ST."""
import unittest
from contextlib import contextmanager

from testbench import t_log, t_store
from testbench.core import Ctx

A, B = 0x080C0000, 0x080E0000


class Store:
    """The live sector holds the secret in force; CHANGE_SECRET compacts into the other one."""

    def __init__(self, secret, broken=""):
        self.mem = {A: secret, B: None}  # the newest secret record of each sector
        self.live, self.broken, self.t = A, broken, 1000
        self.flog, self.sock, self.log, self.ecc = [], [], [(0, 0, "start after power-on: log lost")], set()
        self.commit = None

    def other(self, x=None):
        return A + B - (self.live if x is None else x)

    def erase(self, x):
        self.flog.append(dict(op="erase", address=x, len=131072, t_us=self.t, t_end_us=self.t + 10, result="ok"))
        self.t += 20
        self.mem[x] = None

    def compact(self, secret):
        x = self.other()
        self.erase(x)
        if self.broken == "two-erases":
            self.erase(self.live)
        self.mem[x] = secret
        self.commit = x + 64
        self.flog.append(dict(op="program", address=x, len=96, t_us=self.t, result="ok"))
        self.live = x

    def start(self, power):
        self.log = [] if power else self.log
        self.log.append((len(self.log), 0, "start after power-on: log lost" if power else "start after software: log kept"))
        if self.commit in self.ecc and self.live == self.commit - 64 and self.broken != "no-fallback":
            self.live = self.other()  # the newest commit word is unreadable: the older generation
            self.log.append((len(self.log), 0, "store: commit word unreadable, older generation used"))
        elif self.mem[self.other()] not in (None, self.mem[self.live]) and self.broken != "no-start-check":
            self.erase(self.other())
            self.log.append((len(self.log), 0, "store: stale secret erased"))
        elif self.broken == "always-erases":
            self.erase(self.other())
        self.t += 5
        self.sock.append(dict(action="OPEN", port=7251, t_us=self.t))


class Client:
    reply_s, why = 0.5, ""


class U:
    def __init__(self, st, s):
        self.st, self.s, self.c = st, s, Client()

    def req(self, cmd, payload=b"", window=None):
        return dict(status=0 if self.s == self.st.mem[self.st.live] else 33)


class FakeCtx(Ctx):
    def __init__(self, st, dump=True):
        fx = ["flash.log", "flash.ecc", "power.cycle", "sock.log"] + (["flash.dump"] if dump else [])
        super().__init__(dict(radio=dict(ip="127.0.0.1"), fixtures=fx))
        self.st = st

    def fixture(self, op, /, _timeout=30, **a):
        st = self.st
        if op in ("flash.log", "sock.log"):
            log = st.flog if op == "flash.log" else st.sock
            if a["action"] == "start":
                log.clear()
                return None
            n = a.get("from", 0)
            return dict(count=len(log), dropped=0, entries=log[n:])
        if op == "flash.dump":
            x = a["address"] & ~0x1FFFF
            s = st.mem[x] if a["address"] == x else None
            return dict(hex=((s or b"").ljust(a["len"], b"\xff")).hex())
        if op == "flash.ecc":
            st.ecc.add(a["address"])
        elif op == "power.cycle":
            st.start(True)
        else:
            raise AssertionError(op)


def run(broken="", dump=True):
    st = Store(b"s" * 32, broken)
    ctx = FakeCtx(st, dump)

    @contextmanager
    def mg(ctx_, s):
        yield U(st, s)

    @contextmanager
    def updating(ctx_, s, restart_at_end=True):
        yield U(st, s)
        if restart_at_end:
            st.start(False)

    def change(ctx_, u, new):
        st.compact(new)
        t_store.keep_secret(ctx_, new)
        u.s = new
        return dict(status=0)

    def compaction(ctx_, u, *a, **k):
        st.compact(st.mem[st.live])
        ctx_.__dict__["_reply"] = (1.0, 1)
        return 1, 1
    new = dict(mg=mg, updating=updating, change_secret=change, compaction_writes=compaction, secret=lambda c: b"s" * 32,
               wait_reachable=lambda *a, **k: 0.0, sectors=lambda u: [A, B], store_range=lambda u: (A, B + 0x20000),
               read_log=lambda c: st.log)
    saved = {k: getattr(t_store, k) for k in new}
    saved_entries, t_log.entries = t_log.entries, lambda ctx_: st.log
    for k, v in new.items():
        setattr(t_store, k, v)
    try:
        t_store.t_a1_92(ctx)
    finally:
        t_log.entries = saved_entries
        for k, v in saved.items():
            setattr(t_store, k, v)
    return [c["name"] for c in ctx.checks if not c["ok"]]


class OldSecret(unittest.TestCase):
    def test_correct_store(self):
        self.assertEqual(run(), [])

    def test_correct_store_without_a_dump(self):
        self.assertEqual(run(dump=False), [])

    def test_broken_stores_fail(self):
        for broken in ("no-start-check", "two-erases", "always-erases", "no-fallback"):
            self.assertTrue(run(broken), broken)


if __name__ == "__main__":
    unittest.main()
