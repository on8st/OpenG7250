"""judge_b5_02 (T-B5-02) on synthetic net.log entries. No network. GPL-3.0-or-later, (c) 2026 ON8ST."""
import unittest

from testbench import t_p2

P = 40000


def e(t_ms, d, port, peer=P):
    return dict(t_us=int(t_ms * 1000), dir=d, radio_port=port, peer=f"10.0.0.2:{peer}")


def log(regs_until_ms, status_until_ms, last_cc_ms=100):
    out = [e(0, "to_radio", 1024), e(last_cc_ms, "to_radio", 1027)]
    out += [e(t, "to_radio", 7250) for t in range(200, regs_until_ms + 1, 100)]
    out += [e(t, "from_radio", 1025) for t in range(0, status_until_ms + 1, 50)]
    out += [e(t, "from_radio", 1025, peer=P + 1) for t in range(0, 3000, 50)]  # another host's
    return out


class B502(unittest.TestCase):
    def test_session_ends_at_the_watchdog(self):
        late, reach = t_p2.judge_b5_02(log(1600, 1100), P)
        self.assertEqual((late, reach), (0, 400.0))

    def test_registers_keep_it(self):
        late, _ = t_p2.judge_b5_02(log(1600, 1600), P)
        self.assertGreater(late, 0)

    def test_slow_emulator_window_not_reached(self):
        # 15 register packets over 0.36 s of radio time: they end inside the window
        late, reach = t_p2.judge_b5_02(log(460, 460), P)
        self.assertEqual(late, 0)
        self.assertLess(reach, 300)

    def test_no_cc(self):
        self.assertEqual(t_p2.judge_b5_02([e(0, "from_radio", 1025)], P), (None, None))


if __name__ == "__main__":
    unittest.main()
