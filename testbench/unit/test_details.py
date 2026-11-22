"""The judging parts of t_details and the uptime rule of T-B5-22 (0073, 0074). No network. GPL-3.0-or-later, (c) 2026 ON8ST."""
import unittest

from testbench import t_flash
from testbench.core import Ctx, Skip
from testbench.t_details import COUNTERS, due_during, judge_counters, judge_pages, other_address, renewals


def stats(names, own=()):
    return b"".join(bytes([len(n)]) + n.encode() + (0).to_bytes(8, "big") for n in [*names, *own])


def failed(names, own=()):
    ctx = Ctx({})
    judge_counters(ctx, stats(names, own))
    return [c["name"] for c in ctx.checks if not c["ok"]]


class Counters(unittest.TestCase):
    def test_table_m_6(self):
        self.assertEqual(len(COUNTERS), 45)
        self.assertEqual(failed(COUNTERS), [])
        self.assertEqual(failed(COUNTERS, ["net_busy_max_us"]), [])

    def test_faults(self):
        self.assertTrue(failed(COUNTERS[:-1]))  # one left out
        self.assertTrue(failed(["own"] + COUNTERS))  # an own one first
        self.assertTrue(failed(COUNTERS[1:] + COUNTERS[:1]))  # another order
        self.assertTrue(failed(COUNTERS, ["a_name_of_more_than_16"]))
        self.assertTrue(failed(COUNTERS, ["hp_rx"]))  # twice
        self.assertTrue(failed(COUNTERS, [f"own_counter_{i:03}" for i in range(30)]))  # more than 1400 bytes


class Pages(unittest.TestCase):
    E = [(1, 1, "x")]

    def test_pages(self):
        self.assertTrue(judge_pages([(2, self.E), (2, self.E), (0, self.E), (0, [])])[0])
        self.assertTrue(judge_pages([(0, self.E), (0, [])])[0])
        for bad in ([(0, self.E), (0, self.E), (0, [])],  # never set
                    [(2, self.E), (2, self.E), (0, [])],  # set on the last
                    [(2, self.E), (0, self.E), (2, [])],  # set on the empty page
                    [(2, self.E), (0, self.E), (0, self.E), (0, [])]):
            self.assertFalse(judge_pages(bad)[0], bad)


class Due(unittest.TestCase):
    def test_shown_and_kept(self):
        self.assertEqual(due_during(1_000, 5_000, 90_000, 3_000)[:2], (True, True))
        self.assertEqual(due_during(1_000, 5_000, 250_000, 3_000)[:2], (True, False))  # later than 100 ms
        self.assertEqual(due_during(1_000, 5_000, 4_000, 3_000)[:2], (True, False))  # before the reply
        self.assertFalse(due_during(1_000, 5_000, 90_000, 6_000)[0])  # due after the reply
        self.assertFalse(due_during(4_000, 5_000, 90_000, 3_000)[0])  # due before the request
        self.assertFalse(due_during(1_000, None, 90_000, 3_000)[0])


class Uptime(unittest.TestCase):
    def judge(self, behind, n, width=20.0):
        ctx = Ctx({})
        t_flash.judge_uptime(ctx, "x", (0.0, 1000, width), (10_000.0, 11_000 - behind, 10_000.0 + width), n)
        return ctx.checks[0]["ok"]

    def test_lost_time(self):
        self.assertTrue(self.judge(0, 5))
        self.assertTrue(self.judge(5, 5))
        self.assertTrue(self.judge(20, 5))  # within what the two samples leave open
        self.assertFalse(self.judge(40, 5))
        self.assertFalse(self.judge(4000, 1400))  # one erase not counted over a whole sector of writes
        self.assertTrue(self.judge(1400, 1400))

    def test_no_sample(self):
        ctx = Ctx({})
        t_flash.judge_uptime(ctx, "x", None, (0, 0, 0), 5)
        self.assertEqual((ctx.checks, len(ctx.parts)), ([], 1))


class NearCompaction(unittest.TestCase):
    """A store that appends records of 96 bytes: left writes fit, the next needs a compaction."""

    def run_it(self, used, left, status=0):
        lo, hi = 0x080C0000, 0x08100000
        log, radio = [], dict(at=lo + used, n=0)

        def set_tx_time(u, v, window=None):
            radio["n"] += 1
            log.append(dict(op="program", address=radio["at"], len=96))
            radio["at"] += 96
            return dict(status=status)
        saved = {k: getattr(t_flash, k) for k in ("flog", "flog_start", "set_tx_time")}
        t_flash.flog, t_flash.flog_start, t_flash.set_tx_time = (lambda c, first=0: log[first:]), (lambda c: None), set_tx_time
        ctx = Ctx({})
        try:
            v = t_flash.near_compaction(ctx, None, lo, hi, 200_000, left)
        finally:
            for k, f in saved.items():
                setattr(t_flash, k, f)
        end = lo + 131072 if used < 131072 else hi
        return v, (end - radio["at"]) // 96, radio["n"], ctx

    def test_room_left(self):
        for used in (0, 5000, 130_000, 140_000):
            v, room, n, _ = self.run_it(used, 1)
            self.assertEqual((room, v), (1, 200_000 + n), used)
        self.assertEqual(self.run_it(1000, 60)[1], 60)

    def test_refused(self):
        v, _, _, ctx = self.run_it(0, 1, status=55)
        self.assertIsNone(v)
        self.assertIn("answered 55", ctx.__dict__["_near"])


class Addresses(unittest.TestCase):
    def test_other_address(self):
        ep = dict(hosts=[dict(ip="192.168.73.11"), dict(ip="192.168.73.12")])
        self.assertEqual(other_address(ep, "192.168.73.10", "255.255.255.0"), bytes([192, 168, 73, 13]))
        self.assertEqual(other_address(dict(hosts=[]), "127.0.0.1", "255.0.0.0"), bytes([127, 0, 0, 2]))
        with self.assertRaises(Skip):
            other_address(dict(hosts=[]), "192.168.73.10", "255.255.255.255")

    def test_renewals(self):
        ip = "192.168.73.10"
        log = [dict(type="DHCPREQUEST", src="0.0.0.0:68", t_us=1), dict(type="DHCPREQUEST", src=ip + ":68", t_us=9),
               dict(type="DHCPACK", src="192.168.73.1:67", t_us=10), dict(type="REQUEST", src=ip, t_us=5)]
        self.assertEqual(renewals(log, ip), [5, 9])


if __name__ == "__main__":
    unittest.main()
