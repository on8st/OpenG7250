"""key.log entries (v1.5): at a key-on the T/R lines and the bias must agree with chain A bit 7;
at a key-off they are not judged. GPL-3.0-or-later, (c) 2026 ON8ST."""
import unittest

from testbench.t_cw import Log

RX = dict(PF4=1, PF6=0, PC7=0, PH11=0)
TX = dict(PF4=0, PF6=1, PC7=1, PH11=1)


def rep(*key):
    return dict(key=list(key), hp=[])


class KeyLogTest(unittest.TestCase):
    def test_agree(self):
        log = Log(rep(dict(t_us=0, on=1, tr=TX, bias_nonzero=True), dict(t_us=60000, on=0, tr=TX, bias_nonzero=True)))
        self.assertEqual(log.disagree, [])
        self.assertEqual(log.marks, [(0, 60000)])

    def test_tr_at_rx_on_key_on(self):
        self.assertEqual(len(Log(rep(dict(t_us=0, on=1, tr=RX, bias_nonzero=True))).disagree), 1)

    def test_bias_zero_on_key_on(self):
        self.assertEqual(len(Log(rep(dict(t_us=0, on=1, tr=TX, bias_nonzero=False))).disagree), 1)

    def test_key_off_not_judged(self):
        log = Log(rep(dict(t_us=0, on=1, tr=TX, bias_nonzero=True), dict(t_us=1, on=0, tr=RX, bias_nonzero=False)))
        self.assertEqual(log.disagree, [])


if __name__ == "__main__":
    unittest.main()
