"""The judgement of the log across a reset (0068) on right and wrong logs.
GPL-3.0-or-later, (c) 2026 ON8ST."""
import unittest

from testbench.core import Ctx
from testbench.t_log import judge_run

GOOD = [(6, 100, "x"), (7, 900, "restart: restart"), (8, 0, "start after software: log kept"), (9, 5, "boot: trial of slot B started"),
        (10, 9, "y")]


def bad(log, reason="restart: restart", cause="software", boot=("boot: trial of slot B started",), kind="emulator"):
    ctx = Ctx(dict(radio=dict(ip="127.0.0.1"), fixtures=[], kind=kind))
    judge_run(ctx, "t", log, 6, reason, cause, boot)
    return [c["name"] for c in ctx.checks if not c["ok"]]


class JudgeRun(unittest.TestCase):
    def test_good(self):
        self.assertEqual(bad(GOOD), [])

    def test_wrong(self):
        e = list(GOOD)
        for name, log in (("no start entry", e[:2]), ("lost", [(0, 0, "start after software: log lost")] + e[3:]),
                          ("an entry after the reason", e[:2] + [(8, 901, "z"), (9, 0, e[2][2]), (10, 5, e[3][2])]),
                          ("a gap", e[:2] + [(9, 0, e[2][2]), (10, 5, e[3][2])]),
                          ("a line before the boot line", e[:3] + [(9, 1, "app"), (10, 5, e[3][2])]),
                          ("wrong cause", e[:2] + [(8, 0, "start after watchdog: log kept")] + e[3:]),
                          ("restarted numbers", [(0, 0, e[2][2]), (1, 5, e[3][2])])):
            self.assertTrue(bad(log), name)

    def test_radio_before_p_sys_9(self):
        self.assertEqual(bad([(0, 0, "start after software: log lost")], kind="radio"), [])


if __name__ == "__main__":
    unittest.main()
