"""Verdicts of tests that end early, the host rule of 0069 in the plain request functions, and the
endpoint read again. No network. GPL-3.0-or-later, (c) 2026 ON8ST."""
import struct
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

    def test_whole_claim(self):
        from testbench import t_flash
        for fn in (t_flash.t_a1_83, t_flash.t_a1_86):
            self.assertTrue(fn.whole)

        def fn(ctx):
            ctx.check("one part", True, "detail")
            raise core.PartSkip("the other part")
        fn.whole = True
        self.assertEqual(run(fn)["status"], "skip")

    def test_no_pass(self):
        for end in (None, core.PartSkip("x")):
            def fn(ctx):
                ctx.check("one part", True, "detail")
                ctx.part("another part not judged")
                ctx.no_pass = True
                if end:
                    raise end
            self.assertEqual(run(fn)["status"], "skip")

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


class WholeWhereOffered(unittest.TestCase):
    def test_tuple(self):
        from testbench import t_log, t_stale, t_update
        for fn in (t_update.t_a1_89, t_update.t_a1_90, t_log.t_a1_91, t_stale.t_a1_92):
            self.assertIsInstance(fn.whole, tuple)

        def noted(ctx):
            ctx.check("one part", True, "detail")
            ctx.part("the other part not run")

        def skipped(ctx):
            ctx.check("one part", True, "detail")
            raise core.PartSkip("the other part")
        for fn in (noted, skipped):
            fn.whole = ("x.y", "package_slot_a")
            for fixtures, params, want in ((["x.y"], {}, "pass"), ([], dict(package_slot_a="p"), "pass"),
                                           (["x.y"], dict(package_slot_a="p"), "skip")):
                core.REGISTRY["T-UNIT-V"] = fn
                try:
                    res = core.run_one("T-UNIT-V", core.Ctx(dict(fixtures=fixtures), params))
                finally:
                    del core.REGISTRY["T-UNIT-V"]
                self.assertEqual(res["status"], want)
                self.assertIn("the other part", res["reason"])


class PushBeforeReply(unittest.TestCase):
    """T-B5-21: the disarm push leaves the radio before the reply (A1.9)."""
    PUSH = (b"\0\0\0\x07" + b"\0" * 12, 10.0)

    def judge(self, sends, netlog=True, t_reply=10.0, pushes=None):
        from testbench.mgmt import build
        from testbench.t_mgmt import push_before_reply
        reply = lambda rid: (7251, 41000, build(0x22, b"", rid, typ=2)[:16])
        push = lambda seq: (7250, 38000, struct.pack(">I", seq) + b"\0" * 12)
        make = dict(push=push(7), other=push(6), reply=reply(99), earlier=reply(98))
        entries = [dict(t_us=k, dir="from_radio", radio_port=p, peer=f"1.2.3.4:{h}", head=head.hex())
                   for k, (p, h, head) in enumerate(make[x] for x in sends)]
        ctx = core.Ctx({})
        with mock.patch.object(core.Ctx, "fixture", lambda self, op, /, **a: dict(count=len(entries), dropped=0, entries=entries)):
            push_before_reply(ctx, "claim", [self.PUSH] if pushes is None else pushes, 38000, 99, t_reply, 41000, netlog)
        c = [c for c in ctx.checks if c["name"] == "claim"]
        return (c[0]["ok"], c[0]["detail"]) if c else (None, "")

    def test_net_log_order(self):
        ok, detail = self.judge(["earlier", "other", "push", "reply"])
        self.assertTrue(ok and "net.log" in detail, detail)
        # the reply first, however close, is a fail, whatever the arrivals say
        self.assertFalse(self.judge(["earlier", "other", "reply", "push"], t_reply=99.0)[0])
        self.assertFalse(self.judge(["reply", "push", "reply"])[0])  # a repeat of the reply does not help

    def test_missing_entry_is_the_runners(self):
        for sends in (["push"], ["reply", "other"]):
            with self.assertRaises(core.RunnerFault):
                self.judge(sends)

    def test_two_sockets(self):
        for t_reply, want in ((10.0005, True), (9.9995, True), (9.998, False), (9.996, False)):
            ok, detail = self.judge([], netlog=False, t_reply=t_reply)
            self.assertEqual(ok, want)
            self.assertIn("two sockets", detail)

    def test_no_push(self):
        self.assertIsNone(self.judge(["reply"], pushes=[])[0])


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


START = "start after software: log "
LOG0 = [(4, 0, "start after power-on: log lost"), (5, 900, "an entry")]
AFTER = LOG0 + [(6, 31000, "restart: factory-reset"), (7, 0, START + "kept")]


class ResetDone(unittest.TestCase):
    """An answer between the FACTORY_RESET reply and the reset does not count as the new start."""

    def wait(self, answers, up0=30.0):
        """`answers`: (log, uptime) per poll, the last one repeated; radio time goes on 4 s per poll."""
        class Clock:
            emu, t = False, 100.0

            def now(self):
                self.t += 4.0
                return self.t
        clock, ctx, n = Clock(), core.Ctx(dict(radio=dict(ip="127.0.0.1"))), [0]

        def read(c):
            n[0] += 1
            return answers[min(n[0], len(answers)) - 1][0]
        with mock.patch.object(t_store, "wait_reachable", lambda *a, **k: 0.0), mock.patch.object(t_store, "Mgmt", mock.MagicMock()), \
                mock.patch.object(t_store, "read_log", read), mock.patch.object(t_store.time, "sleep"), \
                mock.patch.object(t_store, "uptime", lambda c: answers[min(n[0], len(answers)) - 1][1]):
            why = t_store.reset_done(ctx, "FACTORY_RESET", (clock, LOG0, up0))
        return why, n[0], [c["ok"] for c in ctx.checks]

    def test_uptime_stands_still_during_erases(self):
        # two stretched erases of 4 s: the uptime runs 8 s late, the image has not restarted
        why, n, checks = self.wait([(LOG0, 30.0), (LOG0, 30.0), (LOG0 + [(6, 30000, "restart: factory-reset")], 30.1), (AFTER, 1.0)])
        self.assertEqual((n, checks), (4, []))
        self.assertIn("start entry", why)

    def test_young_image_late_uptime(self):
        why, n, checks = self.wait([(LOG0, 1.0)], up0=1.0)  # 1 s old before the request, and it never restarts
        self.assertEqual((why, checks), ("", [False]))

    def test_new_start_older_than_before(self):
        # the new image is reached later than the old one's uptime: only the start entry tells
        self.assertIn("start entry", self.wait([(AFTER, 6.0)], up0=2.0)[0])

    def test_log_lost(self):
        new = [(0, 0, START + "lost")]
        self.assertIn("start entry", self.wait([(new, 6.0)], up0=2.0)[0])
        self.assertIn("dropped", self.wait([([], 1.0)])[0])

    def test_no_answer_is_no_evidence(self):
        why, n, checks = self.wait([([], None), (AFTER, None)])
        self.assertEqual((n, checks), (2, []))

    def test_the_log_before_is_read_whole(self):
        with mock.patch.object(mgmt, "req_retransmit", lambda *a, **k: None):
            c = mock.MagicMock(why="no reply")
            self.assertEqual(mgmt.read_log(c), [])
            with self.assertRaises(core.Missing):
                mgmt.read_log(c, complete=True)


class ResetTime(unittest.TestCase):
    """What the 17 s check rests on."""

    def took(self, emu, boot):
        class Clock:
            t, last = 100.0, 0

            def now(self):
                self.t += 1.0
                self.last = int(self.t * 1e6)
                return self.t
        u = mock.MagicMock()
        u.c.clock = Clock()
        u.c.clock.emu = emu
        u.req.return_value = dict(status=0)
        with mock.patch.object(t_store, "state_before", lambda c: None), mock.patch.object(t_store, "reset_done", lambda *a: "seen"), \
                mock.patch.object(t_store, "Mgmt", mock.MagicMock()), mock.patch.object(t_store, "uptime", lambda c: 0.25), \
                mock.patch("testbench.t_a3.boot_t_us", lambda ctx: boot):
            return t_store.factory_reset(core.Ctx(dict(radio=dict(ip="127.0.0.1"))), u)

    def test_basis(self):
        self.assertEqual(self.took(True, 105_000_000), (0, 3.0, "the boot event of hw.trace"))  # the reply at 102 s
        st, took, basis = self.took(True, 50_000_000)  # only the boot event of an earlier start
        self.assertEqual((took, basis), (0.75, t_store.UPTIME_BASIS))
        self.assertIs(self.took(False, 0)[2], t_store.UPTIME_BASIS)


if __name__ == "__main__":
    unittest.main()
