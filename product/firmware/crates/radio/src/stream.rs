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
    /// The offered rates, as record `ddc_rates` (B4.8: bit 0 48, bit 1 96, bit 2 192, bit 3
    /// 384): 384 only with P-NET-1, and only the rates P-RX-2 has a frame for (0060 point 2.4).
    pub rates: u8,
    /// DDC0 I&Q is built in this firmware (step 4). Until then a valid set-up reports reason 2.
    pub ddc0_built: bool,
    /// A firmware update is in progress (reason 10, A1.8).
    pub update: bool,
}

/// Byte 39 values read as the offered format (Big-Endian, 3-byte): 0x00, and only bit [0]
/// and/or bit [2] set (B6.3; Annex X2, C9).
pub fn format_offered(b39: u8) -> bool {
    b39 & !0b101 == 0
}

pub fn rate_offered(ksps: u16, rates: u8) -> bool {
    let bit = match ksps {
        48 => 0,
        96 => 1,
        192 => 2,
        384 => 3,
        _ => return false,
    };
    rates & (1 << bit) != 0
}

/// The radio-wide reasons, first in the order of Table B8-3 (0042 addendum): 10 update in
/// progress, 9 run bit clear, 8 link below 100 Mbit/s.
fn radio_wide(update: bool, running: bool, link_ok: bool) -> Option<u8> {
    if update {
        Some(10)
    } else if !running {
        Some(9)
    } else if !link_ok {
        Some(8)
    } else {
        None
    }
}

/// DDC0 byte of `STREAM_STATUS`, in the order of Table B8-3 (spec/draft 2469917): 10, 9, 8;
/// 1; 13, 6, 11, 12, 5; 2, 14; 3, 4, 7.
pub fn ddc0(i: &DdcInputs) -> u8 {
    if let Some(r) = radio_wide(i.update, i.running, i.link_ok) {
        return r;
    }
    // B5.2: without a DDC-specific packet DDC0 counts as not enabled.
    let Some(d) = i.ddc else { return 1 };
    if !d.enable {
        return 1;
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
    // Before P-RX-2 a DDC0 enable gets reason 2, whatever the rate and size (B6.1; 0042).
    if !i.ddc0_built {
        return 2;
    }
    if !rate_offered(d.rate_ksps, i.rates) {
        return 3;
    }
    if !(d.size == 24 || d.size == 0) {
        return 4;
    }
    RUNNING
}

/// DUC0 byte: the radio-wide reasons first, then 14 TX not calibrated (record `tx` bit 4 = 0,
/// 0039).
pub fn duc0(update: bool, running: bool, link_ok: bool, tx_calibrated: bool) -> u8 {
    if let Some(r) = radio_wide(update, running, link_ok) {
        return r;
    }
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
            rates: 0x07,
            ddc0_built: true,
            update: false,
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
        // A rate P-RX-2 has no frame for is not offered (0060 point 2.4): 96 missing.
        i.rates = 0b101;
        i.ddc = Some(Ddc { enable: true, rate_ksps: 96, size: 24 });
        assert_eq!(ddc0(&i), 3);
        i.ddc = Some(Ddc { enable: true, rate_ksps: 48, size: 24 });
        assert_eq!(ddc0(&i), RUNNING);
        i.rates = 0x07;
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
        i.link_ok = false;
        assert_eq!(ddc0(&i), 8);
        // 9 before 8, 10 before both (Table B8-3 order, 0042 addendum).
        i.running = false;
        assert_eq!(ddc0(&i), 9);
        i.update = true;
        assert_eq!(ddc0(&i), 10);
        // 1 comes before the General-packet refusals.
        let mut i = ok();
        i.ddc = Some(Ddc { enable: false, rate_ksps: 192, size: 24 });
        i.general = Some(General { phase_word: true, format: 0x02, ..Default::default() });
        assert_eq!(ddc0(&i), 1);
        // Order among the General refusals: 13, 6, 11, 5.
        i.ddc = Some(Ddc { enable: true, rate_ksps: 192, size: 24 });
        i.general = Some(General { phase_word: false, timestamp: true, vna: true, format: 0x02, ..Default::default() });
        assert_eq!(ddc0(&i), 13);
        i.general = Some(General { phase_word: true, timestamp: true, vna: true, format: 0x02, ..Default::default() });
        assert_eq!(ddc0(&i), 6);
        i.general = Some(General { phase_word: true, vna: true, format: 0x02, ..Default::default() });
        assert_eq!(ddc0(&i), 11);
        let mut i = ok();
        i.ddc0_built = false;
        assert_eq!(ddc0(&i), 2);
        // Not offered beats rate and size (T-B6-27); the General checks still come first.
        i.ddc = Some(Ddc { enable: true, rate_ksps: 768, size: 16 });
        assert_eq!(ddc0(&i), 2);
        i.general = Some(General { phase_word: true, format: 0x02, ..Default::default() });
        assert_eq!(ddc0(&i), 5);
        assert_eq!(duc0(false, true, true, false), 14);
        assert_eq!(duc0(false, false, true, false), 9);
        assert_eq!(duc0(true, true, true, false), 10);
        assert_eq!(register(9, 14), 0x0202_0E09);
    }
}
