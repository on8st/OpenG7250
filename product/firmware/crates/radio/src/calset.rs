// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Values of the calibration set (A1.12) that this firmware uses at run time.

use g7250_proto::mgmt::payload;
use g7250_safety::rx_frame::{Bad, RxCal};

/// P-RX-2 from a set's entries (Table M-5): `Ok(None)` if the set does not hold it, `Err` if
/// its value does not decode (Table P-1; 0060).
pub fn rx_cal(entries: &[u8]) -> Result<Option<RxCal>, Bad> {
    payload::calibration_value(entries, "P-RX-2").map(RxCal::decode).transpose()
}

/// P-RX-2 from a stored set as `SET_CALIBRATION` gave it (date, source, entries); `None` if
/// there is no set, it does not parse, or it holds no valid P-RX-2.
pub fn rx_cal_of_set(set: &[u8]) -> Option<RxCal> {
    let h = payload::parse_set_calibration(set).ok()?;
    rx_cal(h.entries).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(entries: &[(&str, &[u8])]) -> std::vec::Vec<u8> {
        let mut v = std::vec![0, 0, 0, 1, 3, b'c', b'a', b'p'];
        for (id, val) in entries {
            v.push(id.len() as u8);
            v.extend_from_slice(id.as_bytes());
            v.extend_from_slice(&(val.len() as u16).to_be_bytes());
            v.extend_from_slice(val);
        }
        v
    }

    #[test]
    fn p_rx_2_from_the_set() {
        let mut good = std::vec![0u8, 5, 3, 0x40];
        good.extend_from_slice(&[0; 10]);
        let s = set(&[("P-RF-2", &[1, 2]), ("P-RX-2", &good)]);
        let c = rx_cal_of_set(&s).unwrap();
        assert_eq!((c.ddc0_offset, c.rate_mask()), (5, 0b0001));
        assert_eq!(rx_cal_of_set(&set(&[("P-RF-2", &[1, 2])])), None);
        assert_eq!(rx_cal(&set(&[("P-RX-2", &[0, 3])])[8..]), Err(Bad));
        assert_eq!(rx_cal_of_set(&[0u8; 37][32..]), None, "no set stored (37 zero bytes)");
    }
}
