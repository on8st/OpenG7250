"""Unit test of the hw.trace reader on synthetic events (v1.4 schema, v1.3 events too).
Run: python3 -m unittest discover -s unit -t .  (from testbench/). GPL-3.0-or-later, (c) 2026 ON8ST."""
import unittest

from testbench.trace import check_pins, check_safe


def gpio(seq, pin, level, mode="output", pull=None):
    e = dict(seq=seq, t_us=seq, kind="gpio", pin=pin, level=level, mode=mode)
    if pull:
        e["pull"] = pull
    return e


def spi(seq, bus, data):
    return dict(seq=seq, t_us=seq, kind="spi", bus=bus, cs=None, data=data)


# the snapshot hw.trace start emits (v1.4), for a firmware that holds the HIS 85 safe state
START = [gpio(1, "PF4", 1), gpio(2, "PF6", 0), gpio(3, "PC7", 0), gpio(4, "PH11", 0),
         gpio(5, "PA10", 1), gpio(6, "PH10", 1), gpio(7, "PD13", 1), gpio(8, "PG2", 1),
         gpio(9, "PG5", 1), gpio(10, "PD12", 0), gpio(11, "PB10", 0),
         gpio(12, "PD5", "z", mode="analog")]  # PD5 high-impedance: allowed (HIS 70)
RUN = [spi(20, "SPI1", "01"), gpio(21, "PA11", 1), gpio(22, "PA10", 0),  # good chain A frame, then OE on
       dict(seq=23, t_us=23, kind="reset", cause="power"), dict(seq=24, kind="unknown-kind"),
       spi(25, "SPI1", "ff00ff"), gpio(26, "PH12", 1), gpio(27, "PH10", 0)]  # chain B frame, then OE on
SAFE = START + RUN


def failed(events):
    return [n for n, ok, _ in check_safe(events) if not ok]


class TraceTest(unittest.TestCase):
    def test_safe_trace_passes(self):
        self.assertEqual(failed(SAFE), [])

    def test_v13_events_without_mode(self):
        old = [dict(seq=1, t_us=1, kind="gpio", pin="PF6", level=0), spi(2, "SPI1", "01"),
               dict(seq=3, t_us=3, kind="gpio", pin="PA11", level=1)]
        self.assertEqual(failed(old), [])

    def test_each_violation_is_found(self):
        cases = {
            "PA11 frame with bit 7": ([spi(30, "SPI1", "81"), gpio(31, "PA11", 0)], "chain A bit 7"),
            "T/R line driven to TX": ([gpio(30, "PF6", 1)], "PF6:"),
            "T/R line released": ([gpio(30, "PF4", "z", mode="input", pull="none")], "PF4:"),
            "ATU strobe pulsed": ([gpio(30, "PG2", 0)], "PG2:"),
            "ATU strobe floating": ([gpio(30, "PG5", "z", mode="analog")], "PG5:"),
            "ATU strobe in AF, unknown": ([gpio(30, "PD13", None, mode="af")], "PD13:"),
            "PD12 high": ([gpio(30, "PD12", 1)], "PD12:"),
            "PD12 not held": ([gpio(30, "PD12", "z", mode="analog")], "PD12:"),
            "codec out of reset": ([gpio(30, "PB10", 1)], "PB10:"),
            "PD5 driven low": ([gpio(30, "PD5", 0)], "PD5:"),
        }
        for name, (extra, prefix) in cases.items():
            bad = failed(SAFE + extra)
            self.assertEqual(len(bad), 1, (name, bad))
            self.assertTrue(bad[0].startswith(prefix), (name, bad))

    def test_oe_before_latch(self):
        # OE enabled (low) before any chain A frame: fails; z before the latch: fails too
        self.assertTrue(any(n.startswith("PA10") for n in failed(START + [gpio(30, "PA10", 0)])))
        self.assertTrue(any(n.startswith("PH10") for n in failed(START + [gpio(30, "PH10", "z", mode="analog")])))
        # after a latched frame with bit 7 set the OE is still not free
        bad = failed(START + [spi(30, "SPI1", "80"), gpio(31, "PA11", 1), gpio(32, "PA10", 0)])
        self.assertTrue(any(n.startswith("PA10") for n in bad), bad)

    def test_pd5_high_z_and_unknown_pass(self):
        for v, mode in ((1, "af"), ("z", "analog"), (None, "af")):
            self.assertEqual(failed(SAFE + [gpio(30, "PD5", v, mode=mode)]), [], (v, mode))

    def test_snapshot_pins(self):
        pins = {e["pin"]: dict(level=e["level"], mode=e["mode"]) for e in START}
        self.assertTrue(all(ok for _, ok, _ in check_pins(pins)))
        pins["PG2"] = dict(level="z", mode="analog")
        bad = [n for n, ok, _ in check_pins(pins) if not ok]
        self.assertEqual(len(bad), 1)
        self.assertTrue(bad[0].startswith("PG2"))


if __name__ == "__main__":
    unittest.main()


from testbench.trace import check_fpga  # noqa: E402


def boot(seq, t):
    return dict(seq=seq, t_us=t, kind="boot", at="0x08020000", pc="0x08020101")


def at(seq, t, pin, level, mode="output"):
    return dict(seq=seq, t_us=t, kind="gpio", pin=pin, level=level, mode=mode)


def held_ok(seq0, t):
    """Every held pin at its safe level at time t."""
    lv = dict(PF4=1, PF6=0, PC7=0, PH11=0, PA10=1, PH10=1, PD13=1, PG2=1, PG5=1, PD12=0, PB10=0)
    return [at(seq0 + i, t, p, v) for i, (p, v) in enumerate(lv.items())]


class BootWindowTest(unittest.TestCase):
    def test_z_before_boot_plus_20us_is_outside_the_spec(self):
        ev = [boot(1, 1000), at(2, 1005, "PF4", "z", "analog")] + held_ok(10, 1015)
        self.assertEqual([n for n, ok, _ in check_safe(ev, since="boot") if not ok], [])

    def test_wrong_level_at_boot_plus_20us_fails(self):
        # PF4 still "z" at boot + 20 us: the state at that moment is judged
        ev = [boot(1, 1000), at(2, 1005, "PF4", "z", "analog")] + [e for e in held_ok(10, 1015) if e["pin"] != "PF4"] \
            + [at(30, 1100, "PF4", 1)]
        bad = [n for n, ok, _ in check_safe(ev, since="boot") if not ok]
        self.assertEqual(len(bad), 1)
        self.assertTrue(bad[0].startswith("PF4"))

    def test_start_mode_skips_the_replayed_boot_phase(self):
        # replayed boot phase (within 10 ms) with a bad level, then the start snapshot is right
        ev = [boot(1, 1000), at(2, 1030, "PD12", 1)] + held_ok(10, 50_000)
        self.assertEqual([n for n, ok, _ in check_safe(ev, since="start") if not ok], [])
        self.assertTrue(any(n.startswith("PD12") for n, ok, _ in check_safe(ev, since="boot") if not ok))

    def test_reset_during_the_trace_opens_a_new_window(self):
        ev = held_ok(1, 50_000) + [dict(seq=20, t_us=60_000, kind="reset", cause="power"), boot(21, 60_010),
                                   at(22, 60_015, "PG2", "z", "analog")] + held_ok(30, 60_040)
        self.assertEqual([n for n, ok, _ in check_safe(ev, since="start") if not ok], [])


def cmd_byte(seq, byte):
    """D0-D7 = byte, then CMD and STB up, both down (HIS 80)."""
    ev = [at(seq + i, seq, f"PE{i}", (byte >> i) & 1) for i in range(8)]
    return ev + [at(seq + 8, seq, "PE8", 1), at(seq + 9, seq, "PE9", 1), at(seq + 10, seq, "PE9", 0),
                 at(seq + 11, seq, "PE8", 0)]


class FpgaBusTest(unittest.TestCase):
    def base(self):
        return [at(1, 1, "PE9", 0), at(2, 1, "PE8", 0)] + held_ok(3, 1)

    def test_clean_bus(self):
        ev = self.base() + cmd_byte(100, 0x08) + cmd_byte(200, 0x01)
        self.assertEqual([n for n, ok, _ in check_fpga(ev) if not ok], [])

    def test_forbidden_command(self):
        bad = [n for n, ok, _ in check_fpga(self.base() + cmd_byte(100, 0x05)) if not ok]
        self.assertEqual(len(bad), 1)
        self.assertIn("0x05", bad[0])

    def test_0x01_outside_the_safe_state(self):
        ev = self.base() + [at(50, 50, "PF6", 1)] + cmd_byte(100, 0x01)
        bad = [n for n, ok, _ in check_fpga(ev) if not ok]
        self.assertEqual(len(bad), 1)
        self.assertIn("0x01", bad[0])

    def test_data_back_to_output_with_stb_high(self):
        ev = self.base() + [at(50, 50, "PE0", 0, "input"), at(51, 51, "PE9", 1), at(52, 52, "PE0", 0, "output")]
        bad = [n for n, ok, _ in check_fpga(ev) if not ok]
        self.assertEqual(len(bad), 1)
        self.assertIn("D0-D7", bad[0])
