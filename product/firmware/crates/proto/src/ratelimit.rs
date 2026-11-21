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
