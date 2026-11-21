// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! `STREAM_STATUS` (Table B8-2): one byte per stream, bit 7 running, [6:0] reason (Table B8-3
//! "Stream reason"): 0 none or running; 1 not enabled by the host; 2 not offered by this
//! firmware; 3 sample rate not offered; 4 sample size not offered; 5 data format not offered
//! (General byte 39); 6 time stamp or VITA-49 requested; 8 link below 100 Mbit/s; 9 run bit
//! clear; 11 VNA mode requested; 13 frequency encoding mismatch; 14 TX not calibrated.

use crate::session::{Ddc, General};

pub const RUNNING: u8 = 0x80;

/// What DDC0 depends on.
#[derive(Clone, Copy, Debug)]
pub struct DdcInputs {
    pub link_ok: bool,
    pub running: bool,
    pub general: Option<General>,
    pub ddc: Option<Ddc>,
    /// 384 ksps offered (P-NET-1; B6.7).
    pub offered_384: bool,
    /// DDC0 I&Q is built in this firmware (step 4). Until then a valid set-up reports reason 2.
    pub ddc0_built: bool,
}

/// Byte 39 values read as the offered format (Big-Endian, 3-byte): 0x00, and only bit [0]
/// and/or bit [2] set (B6.3; Annex X2, C9).
pub fn format_offered(b39: u8) -> bool {
    b39 & !0b101 == 0
}

pub fn rate_offered(ksps: u16, offered_384: bool) -> bool {
    matches!(ksps, 48 | 96 | 192) || (ksps == 384 && offered_384)
}

/// DDC0 byte of `STREAM_STATUS`.
pub fn ddc0(i: &DdcInputs) -> u8 {
    if !i.link_ok {
        return 8;
    }
    if !i.running {
        return 9;
    }
    if let Some(g) = i.general {
        if !g.phase_word {
            return 13; // discovery byte 21 is 1: phase words (B3.8)
        }
        if g.timestamp || g.vita {
            return 6;
        }
        if g.vna {
            return 11;
        }
        if !format_offered(g.format) {
            return 5;
        }
    }
    let Some(d) = i.ddc else { return 1 }; // B5.2: without a DDC-specific packet, not enabled
    if !d.enable {
        return 1;
    }
    // Before P-RX-2 a DDC0 enable gets reason 2, whatever the rate and size (B6.1; 0042).
    if !i.ddc0_built {
        return 2;
    }
    if !rate_offered(d.rate_ksps, i.offered_384) {
        return 3;
    }
    if !(d.size == 24 || d.size == 0) {
        return 4;
    }
    RUNNING
}

/// DUC0 byte: TX not calibrated (record `tx` bit 4 = 0) is reason 14 (0039).
pub fn duc0(tx_calibrated: bool) -> u8 {
    if tx_calibrated { 0 } else { 14 }
}

/// The whole register: [7:0] DDC0, [15:8] DUC0, [23:16] microphone, [31:24] wideband; the
/// microphone and wideband streams are not offered in this version (reason 2; 0031; B6.10).
pub fn register(ddc0: u8, duc0: u8) -> u32 {
    ddc0 as u32 | ((duc0 as u32) << 8) | (2 << 16) | (2 << 24)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok() -> DdcInputs {
        DdcInputs {
            link_ok: true,
            running: true,
            general: Some(General { phase_word: true, ..Default::default() }),
            ddc: Some(Ddc { enable: true, rate_ksps: 192, size: 24 }),
            offered_384: false,
            ddc0_built: true,
        }
    }

    #[test]
    fn reasons() {
        assert_eq!(ddc0(&ok()), RUNNING);
        let mut i = ok();
        i.ddc = Some(Ddc { enable: true, rate_ksps: 384, size: 24 });
        assert_eq!(ddc0(&i), 3); // 384 not offered until P-NET-1
        i.ddc = Some(Ddc { enable: true, rate_ksps: 768, size: 24 });
        assert_eq!(ddc0(&i), 3);
        i.ddc = Some(Ddc { enable: true, rate_ksps: 192, size: 16 });
        assert_eq!(ddc0(&i), 4);
        i.ddc = Some(Ddc { enable: true, rate_ksps: 192, size: 0 });
        assert_eq!(ddc0(&i), RUNNING);
        let mut i = ok();
        i.general = Some(General { phase_word: true, format: 0x02, ..Default::default() }); // little-endian
        assert_eq!(ddc0(&i), 5);
        i.general = Some(General { phase_word: true, format: 0x05, ..Default::default() });
        assert_eq!(ddc0(&i), RUNNING);
        let mut i = ok();
        i.ddc = None;
        assert_eq!(ddc0(&i), 1);
        i.running = false;
        assert_eq!(ddc0(&i), 9);
        i.link_ok = false;
        assert_eq!(ddc0(&i), 8);
        let mut i = ok();
        i.ddc0_built = false;
        assert_eq!(ddc0(&i), 2);
        // Not offered beats rate and size (T-B6-27); the General checks still come first.
        i.ddc = Some(Ddc { enable: true, rate_ksps: 768, size: 16 });
        assert_eq!(ddc0(&i), 2);
        i.general = Some(General { phase_word: true, format: 0x02, ..Default::default() });
        assert_eq!(ddc0(&i), 5);
        assert_eq!(duc0(false), 14);
        assert_eq!(register(9, 14), 0x0202_0E09);
    }
}
