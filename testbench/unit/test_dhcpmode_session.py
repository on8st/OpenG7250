"""T-A1-100's session judgments (A1.11a; A2.3; B5.4 "Restart"; A3.6a) against a fake radio that keeps
P2 sessions, on a fake clock: the record does not restart what it observes, an address change is judged
as an end with a new session that carries nothing over, and a session that wrongly ends and is
restarted by the bench's RUN=1 fails a "does not end" check. No network.
GPL-3.0-or-later, (c) 2026 ON8ST."""
import unittest

from testbench import t_dhcpmode as d
from testbench.core import Ctx
from testbench.net import Net
from testbench.p2 import (KEYED_TIMEOUT, PORT_DDC, PORT_DDC0, PORT_DISC, PORT_HP, PORT_REG, STREAM_STATUS, TX_ARM,
                          TX_ARM_STATE, TX_ARM_TOKEN, TX_MODE_CLASS, TX_STATE, parse_regs, regs)

OLD, NEW = "192.168.73.50", "192.168.73.10"
HOST = ("192.168.73.200", 9100)


def net(ip, base):
    ports = (1024, 1025, 1026, 1027, 1029, 1035, 7250, 7251)
    return Net(dict(radio=dict(ip=ip, connect={str(p): f"127.0.0.1:{base + p}" for p in ports})))


class Radio:
    """One radio: a session (owner = the one fake host), its RX side (DDC0 enabled) and TX settings, on
    a fake clock. `broken`: "old-starts" (a packet to the old address still starts a session),
    "carry" (the restart after an address change keeps the RX and TX settings), "wrong-end" (the session
    ends at `end_at` without an address change and is restarted by RUN=1 as by the same host)."""

    def __init__(self, nets, broken=None, change_at=None, end_at=None, former=None, general_delay=0.0):
        self.t, self.nets, self.broken, self.former = 100.0, nets, broken, former
        self.general_delay, self.pending = general_delay, []  # a General taken that long after its arrival
        self.last_hp = 0.0
        self.netlog = []  # net.log entries of the bench host's datagrams (dropped / to_radio)
        self.ip, self.old = nets[0].radio_ip, None
        self.session_ip = self.ip
        self.change_at, self.end_at = change_at, end_at
        self.running, self.network_new = False, False
        self.ddc0, self.pa, self.mc, self.kt, self.wd_general = False, False, 0, 500, False
        self.out, self.next_status, self.next_iq, self.token = [], 0.0, 0.0, 0x1234

    def src(self, port, ip=None):
        if self.former is not None and self.old is not None and (ip or self.ip) == self.old:
            return self.former.connect[port]  # after the move the old address is reached through radio.former
        n = next(n for n in self.nets if n.radio_ip == (ip or self.ip))
        return n.connect[port]

    def end(self, network_new):
        self.running, self.pa, self.mc, self.kt, self.wd_general = False, False, 0, 500, False
        if network_new:
            self.network_new = True
            if self.broken != "carry":
                self.ddc0 = False
        if self.broken == "carry":
            self.pa, self.mc, self.kt = True, 1, 300

    def tick(self, until):
        while self.t < until:
            for g in [g for g in self.pending if g[0] <= self.t]:
                self.pending.remove(g)
                self.general(g[1])
            if self.running and self.t - self.last_hp > 1.0:  # the 1 s watchdog (B5.6): a same-network end
                self.end(False)
            if self.change_at is not None and self.t >= self.change_at:
                self.change_at, self.old, self.ip = None, self.ip, self.nets[1].radio_ip
                self.end(True)
            if self.end_at is not None and self.t >= self.end_at:
                self.end_at = None
                self.end(False)
            if self.running and self.t >= self.next_status:
                self.out.append((b"\0" * 60, self.src(PORT_DDC, self.session_ip), self.t))
                self.next_status = self.t + 0.05
            if self.running and self.ddc0 and self.t >= self.next_iq:
                self.out.append((b"\0" * 1444, self.src(PORT_DDC0, self.session_ip), self.t))
                self.next_iq = self.t + 0.01
            self.t = round(self.t + 0.005, 6)

    def general(self, data):
        self.pa = bool(data[58] & 1)
        self.wd_general = bool(data[38] & 1) and self.broken != "no-general"

    def to_radio(self, dest, port, data):
        if self.former is not None and self.former.connect.get(port) == dest:
            ip = self.former.radio_ip
        else:  # radio.connect: shared before and after the move, it reaches the current address
            ips = [n.radio_ip for n in self.nets if n.connect.get(port) == dest]
            ip = self.ip if self.ip in ips else (ips[0] if ips else None)
        e = dict(t_us=int(self.t * 1e6), socket=0, radio_port=port, peer="%s:%d" % HOST, len=len(data), head="")
        if ip != self.ip and not (self.broken in ("old-starts", "old-taken") and ip == self.old):
            self.netlog.append(dict(e, dir="dropped", reason=d.NOT_OURS))  # the chip's unicast filter
            return  # not this radio's address: never taken
        self.netlog.append(dict(e, dir="to_radio"))
        if self.broken == "old-taken" and ip == self.old:
            return  # taken by the chip, starts nothing
        if port == PORT_HP and data[4] & 1:
            self.last_hp = self.t
            if not self.running:
                self.running, self.next_status, self.session_ip = True, self.t, ip
        elif port == PORT_DISC and len(data) == 60:
            if self.general_delay:
                self.pending.append((self.t + self.general_delay, data))
            else:
                self.general(data)
        elif port == PORT_REG:
            ans = []
            for a, v in parse_regs(data):
                if a == TX_STATE:
                    ans.append((a, (0 if self.pa else 1) << 4))
                elif a == TX_MODE_CLASS:
                    if 1 <= v <= 4:
                        self.mc = v
                    ans.append((a, self.mc))
                elif a == KEYED_TIMEOUT:
                    if 100 <= v <= 500:
                        self.kt = v
                    ans.append((a, self.kt))
                elif a == STREAM_STATUS:
                    ans.append((a, 0x82 if self.ddc0 and self.running else 1))
                elif a == TX_ARM_TOKEN:
                    ans.append((a, self.token))
                elif a == TX_ARM:
                    ans.append((TX_ARM_STATE, (9 if self.wd_general else 3) << 8))
            self.out.append((regs(ans), self.src(PORT_REG), self.t))


class Host:
    """The bench host's face to Stream and the judgments: send, keepalive, write, recv_raw."""

    def __init__(self, radio, n):
        self.r, self.net, self.radio, self.running, self.hp_kw = radio, n, n.radio_ip, True, {}
        self.port = HOST[1]

    def send(self, port, data, kind="unicast"):
        self.r.to_radio(self.net.dest(port, kind), port, data)

    def keepalive(self, **kw):
        from testbench.p2 import high_priority
        self.send(PORT_HP, high_priority(run=True))

    def recv_raw(self, timeout):
        self.r.tick(self.r.t + timeout)
        out, self.r.out = self.r.out, []
        return out

    def write(self, pairs, timeout=0.3):
        self.keepalive()
        self.send(PORT_REG, regs(pairs))
        got = []
        for data, src, _ in self.recv_raw(0.3):  # as Host.write: 0.3 s, other datagrams lost to the read
            if src == self.r.src(PORT_REG, self.r.ip) or src == self.r.src(PORT_REG, self.r.old or self.r.ip):
                if len(data) > 4 and len(data) % 6 == 4 and len(data) < 200:
                    got += parse_regs(data)
        return got


def case(apart=True, former=False, netlog=True, **kw):
    """`former`: the emulator's mapping since minor 10: radio.connect shared before and after the move,
    radio.former at base + 300 for the old address, net.log with the chip's drops."""
    nets = [net(OLD, 41000), net(NEW, 42000 if apart and not former else 41000)]
    fnet = net(OLD, 41300) if former else None
    radio = Radio(nets, former=fnet, **kw)
    h = Host(radio, nets[0])
    ep = dict(radio=dict(ip=OLD), fixtures=["net.log"] if netlog else [])
    if former:
        ep["radio"]["former"] = dict(ip=OLD, connect=dict(fnet.ep["radio"]["connect"]))
    ctx = Ctx(ep)

    def fixture(name, /, **a):
        assert name == "net.log", name
        if a.get("action") == "start":
            radio.netlog = []
            return None
        if a.get("action") == "read":
            return dict(count=len(radio.netlog), dropped=0, entries=radio.netlog[a.get("from", 0):])
        return None
    ctx.fixture = fixture
    c = d.Case.__new__(d.Case)
    c.ctx, c.label, c.h, c.held = ctx, "case", h, OLD
    c.stream = d.Stream(ctx, h, now=lambda: radio.t, radio_now=lambda: radio.t, manual=True)
    h.send(PORT_DISC, b"\0" * 38 + b"\x01" + b"\0" * 21)  # a General packet with the watchdog bit
    radio.ddc0 = True
    c.stream.start()
    c.stream.wait(0.5)
    c.preset = d.tx_preset(ctx, h, c.label, wait=c.stream.wait)
    c.t_reply = c.t_from = radio.t
    return c, radio, nets


def failed(ctx):
    return [x["name"] for x in ctx.checks if not x["ok"]]


class Ends(unittest.TestCase):
    """With another address: the end, then the bench's RUN=1 to the new address starts a session that
    carries nothing over."""

    def run_end(self, **kw):
        c, radio, nets = case(change_at=103.0, **kw)
        c.stream.wait(1.5)  # ended() comes once the radio answers on the new address
        c.ended(nets[1], radio.t)
        return c.ctx

    def test_correct(self):
        ctx = self.run_end()
        self.assertEqual(failed(ctx), [])
        names = [x["name"] for x in ctx.checks]
        for want in ("no status packets from it after the change", "starts none", "starts a new session",
                     "DDC0 STREAM_STATUS reason 1", "no DDC0 I&Q", "TX_MODE_CLASS at its default",
                     "refused with WATCHDOG_DISABLED", "no longer refused"):
            self.assertTrue(any(want in n for n in names), want)
        self.assertFalse(ctx.no_pass)

    def test_record_does_not_restart(self):
        """The old probe's fault: RUN=1 to the radio while it listens. The record sends to the old address
        only until the switch, and on a correct radio nothing comes from there."""
        c, radio, nets = case(change_at=103.0)
        c.stream.wait(2.0)
        st = d.status(c.stream.snapshot(), 103.0 + 0.3)
        self.assertEqual(st, [])

    def test_old_address_starts_a_session_fails(self):
        self.assertTrue(any("starts none" in n for n in failed(self.run_end(broken="old-starts"))))

    def test_carried_over_fails(self):
        bad = failed(self.run_end(broken="carry"))
        for want in ("reason 1", "no DDC0 I&Q", "TX_MODE_CLASS", "KEYED_TIMEOUT", "PA enable"):
            self.assertTrue(any(want in n for n in bad), (want, bad))

    def test_new_early_judged_without_note(self):
        ctx = self.run_end()
        self.assertTrue(any("before the bench sent there" in x["name"] for x in ctx.checks))
        self.assertFalse(any("is not judged" in n for n in ctx.notes))

    def test_shared_mapping_is_a_part(self):
        c, radio, nets = case(apart=False, change_at=103.0)
        c.stream.wait(1.5)  # ended() comes once the radio answers on the new address
        c.ended(nets[1], radio.t)
        self.assertEqual(failed(c.ctx), [])
        self.assertTrue(c.ctx.no_pass)
        self.assertTrue(any("same local addresses" in p for p in c.ctx.parts))


class GeneralTaken(unittest.TestCase):
    """emu 265c769 diagnosis: [P2] orders nothing between ports, so the General (1024) may be taken after an arm
    (7250) sent at once after it. The arm is retried in radio time, never judged on one arm at that instant."""

    def run_end(self, **kw):
        c, radio, nets = case(change_at=103.0, **kw)
        c.stream.wait(1.5)
        c.ended(nets[1], radio.t)
        return c.ctx

    def test_general_taken_late_passes(self):
        ctx = self.run_end(general_delay=0.25)  # later than one write's 0.3 s read: one arm at once would fail
        self.assertEqual(failed(ctx), [])
        self.assertFalse(any("not taken beforehand" in p for p in ctx.parts))  # the TX preset waits as well
        done = [x for x in ctx.checks if "no longer refused" in x["name"]]
        self.assertTrue(done and done[0]["ok"])

    def test_general_never_taken_fails(self):
        bad = failed(self.run_end(broken="no-general"))
        self.assertTrue(any("no longer refused" in n for n in bad), bad)

    def test_arm_after_general_stops_at_the_first_answer(self):
        class H:
            def __init__(self):
                self.n = 0

            def keepalive(self):
                pass

            def write(self, pairs):
                a = pairs[0][0]
                if a == TX_ARM_TOKEN:
                    return [(TX_ARM_TOKEN, 7)]
                self.n += 1
                return [(TX_ARM_STATE, (3 if self.n < 3 else 9) << 8)]
        waited = []
        out = d.arm_after_general(H(), waited.append)
        self.assertEqual(len(out), 3)
        self.assertEqual(out[:2], ["WATCHDOG_DISABLED"] * 2)
        self.assertEqual(waited, [d.ARM_EVERY_S] * 3)


class Former(unittest.TestCase):
    """radio.former (minor 10): the old address reached apart, its drops in net.log."""

    def run_end(self, **kw):
        c, radio, nets = case(former=True, change_at=103.0, **kw)
        c.stream.wait(1.5)  # ended() comes once the radio answers on the new address
        c.ended(nets[1], radio.t, d.former_net(c.ctx))
        return c.ctx

    def test_correct(self):
        ctx = self.run_end()
        self.assertEqual(failed(ctx), [])
        self.assertFalse(ctx.no_pass)
        names = [x["name"] for x in ctx.checks]
        for want in ("starts none", "dropped as 'not our address' (net.log)", "taken by the radio",
                     "starts a new session", "DDC0 STREAM_STATUS reason 1"):
            self.assertTrue(any(want in n for n in names), want)

    def test_old_only_note_in_radio_time(self):
        ctx = self.run_end()
        n = next(n for n in ctx.notes if "from the change on the bench sent to" in n)
        self.assertIn(f"{d.OLD_ONLY_S:g} s asked", n)
        self.assertIn("s of radio time waited", n)

    def test_first_session_judged(self):
        """fw-4e567d2 part 2: the old-only stretch is shorter than the watchdog, so the session the bench's
        keepalives started at the new address since the move is the one judged, not a restart."""
        ctx = self.run_end()
        self.assertTrue(any("the first at " + NEW in n for n in ctx.notes), ctx.notes)

    def test_restart_judged_is_noted(self):
        saved = d.OLD_ONLY_S
        d.OLD_ONLY_S = 1.5  # longer than the watchdog: the session at the new address ends, the switch restarts it
        try:
            ctx = self.run_end()
        finally:
            d.OLD_ONLY_S = saved
        self.assertTrue(any("a restart at " + NEW in n for n in ctx.notes), ctx.notes)

    def test_new_early_not_judged_is_noted(self):
        """radio.connect follows the radio: the check is not made, and a note says so."""
        ctx = self.run_end()
        self.assertFalse(any("before the bench sent there" in x["name"] for x in ctx.checks))
        self.assertIn(f"case: the bench's keepalives already reached {NEW}, so 'no status packets from {NEW} before "
                      "the bench sent there' is not judged", ctx.notes)

    def test_old_address_starts_a_session_fails(self):
        # the fake keeps one session, already running on the new address from the bench's RUN=1 to radio.connect,
        # so the packets to the old address are taken but show no status from it: net.log shows them
        bad = failed(self.run_end(broken="old-starts"))
        for want in ("taken by the radio", "dropped as"):
            self.assertTrue(any(want in n for n in bad), (want, bad))

    def test_old_address_taken_fails(self):
        """Taken by the chip although it starts nothing: net.log shows it."""
        bad = failed(self.run_end(broken="old-taken"))
        self.assertTrue(any("taken by the radio" in n for n in bad), bad)
        self.assertFalse(any("starts none" in n for n in bad), bad)

    def test_without_netlog_a_note(self):
        c, radio, nets = case(former=True, netlog=False, change_at=103.0)
        c.stream.wait(1.5)  # ended() comes once the radio answers on the new address
        c.ended(nets[1], radio.t, d.former_net(c.ctx))
        self.assertEqual(failed(c.ctx), [])
        self.assertFalse(c.ctx.no_pass)
        self.assertTrue(any("no net.log" in n for n in c.ctx.notes))

    def test_former_net(self):
        ctx = Ctx(dict(radio=dict(ip=NEW, former=dict(ip=OLD, connect={"1027": "127.0.0.1:41327"})), fixtures=[]))
        n = d.former_net(ctx)
        self.assertEqual((n.radio_ip, n.dest(1027)), (OLD, ("127.0.0.1", 41327)))
        for bad in ({}, {"former": None}, {"former": {"ip": OLD}}):
            self.assertIsNone(d.former_net(Ctx(dict(radio=dict(ip=NEW, **bad), fixtures=[]))))

    def test_drops_of(self):
        e = [dict(dir="dropped", reason=d.NOT_OURS, peer="1.2.3.4:9100"), dict(dir="dropped", reason="x", peer="1.2.3.4:9100"),
             dict(dir="to_radio", peer="1.2.3.4:9100"), dict(dir="dropped", reason=d.NOT_OURS, peer="1.2.3.4:9101")]
        dropped, taken = d.drops_of(e, 9100)
        self.assertEqual((len(dropped), len(taken)), (1, 1))


class DoesNotEnd(unittest.TestCase):
    def test_window_spanning_a_register_read(self):
        """fw-4e567d2 part 3: a window that spans an earlier TX read (the record paused, the datagrams lost to
        the read) showed a 0.58 s gap that the wire did not have."""
        c, radio, _ = case()
        c.stream.wait(1.0)
        self.assertTrue(c.kept("the session does not end for the mode change"))
        c.stream.wait(1.0)
        self.assertTrue(c.kept("the session does not end (a lease with the same address and netmask)"))
        self.assertEqual(failed(c.ctx), [])
        self.assertTrue(c.stream.pauses)

    def test_short_window_waits(self):
        """fw-4e567d2 part 3: a window of 10 ms holds no status packet; kept() now waits up to MIN_WINDOW_S."""
        c, radio, _ = case()
        t0 = radio.t
        self.assertTrue(c.kept("the session does not end for the mode change", tx=False))
        self.assertGreaterEqual(radio.t - t0, d.MIN_WINDOW_S - 0.11)

    def test_correct(self):
        c, radio, _ = case()
        c.stream.wait(2.0)
        self.assertTrue(c.kept("the session does not end"))
        self.assertEqual(failed(c.ctx), [])

    def test_wrong_end_restarted_by_run_fails(self):
        """Ends at 103 s and is restarted at once by the record's own RUN=1 (as the old probe did): the
        stream barely shows it, the reset TX settings do."""
        c, radio, _ = case(broken="wrong-end", end_at=103.0)
        c.stream.wait(2.0)
        self.assertFalse(c.kept("the session does not end"))
        self.assertIn("reset", c.ctx.checks[-1]["detail"])

    def test_moved_meanwhile_is_a_part(self):
        c, radio, _ = case(broken="wrong-end", end_at=103.0)
        c.stream.wait(2.0)
        c.kept("the session does not end", moved=lambda: True)
        self.assertTrue(c.ctx.no_pass)


class Pure(unittest.TestCase):
    def test_ack_wall(self):
        self.assertEqual(d.ack_wall({"t_wall": 105.0}, 100.0, 106.0), 105.0)
        for bad in ({"t_wall": 5.0}, {"t_wall": 107.0}, {}, {"t_wall": "x"}):  # another clock, later, none
            self.assertIsNone(d.ack_wall(bad, 100.0, 106.0))

    def test_pause_excused(self):
        rec = [(t / 20, PORT_DDC, OLD) for t in range(0, 41) if not 10 <= t <= 22]  # a hole 0.5 to 1.1 s
        self.assertEqual(len(d.kept_faults(rec, 0.0, 2.0)), 1)
        self.assertEqual(d.kept_faults(rec, 0.0, 2.0, pauses=[(0.6, 0.9)]), [])
        self.assertEqual(len(d.kept_faults(rec, 0.0, 2.0, pauses=[(1.5, 1.6)])), 1)  # a pause elsewhere excuses nothing

    def test_judged_session(self):
        first = [(9.0 + t / 20, PORT_DDC, NEW) for t in range(0, 60)]
        self.assertIn("the first at", d.judged_session(first, NEW, 10.0))
        holed = [r for r in first if not 10.0 < r[0] < 11.0]
        self.assertIn("a restart at", d.judged_session(holed, NEW, 10.0))
        self.assertEqual(d.judged_session([], NEW, 10.0), "none seen")

    def test_lease_note(self):
        self.assertIn("came 14 ms of radio time after the reply", d.lease_note(0.014, "the radio's REQUEST"))
        n = d.lease_note(-0.02, "the radio's REQUEST")
        self.assertIn("before the bench's stamp of the reply", n)
        self.assertNotIn(" 0 ms", n)
        self.assertEqual(d.lease_note(None, "x"), "no time for the lease on either clock")

    def test_lease_bound(self):
        class S:
            def ratio(self):
                return 2.0

        class C:
            t_reply, t_reply_us, stream = 50.0, 24_656_600, S()

            def __init__(self, msgs):
                self._m = msgs

            def msgs(self):
                return self._m
        ack = {"type": "ACK", "t_us": 24_673_500, "t_us_approx": True}
        req = {"type": "REQUEST", "t_us": 24_670_000}
        self.assertAlmostEqual(d.lease_bound(C([req, ack]), ack, 60.0), 50.0 + 0.0134 * 2.0)
        self.assertEqual(d.lease_bound(C([req, dict(ack, t_wall=55.0)]), dict(ack, t_wall=55.0), 60.0), 55.0)
        self.assertIsNone(d.lease_bound(C([ack]), ack, 60.0))
        self.assertIsNone(d.lease_bound(C([dict(req, t_us_approx=True), ack]), ack, 60.0))

    def test_kept_faults(self):
        rec = [(t / 20, PORT_DDC, OLD) for t in range(0, 41)]
        self.assertEqual(d.kept_faults(rec, 0.0, 2.0), [])
        holed = [r for r in rec if not 0.5 < r[0] < 1.0]
        self.assertEqual(len(d.kept_faults(holed, 0.0, 2.0)), 1)
        self.assertEqual(d.kept_faults(holed, 0.0, 2.0, ratio=2.0), [])  # 0.55 s of host time, 0.275 s of radio time
        mixed = rec[:20] + [(t, p, NEW) for t, p, _ in rec[20:]]
        self.assertTrue(any("status packets from" in f for f in d.kept_faults(mixed, 0.0, 2.0)))
        self.assertTrue(d.kept_faults([], 0.0, 1.0))

    def test_end_faults(self):
        before = [(t / 20, PORT_DDC, OLD) for t in range(0, 20)]  # up to 0.95 s, the change at 1.0 s
        self.assertEqual(d.end_faults(before, 1.0, 2.0, OLD, NEW), dict(old=[], gap=0.0, new_early=[]))
        on = before + [(1.0 + t / 20, PORT_DDC, OLD) for t in range(0, 20)]
        f = d.end_faults(on, 1.0, 2.0, OLD, NEW)
        self.assertTrue(f["old"])
        self.assertLessEqual(f["gap"], 0.05)
        again = before + [(1.6 + t / 20, PORT_DDC, OLD) for t in range(0, 8)]
        f = d.end_faults(again, 1.0, 2.0, OLD, NEW)
        self.assertTrue(f["old"])
        self.assertGreater(f["gap"], 0.6)
        self.assertTrue(d.end_faults(before + [(1.5, PORT_DDC, NEW)], 1.0, 2.0, OLD, NEW)["new_early"])

    def test_radio_of(self):
        a, b = net(OLD, 41000), net(NEW, 42000)
        s = d.Stream.__new__(d.Stream)
        s.nets = [a, b]
        self.assertEqual(s.radio_of(("127.0.0.1", 41000 + 1025)), OLD)
        self.assertEqual(s.radio_of(("127.0.0.1", 42000 + 1025)), NEW)
        s.nets = [a, net(NEW, 41000)]
        self.assertIsNone(s.radio_of(("127.0.0.1", 41000 + 1025)))
        s.nets = [Net(dict(radio=dict(ip=OLD)))]
        self.assertEqual(s.radio_of((NEW, 1025)), NEW)


if __name__ == "__main__":
    unittest.main()
