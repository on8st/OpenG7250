// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Reply rate limits (B3.2; B11.14; 0022 §16): at most `per_source` replies per second per
//! source IP address and port, and `total` per second in total. Requests above the limit are
//! dropped and counted. Fixed one-second windows; a small table of recent sources, the oldest
//! entry is reused when full (a flood from many sources then still meets the total limit).

#[derive(Clone, Copy, Debug, Default)]
struct Entry {
    ip: [u8; 4],
    port: u16,
    window: u32,
    count: u16,
    used: bool,
}

/// Whether at least `min_ms` passed from `last_ms` to `now_ms`, for a limit of one per `min_ms`.
/// The clock counts whole milliseconds, so two times that many ticks apart may be up to one
/// tick closer in reality (T-A2-13: 99.6 ms measured for 100 ticks): one tick more is needed.
pub const fn apart(last_ms: u64, now_ms: u64, min_ms: u64) -> bool {
    now_ms.saturating_sub(last_ms) > min_ms
}

/// At most one event per interval and source (address and port): a real interval, not a fixed
/// window. A small table; the source seen longest ago gives way.
pub struct Spaced<const N: usize> {
    table: [Option<([u8; 4], u16, u64)>; N],
}

impl<const N: usize> Spaced<N> {
    pub const fn new() -> Self {
        Self { table: [None; N] }
    }

    pub fn allow(&mut self, ip: [u8; 4], port: u16, now_ms: u64, min_ms: u64) -> bool {
        if let Some(e) = self.table.iter_mut().flatten().find(|e| (e.0, e.1) == (ip, port)) {
            if !apart(e.2, now_ms, min_ms) {
                return false;
            }
            e.2 = now_ms;
            return true;
        }
        let i = (0..N).min_by_key(|&i| self.table[i].map_or(0, |e| e.2 + 1)).unwrap_or(0);
        self.table[i] = Some((ip, port, now_ms));
        true
    }
}

impl<const N: usize> Default for Spaced<N> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct RateLimit<const N: usize> {
    per_source: u16,
    total: u16,
    window: u32,
    total_count: u16,
    table: [Entry; N],
    next: usize,
    /// Requests dropped by the limit since start.
    pub dropped: u32,
}

impl<const N: usize> RateLimit<N> {
    pub const fn new(per_source: u16, total: u16) -> Self {
        Self {
            per_source,
            total,
            window: u32::MAX,
            total_count: 0,
            table: [Entry { ip: [0; 4], port: 0, window: 0, count: 0, used: false }; N],
            next: 0,
            dropped: 0,
        }
    }

    /// May a reply go to this source now? `now_ms` is a monotonic millisecond clock.
    pub fn allow(&mut self, ip: [u8; 4], port: u16, now_ms: u64) -> bool {
        let w = (now_ms / 1000) as u32;
        if w != self.window {
            self.window = w;
            self.total_count = 0;
        }
        if self.total_count >= self.total {
            self.dropped = self.dropped.saturating_add(1);
            return false;
        }
        let idx = match self.table.iter().position(|e| e.used && e.ip == ip && e.port == port) {
            Some(i) => i,
            None => {
                let i = self.table.iter().position(|e| !e.used).unwrap_or_else(|| {
                    let i = self.next;
                    self.next = (self.next + 1) % N;
                    i
                });
                self.table[i] = Entry { ip, port, window: w, count: 0, used: true };
                i
            }
        };
        let e = &mut self.table[idx];
        if e.window != w {
            e.window = w;
            e.count = 0;
        }
        if e.count >= self.per_source {
            self.dropped = self.dropped.saturating_add(1);
            return false;
        }
        e.count += 1;
        self.total_count += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_per_interval_also_with_truncated_times() {
        // T-A2-13: events at 0.999 ms and 100.6 ms (99.6 ms apart) are ticks 0 and 100.
        let tick = |us: u64| us / 1000;
        for phase in [0u64, 1, 500, 999] {
            for gap in [99_001u64, 99_600, 99_999] {
                assert!(!apart(tick(phase), tick(phase + gap), 100), "phase {phase} gap {gap}");
            }
            assert!(apart(tick(phase), tick(phase + 101_000), 100));
        }
        // Writes every 30 ms for 0.6 s: 5 answers, never closer than 100 ms.
        let mut s: Spaced<8> = Spaced::new();
        let mut sent = vec![];
        for k in 0..20u64 {
            let us = 999 + 30_000 * k;
            if s.allow([10, 0, 0, 1], 5000, tick(us), 100) {
                sent.push(us);
            }
        }
        assert!(sent.windows(2).all(|w| w[1] - w[0] >= 100_000), "{sent:?}");
        assert_eq!(sent.len(), 5);
        // Another source has its own interval; a full table drops the oldest source.
        assert!(s.allow([10, 0, 0, 2], 5000, 600, 100));
        for k in 0..8u16 {
            assert!(s.allow([10, 0, 0, 3], k, 601 + k as u64, 100));
        }
        assert!(!s.allow([10, 0, 0, 3], 7, 650, 100));
    }

    #[test]
    fn ten_per_source_fifty_total() {
        let mut r: RateLimit<16> = RateLimit::new(10, 50);
        let a = [10, 0, 0, 1];
        let ok = (0..15).filter(|_| r.allow(a, 5000, 100)).count();
        assert_eq!(ok, 10);
        assert_eq!(r.dropped, 5);
        // Next second: allowed again.
        assert!(r.allow(a, 5000, 1100));
        // Another port of the same host is another source.
        assert!(r.allow(a, 5001, 1100));
        // Total cap across many sources.
        let mut r: RateLimit<16> = RateLimit::new(10, 50);
        let mut ok = 0;
        for i in 0..100u16 {
            if r.allow([10, 0, (i >> 8) as u8, i as u8], 6000, 0) {
                ok += 1;
            }
        }
        assert_eq!(ok, 50);
    }
}
