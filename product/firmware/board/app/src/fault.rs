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

/// TX is armed or keyed (set by P2 every pass): the TX path check is not made (A3.6b).
pub static TX_OPEN: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
/// The mismatch that raised TX_PATH_NOT_BLOCKED, for the log (A3.6b: the pin and the comparison).
static DETAIL: Mutex<RefCell<Option<g7250_safety::txpath::Mismatch>>> = Mutex::new(RefCell::new(None));

pub fn set_detail(m: g7250_safety::txpath::Mismatch) {
    cortex_m::interrupt::free(|cs| {
        let mut d = DETAIL.borrow(cs).borrow_mut();
        if d.is_none() {
            *d = Some(m);
        }
    });
}

pub fn detail() -> Option<g7250_safety::txpath::Mismatch> {
    cortex_m::interrupt::free(|cs| *DETAIL.borrow(cs).borrow())
}

/// Raise `code`. Returns true for a new entry. A class 1 code's new entry is counted in the
/// persist block (A3.5 "Counters": since power-on, over resets); its third is escalated.
pub fn raise(code: u8) -> bool {
    let now = embassy_time::Instant::now().as_millis();
    let new = cortex_m::interrupt::free(|cs| {
        let mut l = LATCH.borrow(cs).borrow_mut();
        if l.flags() & (1 << code) != 0 {
            return false;
        }
        if class_of(code) == 1 {
            let mut c = g7250_board::persist::get();
            let n = c.count_entry(code).unwrap_or(1);
            g7250_board::persist::set(&c);
            l.raise_counted(code, now, n)
        } else {
            l.raise_at(code, now)
        }
    });
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

/// `FAULT_CLEAR` accepted by the register map (A3.5 "Clearing"): the latch takes the same rule
/// again (the same value and time) and clears. A clear never re-arms.
pub fn clear(v: u32, now_ms: u64, gone: impl Fn(u8) -> bool) -> bool {
    cortex_m::interrupt::free(|cs| LATCH.borrow(cs).borrow_mut().clear(v, now_ms, gone).is_ok())
}

static CLEARED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// A clear of `code` happened, for the log.
pub fn note_cleared(code: u32) {
    CLEARED.store(code | 0x100, core::sync::atomic::Ordering::Relaxed);
}

/// The code cleared since the last call, if any.
pub fn take_cleared() -> Option<u32> {
    match CLEARED.swap(0, core::sync::atomic::Ordering::Relaxed) {
        0 => None,
        v => Some(v & 0xFF),
    }
}

/// The latch now.
pub fn now() -> Faults {
    cortex_m::interrupt::free(|cs| *LATCH.borrow(cs).borrow())
}

/// Codes entered since the last call (bit per code), to log.
pub fn take_unlogged() -> u32 {
    UNLOGGED.swap(0, core::sync::atomic::Ordering::Relaxed)
}
