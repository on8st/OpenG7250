// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The RF safe state as data. The board applies [`SAFE_STATE`] before anything else at every
//! start (boot stage and application) and again whenever it leaves transmit.
//!
//! Source: [HIS] HIS 85 "Safe state at reset" (`d199d33`), with HIS 20 and decision 0029 (PH9
//! held high), HIS 70 (PD5 never low), HIS 40 (CH394 RST# held low), HIS 50 and decision 0031
//! (codec in reset), decision 0030 (PH6, PH3, PH4 not used), decision 0032 (ATU latches
//! unwritten: bypass). Since HIS `d199d33` the ATU latch strobes are driven to their inactive
//! level, high (MEDIUM), and never pulsed, so no latch is ever written.

/// GPIO port letter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Port {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
}

/// What the safe state does with a pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Push-pull output, driven high. The output register is written before the pin becomes an
    /// output, so the pin never glitches to the other level.
    DriveHigh,
    /// Push-pull output, driven low (same order).
    DriveLow,
    /// Input with the internal pull-up: the pin idles high and is never driven. The pull-up is
    /// set before the pin leaves its reset (analog) mode, so it never floats as an input.
    InputPullUp,
    /// The pin is not touched: it keeps its reset state (input / analog, high-impedance per
    /// [HIS] HIS 85 "Open"). Listed so that tests can prove nobody drives it.
    LeaveUntouched,
}

/// Why a pin is in the table. The RF and power pins are the safe state proper: they are held
/// at every start and whenever not transmitting, and checked by the self-test. A part held in
/// reset "until the product brings it up" ([HIS] HIS 85, CH394 and codec rows) is set at start
/// only; its driver takes the pin over later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Rf,
    Power,
    HeldUntilUsed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PinRule {
    pub port: Port,
    pub pin: u8,
    pub action: Action,
    pub kind: Kind,
    pub what: &'static str,
}

const fn r(port: Port, pin: u8, action: Action, what: &'static str) -> PinRule {
    let kind = match (port, pin) {
        (Port::B, 5) | (Port::B, 10) => Kind::HeldUntilUsed,
        (Port::H, 9) => Kind::Power,
        _ => Kind::Rf,
    };
    PinRule { port, pin, action, kind, what }
}

impl PinRule {
    /// Part of the safe state proper (held whenever not transmitting).
    pub const fn is_safety(&self) -> bool {
        !matches!(self.kind, Kind::HeldUntilUsed)
    }
}

use Action::*;
use Port::*;

/// The safe state, in the order it is applied. RF parts first ([HIS] HIS 85: "before anything
/// else in the RF section"), then the other parts held in reset, then the power hold.
pub const SAFE_STATE: &[PinRule] = &[
    // Output enables of both shift-register chains inactive first, so that whatever the chains
    // hold cannot reach the relays (HIS 85: PA10, PH10 high = outputs disabled).
    r(A, 10, DriveHigh, "SPI1 chain A (TX LPF + T/R) output enable inactive"),
    r(H, 10, DriveHigh, "SPI1 chain B (RX BPF, preamp, attenuator) output enable inactive"),
    // PA DAC in reset: all outputs at VrefL, so bias channels 2, 7, 8 are at 0 (HIS 85, SPI2).
    r(B, 14, DriveLow, "PA bias/drive DAC RESET asserted (outputs at VrefL)"),
    // T/R lines at their RX levels (HIS 85: PF4 high; PF6, PC7, PH11 low).
    r(F, 4, DriveHigh, "T/R line, RX level"),
    r(F, 6, DriveLow, "T/R line, RX level"),
    r(C, 7, DriveLow, "T/R line, RX level"),
    r(H, 11, DriveLow, "T/R line, RX level"),
    r(D, 12, DriveLow, "unknown output, keep low (HIS 85)"),
    // ATU latch strobes inactive (high, HIS 85 `d199d33`, MEDIUM). Driven high from the
    // high-impedance reset state: the level is written before the pin becomes an output, so the
    // strobe never goes through its active level and no latch is clocked.
    r(D, 13, DriveHigh, "ATU latch strobe (network style, bypass, antenna) inactive; never pulsed (HIS 85; 0032)"),
    r(G, 2, DriveHigh, "ATU latch strobe (reactance bank) inactive; never pulsed (HIS 85; 0032)"),
    r(G, 5, DriveHigh, "ATU latch strobe (reactance bank) inactive; never pulsed (HIS 85; 0032)"),
    // Not RF, held in reset until used.
    r(B, 5, DriveLow, "CH394 RST# held (HIS 40, HIS 85)"),
    r(B, 10, DriveLow, "Main codec reset held (HIS 50, HIS 85; 0031)"),
    // Power hold (HIS 20 HYPOTHESIS; 0029 item 1).
    r(H, 9, DriveHigh, "power hold"),
    // Never driven by the safe state.
    r(A, 4, LeaveUntouched, "DAC1 OUT1 RF gain: safe level unknown (HIS 85)"),
    r(A, 12, LeaveUntouched, "unknown output, no safe level known (HIS 85)"),
    r(A, 11, LeaveUntouched, "chain A latch: outputs disabled by PA10 instead (HIS 85)"),
    r(H, 12, LeaveUntouched, "chain B latch: outputs disabled by PH10 instead (HIS 85)"),
    // USART2 TX to the controller: idle level or high-impedance, never low (HIS 70, HIS 85). An
    // input with pull-up is both: it idles high and drives nothing. Earlier builds left it in its
    // reset (analog) mode; the emulator trace showed it as level 0 (T-B10-L11, 2026-11-21).
    r(D, 5, InputPullUp, "USART2 TX to the controller: input with pull-up, idles high, never driven (HIS 70, HIS 85)"),
    r(H, 6, LeaveUntouched, "PTT input, not used in v1.0 (0030)"),
    r(H, 3, LeaveUntouched, "key contact, not used in v1.0 (0030)"),
    r(H, 4, LeaveUntouched, "key contact, not used in v1.0 (0030)"),
    r(H, 5, LeaveUntouched, "power key, ignored in v1.0 (0029 item 2)"),
    // FPGA bus ([HIS] HIS 80): never driven until the bus parameters are cleared (fpga_gate).
    r(E, 0, LeaveUntouched, "FPGA bus D0: undriven until the FPGA bus gate opens (HIS 80)"),
    r(E, 1, LeaveUntouched, "FPGA bus D1: undriven until the FPGA bus gate opens (HIS 80)"),
    r(E, 2, LeaveUntouched, "FPGA bus D2: undriven until the FPGA bus gate opens (HIS 80)"),
    r(E, 3, LeaveUntouched, "FPGA bus D3: undriven until the FPGA bus gate opens (HIS 80)"),
    r(E, 4, LeaveUntouched, "FPGA bus D4: undriven until the FPGA bus gate opens (HIS 80)"),
    r(E, 5, LeaveUntouched, "FPGA bus D5: undriven until the FPGA bus gate opens (HIS 80)"),
    r(E, 6, LeaveUntouched, "FPGA bus D6: undriven until the FPGA bus gate opens (HIS 80)"),
    r(E, 7, LeaveUntouched, "FPGA bus D7: undriven until the FPGA bus gate opens (HIS 80)"),
    r(E, 8, LeaveUntouched, "FPGA bus CMD: undriven until the FPGA bus gate opens (HIS 80)"),
    r(E, 9, LeaveUntouched, "FPGA bus STB: undriven until the FPGA bus gate opens (HIS 80)"),
    r(E, 10, LeaveUntouched, "FPGA bus DRDY (input): untouched (HIS 80)"),
];

/// The rule for a pin, if the safe state lists it.
pub fn rule(port: Port, pin: u8) -> Option<&'static PinRule> {
    SAFE_STATE.iter().find(|r| r.port == port && r.pin == pin)
}

/// One GPIO port's part of the safe state at reset, as register masks (RM0468 GPIO: MODER and
/// PUPDR 2 bits per pin, OTYPER 1 bit, BSRR set bits 0-15 and reset bits 16-31).
///
/// Applied in this order: BSRR (levels), OTYPER (push-pull), PUPDR (pull-ups), MODER (output or
/// input). So a pin becomes an output only after its level is written, and an input only after
/// its pull-up is on, as in the per-pin order of [`Action`]. Pins outside the table, and those
/// marked [`Action::LeaveUntouched`], keep every bit (read-modify-write). The output speed and the
/// floating pull of driven pins do not change a level; the full per-pin apply right after sets
/// them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortStep {
    pub port: Port,
    pub bsrr: u32,
    pub otyper_clear: u32,
    pub pupdr_clear: u32,
    pub pupdr_set: u32,
    pub moder_clear: u32,
    pub moder_set: u32,
}

const fn port_index(p: Port) -> usize {
    p as usize
}

const fn reset_plan(t: &[PinRule]) -> ([PortStep; 8], usize) {
    let empty = PortStep { port: Port::A, bsrr: 0, otyper_clear: 0, pupdr_clear: 0, pupdr_set: 0, moder_clear: 0, moder_set: 0 };
    let mut plan = [empty; 8];
    // Ports in the order of their first rule that acts: the output enables (PA10, PH10) first.
    let mut slot_of = [usize::MAX; 8];
    let mut n = 0;
    let mut i = 0;
    while i < t.len() {
        let r = t[i];
        i += 1;
        let (bsrr, ot, pu_clear, pu_set, mo_set) = match r.action {
            Action::DriveHigh => (1u32 << r.pin, 1u32 << r.pin, 0u32, 0u32, 1u32),
            Action::DriveLow => (1u32 << (r.pin + 16), 1u32 << r.pin, 0, 0, 1),
            Action::InputPullUp => (1u32 << r.pin, 0, 3u32 << (2 * r.pin), 1u32 << (2 * r.pin), 0),
            Action::LeaveUntouched => continue,
        };
        let k = port_index(r.port);
        if slot_of[k] == usize::MAX {
            slot_of[k] = n;
            plan[n].port = r.port;
            n += 1;
        }
        let s = &mut plan[slot_of[k]];
        s.bsrr |= bsrr;
        s.otyper_clear |= ot;
        s.pupdr_clear |= pu_clear;
        s.pupdr_set |= pu_set;
        s.moder_clear |= 3u32 << (2 * r.pin);
        s.moder_set |= mo_set << (2 * r.pin);
    }
    (plan, n)
}

const PLAN: ([PortStep; 8], usize) = reset_plan(SAFE_STATE);
/// The safe state at reset, one step per port ([`PortStep`]); the first [`RESET_PORTS`] are used.
/// Computed at build time, so the board applies it as straight-line code (0045: within 20 us of
/// the boot stage's first instruction).
pub const RESET_PLAN: [PortStep; 8] = PLAN.0;
pub const RESET_PORTS: usize = PLAN.1;

#[cfg(test)]
mod tests {
    use super::*;

    fn act(p: Port, n: u8) -> Action {
        rule(p, n).unwrap().action
    }

    #[test]
    fn his85_levels() {
        assert_eq!(act(F, 4), DriveHigh);
        assert_eq!(act(F, 6), DriveLow);
        assert_eq!(act(C, 7), DriveLow);
        assert_eq!(act(H, 11), DriveLow);
        assert_eq!(act(A, 10), DriveHigh);
        assert_eq!(act(H, 10), DriveHigh);
        assert_eq!(act(B, 14), DriveLow);
        assert_eq!(act(D, 12), DriveLow);
        assert_eq!(act(B, 5), DriveLow);
        assert_eq!(act(B, 10), DriveLow);
        // ATU strobes inactive high (HIS 85 `d199d33`).
        for (p, n) in [(D, 13), (G, 2), (G, 5)] {
            assert_eq!(act(p, n), DriveHigh, "P{p:?}{n}");
            assert_eq!(rule(p, n).unwrap().kind, Kind::Rf);
        }
    }

    #[test]
    fn fpga_bus_is_never_driven_by_the_safe_state() {
        for n in 0..=10 {
            assert_eq!(act(E, n), LeaveUntouched, "PE{n}");
        }
        assert!(!SAFE_STATE.iter().any(|r| r.port == E && r.action != LeaveUntouched));
    }

    #[test]
    fn pd5_is_never_driven_low() {
        // HIS 70 / HIS 85: high or high-impedance, never low.
        let r = rule(D, 5).unwrap();
        assert_eq!(r.action, InputPullUp);
        assert!(r.is_safety());
        // No rule anywhere drives PD5.
        assert!(!SAFE_STATE.iter().any(|x| x.port == D && x.pin == 5 && matches!(x.action, DriveLow | DriveHigh)));
    }

    #[test]
    fn power_hold_high_0029() {
        assert_eq!(act(H, 9), DriveHigh);
    }

    #[test]
    fn never_driven() {
        for (p, n) in [(A, 4), (A, 12), (H, 6), (H, 3), (H, 4), (H, 5)] {
            assert_eq!(act(p, n), LeaveUntouched, "P{p:?}{n}");
        }
    }

    #[test]
    fn output_enables_before_anything_else() {
        assert_eq!((SAFE_STATE[0].port, SAFE_STATE[0].pin), (A, 10));
        assert_eq!((SAFE_STATE[1].port, SAFE_STATE[1].pin), (H, 10));
        assert_eq!((SAFE_STATE[2].port, SAFE_STATE[2].pin), (B, 14));
    }

    #[test]
    fn kinds() {
        assert_eq!(rule(B, 5).unwrap().kind, Kind::HeldUntilUsed);
        assert_eq!(rule(B, 10).unwrap().kind, Kind::HeldUntilUsed);
        assert_eq!(rule(H, 9).unwrap().kind, Kind::Power);
        for p in [(A, 10), (H, 10), (B, 14), (F, 4), (F, 6), (C, 7), (H, 11), (D, 12)] {
            assert_eq!(rule(p.0, p.1).unwrap().kind, Kind::Rf);
            assert!(rule(p.0, p.1).unwrap().is_safety());
        }
    }

    #[test]
    fn each_pin_once() {
        for (i, a) in SAFE_STATE.iter().enumerate() {
            assert!(a.pin < 16);
            for b in &SAFE_STATE[i + 1..] {
                assert!(!(a.port == b.port && a.pin == b.pin), "P{:?}{} twice", a.port, a.pin);
            }
        }
    }

    /// GPIO registers of one port.
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Regs {
        moder: u32,
        otyper: u32,
        pupdr: u32,
        odr: u32,
    }

    fn field(v: u32, pin: u8, bits: u32) -> u32 {
        (v >> (bits * pin as u32)) & ((1 << bits) - 1)
    }

    #[test]
    fn reset_plan_gives_the_table_and_touches_nothing_else() {
        // Arbitrary start values (whatever the vendor bootloader left), several patterns.
        for seed in [0u32, 0xFFFF_FFFF, 0xA5A5_5A5A, 0x1234_5678, 0xDEAD_BEEF] {
            let mut regs = [Regs { moder: seed, otyper: seed.rotate_left(7), pupdr: seed.rotate_left(13), odr: seed.rotate_left(3) }; 8];
            let before = regs;
            let mut order = vec![];
            for st in &RESET_PLAN[..RESET_PORTS] {
                let g = &mut regs[st.port as usize];
                order.push(st.port);
                // BSRR: set bits win over reset bits (RM0468); the plan never sets both.
                assert_eq!(st.bsrr & (st.bsrr >> 16) & 0xFFFF, 0);
                g.odr = (g.odr | (st.bsrr & 0xFFFF)) & !(st.bsrr >> 16);
                g.otyper &= !st.otyper_clear;
                g.pupdr = (g.pupdr & !st.pupdr_clear) | st.pupdr_set;
                // Before MODER changes: every pin that becomes an output already has its level.
                g.moder = (g.moder & !st.moder_clear) | st.moder_set;
            }
            assert_eq!(&order[..3], &[A, H, B], "output enables, then the PA DAC reset");
            for p in [A, B, C, D, E, F, G, H] {
                let (g, b) = (regs[p as usize], before[p as usize]);
                for pin in 0..16u8 {
                    match rule(p, pin).map(|r| r.action) {
                        Some(DriveHigh) | Some(DriveLow) => {
                            let high = rule(p, pin).unwrap().action == DriveHigh;
                            assert_eq!(field(g.moder, pin, 2), 1, "P{p:?}{pin} output");
                            assert_eq!(field(g.otyper, pin, 1), 0, "P{p:?}{pin} push-pull");
                            assert_eq!(field(g.odr, pin, 1), high as u32, "P{p:?}{pin} level");
                            assert_eq!(field(g.pupdr, pin, 2), field(b.pupdr, pin, 2));
                        }
                        Some(InputPullUp) => {
                            assert_eq!(field(g.moder, pin, 2), 0, "P{p:?}{pin} input");
                            assert_eq!(field(g.pupdr, pin, 2), 1, "P{p:?}{pin} pull-up");
                            assert_eq!(field(g.odr, pin, 1), 1, "P{p:?}{pin} output register high");
                            assert_eq!(field(g.otyper, pin, 1), field(b.otyper, pin, 1));
                        }
                        Some(LeaveUntouched) | None => {
                            for (x, y, bits) in [(g.moder, b.moder, 2), (g.pupdr, b.pupdr, 2), (g.otyper, b.otyper, 1), (g.odr, b.odr, 1)] {
                                assert_eq!(field(x, pin, bits), field(y, pin, bits), "P{p:?}{pin} touched");
                            }
                        }
                    }
                }
            }
        }
        // Port E (FPGA bus) has no step at all.
        assert!(!RESET_PLAN[..RESET_PORTS].iter().any(|s| s.port == E));
    }
}
