// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! What runs while the radio idles (emulator time.advance; power): each task counts its wakes and
//! the time it is busy after each; once a second the main task turns the counts into the rates of
//! the last whole second, read with GET_STATS (wakes_*_per_s, busy_*_us_per_s). Interrupts that
//! wake the core without waking a task (DMA, EXTI with no waiter) are not counted.

use core::sync::atomic::{AtomicU32, Ordering::Relaxed};

/// The tasks counted.
#[derive(Clone, Copy)]
pub enum Task {
    Net = 0,
    TxPath = 1,
    Watchdog = 2,
    Console = 3,
    Main = 4,
}
pub const TASKS: usize = 5;

static WAKES: [AtomicU32; TASKS] = [const { AtomicU32::new(0) }; TASKS];
static BUSY_US: [AtomicU32; TASKS] = [const { AtomicU32::new(0) }; TASKS];
/// The last whole second: wakes and busy microseconds per task.
static LAST_WAKES: [AtomicU32; TASKS] = [const { AtomicU32::new(0) }; TASKS];
static LAST_BUSY_US: [AtomicU32; TASKS] = [const { AtomicU32::new(0) }; TASKS];

/// A task woke and was busy `busy_us` until its next wait.
pub fn note(t: Task, busy_us: u64) {
    WAKES[t as usize].fetch_add(1, Relaxed);
    BUSY_US[t as usize].fetch_add(busy_us.min(u32::MAX as u64) as u32, Relaxed);
}

/// Once a second (the main task): the counts of the second that ended become the rates.
pub fn roll() {
    for i in 0..TASKS {
        LAST_WAKES[i].store(WAKES[i].swap(0, Relaxed), Relaxed);
        LAST_BUSY_US[i].store(BUSY_US[i].swap(0, Relaxed), Relaxed);
    }
}

/// The rates of the last whole second: (wakes, busy us) per task.
pub fn rates() -> [(u32, u32); TASKS] {
    core::array::from_fn(|i| (LAST_WAKES[i].load(Relaxed), LAST_BUSY_US[i].load(Relaxed)))
}
