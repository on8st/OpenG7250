// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Board glue for the store in sectors 6 and 7 (`g7250-store`) and the hardware RNG.
//!
//! Every program and erase is checked by `FlashGuard` inside the store and again in the driver
//! (`g7250_board::flash`), so that a bug in the store cannot reach sector 0 either ([HIS] HIS 15
//! rule 1).

use g7250_layout::{Slot, Writer};

/// The store's flash: the raw driver shared with the boot stage (`g7250_board::flash`), which
/// passes `FlashGuard` for every program and erase and runs both from RAM.
pub type BoardFlash = g7250_board::flash::RawFlash;

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
