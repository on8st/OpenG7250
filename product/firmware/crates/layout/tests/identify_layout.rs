// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! IDENTIFY tag 25 (0062; T-A1-79): the layout in 128 KiB sectors, from the layout constants.

use g7250_layout::*;

#[test]
fn tag_25_is_the_layout() {
    // Slot A sectors 2-3, slot B 4-5, the store 6-7.
    assert_eq!(IDENTIFY_LAYOUT, [2, 2, 4, 2, 6, 2]);
    let r = |first: u8, n: u8| (first as u32, first as u32 + n as u32);
    let [a0, an, b0, bn, s0, sn] = IDENTIFY_LAYOUT;
    let parts = [r(a0, an), r(b0, bn), r(s0, sn)];
    for (i, p) in parts.iter().enumerate() {
        assert!(p.0 >= 2 && p.1 <= 8 && p.0 < p.1, "within sectors 2-7, never 0 or 1: {p:?}");
        for q in &parts[i + 1..] {
            assert!(p.1 <= q.0 || q.1 <= p.0, "no overlap: {p:?} {q:?}");
        }
    }
    // The 61 slot size (0061 addendum) is the sector count x 128 KiB.
    assert_eq!(Slot::A.range().len(), an as u32 * SECTOR_SIZE);
    assert_eq!(Slot::B.range().len(), bn as u32 * SECTOR_SIZE);
    // The slots end where the store begins.
    assert_eq!(SLOT_B.end, STORE.start);
}
