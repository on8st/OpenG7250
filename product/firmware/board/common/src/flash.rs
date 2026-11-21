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
//! SCB CCR.BFHFNMIGN, ARMv7-M) and reports such a word from FLASH_SR1, instead of a fault and a
//! watchdog reset at every start. The store scan skips the word like a torn one; a slot with
//! one is not bootable ([`probe`]).

use g7250_layout::{FLASH_BASE, FLASH_WORD, FlashGuard, Op, SECTOR_SIZE, Writer};
use g7250_store::FlashIo;

const FLASH_KEYR1: u32 = 0x5200_2004;
const FLASH_CR1: u32 = 0x5200_200C;
const FLASH_SR1: u32 = 0x5200_2010;
const FLASH_CCR1: u32 = 0x5200_2014;
const IWDG1_KR: u32 = 0x5800_4800;
/// FLASH_SR1 / FLASH_CCR1: SNECCERR1 bit 25 (single error, corrected), DBECCERR1 bit 26 (double
/// error: the read returned a bus error).
const SR_SNECC: u32 = 1 << 25;
const SR_DBECC: u32 = 1 << 26;
/// SCB CCR.BFHFNMIGN (bit 8): at priority -1 (FAULTMASK) a precise data bus fault is ignored.
const SCB_CCR: u32 = 0xE000_ED14;
const CCR_BFHFNMIGN: u32 = 1 << 8;
/// SCB CFSR; the BusFault status is bits 8-15, cleared by writing ones.
const SCB_CFSR: u32 = 0xE000_ED28;
const CFSR_BFSR: u32 = 0x0000_FF00;
/// FLASH_CR1: LOCK bit 0, PG bit 1, SER bit 2, START bit 7, SNB bits 10:8.
const CR_LOCK: u32 = 1 << 0;
const CR_PG: u32 = 1 << 1;
const CR_SER: u32 = 1 << 2;
const CR_START: u32 = 1 << 7;
/// FLASH_SR1: BSY bit 0, WBNE bit 1, QW bit 2; error flags bits 17-26.
const SR_BUSY: u32 = (1 << 0) | (1 << 1) | (1 << 2);
const SR_ERRORS: u32 = 0x07EE_0000;
/// Returned instead of SR when the poll bound ran out.
pub const TIMEOUT: u32 = 1 << 31;
const FAILED: u32 = SR_ERRORS | TIMEOUT;
/// Polls before giving up: far above any datasheet erase maximum at 64 MHz from RAM.
const POLLS: u32 = 400_000_000;

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
/// any word read had a double ECC error (the bytes from it are not valid). Interrupts (and every
/// fault but NMI) are held off for the copy only.
/// One instance (never inlined): `board/check-faultmask.py` checks that nothing between `cpsid f`
/// and `cpsie f` calls or branches out of it.
#[inline(never)]
pub fn read_tolerant(addr: u32, buf: &mut [u8]) -> Result<(), ()> {
    use core::ptr::{read_volatile as rd, write_volatile as wr};
    // SAFETY: on-chip flash, always mapped; the SCB and FLASH registers are the core's and the
    // flash interface's. FAULTMASK is set only around the copy and restored before returning.
    unsafe {
        wr(FLASH_CCR1 as *mut u32, SR_DBECC | SR_SNECC);
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
        wr(SCB_CCR as *mut u32, ccr);
        cortex_m::asm::dsb();
        cortex_m::asm::isb();
        core::arch::asm!("cpsie f", options(nostack, preserves_flags));
        let bad = rd(FLASH_SR1 as *const u32) & SR_DBECC != 0;
        wr(FLASH_CCR1 as *mut u32, SR_DBECC | SR_SNECC);
        if bad {
            // The ignored fault left its status in the BFSR; clear it so that a later real fault
            // is reported alone.
            wr(SCB_CFSR as *mut u32, CFSR_BFSR);
            return Err(());
        }
    }
    Ok(())
}

/// The flash of `r` as a slice, once [`probe`] found every word of it readable; else the address
/// of the first unreadable word. Image checks read through this, never through a raw slice.
pub fn mapped(r: g7250_layout::Range) -> Result<&'static [u8], u32> {
    probe(r.start, r.end)?;
    // SAFETY: on-chip flash, always mapped, and every word of it reads without an ECC error.
    Ok(unsafe { core::slice::from_raw_parts(r.start as *const u8, r.len() as usize) })
}

/// Check that every flash word of `[start, end)` can be read (one read per 32-byte word, which
/// makes the flash check the whole word's ECC). Returns the address of the first unreadable
/// word. A caller validates an image from its mapped slice only after this, so that a torn word
/// in it cannot fault (a slot staged when the power failed).
pub fn probe(start: u32, end: u32) -> Result<(), u32> {
    let mut a = start & !(FLASH_WORD - 1);
    let mut b = [0u8; 1];
    while a < end {
        if read_tolerant(a, &mut b).is_err() {
            return Err(a);
        }
        a += FLASH_WORD;
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
unsafe fn wait_and_lock(clear_bits: u32) -> u32 {
    use core::ptr::{read_volatile as rd, write_volatile as wr};
    unsafe {
        let mut sr = rd(FLASH_SR1 as *const u32);
        let mut n: u32 = 0;
        while sr & SR_BUSY != 0 {
            wr(IWDG1_KR as *mut u32, 0xAAAA);
            n += 1;
            if n > POLLS {
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
unsafe fn flash_erase_from_ram(snb: u32) -> u32 {
    use core::ptr::{read_volatile as rd, write_volatile as wr};
    unsafe {
        unlock_and_clear();
        let cr = rd(FLASH_CR1 as *const u32) & !(0x7 << 8);
        wr(FLASH_CR1 as *mut u32, cr | CR_SER | (snb << 8));
        wr(FLASH_CR1 as *mut u32, cr | CR_SER | (snb << 8) | CR_START);
        wait_and_lock(CR_SER | (0x7 << 8))
    }
}

/// Program one 32-byte flash word at `addr` from `words` and wait, from RAM.
#[unsafe(link_section = ".data.g7250_ramfunc")]
#[inline(never)]
unsafe fn flash_program_from_ram(addr: u32, words: &[u32; 8]) -> u32 {
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
        wait_and_lock(CR_PG)
    }
}

impl FlashIo for RawFlash {
    type Error = FlashError;

    fn read(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), FlashError> {
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
            // SAFETY: the guard allowed [addr, addr+len) for this writer; the routine runs from
            // RAM and touches only FLASH bank 1, the target word and IWDG1.
            let sr = cortex_m::interrupt::free(|_| unsafe { flash_program_from_ram(a, &w) });
            if sr & FAILED != 0 {
                return Err(FlashError::Hw(sr));
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
        // SAFETY: as above, for one whole sector the guard allowed.
        let sr = cortex_m::interrupt::free(|_| unsafe { flash_erase_from_ram(snb) });
        if sr & FAILED != 0 { Err(FlashError::Hw(sr)) } else { Ok(()) }
    }
}
