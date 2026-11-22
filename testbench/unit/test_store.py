"""T-A1-92 (the old secret in flash, the start check; 0069 and its addendum) against a fake two-sector
store: it must pass on a correct one and fail on each broken one. The fake's layout is invented for
this test. No network. GPL-3.0-or-later, (c) 2026 ON8ST."""
import struct
import unittest
from contextlib import contextmanager
from unittest import mock

from testbench import core, t_flash, t_log, t_stale

A, B = 0x080C0000, 0x080E0000
SECTOR, W = 131072, 32
BLANK = b"\xff" * W


class Cut(Exception):
    pass


def word(kind: bytes, value: int = 0) -> bytes:
    return kind + struct.pack(">I", value) + b"\0" * (W - 5)


class Store:
    """Words of 32 bytes: F first generation, H n compacted generation n, S then 32 bytes a secret,
    T a configuration value, B the boot counter, C n the commit of generation n."""

    def __init__(self, secret, broken=""):
        self.mem, self.ecc = {A: {}, B: {}}, set()
        self.broken, self.t, self.flog, self.sock, self.log = broken, 1000, [], [], [(0, 0, "start after power-on: log lost")]
        self.cut, self.boots = None, 0
        for x, words in ((A, [word(b"H", 1), word(b"S"), secret, word(b"T", 300_000), word(b"C", 1)]),):
            self.mem[x] = dict(enumerate(words))

    def other(self, x):
        return A + B - x

    # ---- flash
    def erase(self, x):
        e = dict(op="erase", address=x, len=SECTOR, t_us=self.t, t_end_us=self.t + 10, result="ok")
        self.flog.append(e)
        self.t += 20
        n = len(self.mem[x])
        if self.cut and self.cut[0] == "erase":
            f, self.cut = self.cut[1], None
            e.update(result="cut", fraction_done=f)
            for k in sorted(self.mem[x])[:max(1, int(n * f))]:
                self.blank(x, k)
            raise Cut
        for k in list(self.mem[x]):
            self.blank(x, k)

    def blank(self, x, k):
        self.mem[x].pop(k, None)
        self.ecc.discard(x + k * W)

    def program(self, x, words):
        at = max(self.mem[x], default=-1) + 1
        e = dict(op="program", address=x + at * W, len=0, t_us=self.t, result="ok")
        self.flog.append(e)
        for k, w in enumerate(words):
            e["len"] += W
            self.mem[x][at + k] = w
            if self.cut and self.cut[0] == "words":
                self.cut = ("words", self.cut[1] - 1)
                if not self.cut[1]:
                    self.cut = None
                    self.mem[x][at + k] = w[:16] + b"\xff" * 16
                    self.ecc.add(x + (at + k) * W)
                    e.update(result="cut", words_written=k)
                    raise Cut

    def read(self, x):
        """(generation or None, its complete records, any word not in a complete checked record)."""
        m, gen, recs, junk, k = self.mem[x], None, [], False, 0
        for k in sorted(m):
            w, ok = m[k], x + k * W not in self.ecc
            c = w[:1]
            n = struct.unpack(">I", w[1:5])[0]
            valid = ok and w[5:] == b"\0" * (W - 5) and c in b"FHSTBC"
            if k - 1 in m and m[k - 1] == word(b"S") and x + (k - 1) * W not in self.ecc and k != 0:
                if ok:
                    recs.append((b"S", w))
                else:
                    junk = True
            elif not valid:
                junk = True
            elif c == b"F":
                gen = ("first", 0)
            elif c == b"H":
                gen = ("new", n)
            elif c == b"C":
                gen, junk = (("committed", n), junk) if gen == ("new", n) else (gen, True)
            elif c in b"TB":
                recs.append((c, n))
            elif c == b"S" and k + 1 not in m:
                junk = True
        if gen and gen[0] == "new":
            if self.broken == "no-fallback":
                gen = ("committed", gen[1])
            else:
                junk = True  # records of a generation never committed stay recognisable, the sector does not
        return gen, recs, junk

    # ---- the store
    def live(self):
        best = None
        for x in (A, B):
            g = self.read(x)[0]
            if g and g[0] in ("first", "committed") and (best is None or g[1] > best[1]):
                best = (x, g[1])
        return best and best[0]

    def get(self, kind, x=None):
        x = x or self.live()
        got = [v for k, v in self.read(x)[1] if k == kind] if x else []
        return got[-1] if got else None

    @property
    def secret(self):
        return self.get(b"S")

    def append(self, words):
        x = self.live()
        if x is None:  # the first write erases the sector it writes into, unless it is blank
            x = B if not self.mem[B] or self.mem[A] else A
            if self.mem[x]:
                self.erase(x)
            words = [word(b"F")] + words
        self.program(x, words)

    def compact(self, secret=None):
        x, live = self.other(self.live()), self.live()
        n = self.read(live)[0][1] + 1
        secret, cfg = secret or self.secret, self.get(b"T")
        self.erase(x)
        if self.broken == "two-erases":
            self.erase(live)
        body = [word(b"H", n)] + ([word(b"S"), secret] if secret else []) + ([word(b"T", cfg)] if cfg else [])
        self.program(x, body + [word(b"C", n)])

    def config(self, ms):
        self.append([word(b"T", ms)])

    # ---- a start
    def start(self, power):
        while True:
            try:
                return self.boot(power)
            except Cut:
                power = True

    def say(self, text):
        self.log.append((len(self.log), 0, text))

    def boot(self, power):
        self.log = [] if power else self.log
        self.say("start after power-on: log lost" if power else "start after software: log kept")
        self.boots += 1
        for x in (A, B):
            g = self.read(x)[0]
            if g and g[0] == "new" and len(self.mem[x]) > 3:
                self.say("store: commit word unreadable, older generation used")
        bare = self.live() is None
        self.append([word(b"B", self.boots)])
        self.check(bare)
        self.t += 5
        self.sock.append(dict(action="OPEN", port=7251, t_us=self.t))

    def check(self, bare):
        x, br = self.live(), self.broken
        if br == "no-start-check":
            return
        gen, recs, _ = self.read(x)
        kinds = {k for k, _ in recs}
        need = {b"S"} if br == "config-does-not-count" else {b"S", b"T", b"B"} if br == "counter-counts" else {b"S", b"T"}
        if gen[0] == "first" and not kinds & need and br != "no-complete-rule":
            return
        y = self.other(x)
        g, recs, junk = self.read(y)
        stale = any(k == b"S" and v != self.secret for k, v in recs)
        if br == "keeps-failed" and g and g[0] == "new":
            return
        if br == "keeps-torn":
            junk = False
        if br == "whole-records-only" and not any(k != b"B" for k, _ in recs) and len(self.mem[y]) > 1:
            junk = False
        if stale or junk or br == "always-erases":
            self.erase(y)
            if br == "wrong-line":
                stale, junk = not stale, not junk
            self.say(t_stale.STALE if stale else t_stale.UNREC)

    # ---- fixtures
    def dump(self, address, n):
        x = A if address < B else B
        out = bytearray(b"\xff" * SECTOR)
        for k, w in self.mem[x].items():
            out[k * W:k * W + W] = w
        return bytes(out[address - x:address - x + n])

    def fill(self, address, n, data):
        x = A if address < B else B
        data = (data * (n // len(data) + 1))[:n] if data else b"\xff" * n
        for k in range(0, n, W):
            i = (address - x + k) // W
            self.blank(x, i)
            if data[k:k + W] != BLANK:
                self.mem[x][i] = data[k:k + W]


class Client:
    reply_s, why = 0.5, ""


class U:
    def __init__(self, st, s):
        self.st, self.s, self.c = st, s, Client()


FIXTURES = ["flash.log", "flash.ecc", "power.cycle", "sock.log", "flash.dump", "flash.fill", "power.cut"]


class FakeCtx(core.Ctx):
    def __init__(self, st, without=()):
        super().__init__(dict(radio=dict(ip="127.0.0.1"), fixtures=[f for f in FIXTURES if f not in without]))
        self.st = st

    def fixture(self, op, /, _timeout=30, **a):
        st = self.st
        self.need(op)
        if op in ("flash.log", "sock.log"):
            log = st.flog if op == "flash.log" else st.sock
            if a["action"] == "start":
                log.clear()
                return None
            return dict(count=len(log), dropped=0, entries=[dict(e) for e in log[a.get("from", 0):]])
        if op == "flash.dump":
            return dict(hex=st.dump(a["address"], a["len"]).hex())
        if op == "flash.fill":
            st.fill(a["address"], a["len"], bytes.fromhex(a.get("data", "")))
        elif op == "flash.ecc":
            st.ecc.add(a["address"])
        elif op == "power.cut":
            st.cut = ("erase", a["during_erase"]) if "during_erase" in a else ("words", a["after_flash_words"])
        elif op == "power.cycle":
            st.start(True)
        else:
            raise AssertionError(op)


def run(broken="", without=()):
    original = bytes(range(32))
    st = Store(original, broken)
    ctx = FakeCtx(st, without)

    @contextmanager
    def mg(ctx_, s):
        yield U(st, s)

    @contextmanager
    def updating(ctx_, s, restart_at_end=True):
        yield U(st, s)
        st.start(False)

    def safely(work):
        try:
            work()
        except Cut:
            st.start(True)
            return None
        return dict(status=0)

    def change(ctx_, u, new, window=16.0):
        if u.s != st.secret:
            return dict(status=33)
        r = safely(lambda: st.compact(new))
        if r:
            t_stale.keep_secret(ctx_, new)
        return r

    def compaction(ctx_, u, *a, **k):
        st.compact()
        ctx_.__dict__["_reply"] = (1.0, 1)
        return 1, 1

    def first(ctx_, new):
        if st.secret is not None:
            return 36
        st.append([word(b"S"), new])
        return 0

    def config(u, ms, window=None):
        st.config(ms)
        return dict(status=0)
    new = dict(mg=mg, updating=updating, change_secret=change, compaction_writes=compaction, secret=lambda c: original,
               wait_reachable=lambda *a, **k: 0.0, sectors=lambda u: [A, B], store_range=lambda u: (A, B + SECTOR),
               first_secret=first, set_tx_time=config, tx_time=lambda u: st.get(b"T"), accepts=lambda c, s: 0 if s == st.secret else 33,
               ident=lambda c: {11: b"\1" if st.secret else b"\0", 12: b"\0" if st.secret else b"\1\1\x2c", 23: b"\1"},
               Mgmt=mock.MagicMock(), run_texts=lambda c: [e[2] for e in st.log[t_log.last_start(st.log):]])
    with mock.patch.multiple(t_stale, **new):
        t_stale.t_a1_92(ctx)
    ctx.store = st
    return ctx


def failed(ctx):
    return [f"{c['name']}: {c['detail']}" for c in ctx.checks if not c["ok"]]


class OldSecret(unittest.TestCase):
    def test_correct_store(self):
        ctx = run()
        self.assertEqual(failed(ctx), [])
        self.assertEqual(ctx.parts, [])  # every case ran to its end
        self.assertEqual(ctx.notes.count(t_flash.ECC_LIMIT), 1)  # what the emulator cannot show is in the result, once
        self.assertEqual(ctx.store.secret, bytes(range(32)))  # put back

    def test_every_case_judged(self):
        names = " ".join(c["name"] for c in run().checks)
        for what in ("a second start: nothing erased", "only the live secret is not erased", "foreign word at offset 0x1ffe0", "cut at 0.5: completed", "cut at 0.9, not erased again",
                     "cut at word 1 ", "the old secret authenticates, the new one gets 33", "lost sector 6, the second start",
                     "the next start erases sector 6 once", "the next start erases the other sector", "the first generation kept"):
            self.assertIn(what, names)

    def test_parts_without_a_fixture(self):
        for f in ("flash.dump", "flash.fill", "flash.ecc", "power.cut"):
            ctx = run(without=(f,))
            self.assertEqual(failed(ctx), [], f)
            self.assertTrue(ctx.parts, f)

    def test_broken_stores_fail(self):
        # each fault and one check that must see it
        for broken, seen in (("no-start-check", "RESTART: the stale sector erased once"), ("two-erases", "one erase"),
                             ("always-erases", "a second start: nothing erased"), ("no-fallback", "fall-back"),
                             ("keeps-failed", "fall-back: the sector with the failed newer generation"),
                             ("keeps-torn", "a foreign word at offset"), ("wrong-line", "the start check's lines"),
                             ("no-complete-rule", "lost sector 6, the first start: sector 0x80c0000 not erased"),
                             ("counter-counts", "lost sector 6, the first start: sector 0x80c0000 not erased"),
                             ("config-does-not-count", "only configuration: the failed newer generation erased"),
                             ("whole-records-only", "cut at")):
            try:
                bad = failed(run(broken))
            except core.RunnerFault:
                bad = ["runner"]
            self.assertTrue(any(seen in b for b in bad), (broken, bad[:6]))

    def test_planned_cuts_not_seen(self):
        with mock.patch.object(FakeCtx, "fixture", lambda self, op, /, _f=FakeCtx.fixture, **a: None if op == "power.cut" else _f(self, op, **a)):
            with self.assertRaises(core.RunnerFault):
                run()


if __name__ == "__main__":
    unittest.main()
