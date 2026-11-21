// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Applies the safe-state table of `g7250-safety` with raw GPIO writes.

use crate::pac;
use crate::pac::gpio::vals::{Moder, Ot, Ospeedr, Pupdr};
use g7250_safety::safe_state::{Action, Port, SAFE_STATE};

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

/// Apply the whole safe state, in table order.
pub fn apply() {
    enable_gpio_clocks();
    for r in SAFE_STATE {
        match r.action {
            Action::DriveHigh => drive(r.port, r.pin, true),
            Action::DriveLow => drive(r.port, r.pin, false),
            Action::LeaveUntouched => {}
        }
    }
}

/// Re-apply the safe state proper (RF and power pins) without touching the parts that drivers
/// have taken out of reset (CH394, codec). Used whenever not transmitting.
pub fn reassert() {
    for r in SAFE_STATE.iter().filter(|r| r.is_safety()) {
        match r.action {
            Action::DriveHigh => drive(r.port, r.pin, true),
            Action::DriveLow => drive(r.port, r.pin, false),
            Action::LeaveUntouched => {}
        }
    }
}

/// Self-check: every driven RF and power pin of the table is an output at its safe level. Used
/// at start-up and periodically; a failure is a class 2 fault (0007 sub-decision 5).
pub fn holds() -> bool {
    SAFE_STATE.iter().filter(|r| r.is_safety()).all(|r| {
        let g = gpio(r.port);
        let n = r.pin as usize;
        let out = g.moder().read().moder(n) == Moder::OUTPUT;
        let level = g.odr().read().odr(n) == pac::gpio::vals::Odr::HIGH;
        match r.action {
            Action::DriveHigh => out && level,
            Action::DriveLow => out && !level,
            Action::LeaveUntouched => true,
        }
    })
}
