// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Board glue for the sector-7 store (`g7250-store`) and the hardware RNG.
//!
//! Every program and erase is checked by `FlashGuard` inside the store before it gets here; this
//! adapter adds a second check of its own, so that a bug in the store cannot reach sector 0 either
//! ([HIS] HIS 15 rule 1).

use embassy_stm32::flash::{Blocking, Flash};
use g7250_layout::{CONFIG, FLASH_BASE, FlashGuard, Op, Slot, Writer};
use g7250_store::FlashIo;

pub struct BoardFlash {
    pub flash: Flash<'static, Blocking>,
    pub writer: Writer,
}

#[derive(Debug)]
pub enum NvError {
    Hal(embassy_stm32::flash::Error),
    Guard,
}

impl FlashIo for BoardFlash {
    type Error = NvError;

    fn read(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), NvError> {
        self.flash.blocking_read(addr - FLASH_BASE, buf).map_err(NvError::Hal)
    }

    fn program(&mut self, addr: u32, data: &[u8]) -> Result<(), NvError> {
        FlashGuard::check(self.writer, Op::Program, addr, data.len() as u32).map_err(|_| NvError::Guard)?;
        if !CONFIG.contains_span(addr, data.len() as u32) {
            return Err(NvError::Guard);
        }
        g7250_board::wdg::refresh();
        self.flash.blocking_write(addr - FLASH_BASE, data).map_err(NvError::Hal)
    }

    fn erase_sector(&mut self, addr: u32) -> Result<(), NvError> {
        FlashGuard::check(self.writer, Op::Erase, addr, g7250_layout::SECTOR_SIZE).map_err(|_| NvError::Guard)?;
        if addr != CONFIG.start {
            return Err(NvError::Guard);
        }
        // The CPU stalls while the bank it runs from is erased (RM0468); refresh right before.
        g7250_board::wdg::refresh();
        let r = self.flash.blocking_erase(addr - FLASH_BASE, addr - FLASH_BASE + g7250_layout::SECTOR_SIZE).map_err(NvError::Hal);
        g7250_board::wdg::refresh();
        r
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
