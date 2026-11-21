"""The net.log path of the takeover tests (format v1.6) on a fake fixture: the copy spacing is
judged on the radio's send times, not on host arrival. GPL-3.0-or-later, (c) 2026 ON8ST."""
import struct
import unittest

from testbench import t_a2
from testbench.core import Ctx


def entry(t_us, port, seq=0, addr=0x0202, d="from_radio", radio_port=7250):
    head = struct.pack(">IHI", seq, addr, 0x40000)[:16]
    return dict(t_us=t_us, dir=d, socket=3, radio_port=radio_port, peer=f"169.254.0.20:{port}", len=10, head=head.hex())


class FakeCtx(Ctx):
    def __init__(self, entries):
        super().__init__(dict(radio=dict(ip="127.0.0.1"), fixtures=["net.log"]))
        self.entries = entries

    def fixture(self, op, **args):
        assert op == "net.log"
        if args.get("action") == "start":
            return None
        n = args.get("from", 0)
        return dict(t_us=0, count=len(self.entries), dropped=0, entries=self.entries[n:])


def pkt(seq):
    return (7250, struct.pack(">IHI", seq, 0x0202, 0x40000), 0.0)


class NetLogTest(unittest.TestCase):
    def run_copies(self, times):
        ctx = FakeCtx([entry(t, 38000, i) for i, t in enumerate(times)] + [entry(5, 38001)])
        t_a2.netlog_start(ctx)
        # host arrival all at the same instant: only the net.log stamps can pass
        t_a2.check_copies(ctx, "A", [pkt(2), pkt(3), pkt(4)], 1, 38000)
        return {c["name"]: c for c in ctx.checks}

    def test_spacing_from_send_times(self):
        c = self.run_copies([1_000_000, 1_010_200, 1_019_800])
        sp = [v for k, v in c.items() if "10 ms apart" in k][0]
        self.assertTrue(sp["ok"], sp)
        self.assertIn("net.log", sp["name"])

    def test_bad_spacing_fails(self):
        c = self.run_copies([1_000_000, 1_034_000, 1_042_000])
        sp = [v for k, v in c.items() if "10 ms apart" in k][0]
        self.assertFalse(sp["ok"], sp)


if __name__ == "__main__":
    unittest.main()
