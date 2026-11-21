// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The DUC FIFO (spec B10.5, B10.7, B11.7): the owner's DUC I&Q on port 1029, one slot per
//! packet (240 samples of 24-bit I and Q, 1440 bytes after the 4-byte sequence number, [P2] DUC
//! I&Q Data packet).
//!
//! - While unkeyed it keeps only the latest 4800 samples (20 packets) received within the last
//!   25 ms; older or excess packets are dropped oldest first and counted (B10.7, 0022 §63).
//! - While keyed it holds what the host sends up to its capacity; a full FIFO discards whole
//!   packets (B11.7). The capacity while keyed is pending hardware (P-TIM-2; HIS-18); this
//!   firmware uses the same 20 packets until it is set.
//! - It is emptied at every unkey, session end and takeover (B10.7, 0022 §32), and at a key whose
//!   RF the CW keyer forms (break-in clear, CW selected): DUC I&Q received while that
//!   transmission is keyed is dropped and counted (B10.5b item 2.3).
//! - The DUC takes 192 samples per ms while keyed; an empty FIFO gives it zero samples, and the
//!   underflow starts at the key if the FIFO is empty then, else when it runs empty (B10.5,
//!   0022 §57). Every packet that enters ends the underflow.
//!
//! The DUC itself is behind the closed FPGA bus, so [`DucFifo::consume`] only advances the read
//! position; the caller decides what to do with the samples once a DUC exists. Whether a packet
//! may enter at all (owner, session, record `tx` bits 1 and 4, refusals) is the caller's
//! decision (B10.7).

/// I&Q bytes of one DUC I&Q packet after its sequence number.
pub const PAYLOAD: usize = 1440;
/// A DUC I&Q packet ([P2]): 4-byte sequence number and 240 samples.
pub const PACKET_LEN: usize = 4 + PAYLOAD;
pub const PACKET_SAMPLES: u64 = 240;
/// 192 ksps (B10.7).
pub const SAMPLES_PER_MS: u64 = 192;
/// Unkeyed depth: 4800 samples, 20 packets (B10.7).
pub const UNKEYED_PACKETS: usize = 20;
/// Unkeyed age limit (B10.7).
pub const UNKEYED_MAX_AGE_MS: u64 = 25;

#[derive(Clone, Copy)]
struct Slot {
    at: u64,
    data: [u8; PAYLOAD],
}

pub struct DucFifo<const N: usize> {
    slots: [Slot; N],
    head: usize,
    len: usize,
    /// Samples of the head packet already taken by the DUC.
    read: u64,
    keyed: bool,
    /// The keyer forms the RF: no I&Q enters (B10.5b item 2.3).
    drop_all: bool,
    last_ms: u64,
    underflow_since: Option<u64>,
    /// Counted for `GET_STATS` (B10.7, B11.7).
    pub dropped_old: u64,
    pub dropped_full: u64,
    pub dropped_cw: u64,
}

impl<const N: usize> Default for DucFifo<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> DucFifo<N> {
    pub const fn new() -> Self {
        DucFifo {
            slots: [Slot { at: 0, data: [0; PAYLOAD] }; N],
            head: 0,
            len: 0,
            read: 0,
            keyed: false,
            drop_all: false,
            last_ms: 0,
            underflow_since: None,
            dropped_old: 0,
            dropped_full: 0,
            dropped_cw: 0,
        }
    }

    pub fn len_packets(&self) -> usize {
        self.len
    }

    /// Samples waiting for the DUC.
    pub fn len_samples(&self) -> u64 {
        (self.len as u64 * PACKET_SAMPLES).saturating_sub(self.read)
    }

    /// The arrival time of the oldest packet waiting.
    pub fn oldest_ms(&self) -> Option<u64> {
        (self.len > 0).then(|| self.slots[self.head].at)
    }

    /// Since when the DUC has had no samples while keyed (High Priority status byte 30 bit [2];
    /// the underrun check of B10.5).
    pub fn underflow_since(&self) -> Option<u64> {
        self.underflow_since
    }

    fn pop(&mut self) {
        self.head = (self.head + 1) % N;
        self.len -= 1;
        self.read = 0;
    }

    fn expire(&mut self, now: u64) {
        while self.len > 0 && now.saturating_sub(self.slots[self.head].at) >= UNKEYED_MAX_AGE_MS {
            self.pop();
            self.dropped_old += 1;
        }
    }

    fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
        self.read = 0;
    }

    /// A DUC I&Q packet the caller has accepted (B10.7). Returns true if it entered the FIFO.
    pub fn push(&mut self, p: &[u8], now: u64) -> bool {
        if p.len() < PACKET_LEN {
            return false;
        }
        if self.drop_all {
            self.dropped_cw += 1;
            return false;
        }
        if self.keyed {
            if self.len == N {
                self.dropped_full += 1;
                return false;
            }
        } else {
            self.expire(now);
            if self.len == UNKEYED_PACKETS.min(N) {
                self.pop();
                self.dropped_old += 1;
            }
        }
        let i = (self.head + self.len) % N;
        self.slots[i].at = now;
        self.slots[i].data.copy_from_slice(&p[4..PACKET_LEN]);
        self.len += 1;
        self.underflow_since = None;
        true
    }

    /// Unkeyed upkeep: drop what has aged past 25 ms (B10.7). Call often.
    pub fn idle(&mut self, now: u64) {
        if !self.keyed {
            self.expire(now);
        }
    }

    /// A key. `iq` false: the CW keyer forms the RF, the FIFO is emptied and no I&Q enters
    /// until the unkey (B10.5b item 2.3).
    pub fn key(&mut self, now: u64, iq: bool) {
        self.keyed = true;
        self.last_ms = now;
        if iq {
            self.expire(now);
            self.underflow_since = (self.len == 0).then_some(now);
        } else {
            self.clear();
            self.drop_all = true;
            self.underflow_since = None;
        }
    }

    /// Every unkey, session end and takeover (B10.7).
    pub fn unkey(&mut self) {
        self.clear();
        self.keyed = false;
        self.drop_all = false;
        self.underflow_since = None;
    }

    /// The DUC takes its samples up to `now` (192 per ms). Returns how many it took from the
    /// FIFO; the rest of the period it gets zeros.
    pub fn consume(&mut self, now: u64) -> u64 {
        if !self.keyed || self.drop_all {
            return 0;
        }
        let want = now.saturating_sub(self.last_ms) * SAMPLES_PER_MS;
        self.last_ms = now.max(self.last_ms);
        let mut taken = 0;
        while taken < want && self.len > 0 {
            let n = (PACKET_SAMPLES - self.read).min(want - taken);
            self.read += n;
            taken += n;
            if self.read == PACKET_SAMPLES {
                self.pop();
            }
        }
        if taken < want && self.underflow_since.is_none() {
            // It ran empty inside this period, at the moment its last sample went out.
            let ran_out = now - (want - taken) / SAMPLES_PER_MS;
            self.underflow_since = Some(ran_out);
        }
        taken
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(seq: u32) -> [u8; PACKET_LEN] {
        let mut p = [0u8; PACKET_LEN];
        p[..4].copy_from_slice(&seq.to_be_bytes());
        p[4] = seq as u8;
        p
    }

    #[test]
    fn unkeyed_keeps_at_most_20_packets_of_the_last_25_ms_t_a3_26() {
        let mut f: DucFifo<20> = DucFifo::new();
        // A host streaming without pause, one packet every 1.25 ms, for 10 s.
        let mut t_us = 0u64;
        let mut seq = 0;
        while t_us < 10_000_000 {
            f.push(&pkt(seq), t_us / 1000);
            assert!(f.len_samples() <= 4800);
            seq += 1;
            t_us += 1250;
        }
        let now = t_us / 1000;
        f.idle(now);
        assert!(f.oldest_ms().is_some_and(|o| now - o < 25), "first sample at the key at most 25 ms old");
        assert!(f.dropped_old > 7000);
        // A pause: everything ages out.
        f.idle(now + 25);
        assert_eq!(f.len_packets(), 0);
    }

    #[test]
    fn keyed_full_fifo_discards_whole_packets_and_every_unkey_empties_it() {
        let mut f: DucFifo<20> = DucFifo::new();
        f.key(0, true);
        for s in 0..25 {
            f.push(&pkt(s), 0);
        }
        assert_eq!((f.len_packets(), f.dropped_full), (20, 5));
        f.unkey();
        assert_eq!(f.len_packets(), 0);
        assert!(f.underflow_since().is_none());
    }

    #[test]
    fn underflow_starts_at_the_key_if_empty_or_when_it_runs_empty_and_ends_with_a_packet() {
        let mut f: DucFifo<20> = DucFifo::new();
        f.key(100, true);
        assert_eq!(f.underflow_since(), Some(100));
        f.push(&pkt(0), 101);
        assert_eq!(f.underflow_since(), None);
        f.push(&pkt(1), 101);
        // 480 samples = 2.5 ms at 192 ksps; consumed by 102.5 ms.
        assert_eq!(f.consume(102), 384);
        assert_eq!(f.underflow_since(), None);
        assert_eq!(f.consume(104), 96);
        assert_eq!(f.underflow_since(), Some(103), "ran empty at 102.5 ms");
        // While the CW keyer forms the RF, nothing enters and there is no underflow.
        let mut f: DucFifo<20> = DucFifo::new();
        f.push(&pkt(0), 0);
        f.key(1, false);
        assert_eq!(f.len_packets(), 0);
        assert!(!f.push(&pkt(1), 2));
        assert_eq!(f.dropped_cw, 1);
        assert_eq!(f.consume(50), 0);
        assert!(f.underflow_since().is_none());
    }
}
