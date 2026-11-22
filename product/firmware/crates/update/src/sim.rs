// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The application area (sectors 1-7) in RAM with the rules of H7 flash: program only erased,
//! aligned, whole 32-byte words, each once; erase whole sectors; never sector 0 (outside the
//! model, so any attempt fails). Power is cut after `budget` steps (one step = one flash word or
//! one erase); a cut erase leaves the sector half erased. With `ecc_on_cut` the word that a cut
//! hits is left torn: half programmed (or, for an erase, the word at the boundary), its ECC bad,
//! so that a read of it is an error, as on the H7 (R0; `FlashIo::unreadable`).

use g7250_layout::{APPLICATION_AREA, FLASH_WORD, SECTOR_SIZE};
use g7250_store::FlashIo;

#[derive(Clone)]
pub struct Sim {
    pub mem: Vec<u8>,
    written: Vec<bool>,
    /// Words whose ECC is bad (torn by a cut).
    bad: Vec<bool>,
    pub ecc_on_cut: bool,
    pub budget: Option<usize>,
    pub steps: usize,
    /// A step failed for lack of power since the last `power_on`.
    pub lost: bool,
}

#[derive(Debug, PartialEq)]
pub enum SimErr {
    PowerLost,
    Ecc,
    Rule(&'static str),
}

impl Sim {
    pub fn new() -> Self {
        let n = APPLICATION_AREA.len() as usize;
        Sim { mem: vec![0xFF; n], written: vec![false; n / 32], bad: vec![false; n / 32], ecc_on_cut: false, budget: None, steps: 0, lost: false }
    }

    pub fn power_on(&mut self) {
        self.budget = None;
        self.lost = false;
    }

    fn step(&mut self) -> Result<(), SimErr> {
        if let Some(b) = self.budget.as_mut() {
            if *b == 0 {
                self.lost = true;
                return Err(SimErr::PowerLost);
            }
            *b -= 1;
        }
        self.steps += 1;
        Ok(())
    }

    /// Every word of the span reads without an ECC error (the board's `flash::probe`).
    pub fn readable(&self, start: u32, len: u32) -> bool {
        let o = ((start - APPLICATION_AREA.start) / 32) as usize;
        let n = len.div_ceil(32) as usize;
        !self.bad[o..o + n].iter().any(|&b| b)
    }

    /// Make the flash word at `addr` unreadable (a double ECC error).
    pub fn tear(&mut self, addr: u32) {
        self.bad[((addr - APPLICATION_AREA.start) / FLASH_WORD) as usize] = true;
    }

    pub fn bytes(&self, start: u32, len: u32) -> &[u8] {
        let o = (start - APPLICATION_AREA.start) as usize;
        &self.mem[o..o + len as usize]
    }

    /// Put bytes in directly (the first install), word-padded with 0xFF.
    pub fn install(&mut self, start: u32, data: &[u8]) {
        let mut v = data.to_vec();
        v.resize(data.len().div_ceil(32) * 32, 0xFF);
        self.program(start, &v).unwrap();
    }
}

impl FlashIo for Sim {
    type Error = SimErr;
    fn read(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), SimErr> {
        if !APPLICATION_AREA.contains_span(addr, buf.len() as u32) {
            return Err(SimErr::Rule("read outside"));
        }
        let o = (addr - APPLICATION_AREA.start) as usize;
        if !buf.is_empty() && self.bad[o / 32..=(o + buf.len() - 1) / 32].iter().any(|&b| b) {
            return Err(SimErr::Ecc);
        }
        buf.copy_from_slice(&self.mem[o..o + buf.len()]);
        Ok(())
    }
    fn unreadable(e: &SimErr) -> bool {
        *e == SimErr::Ecc
    }
    fn program(&mut self, addr: u32, data: &[u8]) -> Result<(), SimErr> {
        if addr % FLASH_WORD != 0 || data.len() % FLASH_WORD as usize != 0 || !APPLICATION_AREA.contains_span(addr, data.len() as u32) {
            return Err(SimErr::Rule("alignment or range"));
        }
        for (i, w) in data.chunks(FLASH_WORD as usize).enumerate() {
            let wi = ((addr - APPLICATION_AREA.start) / FLASH_WORD) as usize + i;
            if let Err(e) = self.step() {
                if self.ecc_on_cut && !self.written[wi] {
                    // Half the bits made it: neither erased nor the data, ECC bad.
                    self.written[wi] = true;
                    self.bad[wi] = true;
                    self.mem[wi * 32..wi * 32 + 16].copy_from_slice(&w[..16]);
                }
                return Err(e);
            }
            if self.written[wi] {
                return Err(SimErr::Rule("word programmed twice"));
            }
            self.written[wi] = true;
            self.mem[wi * 32..wi * 32 + 32].copy_from_slice(w);
        }
        Ok(())
    }
    fn erase_sector(&mut self, addr: u32) -> Result<(), SimErr> {
        if (addr - APPLICATION_AREA.start) % SECTOR_SIZE != 0 || !APPLICATION_AREA.contains_span(addr, SECTOR_SIZE) {
            return Err(SimErr::Rule("erase not a sector of the application area"));
        }
        let o = (addr - APPLICATION_AREA.start) as usize;
        let n = SECTOR_SIZE as usize;
        let cut = self.step().is_err();
        let end = if cut { o + n / 2 } else { o + n };
        self.mem[o..end].fill(0xFF);
        for w in &mut self.written[o / 32..end / 32] {
            *w = false;
        }
        self.bad[o / 32..end / 32].fill(false);
        if cut && self.ecc_on_cut {
            // The word at the boundary was being erased: unreadable.
            self.bad[end / 32] = true;
            self.written[end / 32] = true;
        }
        if cut { Err(SimErr::PowerLost) } else { Ok(()) }
    }
}
