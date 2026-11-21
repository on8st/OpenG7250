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
    r(D, 5, LeaveUntouched, "USART2 TX to the controller: idle or high-impedance, never low (HIS 70)"),
    r(H, 6, LeaveUntouched, "PTT input, not used in v1.0 (0030)"),
    r(H, 3, LeaveUntouched, "key contact, not used in v1.0 (0030)"),
    r(H, 4, LeaveUntouched, "key contact, not used in v1.0 (0030)"),
    r(H, 5, LeaveUntouched, "power key, ignored in v1.0 (0029 item 2)"),
];

/// The rule for a pin, if the safe state lists it.
pub fn rule(port: Port, pin: u8) -> Option<&'static PinRule> {
    SAFE_STATE.iter().find(|r| r.port == port && r.pin == pin)
}

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
    fn power_hold_high_0029() {
        assert_eq!(act(H, 9), DriveHigh);
    }

    #[test]
    fn never_driven() {
        for (p, n) in [(A, 4), (A, 12), (D, 5), (H, 6), (H, 3), (H, 4), (H, 5)] {
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
}
