"""Unit test of the hw.trace reader on synthetic events (v1.2 schema).
Run: python3 -m unittest discover -s unit  (from testbench/). GPL-3.0-or-later, (c) 2026 ON8ST."""
import unittest

from testbench.trace import check_safe


def gpio(seq, pin, level):
    return dict(seq=seq, t_us=seq, kind="gpio", pin=pin, level=level)


def spi(seq, bus, data):
    return dict(seq=seq, t_us=seq, kind="spi", bus=bus, cs=None, data=data)


SAFE = [gpio(1, "PA10", 1), spi(2, "SPI1", "01"), gpio(3, "PA11", 1),
        dict(seq=4, t_us=4, kind="reset", cause="power"), dict(seq=5, kind="unknown-kind"),
        spi(6, "SPI1", "ff00ff"), gpio(7, "PH12", 1)]  # a chain B frame of 3 bytes is not chain A


class TraceTest(unittest.TestCase):
    def test_safe_trace_passes(self):
        self.assertTrue(all(ok for _, ok, _ in check_safe(SAFE)))

    def test_each_violation_is_found(self):
        cases = {
            "chain A bit 7 clear in every latched frame": [spi(10, "SPI1", "81"), gpio(11, "PA11", 0)],
            "T/R lines never leave their RX level": [gpio(10, "PF6", 1)],
            "ATU latches never toggled (0032)": [gpio(10, "PG2", 0)],
            "codec held in reset (0031)": [gpio(10, "PB10", 1)],
        }
        for name, extra in cases.items():
            res = {n: ok for n, ok, _ in check_safe(SAFE + extra)}
            self.assertFalse(res[name], name)
            self.assertEqual(sum(not ok for ok in res.values()), 1, name)


if __name__ == "__main__":
    unittest.main()
