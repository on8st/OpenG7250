// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Raw flash driver for the Main's single bank (RM0468, FLASH bank 1 registers), shared by the
//! boot stage and the application, without the HAL.
//!
//! Every program and erase passes [`FlashGuard::check`] for the writer first, so sector 0 is out
//! of reach for every writer ([HIS] HIS 15 rule 1) and each writer reaches only its own regions
//! (the boot stage the store, the application the store and the other slot). The option-byte
//! registers are never touched (HIS 15 rule 2).
//!
//! Erase and program run from RAM with interrupts masked: the CPU cannot fetch from the bank
//! while it is erased or programmed (vector table and handlers are in it), and a sector erase can
//! outlast the 2 s IWDG, so the routines refresh it on every poll. `board/check-ramfunc.sh` checks
//! that they are in RAM and never branch to flash.
//!
//! Reads are fault-tolerant (R0): a flash word left half-programmed or half-erased by a power
//! loss can carry an invalid ECC, and a CPU read of it returns a bus error (RM0468, flash ECC:
//! double error, DBECCERR1). [`read_tolerant`] reads with bus faults ignored (FAULTMASK set and
//! SCB CCR.BFHFNMIGN, ARMv7-M) and reports such a word from FLASH_SR1 and the BusFault status
//! (g7250_safety::tolerant), instead of a fault and a watchdog reset at every start. The store
//! scan skips the word like a torn one; an image with one fails the image check
//! (g7250_update::image).

use g7250_layout::{FLASH_BASE, FLASH_WORD, FlashGuard, Op, SECTOR_SIZE, Writer};
use g7250_safety::tolerant::{self, CFSR_BFSR};
use g7250_store::FlashIo;

const FLASH_KEYR1: u32 = 0x5200_2004;
const FLASH_CR1: u32 = 0x5200_200C;
const FLASH_SR1: u32 = 0x5200_2010;
const FLASH_CCR1: u32 = 0x5200_2014;
const IWDG1_KR: u32 = 0x5800_4800;
/// FLASH_SR1 / FLASH_CCR1: SNECCERR1 bit 25 (single error, corrected), DBECCERR1 bit 26 (double
/// error: the read returned a bus error).
const SR_SNECC: u32 = 1 << 25;
const SR_DBECC: u32 = tolerant::DBECCERR1;
/// SCB CCR.BFHFNMIGN (bit 8): at priority -1 (FAULTMASK) a precise data bus fault is ignored.
const SCB_CCR: u32 = 0xE000_ED14;
const CCR_BFHFNMIGN: u32 = 1 << 8;
/// SCB CFSR; the BusFault status is bits 8-15, cleared by writing ones.
const SCB_CFSR: u32 = 0xE000_ED28;
/// FLASH_CR1: LOCK bit 0, PG bit 1, SER bit 2, START bit 7, SNB bits 10:8.
const CR_LOCK: u32 = 1 << 0;
const CR_PG: u32 = 1 << 1;
const CR_SER: u32 = 1 << 2;
const CR_START: u32 = 1 << 7;
/// FLASH_SR1: BSY bit 0, WBNE bit 1, QW bit 2; error flags bits 17-26.
const SR_BUSY: u32 = (1 << 0) | (1 << 1) | (1 << 2);
const SR_ERRORS: u32 = 0x07EE_0000;
/// Returned instead of SR when the operation did not end within [`LIMIT_MS`].
pub const TIMEOUT: u32 = 1 << 31;
/// Returned when a programmed flash word reads back different from what was written.
pub const READBACK: u32 = 1 << 30;
const FAILED: u32 = SR_ERRORS | TIMEOUT;
/// `t_erase_max`: the maximum 128 KiB sector erase time of the MCU fitted (B2.3; 0065 point 1):
/// [STM32H72x] ST DS13311 Rev 5 (November 2023), section 6.3.13, Table 48 "Flash memory
/// programming", tERASE (sector, 128 KiB) max, guaranteed by characterization: 4 s at
/// parallelism x8, 3.6 s at x16, 2.2 s at x32; no x64 row. This driver never writes FLASH_CR
/// PSIZE (it keeps its reset value), so the bound is the largest row, x8, 4 s, which covers every
/// parallelism. Word programming (266 bits) is at most 580 us (x8, after 10k cycles), far inside
/// the same 2 x t_erase_max limit.
pub const T_ERASE_MAX_MS: u32 = 4_000;
/// An erase or program that does not end within 2 x `t_erase_max` has failed (B2.3 "flash work
/// that fails"; 0065 point 2).
pub const LIMIT_MS: u32 = 2 * T_ERASE_MAX_MS;
/// The same limit in core cycles (SYSCLK is HSI 64 MHz, no PLL); measured with the DWT cycle
/// counter, which wraps after 67 s, far above the limit.
const LIMIT_CYCLES: u32 = LIMIT_MS * (crate::console::CORE_HZ / 1000);
const _: () = assert!((LIMIT_MS as u64) * (crate::console::CORE_HZ as u64 / 1000) < u32::MAX as u64 / 2);
/// The TX path check's 10 ms in cycles (A3.6b).
const HOLD_CYCLES: u32 = g7250_safety::txpath::HOLD_MS as u32 * (crate::console::CORE_HZ / 1000);
/// DWT cycle counter (ARMv7-M): DEMCR.TRCENA (bit 24), DWT_CTRL.CYCCNTENA (bit 0), CYCCNT, and
/// the Cortex-M7 DWT lock access register.
const DEMCR: u32 = 0xE000_EDFC;
const DWT_CTRL: u32 = 0xE000_1000;
const DWT_CYCCNT: u32 = 0xE000_1004;
const DWT_LAR: u32 = 0xE000_1FB0;

/// A flash operation failed (error flag or timeout) since the last [`take_failure`]; the caller
/// raises `INTERNAL_ERROR` and logs it (0065 point 2).
static FAILURE: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// The final FLASH_SR1 (or [`TIMEOUT`]) of a failed operation since the last call, if any.
pub fn take_failure() -> Option<u32> {
    match FAILURE.swap(0, core::sync::atomic::Ordering::Relaxed) {
        0 => None,
        sr => Some(sr),
    }
}

fn note_failure(sr: u32) {
    FAILURE.store(sr, core::sync::atomic::Ordering::Relaxed); // never 0: an error flag or TIMEOUT
}

/// The Table A3-6 pins for the wait loop's samples (A3.6b, 0066 addendum (a)), in RAM: the
/// loop runs from RAM while the flash is busy and must not read flash (the pin table is there).
#[derive(Clone, Copy)]
struct RamPins {
    base: [u32; 6],
    pin: [u32; 6],
    high: [bool; 6],
    /// No samples while TX is armed or keyed (the path is meant to be open).
    on: bool,
}

fn ram_pins() -> RamPins {
    let mut p = RamPins { base: [0; 6], pin: [0; 6], high: [false; 6], on: !crate::safe::TX_OPEN.load(core::sync::atomic::Ordering::Relaxed) };
    for (k, (port, pin, high)) in g7250_safety::txpath::PINS.iter().enumerate() {
        p.base[k] = crate::safe::gpio_base(*port);
        p.pin[k] = *pin as u32;
        p.high[k] = *high;
    }
    p
}

/// The samples of the last flash operation, for the TX path check (taken by
/// [`take_samples`]): a mismatch held for 10 ms (pin index, bit 7 = the configuration, else the
/// pin level), and a run still open at the end (cycles before the end, index).
static SAMPLES: cortex_m::interrupt::Mutex<core::cell::Cell<Option<(Option<u8>, Option<(u32, u8)>)>>> = cortex_m::interrupt::Mutex::new(core::cell::Cell::new(None));
/// Called when a flash operation ends (the application wakes its TX path check); 0 = none.
static ON_END: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// Called around the masked part of every erase and program with the operation to run (the
/// application keeps its time over it, timekeep.rs); 0 = none (the boot stage has no time driver).
static AROUND: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// Register the hook around the masked part; it must call its argument exactly once.
pub fn around_operation(f: fn(&mut dyn FnMut())) {
    AROUND.store(f as usize, core::sync::atomic::Ordering::Relaxed);
}

/// Run one operation with interrupts masked; the hook runs masked too, from flash, before the
/// operation starts and after it has ended or failed.
fn masked(op: impl Fn(&mut g7250_safety::txpath::Sampler) -> u32, smp: &mut g7250_safety::txpath::Sampler) -> u32 {
    let mut sr = TIMEOUT;
    cortex_m::interrupt::free(|_| {
        let mut run = || sr = op(smp);
        match AROUND.load(core::sync::atomic::Ordering::Relaxed) {
            0 => run(),
            f => {
                // SAFETY: only `around_operation` stores here, always this type.
                let f: fn(&mut dyn FnMut()) = unsafe { core::mem::transmute::<usize, fn(&mut dyn FnMut())>(f) };
                f(&mut run)
            }
        }
    });
    sr
}

/// Register the hook called after every erase or program.
pub fn on_operation_end(f: fn()) {
    ON_END.store(f as usize, core::sync::atomic::Ordering::Relaxed);
}

/// The wait-loop samples of the flash operations since the last call, if any.
pub fn take_samples() -> Option<(Option<u8>, Option<(u32, u8)>)> {
    cortex_m::interrupt::free(|cs| SAMPLES.borrow(cs).take())
}

fn operation_ended(s: &g7250_safety::txpath::Sampler) {
    let end = cycles_now();
    let r = (s.held, s.open_at(end));
    cortex_m::interrupt::free(|cs| {
        let c = SAMPLES.borrow(cs);
        // Keep the worst of several operations since the last take.
        let merged = match c.get() {
            Some((h, o)) => (h.or(r.0), r.1.or(o)),
            None => r,
        };
        c.set(Some(merged));
    });
    let f = ON_END.load(core::sync::atomic::Ordering::Relaxed);
    if f != 0 {
        // SAFETY: only `on_operation_end` stores here, always a `fn()`.
        let f: fn() = unsafe { core::mem::transmute::<usize, fn()>(f) };
        f();
    }
}

/// The DWT cycle counter (running from the first flash operation on, [`cycle_counter_on`]).
pub fn cycles_now() -> u32 {
    // SAFETY: a read of DWT CYCCNT.
    unsafe { core::ptr::read_volatile(DWT_CYCCNT as *const u32) }
}

/// One sample of the pins in the wait loop: the index of the first mismatch, bit 7 set when it
/// is the configuration (mode, output type, output register), clear for the pin level (input
/// register). Register reads only (RM0468 GPIO: MODER 0x00, OTYPER 0x04, IDR 0x10, ODR 0x14).
#[inline(always)]
unsafe fn sample_pins(p: &RamPins) -> Option<u8> {
    use core::ptr::read_volatile as rd;
    let mut k = 0;
    while k < 6 {
        let b = p.base[k];
        let n = p.pin[k];
        // SAFETY: GPIO registers of the ports of Table A3-6 (clocked since the safe state).
        let (moder, otyper, idr, odr) = unsafe { (rd(b as *const u32), rd((b + 0x04) as *const u32), rd((b + 0x10) as *const u32), rd((b + 0x14) as *const u32)) };
        let out = (moder >> (2 * n)) & 3 == 1 && (otyper >> n) & 1 == 0 && ((odr >> n) & 1 == 1) == p.high[k];
        if !out {
            return Some(k as u8 | 0x80);
        }
        if ((idr >> n) & 1 == 1) != p.high[k] {
            return Some(k as u8);
        }
        k += 1;
    }
    None
}

/// Start the DWT cycle counter (idempotent).
fn cycle_counter_on() {
    use core::ptr::{read_volatile as rd, write_volatile as wr};
    // SAFETY: core debug registers, always present on the Cortex-M7; only the counter is enabled.
    unsafe {
        wr(DEMCR as *mut u32, rd(DEMCR as *const u32) | (1 << 24));
        wr(DWT_LAR as *mut u32, 0xC5AC_CE55);
        wr(DWT_CTRL as *mut u32, rd(DWT_CTRL as *const u32) | 1);
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum FlashError {
    /// The guard refused the operation.
    Guard,
    /// Final FLASH_SR1 of a failed operation, or [`TIMEOUT`].
    Hw(u32),
    /// A read met a flash word with a double ECC error (unreadable; for example torn by a power
    /// loss while it was programmed or its sector erased).
    Ecc,
}

/// Read `buf.len()` bytes of flash at `addr` without faulting on an unreadable word: `Err(())` if
/// any word read had a double ECC error or the read met a bus fault (the bytes are not valid).
/// Interrupts (and every fault but NMI) are held off for the copy only.
/// One instance (never inlined): `board/check-faultmask.py` checks that nothing between `cpsid f`
/// and `cpsie f` calls or branches out of it.
#[inline(never)]
pub fn read_tolerant(addr: u32, buf: &mut [u8]) -> Result<(), ()> {
    use core::ptr::{read_volatile as rd, write_volatile as wr};
    // SAFETY: on-chip flash, always mapped; the SCB and FLASH registers are the core's and the
    // flash interface's. FAULTMASK is set only around the copy and restored before returning.
    unsafe {
        // Both statuses cleared first, so that what they say afterwards is of this read.
        wr(FLASH_CCR1 as *mut u32, SR_DBECC | SR_SNECC);
        wr(SCB_CFSR as *mut u32, CFSR_BFSR);
        core::arch::asm!("cpsid f", options(nostack, preserves_flags));
        let ccr = rd(SCB_CCR as *const u32);
        wr(SCB_CCR as *mut u32, ccr | CCR_BFHFNMIGN);
        cortex_m::asm::dsb();
        cortex_m::asm::isb();
        // Pointer steps, not checked integer adds: no panic path while FAULTMASK is set.
        let src = addr as *const u8;
        for (i, b) in buf.iter_mut().enumerate() {
            *b = rd(src.add(i));
        }
        cortex_m::asm::dsb();
        // Both judged and cleared before anything else runs: an ignored fault's status must not
        // be left for a later real fault, nor a later read's taken for this one's.
        let sr1 = rd(FLASH_SR1 as *const u32);
        let cfsr = rd(SCB_CFSR as *const u32);
        wr(FLASH_CCR1 as *mut u32, SR_DBECC | SR_SNECC);
        wr(SCB_CFSR as *mut u32, CFSR_BFSR);
        wr(SCB_CCR as *mut u32, ccr);
        cortex_m::asm::dsb();
        cortex_m::asm::isb();
        core::arch::asm!("cpsie f", options(nostack, preserves_flags));
        if tolerant::flash_unreadable(sr1, cfsr) {
            return Err(());
        }
    }
    Ok(())
}

/// Flash access for one writer.
pub struct RawFlash {
    pub writer: Writer,
}

impl RawFlash {
    pub const fn new(writer: Writer) -> Self {
        RawFlash { writer }
    }
}

#[inline(always)]
unsafe fn unlock_and_clear() {
    use core::ptr::{read_volatile as rd, write_volatile as wr};
    unsafe {
        if rd(FLASH_CR1 as *const u32) & CR_LOCK != 0 {
            wr(FLASH_KEYR1 as *mut u32, 0x4567_0123);
            wr(FLASH_KEYR1 as *mut u32, 0xCDEF_89AB);
        }
        wr(FLASH_CCR1 as *mut u32, SR_ERRORS | (1 << 16));
    }
}

#[inline(always)]
unsafe fn wait_and_lock(clear_bits: u32, pins: &RamPins, s: &mut g7250_safety::txpath::Sampler) -> u32 {
    use core::ptr::{read_volatile as rd, write_volatile as wr};
    unsafe {
        let mut sr = rd(FLASH_SR1 as *const u32);
        let t0 = rd(DWT_CYCCNT as *const u32);
        while sr & SR_BUSY != 0 {
            wr(IWDG1_KR as *mut u32, 0xAAAA);
            // A3.6b during flash work (0066 addendum (a)): a sample where the IWDG is fed.
            if pins.on {
                s.sample(rd(DWT_CYCCNT as *const u32), sample_pins(pins), HOLD_CYCLES);
            }
            if rd(DWT_CYCCNT as *const u32).wrapping_sub(t0) > LIMIT_CYCLES {
                sr = TIMEOUT;
                break;
            }
            sr = rd(FLASH_SR1 as *const u32);
        }
        wr(IWDG1_KR as *mut u32, 0xAAAA);
        let cr = rd(FLASH_CR1 as *const u32) & !clear_bits;
        wr(FLASH_CR1 as *mut u32, cr | CR_LOCK);
        wr(FLASH_CCR1 as *mut u32, SR_ERRORS | (1 << 16));
        sr
    }
}

/// Erase sector `snb` (1-7) and wait, from RAM.
#[unsafe(link_section = ".data.g7250_ramfunc")]
#[inline(never)]
unsafe fn flash_erase_from_ram(snb: u32, pins: &RamPins, s: &mut g7250_safety::txpath::Sampler) -> u32 {
    use core::ptr::{read_volatile as rd, write_volatile as wr};
    unsafe {
        unlock_and_clear();
        let cr = rd(FLASH_CR1 as *const u32) & !(0x7 << 8);
        wr(FLASH_CR1 as *mut u32, cr | CR_SER | (snb << 8));
        wr(FLASH_CR1 as *mut u32, cr | CR_SER | (snb << 8) | CR_START);
        wait_and_lock(CR_SER | (0x7 << 8), pins, s)
    }
}

/// Program one 32-byte flash word at `addr` from `words` and wait, from RAM.
#[unsafe(link_section = ".data.g7250_ramfunc")]
#[inline(never)]
unsafe fn flash_program_from_ram(addr: u32, words: &[u32; 8], pins: &RamPins, s: &mut g7250_safety::txpath::Sampler) -> u32 {
    use core::ptr::{read_volatile as rd, write_volatile as wr};
    unsafe {
        unlock_and_clear();
        let cr = rd(FLASH_CR1 as *const u32);
        wr(FLASH_CR1 as *mut u32, cr | CR_PG);
        let mut i = 0;
        while i < 8 {
            wr((addr + 4 * i as u32) as *mut u32, words[i]);
            i += 1;
        }
        cortex_m::asm::dsb();
        wait_and_lock(CR_PG, pins, s)
    }
}

impl FlashIo for RawFlash {
    type Error = FlashError;

    fn read(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), FlashError> {
        crate::safe::boot_poll();
        read_tolerant(addr, buf).map_err(|_| FlashError::Ecc)
    }

    fn unreadable(e: &FlashError) -> bool {
        *e == FlashError::Ecc
    }

    fn program(&mut self, addr: u32, data: &[u8]) -> Result<(), FlashError> {
        FlashGuard::check(self.writer, Op::Program, addr, data.len() as u32).map_err(|_| FlashError::Guard)?;
        for (k, chunk) in data.chunks_exact(FLASH_WORD as usize).enumerate() {
            let mut w = [0u32; 8];
            for (i, b) in chunk.chunks_exact(4).enumerate() {
                w[i] = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            }
            let a = addr + k as u32 * FLASH_WORD;
            cycle_counter_on();
            let pins = ram_pins();
            let mut smp = g7250_safety::txpath::Sampler::default();
            // SAFETY: the guard allowed [addr, addr+len) for this writer; the routine runs from
            // RAM and touches only FLASH bank 1, the target word and IWDG1 (and reads GPIO).
            let sr = masked(|s| unsafe { flash_program_from_ram(a, &w, &pins, s) }, &mut smp);
            operation_ended(&smp);
            if sr & FAILED != 0 {
                note_failure(sr);
                return Err(FlashError::Hw(sr));
            }
            // Read the word back: a program that ended without an error flag but did not land
            // (a word not taken by the flash) is a failure too, so that its block gets 55, not 0
            // (B2.3 "flash work that fails"; A1.8).
            let mut back = [0u8; FLASH_WORD as usize];
            if read_tolerant(a, &mut back).is_err() || back[..] != chunk[..] {
                note_failure(READBACK);
                return Err(FlashError::Hw(READBACK));
            }
        }
        Ok(())
    }

    fn erase_sector(&mut self, addr: u32) -> Result<(), FlashError> {
        FlashGuard::check(self.writer, Op::Erase, addr, SECTOR_SIZE).map_err(|_| FlashError::Guard)?;
        let snb = (addr - FLASH_BASE) / SECTOR_SIZE;
        if snb == 0 || snb > 7 {
            return Err(FlashError::Guard); // never sector 0 (also refused by the guard)
        }
        cycle_counter_on();
        let pins = ram_pins();
        let mut smp = g7250_safety::txpath::Sampler::default();
        // SAFETY: as above, for one whole sector the guard allowed.
        let sr = masked(|s| unsafe { flash_erase_from_ram(snb, &pins, s) }, &mut smp);
        operation_ended(&smp);
        if sr & FAILED != 0 {
            note_failure(sr);
            Err(FlashError::Hw(sr))
        } else {
            Ok(())
        }
    }
}
