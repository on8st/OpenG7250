"""watchdog_out (T-A3-06, T-B10-L11): the watchdog standby waited in radio time where time.wait is
offered, else on the host clock with a note on an emulator. No network. GPL-3.0-or-later, (c) 2026 ON8ST."""
import unittest
from unittest import mock

from testbench import t_tx_locked
from testbench.core import Ctx


class WatchdogOut(unittest.TestCase):
    def test_radio_time(self):
        ctx = Ctx(dict(kind="emulator", fixtures=["time.wait"]))
        calls = []
        ctx.fixture = lambda op, **a: calls.append((op, a))
        with mock.patch("time.sleep") as sl:
            t_tx_locked.watchdog_out(ctx)
        self.assertEqual([(op, a["us"]) for op, a in calls], [("time.wait", 1_300_000)])
        sl.assert_not_called()
        self.assertEqual(ctx.notes, [])

    def test_host_clock_noted(self):
        ctx = Ctx(dict(kind="emulator", fixtures=[]))
        ctx.fixture = lambda op, **a: self.fail(op)
        with mock.patch("time.sleep") as sl:
            t_tx_locked.watchdog_out(ctx)
            t_tx_locked.watchdog_out(ctx)
        self.assertEqual(sl.call_count, 2)
        self.assertAlmostEqual(sl.call_args[0][0], 1.3)
        self.assertEqual(len(ctx.notes), 1)

    def test_model_no_note(self):
        ctx = Ctx(dict(kind="model", fixtures=[]))
        with mock.patch("time.sleep"):
            t_tx_locked.watchdog_out(ctx)
        self.assertEqual(ctx.notes, [])


if __name__ == "__main__":
    unittest.main()
