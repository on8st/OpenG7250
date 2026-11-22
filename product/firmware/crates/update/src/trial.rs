// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The times of a trial start (A1.8 U-6; 0044). Uptime in ms, from the start of the image.

/// Run time, with the network up, after which a trial image confirms itself.
pub const CONFIRM_AFTER_MS: u64 = 10_000;
/// A trial must confirm itself within 60 s of its start.
pub const LIMIT_MS: u64 = 60_000;

/// The trial's time is over: a commanded reset into the previous image.
pub const fn expired(now_ms: u64) -> bool {
    now_ms >= LIMIT_MS
}

/// A confirmation may start at `now_ms`, with the network up since `network_up_ms`: long enough
/// after it, and before the limit. One that started before the limit counts, however long its
/// write takes; one that has not started then never does, whichever task looks first.
pub const fn may_start(now_ms: u64, network_up_ms: u64) -> bool {
    now_ms >= network_up_ms + CONFIRM_AFTER_MS && !expired(now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_confirmation_starts_only_before_the_limit() {
        assert!(may_start(59_999, 0) && !expired(59_999), "59.999 s counts");
        assert!(!may_start(60_000, 0) && expired(60_000), "60.000 s does not");
        // Never after it, as after an erase that ended long past it; also if the network came up
        // just in time, or late.
        for late in [60_001, 61_000, 68_000, u32::MAX as u64, u32::MAX as u64 + 20_000] {
            assert!(!may_start(late, 0) && expired(late));
            assert!(!may_start(late, 49_999));
        }
        assert!(may_start(59_999, 49_999) && !may_start(59_998, 49_999));
        assert!((0..70_000).all(|t| !may_start(t, 50_000)), "network up too late: never");
        // At every millisecond at most one of the two, and none before 10 s.
        for t in 0..70_000 {
            assert_eq!(may_start(t, 1_000), (11_000..60_000).contains(&t), "{t}");
            assert_eq!(expired(t), t >= 60_000);
        }
    }
}
