"""T-A1-28: what counts as a logged loosening (A1.6, A1.7); T-A1-26: a counted excess. No network. GPL-3.0-or-later, (c) 2026 ON8ST."""
import unittest

from testbench.t_mgmt import counted_excess
from testbench.t_store import logged_change, names_value

IP = "192.168.7.9"


def logged(text, old=200_000, new=250_000, scales=(1, 1000)):
    return logged_change([(5, 10, text)], old, new, IP, scales)


class Logged(unittest.TestCase):
    def test_accepted_texts(self):
        for t in (f"C-3 limit 0: 200000 -> 250000 by {IP}:7300", f"config {IP}: TX time 200 s to 250 s",
                  f"max TX time 200.0 -> 250.0 s from {IP}"):
            self.assertTrue(logged(t), t)
        self.assertTrue(logged(f"band 0 40.0 W -> 60.0 W by {IP}", 400, 600, (1, 10, 0.01)))
        self.assertTrue(logged(f"band 0 40000 mW -> 60000 mW by {IP}", 400, 600, (1, 10, 0.01)))

    def test_refused_texts(self):
        for t in ("C-3 limit 0: 200000 -> 250000 by management", f"C-3 limit 0: 250000 by {IP}",
                  f"configuration changed by {IP}", f"C-3 limit 0: 200000 by {IP}",
                  "200000 -> 250000 by 192.168.7.90", "200000 -> 250000 by 1192.168.7.9",
                  f"C-3 limit 0: 2000001 -> 12500000 by {IP}"):
            self.assertFalse(logged(t), t)

    def test_two_entries_are_not_one(self):
        self.assertFalse(logged_change([(5, 1, "200000 -> 250000"), (6, 1, f"changed by {IP}")], 200_000, 250_000, IP, (1, 1000)))

    def test_numbers_of_the_address_are_no_values(self):
        self.assertFalse(logged_change([(5, 1, "changed by 10.0.40.60")], 400, 600, "10.0.40.60", (1, 10)))
        self.assertFalse(logged_change([(5, 1, "changed by 10.0.0.1:400, 600")], 400, 600, "10.0.0.1", (1, 10)))

    def test_values(self):
        self.assertTrue(names_value("a 80.0 b", 800, (1, 10)))
        self.assertFalse(names_value("a 80.5 b", 800, (1, 10)))


class Excess(unittest.TestCase):
    def test_a_counter_rises_by_it(self):
        self.assertTrue(counted_excess({"drop": 3, "rx": 10}, {"drop": 28, "rx": 56}, 25)[0])
        self.assertTrue(counted_excess({}, {"drop": 25}, 25)[0])  # not listed while 0

    def test_not_counted(self):
        self.assertFalse(counted_excess({"rx": 10}, {"rx": 56}, 25)[0])
        self.assertFalse(counted_excess({"drop": 25}, {"drop": 25}, 25)[0])
        self.assertFalse(counted_excess({"drop": 0}, {"drop": 24}, 25)[0])


if __name__ == "__main__":
    unittest.main()
