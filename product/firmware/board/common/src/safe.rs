// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Applies the safe-state table of `g7250-safety` with raw GPIO writes.

use crate::pac;
use crate::pac::gpio::vals::{Moder, Ot, Ospeedr, Pupdr};
use g7250_safety::safe_state::{Action, Port, RESET_PLAN, RESET_PORTS, SAFE_STATE};

fn gpio(p: Port) -> pac::gpio::Gpio {
    match p {
        Port::A => pac::GPIOA,
        Port::B => pac::GPIOB,
        Port::C => pac::GPIOC,
        Port::D => pac::GPIOD,
        Port::E => pac::GPIOE,
        Port::F => pac::GPIOF,
        Port::G => pac::GPIOG,
        Port::H => pac::GPIOH,
    }
}

fn enable_gpio_clocks() {
    pac::RCC.ahb4enr().modify(|w| {
        w.set_gpioaen(true);
        w.set_gpioben(true);
        w.set_gpiocen(true);
        w.set_gpioden(true);
        w.set_gpioeen(true);
        w.set_gpiofen(true);
        w.set_gpiogen(true);
        w.set_gpiohen(true);
    });
    // Read back so the clock is running before the first GPIO access.
    let _ = pac::RCC.ahb4enr().read();
}

/// Drive one pin as a push-pull output. The level is written first, so the pin never shows
/// the opposite level when it becomes an output.
pub fn drive(port: Port, pin: u8, high: bool) {
    let g = gpio(port);
    let n = pin as usize;
    g.bsrr().write(|w| if high { w.set_bs(n, true) } else { w.set_br(n, true) });
    g.otyper().modify(|w| w.set_ot(n, Ot::PUSH_PULL));
    g.pupdr().modify(|w| w.set_pupdr(n, Pupdr::FLOATING));
    g.ospeedr().modify(|w| w.set_ospeedr(n, Ospeedr::LOW_SPEED));
    g.moder().modify(|w| w.set_moder(n, Moder::OUTPUT));
}

/// Input with the internal pull-up. The pull-up is set first, while the pin is still in its
/// reset (analog) mode, so the pin is never a floating input and is never driven.
pub fn input_pull_up(port: Port, pin: u8) {
    let g = gpio(port);
    let n = pin as usize;
    // Output register high as well: should anything ever switch the pin to an output, it drives
    // the idle level, not low.
    g.bsrr().write(|w| w.set_bs(n, true));
    g.pupdr().modify(|w| w.set_pupdr(n, Pupdr::PULL_UP));
    g.moder().modify(|w| w.set_moder(n, Moder::INPUT));
}

/// The safe state at reset (0045; A3.6): called from the boot stage's `__pre_init`, before the
/// RAM is initialised and before any clock, peripheral or console set-up. GPIO clocks on, then
/// one step per port from [`RESET_PLAN`], unrolled, so the code is straight-line with constant
/// masks: about 17 register reads and 23 writes. It uses only the stack and flash constants (no
/// static: `.data` and `.bss` are not set up yet). `board/check-safe-margin.py` checks the
/// bound of 20 us on the built boot stage. [`apply`] follows in `early_init` and sets the rest.
/// Inlined into `__pre_init`, so that the path has no call frame of its own.
#[inline(always)]
pub fn apply_at_reset() {
    // GPIOA-GPIOH clock enables, AHB4ENR bits 0-7 (RM0468); read back before the first access.
    pac::RCC.ahb4enr().modify(|w| w.0 |= 0xFF);
    let _ = pac::RCC.ahb4enr().read();
    reset_step::<0>();
    reset_step::<1>();
    reset_step::<2>();
    reset_step::<3>();
    reset_step::<4>();
    reset_step::<5>();
    reset_step::<6>();
    reset_step::<7>();
}

#[inline(always)]
fn reset_step<const I: usize>() {
    if I >= RESET_PORTS {
        return;
    }
    let s = RESET_PLAN[I];
    let g = gpio(s.port);
    g.bsrr().write_value(pac::gpio::regs::Bsrr(s.bsrr));
    if s.otyper_clear != 0 {
        g.otyper().modify(|w| w.0 &= !s.otyper_clear);
    }
    if s.pupdr_clear != 0 {
        g.pupdr().modify(|w| w.0 = (w.0 & !s.pupdr_clear) | s.pupdr_set);
    }
    g.moder().modify(|w| w.0 = (w.0 & !s.moder_clear) | s.moder_set);
}

/// Apply the whole safe state, in table order.
pub fn apply() {
    enable_gpio_clocks();
    for r in SAFE_STATE {
        match r.action {
            Action::DriveHigh => drive(r.port, r.pin, true),
            Action::DriveLow => drive(r.port, r.pin, false),
            Action::InputPullUp => input_pull_up(r.port, r.pin),
            Action::LeaveUntouched => {}
        }
    }
}

/// The safe state without the TX path check's pins (Table A3-6): what the start-up paths after
/// the boot stage's reset-time set-up (0045) apply. Between that set-up and the first TX path
/// check no code writes those pins or their configuration (A3.6b; 0066 addendum), so that a pin
/// found wrong is seen by the check, not quietly put back.
pub fn apply_except_tx_path() {
    enable_gpio_clocks();
    for r in SAFE_STATE.iter().filter(|r| !is_tx_path(r.port, r.pin)) {
        match r.action {
            Action::DriveHigh => drive(r.port, r.pin, true),
            Action::DriveLow => drive(r.port, r.pin, false),
            Action::InputPullUp => input_pull_up(r.port, r.pin),
            Action::LeaveUntouched => {}
        }
    }
}

/// As [`reassert`], without the TX path check's pins (start-up, see [`apply_except_tx_path`]).
pub fn reassert_except_tx_path() {
    for r in SAFE_STATE.iter().filter(|r| r.is_safety() && !is_tx_path(r.port, r.pin)) {
        match r.action {
            Action::DriveHigh => drive(r.port, r.pin, true),
            Action::DriveLow => drive(r.port, r.pin, false),
            Action::InputPullUp => input_pull_up(r.port, r.pin),
            Action::LeaveUntouched => {}
        }
    }
}

fn is_tx_path(port: Port, pin: u8) -> bool {
    g7250_safety::txpath::PINS.iter().any(|p| p.0 == port && p.1 == pin)
}

/// Start the DWT cycle counter at 0: the boot stage's first step, so that the application can
/// measure the time from the boot stage's start to its first TX path check (A3.6b: at most
/// 100 ms; 0066 addendum). SYSCLK is HSI 64 MHz in both stages.
pub fn start_cycle_count() {
    use core::ptr::{read_volatile as rd, write_volatile as wr};
    // SAFETY: core debug registers (ARMv7-M DEMCR, DWT), always present on the Cortex-M7.
    unsafe {
        wr(0xE000_EDFC as *mut u32, rd(0xE000_EDFC as *const u32) | (1 << 24));
        wr(0xE000_1FB0 as *mut u32, 0xC5AC_CE55);
        wr(0xE000_1004 as *mut u32, 0);
        wr(0xE000_1000 as *mut u32, rd(0xE000_1000 as *const u32) | 1);
    }
}

/// Microseconds since [`start_cycle_count`] (wraps after 67 s).
pub fn us_since_boot_stage() -> u32 {
    // SAFETY: a read of DWT CYCCNT.
    unsafe { core::ptr::read_volatile(0xE000_1004 as *const u32) / (crate::console::HSI_HZ / 1_000_000) }
}

/// Re-apply the safe state proper (RF and power pins) without touching the parts that drivers
/// have taken out of reset (CH394, codec). Used whenever not transmitting.
pub fn reassert() {
    for r in SAFE_STATE.iter().filter(|r| r.is_safety()) {
        match r.action {
            Action::DriveHigh => drive(r.port, r.pin, true),
            Action::DriveLow => drive(r.port, r.pin, false),
            Action::InputPullUp => input_pull_up(r.port, r.pin),
            Action::LeaveUntouched => {}
        }
    }
}

/// The TX path check's reading (A3.6b; g7250_safety::txpath): the first TX-side pin whose MCU
/// configuration (push-pull output, output register at the safe level) or input register (the
/// level at the pin) differs from the safe state.
pub fn tx_path_mismatch() -> Option<g7250_safety::txpath::Mismatch> {
    use g7250_safety::txpath::{Comparison, Mismatch, PINS, bias_mismatch};
    // PA bias channels 2, 7, 8: the value last written (no read-back; Table A3-6).
    let last = [0, 1, 2].map(|i| DAC_BIAS_LAST[i].load(core::sync::atomic::Ordering::Relaxed));
    if let Some(ch) = bias_mismatch(last) {
        let (port, pin, _) = PINS[5]; // the DAC's RESET pin, PB14
        return Some(Mismatch { port, pin, what: Comparison::DacValue(ch) });
    }
    for (port, pin, high) in PINS {
        let g = gpio(port);
        let n = pin as usize;
        let config = g.moder().read().moder(n) == Moder::OUTPUT
            && g.otyper().read().ot(n) == Ot::PUSH_PULL
            && (g.odr().read().odr(n) == pac::gpio::vals::Odr::HIGH) == high;
        if !config {
            return Some(Mismatch { port, pin, what: Comparison::Configuration });
        }
        if (g.idr().read().idr(n) == pac::gpio::vals::Idr::HIGH) != high {
            return Some(Mismatch { port, pin, what: Comparison::PinLevel });
        }
    }
    None
}

/// The values last written to PA DAC channels 2, 7, 8 (g7250_safety::txpath::BIAS_CHANNELS).
/// This firmware has no DAC driver and never writes the DAC (RESET held, all outputs at VrefL),
/// so they stay 0; a DAC driver stores every value it writes here.
pub static DAC_BIAS_LAST: [core::sync::atomic::AtomicU16; 3] = [const { core::sync::atomic::AtomicU16::new(0) }; 3];

/// The safety pins other than the TX-side ones of the TX path check (A3.6b has its own time
/// filter for those): output at the safe level, or input with pull-up.
pub fn holds_except_tx_path() -> bool {
    SAFE_STATE
        .iter()
        .filter(|r| r.is_safety() && !g7250_safety::txpath::PINS.iter().any(|p| p.0 == r.port && p.1 == r.pin))
        .all(rule_holds)
}

/// Self-check: every driven RF and power pin of the table is an output at its safe level. Used
/// at start-up and periodically; a failure is a class 2 fault (0007 sub-decision 5).
pub fn holds() -> bool {
    SAFE_STATE.iter().filter(|r| r.is_safety()).all(rule_holds)
}

fn rule_holds(r: &g7250_safety::safe_state::PinRule) -> bool {
    {
        let g = gpio(r.port);
        let n = r.pin as usize;
        let out = g.moder().read().moder(n) == Moder::OUTPUT;
        let level = g.odr().read().odr(n) == pac::gpio::vals::Odr::HIGH;
        match r.action {
            Action::DriveHigh => out && level,
            Action::DriveLow => out && !level,
            Action::InputPullUp => g.moder().read().moder(n) == Moder::INPUT && g.pupdr().read().pupdr(n) == Pupdr::PULL_UP,
            Action::LeaveUntouched => true,
        }
    }
}
