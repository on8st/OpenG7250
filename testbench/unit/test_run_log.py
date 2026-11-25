"""run_log (T-A1-90 case 2): after a power cycle the log is lost and numbered from 0 again, so the whole
log is read; after another reset the entries after the number read before. No network.
GPL-3.0-or-later, (c) 2026 ON8ST."""
import unittest
from unittest import mock

from testbench import t_update

AFTER_POWER_ON = [(0, 0, "start after power-on: log lost"), (1, 0, "boot: slot B refused: integrity"),
                  (2, 0, "boot: fallback to slot A (active slot refused, no trial)")]


class U:
    c = None


class RunLog(unittest.TestCase):
    def test_after_a_power_cycle_the_whole_log(self):
        with mock.patch("testbench.mgmt.read_log", lambda c, *a, **k: AFTER_POWER_ON):
            texts = t_update.run_log(U(), 57, True)
        self.assertEqual(texts[1:], [AFTER_POWER_ON[1][2], AFTER_POWER_ON[2][2]])
        # the reading before the fix: entries after 57 of a log numbered from 0 again
        with mock.patch("testbench.mgmt.read_log", lambda c, *a, **k: AFTER_POWER_ON):
            self.assertEqual(t_update.run_log(U(), 57, False), [])

    def test_after_another_reset_the_entries_after_n0(self):
        log = [(56, 0, "old"), (57, 0, "restart: requested"), (58, 0, "start after reset: log kept"), (59, 0, "new")]
        with mock.patch("testbench.mgmt.read_log", lambda c, *a, **k: log):
            self.assertEqual(t_update.run_log(U(), 57, False), ["start after reset: log kept", "new"])


if __name__ == "__main__":
    unittest.main()
