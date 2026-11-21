"""Run-wide flash rules (flash.log, v1.8). GPL-3.0-or-later, (c) 2026 ON8ST."""
import unittest

from testbench.flash import check_flash


def bad(entries):
    return [n for n, ok, _ in check_flash(entries) if not ok]


class FlashTest(unittest.TestCase):
    def test_store_and_slot_writes_pass(self):
        ev = [dict(op="erase", bank=1, sector=6, address=0x080C0000, len=131072, result="ok"),
              dict(op="program", bank=1, sector=7, address=0x080E0000, len=64, words=2, result="ok"),
              dict(op="program", bank=1, sector=3, address="0x08060000", len=32, words=1, result="ok")]
        self.assertEqual(bad(ev), [])

    def test_sector0_option_outside_fail(self):
        self.assertTrue(bad([dict(op="erase", sector=0, address=0x08000000, len=131072)])[0].startswith("flash: sector 0"))
        self.assertTrue(bad([dict(op="option_write", result="refused")])[0].startswith("flash: no option-byte"))
        self.assertTrue(bad([dict(op="program", address=0x080FFFF0, len=32)])[0].startswith("flash: no operation outside"))
        self.assertTrue(bad([dict(op="bank_erase", bank=1)])[0].startswith("flash: sector 0"))


if __name__ == "__main__":
    unittest.main()
