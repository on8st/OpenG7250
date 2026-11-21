// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The application area (sectors 1-7) in RAM with the rules of H7 flash: program only erased,
//! aligned, whole 32-byte words, each once; erase whole sectors; never sector 0 (outside the
//! model, so any attempt fails). Power is cut after `budget` steps (one step = one flash word or
//! one erase); a cut erase leaves the sector half erased.

use g7250_layout::{APPLICATION_AREA, FLASH_WORD, SECTOR_SIZE};
use g7250_store::FlashIo;

#[derive(Clone)]
pub struct Sim {
    pub mem: Vec<u8>,
    written: Vec<bool>,
    pub budget: Option<usize>,
    pub steps: usize,
    /// A step failed for lack of power since the last `power_on`.
    pub lost: bool,
}

#[derive(Debug, PartialEq)]
pub enum SimErr {
    PowerLost,
    Rule(&'static str),
}

impl Sim {
    pub fn new() -> Self {
        let n = APPLICATION_AREA.len() as usize;
        Sim { mem: vec![0xFF; n], written: vec![false; n / 32], budget: None, steps: 0, lost: false }
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
        buf.copy_from_slice(&self.mem[o..o + buf.len()]);
        Ok(())
    }
    fn program(&mut self, addr: u32, data: &[u8]) -> Result<(), SimErr> {
        if addr % FLASH_WORD != 0 || data.len() % FLASH_WORD as usize != 0 || !APPLICATION_AREA.contains_span(addr, data.len() as u32) {
            return Err(SimErr::Rule("alignment or range"));
        }
        for (i, w) in data.chunks(FLASH_WORD as usize).enumerate() {
            self.step()?;
            let wi = ((addr - APPLICATION_AREA.start) / FLASH_WORD) as usize + i;
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
        if cut { Err(SimErr::PowerLost) } else { Ok(()) }
    }
}
