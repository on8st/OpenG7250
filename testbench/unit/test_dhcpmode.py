"""T-A1-100's judgments (0082) on synthetic dhcp.log and sock.log entries, and its registration. No network.
GPL-3.0-or-later, (c) 2026 ON8ST."""
import unittest

from testbench import t_dhcpmode as d
from testbench.core import REGISTRY

S = "192.168.73.50"


def msg(t, kind="DISCOVER", src=S, ciaddr="0.0.0.0", broadcast=True, t_us=0):
    return dict(type=kind, src=f"{src}:68", ciaddr=ciaddr, broadcast=broadcast, t_us=t_us or t)


class Messages(unittest.TestCase):
    def test_good(self):
        self.assertEqual(d.bad_client_msgs([msg(1), msg(2, "REQUEST"), msg(3, "OFFER", src="192.168.73.1")], S), [])

    def test_each_field(self):
        for bad in (msg(1, src="0.0.0.0"), msg(1, ciaddr=S), msg(1, broadcast=False)):
            self.assertEqual(len(d.bad_client_msgs([bad], S)), 1, bad)

    def test_server_messages_not_judged(self):
        self.assertEqual(d.bad_client_msgs([msg(1, "ACK", src="192.168.73.1", broadcast=False)], S), [])


class Borrows(unittest.TestCase):
    def test_long_only(self):
        b = [(1_000_000, 2_500_000), (10_000_000, 12_100_000), (20_000_000, 23_000_000)]
        self.assertEqual(d.long_borrows(b, 0), [2.1, 3.0])
        self.assertEqual(d.long_borrows(b, 0, 15_000_000), [2.1])  # up to the address change
        self.assertEqual(d.long_borrows(b, 15_000_000), [3.0])

    def test_closed_span(self):
        sock = [dict(t_us=5_000_000, action="CLOSE", port=7251, boot=1), dict(t_us=5_000_100, action="CLOSE", port=1024, boot=1),
                dict(t_us=13_000_000, action="OPEN", port=1024, boot=1), dict(t_us=13_000_100, action="OPEN", port=7251, boot=1)]
        first, back = d.closed_span(sock, 1.0)
        self.assertEqual((first, back), (5.0, 13.0))
        self.assertEqual(d.closed_span(sock, 6.0), (None, None))


class Registered(unittest.TestCase):
    def test_registered(self):
        import testbench.__main__  # noqa: F401  (registers every module)
        self.assertIn("T-A1-100", REGISTRY)


if __name__ == "__main__":
    unittest.main()
