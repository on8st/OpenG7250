"""T-A3-47 (t_unarmed) against a fake radio with the rules of A3.2 "Drop events while not armed":
it passes on a correct one and fails on each broken one. No network.

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
import unittest
from contextlib import contextmanager
from unittest import mock

from testbench import t_flash, t_mgmt, t_unarmed, t_update
from testbench.core import Ctx
from testbench.p2 import PORT_REG, TX_ARM, TX_ARM_STATE, TX_ARM_TOKEN

RESET, TAKEOVER = 5, 4


class Radio:
    def __init__(self, broken=""):
        self.broken, self.t, self.token, self.drop, self.gen = broken, 0.0, 0x1001, RESET, 0
        self.owner, self.last_cc, self.pending = None, 0.0, {}

    def state(self) -> int:
        return (self.gen << 24) | (self.drop << 16)

    def new_token(self):
        self.token += 1

    def push_token(self, h):
        self.pending.setdefault(h, []).append([(TX_ARM_TOKEN, self.token)])

    def watchdog(self):
        if self.owner is not None and self.t - self.last_cc > 1.0:
            self.end()

    def end(self):
        self.owner = None
        if self.broken != "end-no-token":
            self.new_token()  # a new token at every session end (A3.6)

    def cc(self, h):
        """C&C with run set from host h: a session start, a takeover, or a keepalive."""
        self.watchdog()
        if self.owner is not None and self.owner != h:  # a takeover while not armed
            if self.broken != "takeover-no-reason":
                self.drop = TAKEOVER
            self.new_token()
            self.owner = None
        if self.owner is None:
            self.owner = h
            self.push_token(h)
        self.last_cc = self.t

    def write(self, h, pairs):
        self.watchdog()
        out = []
        for a, v in pairs:
            if a == TX_ARM and v == 0:  # not armed: nothing changes
                if self.broken == "arm0-token":
                    self.new_token()
                if self.broken == "arm0-reason":
                    self.drop = 1
                out.append((TX_ARM_STATE, self.state()))
            elif a == TX_ARM_STATE:
                out.append((a, self.state()))
            elif a == TX_ARM_TOKEN:
                out.append((a, self.token))
        return out


class FakeHost:
    radio = None

    def __init__(self, net):
        self.r = FakeHost.radio

    def __enter__(self):
        return self

    def __exit__(self, *a):
        pass

    def start(self, **k):
        self.r.cc(self)

    def keepalive(self, **k):
        self.r.cc(self)

    def stop(self, settle=0.05):
        if self.r.owner is self:
            self.r.end()

    def recv(self, timeout=0.5, from_port=None, until=None):
        self.r.t += timeout
        self.r.watchdog()
        return [(PORT_REG, regs, self.r.t) for regs in self.r.pending.pop(self, [])]

    def recv_alive(self, timeout, from_port=None, every=0.1):
        got, n = [], max(1, int(round(timeout / every)))
        for _ in range(n):
            self.keepalive()
            got += self.recv(every)
        return got

    def write(self, pairs, timeout=0.3):
        if self.r.owner is self:
            self.keepalive()
        return self.r.write(self, pairs)


def run(broken=""):
    r = Radio(broken)
    FakeHost.radio = r
    ctx = Ctx(dict(radio=dict(ip="127.0.0.1"), fixtures=[]))

    class Clock:
        emu = False

        def now(self):
            return r.t

    class U:
        def __init__(self):
            self.c = type("C", (), {"clock": Clock()})()

    @contextmanager
    def updating(ctx_, s, restart_at_end=True):
        yield U()

    def set_tx_time(u, ms, window=None):
        if broken == "c3-token":
            r.new_token()
        if broken == "c3-reason":
            r.drop = 11
        return dict(status=0)
    with mock.patch.multiple(t_unarmed, Host=FakeHost, parse_regs=lambda d: d), \
            mock.patch.multiple(t_flash, set_tx_time=set_tx_time, tx_time=lambda u: 300_000), \
            mock.patch.object(t_mgmt, "secret_of", lambda c: b"s" * 32), \
            mock.patch.object(t_update, "updating", updating):
        t_unarmed.t_a3_47(ctx)
    return [c["name"] for c in ctx.checks if not c["ok"]], ctx


class Unarmed(unittest.TestCase):
    def test_correct_radio(self):
        bad, ctx = run()
        self.assertEqual(bad, [])
        self.assertGreaterEqual(len(ctx.checks), 14)

    def test_broken_radios(self):
        for broken, seen in (("arm0-token", "TX_ARM = 0 while not armed: no new token"),
                             ("arm0-reason", "TX_ARM = 0 while not armed: its answer"),
                             ("end-no-token", "a new token at the session end"),
                             ("takeover-no-reason", "drop reason 4 TAKEOVER"),
                             ("c3-token", "a C-3 change while not armed: no new token"),
                             ("c3-reason", "a C-3 change while not armed: no field")):
            bad, _ = run(broken)
            self.assertTrue(any(seen in b for b in bad), (broken, bad[:4]))


if __name__ == "__main__":
    unittest.main()
