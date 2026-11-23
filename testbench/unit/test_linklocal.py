"""The judgments of T-A1-70 to T-A1-72 (t_linklocal) on synthetic eth.log entries. No network.
GPL-3.0-or-later, (c) 2026 ON8ST."""
import unittest

from testbench import t_linklocal as L
from testbench.core import Ctx

A = "169.254.10.20"


def arp(t, op, spa, tpa, d="from_radio"):
    return dict(t_us=int(t * 1e6), dir=d, arp=dict(op=op, spa=spa, tpa=tpa, sha="m", tha="0"))


def start(probe_gaps=(1.5, 1.5), wait=2.0, interval=2.0, n_ann=2):
    t, log = 1.0, []
    for g in (0.0,) + tuple(probe_gaps):
        t += g
        log.append(arp(t, 1, L.ZERO, A))
    last = t
    for i in range(n_ann):
        log.append(arp(last + wait + i * interval, 1, A, A))
    log.append(dict(t_us=0, dir="to_radio", arp=dict(op=2, spa=A, tpa=A)))  # not the radio's
    return log


def judged(log):
    ctx = Ctx({})
    f = L.arps(log)
    ts = [t for t, _ in L.probes(f, A)]
    L.judge_probe(ctx, "p", ts)
    L.judge_announce(ctx, "a", ts[-1], L.announcements(f, A))
    return [c["name"] for c in ctx.checks if not c["ok"]]


class LinkLocal(unittest.TestCase):
    def test_good(self):
        self.assertEqual(judged(start()), [])
        self.assertEqual(judged(start(probe_gaps=(1.0, 2.0))), [])

    def test_bad_timing(self):
        for kw in (dict(probe_gaps=(0.5, 1.5)), dict(probe_gaps=(1.5, 2.5)), dict(probe_gaps=(1.5,)), dict(wait=1.0),
                   dict(interval=1.0), dict(n_ann=1)):
            self.assertTrue(judged(start(**kw)), kw)

    def test_defence(self):
        f = L.arps([arp(5.0, 2, A, "169.254.1.1"), arp(6.0, 1, L.ZERO, "169.254.3.3")])
        self.assertEqual(L.no_defence(f, A, 4.0), [5.0])
        self.assertEqual(L.no_defence(f, A, 5.5), [])

    def test_link_local(self):
        self.assertTrue(L.is_link_local(A))
        self.assertFalse(L.is_link_local("169.254.0.1"))
        self.assertFalse(L.is_link_local("192.168.1.5"))


if __name__ == "__main__":
    unittest.main()
