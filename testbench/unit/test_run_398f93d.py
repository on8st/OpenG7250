"""The bench faults of the fw-4e567d2 comparison run on emulator 398f93d and of emu's diagnosis runs on
265c769, and the stored C-1 configuration in force at every start (fw/v2 512764a, A1.6): each judgment
or wait on synthetic logs or a fake runner. No network. GPL-3.0-or-later, (c) 2026 ON8ST."""
import struct
import unittest
from unittest import mock

from testbench import t_a2, t_linklocal as L, t_update
from testbench.core import Ctx
from testbench.p2 import PORT_DUCIQ, PORT_REG, TX_ARM_STATE

OLD, NEW = "169.254.10.20", "169.254.30.40"


class Reaction(unittest.TestCase):
    """Item 2: the next conflict raised only after the image reacted to the one before."""

    def test_sock_close_after_the_raise(self):
        sock = [dict(t_us=1_000_000, boot=1, action="CLOSE", port=1024), dict(t_us=5_200_000, boot=1, action="CLOSE", port=1024)]
        self.assertIn("5.200", L.reaction(sock, None, 5.0, OLD, OLD))

    def test_a_close_before_the_raise_is_none(self):
        sock = [dict(t_us=1_000_000, boot=1, action="CLOSE", port=1024), dict(t_us=1_100_000, boot=1, action="OPEN", port=1024)]
        self.assertIsNone(L.reaction(sock, None, 5.0, OLD, OLD))

    def test_probe_or_address(self):
        frames = [(5.5, 1, L.ZERO, NEW)]
        self.assertIn("probe", L.reaction(None, frames, 5.0, OLD, OLD))
        self.assertIn(NEW, L.reaction(None, None, 5.0, OLD, NEW))
        self.assertIsNone(L.reaction([], [], 5.0, OLD, OLD))

    def test_conflict_waits_for_the_reaction(self):
        """The fault of the run: 'reachable 0.5 s after' on the old address, before the image acted, so the next
        raise found the conflict still set. conflict() returns only once the CLOSE is in and the address moved."""
        state = dict(t=10.0, raised=None, ip=OLD, polls=0)
        sock = []

        def fixture(name, /, **a):
            if name == "hw.observe":
                return dict(t_us=int(state["t"] * 1e6))
            if name == "ip.conflict":
                state["raised"] = state["t"]
                return {}
            if name == "sock.log" and a.get("action") == "read":
                state["polls"] += 1
                state["t"] += 0.2
                if state["raised"] is not None and state["t"] - state["raised"] >= 1.0 and not sock:
                    sock.append(dict(t_us=int(state["t"] * 1e6), boot=1, action="CLOSE", port=1024))
                return dict(count=len(sock), dropped=0, entries=sock[a.get("from", 0):])
            if name == "eth.log" and a.get("action") == "read":
                return dict(count=0, dropped=0, entries=[])
            return {}
        ctx = Ctx(dict(radio=dict(ip=OLD), fixtures=["hw.observe", "sock.log", "eth.log", "ip.conflict"]))
        ctx.fixture = fixture
        reach = []

        def reachable(c, label, limit_s=40):
            reach.append(state["t"])
            if sock:
                c.endpoint["radio"]["ip"] = NEW
        with mock.patch.object(L, "wait_reachable", reachable), mock.patch.object(L.time, "sleep", lambda s: None), \
                mock.patch("testbench.core.RadioClock.now", lambda self: state["t"]):
            _, _, t0, why = L.conflict(ctx, "c")
        self.assertIn("closed", why)
        self.assertGreaterEqual(reach[0] - t0, 1.0)  # not reachable-tested before the image acted
        self.assertEqual(ctx.endpoint["radio"]["ip"], NEW)


class DhcpAgain(unittest.TestCase):
    """Item 3: 'DHCP again' read until a DISCOVER follows the DECLINE, in radio time."""

    def test_pure(self):
        self.assertTrue(L.discover_after_decline(["DISCOVER", "OFFER", "DECLINE", "DISCOVER"]))
        self.assertFalse(L.discover_after_decline(["DISCOVER", "DECLINE"]))
        self.assertFalse(L.discover_after_decline(["DISCOVER"]))

    def test_waits_for_the_restart(self):
        state = dict(t=0.0)
        log = [dict(type="DHCPDECLINE")]

        def fixture(name, /, **a):
            if name == "dhcp.log":
                state["t"] += 0.5
                if state["t"] >= 11.7 and len(log) == 1:  # "DHCP again in 10 s" after the DECLINE at 1.7 s
                    log.append(dict(type="DHCPDISCOVER"))
                return dict(messages=list(log))
            return {}
        ctx = Ctx(dict(radio=dict(ip=OLD), fixtures=["dhcp.log"]))
        ctx.fixture = fixture
        with mock.patch.object(L.time, "sleep", lambda s: None), mock.patch("testbench.core.RadioClock.now", lambda self: state["t"]):
            kinds = L.wait_dhcp_again(ctx, 0)
        self.assertEqual(kinds, ["DECLINE", "DISCOVER"])


class StoredMode(unittest.TestCase):
    """fw/v2 512764a: a start brings up the stored C-1 configuration; a test that expects a DHCP start sets the
    stored address mode to DHCP first, and says so."""

    def run_with(self, mode, status=0):
        ctx = Ctx(dict(radio=dict(ip=OLD), fixtures=[]))
        sets = []

        def set_config(c, s, *items, on_reply=None):
            sets.append(items)
            return status
        with mock.patch.object(L, "stored_mode", lambda c: mode), mock.patch("testbench.t_dhcpmode.set_config", set_config), \
                mock.patch("testbench.t_mgmt.secret_of", lambda c: b"k"):
            ok = L.dhcp_mode_stored(ctx, "start")
        return ctx, ok, sets

    def test_dhcp_stored_does_nothing(self):
        ctx, ok, sets = self.run_with(b"\x00")
        self.assertTrue(ok)
        self.assertEqual((sets, ctx.notes, ctx.parts), ([], [], []))

    def test_static_stored_is_set_back_with_a_note(self):
        ctx, ok, sets = self.run_with(b"\x01")
        self.assertTrue(ok)
        self.assertEqual(sets, [((1, b"\x00"),)])
        self.assertTrue(any("left by an earlier test" in n for n in ctx.notes))
        self.assertFalse(ctx.no_pass)

    def test_not_settable_or_unreadable_is_a_part(self):
        for mode, st in ((b"\x01", 35), (None, 0)):
            ctx, ok, _ = self.run_with(mode, st)
            self.assertFalse(ok)
            self.assertTrue(ctx.no_pass)
            self.assertTrue(ctx.parts)
            self.assertEqual([c for c in ctx.checks if not c["ok"]], [])

    def test_static_back_after_an_error(self):
        ctx = Ctx(dict(radio=dict(ip=OLD), fixtures=[]))
        sets = []
        with mock.patch.object(L, "stored_mode", lambda c: b"\x01"), \
                mock.patch("testbench.t_dhcpmode.set_config", lambda c, s, *i, on_reply=None: sets.append(i) or 0):
            L.static_back(ctx, b"k", None)
        self.assertEqual(sets, [((1, b"\x00"),)])


def reg_entry(t_us, port, addr=TX_ARM_STATE):
    head = (b"\0\0\0\0" + struct.pack(">H", addr) + b"\0\0").hex()
    return dict(t_us=t_us, dir="from_radio", radio_port=PORT_REG, peer=f"10.0.0.2:{port}", head=head)


class Notices(unittest.TestCase):
    """Item 4: T-A2-11's notices timed by net.log, not by the host's receive times."""

    def test_notice_times_from_the_first_duc(self):
        e = [reg_entry(1_000, 9000), dict(t_us=50_000, dir="to_radio", radio_port=PORT_DUCIQ, peer="10.0.0.2:9000")]
        e += [reg_entry(50_000 + i * 100_000, 9000) for i in range(10)] + [reg_entry(60_000, 9001)]
        start = t_a2.first_from(e, 9000, PORT_DUCIQ)
        self.assertEqual(start, 50_000)
        ts = t_a2.notice_times(e, 9000, start)
        self.assertEqual(len(ts), 10)
        self.assertTrue(all(b - a == 100_000 for a, b in zip(ts, ts[1:])))


class Trial(unittest.TestCase):
    """Item 5: 'still a trial' judged at any uptime under 9.5 s (8.1 s since the image probes its address)."""

    def test_window(self):
        self.assertEqual(t_update.trial_step(2.5), 5.5)
        self.assertEqual(t_update.trial_step(8.1), 0.0)
        self.assertIsNone(t_update.trial_step(9.6))
        self.assertIsNone(t_update.trial_step(None))


if __name__ == "__main__":
    unittest.main()
