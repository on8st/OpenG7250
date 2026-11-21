"""core.Ctx.need: a fixture listed by its group name or by its operation name (testlist-format
section 4). GPL-3.0-or-later, (c) 2026 ON8ST."""
import unittest

from testbench.core import Ctx, Skip


def ctx(fixtures):
    return Ctx(dict(radio=dict(ip="127.0.0.1"), fixtures=fixtures))


class NeedTest(unittest.TestCase):
    def test_group_listed_offers_its_operations(self):
        c = ctx(["dhcp.server", "net.endpoint"])
        for op in ("dhcp.start", "dhcp.stop", "dhcp.offer", "dhcp.nak", "dhcp.expire", "dhcp.log", "dhcp.server", "net.endpoint"):
            c.need(op)

    def test_operations_listed_offer_the_group(self):
        c = ctx(["dhcp.start", "dhcp.stop", "dhcp.log"])
        c.need("dhcp.server")
        c.need("dhcp.stop")

    def test_missing_skips(self):
        c = ctx(["hw.observe"])
        for op in ("dhcp.stop", "dhcp.server", "net.endpoint", "hw.trace"):
            with self.assertRaises(Skip):
                c.need(op)


if __name__ == "__main__":
    unittest.main()
