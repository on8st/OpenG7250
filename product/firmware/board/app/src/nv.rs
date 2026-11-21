// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Board glue for the store in sectors 6 and 7 (`g7250-store`) and the hardware RNG.
//!
//! Every program and erase is checked by `FlashGuard` inside the store before it gets here; this
//! adapter adds a second check of its own, so that a bug in the store cannot reach sector 0 either
//! ([HIS] HIS 15 rule 1).

use embassy_stm32::flash::{Blocking, Flash};
use g7250_layout::{FLASH_BASE, FlashGuard, Op, STORE, STORE_SECTORS, Slot, Writer};
use g7250_store::FlashIo;

pub struct BoardFlash {
    pub flash: Flash<'static, Blocking>,
    pub writer: Writer,
}

#[derive(Debug)]
pub enum NvError {
    Hal(embassy_stm32::flash::Error),
    Guard,
    /// Final FLASH_SR1 of a failed erase, or ERASE_TIMEOUT.
    Erase(u32),
}

impl FlashIo for BoardFlash {
    type Error = NvError;

    fn read(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), NvError> {
        self.flash.blocking_read(addr - FLASH_BASE, buf).map_err(NvError::Hal)
    }

    fn program(&mut self, addr: u32, data: &[u8]) -> Result<(), NvError> {
        FlashGuard::check(self.writer, Op::Program, addr, data.len() as u32).map_err(|_| NvError::Guard)?;
        if !STORE.contains_span(addr, data.len() as u32) {
            return Err(NvError::Guard);
        }
        g7250_board::wdg::refresh();
        self.flash.blocking_write(addr - FLASH_BASE, data).map_err(NvError::Hal)
    }

    fn erase_sector(&mut self, addr: u32) -> Result<(), NvError> {
        FlashGuard::check(self.writer, Op::Erase, addr, g7250_layout::SECTOR_SIZE).map_err(|_| NvError::Guard)?;
        let snb = match addr {
            a if a == STORE_SECTORS[0].start => 6,
            a if a == STORE_SECTORS[1].start => 7,
            _ => return Err(NvError::Guard),
        };
        // A sector erase can outlast the 2 s watchdog, and the CPU cannot fetch from the bank
        // being erased. The erase runs from RAM with interrupts masked (the vector table and the
        // handlers are in this bank) and refreshes the IWDG on every poll.
        let sr = cortex_m::interrupt::free(|_| {
            // SAFETY: the guard and the match above allow only sectors 6 and 7; the function
            // runs from RAM and touches only FLASH bank 1 and IWDG1 registers.
            unsafe { erase_sector_from_ram(snb) }
        });
        if sr & ERASE_FAILED != 0 { Err(NvError::Erase(sr)) } else { Ok(()) }
    }
}

const FLASH_KEYR1: u32 = 0x5200_2004;
const FLASH_CR1: u32 = 0x5200_200C;
const FLASH_SR1: u32 = 0x5200_2010;
const FLASH_CCR1: u32 = 0x5200_2014;
const IWDG1_KR: u32 = 0x5800_4800;
/// FLASH_CR1 (RM0468): LOCK bit 0, SER bit 2, START bit 7, SNB bits 10:8.
const CR_LOCK: u32 = 1 << 0;
const CR_SER: u32 = 1 << 2;
const CR_START: u32 = 1 << 7;
/// FLASH_SR1: BSY bit 0, QW bit 2; error flags bits 17-26.
const SR_BUSY: u32 = (1 << 0) | (1 << 2);
const SR_ERRORS: u32 = 0x07EE_0000;
/// Returned instead of SR when the poll bound ran out.
pub const ERASE_TIMEOUT: u32 = 1 << 31;
const ERASE_FAILED: u32 = SR_ERRORS | ERASE_TIMEOUT;
/// Polls of SR before giving up. One poll is a few cycles at 64 MHz from RAM; 400 000 000 polls are
/// well above 10 s, far over the sector erase maximum of the STM32H725/735 datasheet.
const ERASE_POLLS: u32 = 400_000_000;

/// Erase sector `snb` of bank 1 and wait for the end, from RAM. Everything it calls is inlined
/// volatile access; no flash code runs while the erase is in progress (checked in the build by
/// `board/check-ramfunc.sh`: the function body has no branch out of RAM). Returns the final SR,
/// or [`ERASE_TIMEOUT`] if the poll bound ran out.
#[unsafe(link_section = ".data.g7250_ramfunc")]
#[inline(never)]
unsafe fn erase_sector_from_ram(snb: u32) -> u32 {
    use core::ptr::{read_volatile as rd, write_volatile as wr};
    unsafe {
        if rd(FLASH_CR1 as *const u32) & CR_LOCK != 0 {
            wr(FLASH_KEYR1 as *mut u32, 0x4567_0123);
            wr(FLASH_KEYR1 as *mut u32, 0xCDEF_89AB);
        }
        wr(FLASH_CCR1 as *mut u32, SR_ERRORS | (1 << 16));
        let cr = rd(FLASH_CR1 as *const u32) & !(0x7 << 8);
        wr(FLASH_CR1 as *mut u32, cr | CR_SER | (snb << 8));
        wr(FLASH_CR1 as *mut u32, cr | CR_SER | (snb << 8) | CR_START);
        let mut sr = rd(FLASH_SR1 as *const u32);
        let mut n: u32 = 0;
        while sr & SR_BUSY != 0 {
            wr(IWDG1_KR as *mut u32, 0xAAAA);
            n += 1;
            if n > ERASE_POLLS {
                sr = ERASE_TIMEOUT;
                break;
            }
            sr = rd(FLASH_SR1 as *const u32);
        }
        wr(IWDG1_KR as *mut u32, 0xAAAA);
        let cr = rd(FLASH_CR1 as *const u32) & !(CR_SER | (0x7 << 8));
        wr(FLASH_CR1 as *mut u32, cr | CR_LOCK);
        wr(FLASH_CCR1 as *mut u32, SR_ERRORS | (1 << 16));
        sr
    }
}

pub fn writer_for(slot: Slot) -> Writer {
    Writer::Application { running: slot }
}

/// Hardware RNG ([HIS] HIS 10: present on the H725/H735; HIS-20), raw registers (RM0468 RNG):
/// kernel clock HSI48 (RNGSEL reset value), RNGEN, wait DRDY with a bound, check the seed and
/// clock error flags. Returns false on any error.
pub fn rng_fill(out: &mut [u8; 16]) -> bool {
    use embassy_stm32::pac;
    pac::RCC.ahb2enr().modify(|w| w.set_rngen(true));
    let _ = pac::RCC.ahb2enr().read();
    pac::RNG.cr().modify(|w| w.set_rngen(true));
    for chunk in out.chunks_mut(4) {
        let mut got = None;
        for _ in 0..100_000 {
            let sr = pac::RNG.sr().read();
            if sr.secs() || sr.cecs() {
                return false;
            }
            if sr.drdy() {
                got = Some(pac::RNG.dr().read());
                break;
            }
        }
        let Some(v) = got else { return false };
        if v == 0 {
            return false;
        }
        chunk.copy_from_slice(&v.to_be_bytes()[..chunk.len()]);
    }
    true
}
