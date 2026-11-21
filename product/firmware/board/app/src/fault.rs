// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The radio's fault latch (A3.5; g7250_safety::fault), shared by the tasks that raise a fault
//! (the watchdog task's safe-state check, the net loop after flash work) and read by P2 for the
//! registers, the arm and the key path. A class 2 fault also stops a trial from confirming
//! itself (0044).

use core::cell::RefCell;
use cortex_m::interrupt::Mutex;
use g7250_safety::fault::{Faults, class_of};

static LATCH: Mutex<RefCell<Faults>> = Mutex::new(RefCell::new(Faults::new()));
/// Wakes the net loop on a new entry, so that the registers are pushed at once.
pub static WAKE: embassy_sync::signal::Signal<embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex, ()> = embassy_sync::signal::Signal::new();
/// Codes raised and not yet logged (bit per code), for the net loop's event log.
static UNLOGGED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Raise `code`. Returns true for a new entry.
pub fn raise(code: u8) -> bool {
    let new = cortex_m::interrupt::free(|cs| LATCH.borrow(cs).borrow_mut().raise(code));
    if class_of(code) == 2 {
        crate::update::set_class2_fault();
    }
    if new {
        UNLOGGED.fetch_or(1 << code, core::sync::atomic::Ordering::Relaxed);
        WAKE.signal(());
        g7250_board::println!("app: FAULT code {} (class {}) entered", code, class_of(code));
    }
    new
}

/// The latch now.
pub fn now() -> Faults {
    cortex_m::interrupt::free(|cs| *LATCH.borrow(cs).borrow())
}

/// Codes entered since the last call (bit per code), to log.
pub fn take_unlogged() -> u32 {
    UNLOGGED.swap(0, core::sync::atomic::Ordering::Relaxed)
}
