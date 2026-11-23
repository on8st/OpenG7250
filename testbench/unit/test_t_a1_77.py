"""judge_77 (T-A1-77) on synthetic flash.log entries. No network. GPL-3.0-or-later, (c) 2026 ON8ST."""
import unittest

from testbench import t_flash
from testbench.core import Ctx

S = t_flash.SECTOR
STAGING = (0x08020000, 0x08020000 + 2 * S)
STORE = (0x080C0000, 0x080C0000 + 2 * S)
A6, A7 = STORE[0], STORE[0] + S


def judged(entries):
    ctx = Ctx({})
    t_flash.judge_77(ctx, "x", entries, STAGING, STORE)
    return [c["name"] for c in ctx.checks if not c["ok"]]


class Judge77(unittest.TestCase):
    def test_good(self):
        log = [dict(op="erase", address=STAGING[0], len=S), dict(op="program", address=STAGING[0], len=512),
               dict(op="program", address=A6 + 64, len=32), dict(op="erase", address=A7, len=S),
               dict(op="program", address=hex(A7), len=96), dict(op="ecc", address=A6)]
        self.assertEqual(judged(log), [])

    def test_outside(self):
        bad = judged([dict(op="program", address=0x08000000, len=32)])
        self.assertTrue(any("outside" in b for b in bad), bad)
        bad = judged([dict(op="erase", address=STAGING[1] - 512, len=S)])  # runs past the slot
        self.assertTrue(any("outside" in b for b in bad), bad)

    def test_live_sector_erased(self):
        bad = judged([dict(op="program", address=A6 + 32, len=32), dict(op="erase", address=A6, len=S)])
        self.assertTrue(any("live store sector" in b for b in bad), bad)


if __name__ == "__main__":
    unittest.main()
