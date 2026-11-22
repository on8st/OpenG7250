"""Verdicts of tests that end early, the host rule of 0069 in the plain request functions, and the
endpoint read again. No network. GPL-3.0-or-later, (c) 2026 ON8ST."""
import unittest
from unittest import mock

from testbench import core, mgmt, p2, t_store


def run(fn, fixtures=()):
    core.REGISTRY["T-UNIT-V"] = fn
    try:
        return core.run_one("T-UNIT-V", core.Ctx(dict(fixtures=list(fixtures))))
    finally:
        del core.REGISTRY["T-UNIT-V"]


class Verdicts(unittest.TestCase):
    def case(self, exc, ok=None):
        def fn(ctx):
            if ok is not None:
                ctx.check("a check", ok, "detail")
            if exc:
                raise exc
        return run(fn)["status"]

    def test_early_ends(self):
        self.assertEqual(self.case(core.NotShown("x"), True), "skip")
        self.assertEqual(self.case(core.NotShown("x"), False), "fail")
        self.assertEqual(self.case(core.PartSkip("x"), True), "pass")
        self.assertEqual(self.case(core.PartSkip("x"), False), "fail")
        self.assertEqual(self.case(core.PartSkip("x")), "skip")
        self.assertEqual(self.case(core.Skip("x"), False), "fail")
        self.assertEqual(self.case(core.Skip("x"), True), "skip")
        self.assertEqual(self.case(None), "fail")

    def test_flash_rules(self):
        bad = dict(op="erase", address="0x08000000", len=131072)

        def call(entries, exc, ok=None):
            def fixture(self, op, /, _timeout=30, **a):
                return dict(count=len(entries), dropped=0, entries=entries) if a.get("action") == "read" else None

            def fn(ctx):
                if ok is not None:
                    ctx.check("a check", ok, "detail")
                if exc:
                    raise exc
            with mock.patch.object(core.Ctx, "fixture", fixture):
                return run(fn, ["flash.log"])["status"]
        self.assertEqual(call([], None, True), "pass")
        self.assertEqual(call([], None), "fail")  # the rules alone show nothing of the claim
        self.assertEqual(call([], core.PartSkip("x")), "skip")
        self.assertEqual(call([], core.PartSkip("x"), True), "pass")
        self.assertEqual(call([bad], core.PartSkip("x"), True), "fail")  # judged also when a part is not run


class Net:
    radio_ip = "127.0.0.1"

    def bind(self, s, idx, port):
        return 0


class HostRule(unittest.TestCase):
    """A radio that answers only the n-th datagram."""

    def sends(self, call, nth):
        c, sent = mgmt.Mgmt(Net()), []

        def raw(data, timeout=0.5, kind="unicast"):
            sent.append(data)
            return [dict(status=0, payload=b"c" * 16)] if len(sent) >= nth else []
        c.raw = raw
        try:
            return call(c), sent
        finally:
            c.close()

    def test_retransmitted(self):
        for call in (lambda c: c.req("SET_FIRST_SECRET", b"x" * 32),
                     lambda c: c.authed("CHANGE_SECRET", b"s" * 32, b"x" * 32, challenge=b"c" * 16)):
            r, sent = self.sends(call, 4)
            self.assertEqual((r["status"], len(sent), len(set(sent))), (0, 4, 1))  # the same bytes

    def test_not_retransmitted(self):
        for call in (lambda c: c.req("IDENTIFY"), lambda c: c.authed("END_WRITE_SESSION", b"s" * 32, challenge=b"c" * 16),
                     lambda c: c.req("SET_FIRST_SECRET", b"x" * 32, timeout=0.1)):
            r, sent = self.sends(call, 2)
            self.assertEqual((r, len(sent)), (None, 1))


class Reread(unittest.TestCase):
    def test_lost_keys_kept_and_named(self):
        ctx = core.Ctx(dict(fixtures=["net.endpoint", "flash.timing"], radio=dict(ip="1"), flash_timing=dict(t_erase_max_us=4), minor=7,
                            _learned=dict(secret="aa")))
        with mock.patch.object(core.Ctx, "fixture", lambda self, op, /, **a: dict(fixtures=["net.endpoint"], radio=dict(ip="2"), minor=8)):
            p2.reread_endpoint(ctx)
            p2.reread_endpoint(ctx)
        e = ctx.endpoint
        self.assertEqual((e["radio"]["ip"], e["minor"], e["flash_timing"], e["_learned"]), ("2", 8, dict(t_erase_max_us=4), dict(secret="aa")))
        self.assertEqual(len(ctx.notes), 1)
        self.assertIn("flash_timing", ctx.notes[0])
        self.assertNotIn("_learned", ctx.notes[0])

    def test_which_is_missing(self):
        for ep, yes, no in ((dict(fixtures=["flash.timing"]), "no flash_timing", "not offered"),
                            (dict(fixtures=[], flash_timing=dict(t_erase_max_us=4)), "not offered", "no flash_timing")):
            ctx = core.Ctx(ep)
            t_store.stretched(ctx)()
            self.assertIn(yes, ctx.notes[0])
            self.assertNotIn(no, ctx.notes[0])


class ResetDone(unittest.TestCase):
    """An answer between the FACTORY_RESET reply and the reset does not count as the new start."""

    def wait(self, starts, up0=30.0):
        class Clock:
            t = 100.0

            def now(self):
                self.t += 0.5
                return self.t
        clock, ctx, it = Clock(), core.Ctx(dict(radio=dict(ip="127.0.0.1"))), iter(starts)
        with mock.patch.object(t_store, "wait_reachable", lambda *a, **k: 0.0), mock.patch.object(t_store, "Mgmt", mock.MagicMock()), \
                mock.patch.object(t_store, "start_of", lambda c: (clock, next(it, 70.0), 1.0)), mock.patch.object(t_store.time, "sleep"):
            return t_store.reset_done(ctx, "FACTORY_RESET", (clock, 70.0, up0)), [c["ok"] for c in ctx.checks]

    def test_waits_for_the_new_start(self):
        self.assertEqual(self.wait([70.1, None, 70.0, 112.0]), (112.0, []))

    def test_no_reset_fails(self):
        self.assertEqual(self.wait([]), (None, [False]))


if __name__ == "__main__":
    unittest.main()
