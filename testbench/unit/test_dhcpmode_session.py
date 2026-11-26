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

    def __init__(self, nets, broken=None, change_at=None, end_at=None):
        self.t, self.nets, self.broken = 100.0, nets, broken
        self.ip, self.old = nets[0].radio_ip, None
        self.session_ip = self.ip
        self.change_at, self.end_at = change_at, end_at
        self.running, self.network_new = False, False
        self.ddc0, self.pa, self.mc, self.kt, self.wd_general = False, False, 0, 500, False
        self.out, self.next_status, self.next_iq, self.token = [], 0.0, 0.0, 0x1234

    def src(self, port, ip=None):
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

    def to_radio(self, dest, port, data):
        ips = [n.radio_ip for n in self.nets if n.connect.get(port) == dest]  # a shared mapping: the current one
        ip = self.ip if self.ip in ips else (ips[0] if ips else None)
        if ip != self.ip and not (self.broken == "old-starts" and ip == self.old):
            return  # not this radio's address: never taken
        if port == PORT_HP and data[4] & 1:
            if not self.running:
                self.running, self.next_status, self.session_ip = True, self.t, ip
        elif port == PORT_DISC and len(data) == 60:
            self.pa, self.wd_general = bool(data[58] & 1), bool(data[38] & 1)
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
        for data, src, _ in self.recv_raw(0.02):
            if src == self.r.src(PORT_REG, self.r.ip) or src == self.r.src(PORT_REG, self.r.old or self.r.ip):
                if len(data) > 4 and len(data) % 6 == 4 and len(data) < 200:
                    got += parse_regs(data)
        return got


def case(apart=True, **kw):
    nets = [net(OLD, 41000), net(NEW, 42000 if apart else 41000)]
    radio = Radio(nets, **kw)
    h = Host(radio, nets[0])
    ctx = Ctx(dict(radio=dict(ip=OLD), fixtures=[]))
    c = d.Case.__new__(d.Case)
    c.ctx, c.label, c.h, c.held = ctx, "case", h, OLD
    c.stream = d.Stream(ctx, h, now=lambda: radio.t, radio_now=lambda: radio.t, manual=True)
    h.send(PORT_DISC, b"\0" * 38 + b"\x01" + b"\0" * 21)  # a General packet with the watchdog bit
    radio.ddc0 = True
    c.stream.start()
    c.stream.wait(0.5)
    c.preset = d.tx_preset(ctx, h, c.label)
    c.t_reply = c.t_from = radio.t
    return c, radio, nets


def failed(ctx):
    return [x["name"] for x in ctx.checks if not x["ok"]]


class Ends(unittest.TestCase):
    """With another address: the end, then the bench's RUN=1 to the new address starts a session that
    carries nothing over."""

    def run_end(self, **kw):
        c, radio, nets = case(change_at=101.0, **kw)
        c.stream.wait(1.0)
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
        c, radio, nets = case(change_at=101.0)
        c.stream.wait(2.0)
        st = d.status(c.stream.snapshot(), 101.0 + 0.3)
        self.assertEqual(st, [])

    def test_old_address_starts_a_session_fails(self):
        self.assertTrue(any("starts none" in n for n in failed(self.run_end(broken="old-starts"))))

    def test_carried_over_fails(self):
        bad = failed(self.run_end(broken="carry"))
        for want in ("reason 1", "no DDC0 I&Q", "TX_MODE_CLASS", "KEYED_TIMEOUT", "PA enable"):
            self.assertTrue(any(want in n for n in bad), (want, bad))

    def test_shared_mapping_is_a_part(self):
        c, radio, nets = case(apart=False, change_at=101.0)
        c.stream.wait(1.0)
        c.ended(nets[1], radio.t)
        self.assertEqual(failed(c.ctx), [])
        self.assertTrue(c.ctx.no_pass)
        self.assertTrue(any("same local addresses" in p for p in c.ctx.parts))


class DoesNotEnd(unittest.TestCase):
    def test_correct(self):
        c, radio, _ = case()
        c.stream.wait(2.0)
        self.assertTrue(c.kept("the session does not end"))
        self.assertEqual(failed(c.ctx), [])

    def test_wrong_end_restarted_by_run_fails(self):
        """Ends at 101 s and is restarted at once by the record's own RUN=1 (as the old probe did): the
        stream barely shows it, the reset TX settings do."""
        c, radio, _ = case(broken="wrong-end", end_at=101.0)
        c.stream.wait(2.0)
        self.assertFalse(c.kept("the session does not end"))
        self.assertIn("reset", c.ctx.checks[-1]["detail"])

    def test_moved_meanwhile_is_a_part(self):
        c, radio, _ = case(broken="wrong-end", end_at=101.0)
        c.stream.wait(2.0)
        c.kept("the session does not end", moved=lambda: True)
        self.assertTrue(c.ctx.no_pass)


class Pure(unittest.TestCase):
    def test_ack_wall(self):
        self.assertEqual(d.ack_wall({"t_wall": 105.0}, 100.0, 106.0), 105.0)
        for bad in ({"t_wall": 5.0}, {"t_wall": 107.0}, {}, {"t_wall": "x"}):  # another clock, later, none
            self.assertIsNone(d.ack_wall(bad, 100.0, 106.0))

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
