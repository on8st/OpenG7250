// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! What a read made with bus faults ignored says afterwards (the board's `flash::read_tolerant`
//! and `kept::read_tolerant`): whether the data it returned may be used. Pure decisions on the
//! register values, so that host tests cover them.

/// FLASH_SR1 DBECCERR1 (RM0468): a double ECC error; the read returned a bus error.
pub const DBECCERR1: u32 = 1 << 26;
/// SCB CFSR: the BusFault status, bits 8-15 (ARMv7-M). A bus fault that is ignored at priority
/// -1 with CCR.BFHFNMIGN sets its status all the same.
pub const CFSR_BFSR: u32 = 0x0000_FF00;

/// A read of RAM: it met a bus fault (any BusFault status bit), so the word is not there.
pub const fn ram_unreadable(cfsr: u32) -> bool {
    cfsr & CFSR_BFSR != 0
}

/// A read of flash: the flash interface counted a double ECC error, or the core met a bus fault.
/// Either alone is enough: a bus error need not be an ECC error, and an ECC error the interface
/// reports is one whatever the core says.
pub const fn flash_unreadable(sr1: u32, cfsr: u32) -> bool {
    sr1 & DBECCERR1 != 0 || cfsr & CFSR_BFSR != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bus_fault_status_bit_alone_is_enough_and_no_other() {
        for bit in 0..32 {
            let v = 1u32 << bit;
            let bus = (8..16).contains(&bit);
            assert_eq!(ram_unreadable(v), bus, "CFSR bit {bit}");
            assert_eq!(flash_unreadable(0, v), bus, "CFSR bit {bit}");
            // The flash status: only the double error; a corrected single error (bit 25) is data.
            assert_eq!(flash_unreadable(v, 0), bit == 26, "FLASH_SR1 bit {bit}");
        }
        assert!(!ram_unreadable(0));
        assert!(!flash_unreadable(0, 0));
        assert!(!flash_unreadable(!DBECCERR1, !CFSR_BFSR));
        assert!(flash_unreadable(DBECCERR1, CFSR_BFSR));
    }
}
