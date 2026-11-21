// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! IPv4 link-local address claim, RFC 3927 sections 2.1, 2.2.1, 2.4 and 9 (spec A1.11; 0048):
//! probe a candidate three times, announce it twice, and take the next candidate on a conflict.
//! The board sends and receives the ARP frames on CH394 socket 0 in MACRAW mode, before any P2
//! socket is open, with the chip's own address at 0.0.0.0 (it then answers no ARP itself).
//!
//! Defence while the address is in use (section 2.5) is not here: the board takes option (a)
//! on the chip's IP_CONFLI interrupt, pending the spec's answer (Q2).

use crate::netcfg::{self, NetConfig};

/// RFC 3927 section 9, ms.
pub const PROBE_WAIT_MS: u64 = 1000;
pub const PROBE_NUM: u8 = 3;
pub const PROBE_MIN_MS: u64 = 1000;
pub const PROBE_MAX_MS: u64 = 2000;
pub const ANNOUNCE_WAIT_MS: u64 = 2000;
pub const ANNOUNCE_NUM: u8 = 2;
pub const ANNOUNCE_INTERVAL_MS: u64 = 2000;
pub const MAX_CONFLICTS: u16 = 10;
pub const RATE_LIMIT_INTERVAL_MS: u64 = 60_000;

/// An ARP request in an Ethernet frame, padded to the 60-byte minimum (without FCS).
pub const FRAME_LEN: usize = 60;
const ETH_ARP: [u8; 2] = [0x08, 0x06];

/// ARP request from `mac`: sender `spa`, target `tpa`, to the broadcast address (RFC 826; RFC
/// 3927 2.2.1: a probe has sender IP 0.0.0.0, an announcement has sender = target = the address).
pub fn arp_request(mac: [u8; 6], spa: [u8; 4], tpa: [u8; 4]) -> [u8; FRAME_LEN] {
    let mut f = [0u8; FRAME_LEN];
    f[0..6].copy_from_slice(&[0xFF; 6]);
    f[6..12].copy_from_slice(&mac);
    f[12..14].copy_from_slice(&ETH_ARP);
    f[14..22].copy_from_slice(&[0x00, 0x01, 0x08, 0x00, 6, 4, 0x00, 0x01]); // Ethernet, IPv4, request
    f[22..28].copy_from_slice(&mac);
    f[28..32].copy_from_slice(&spa);
    // target hardware address 32..38 stays 0 (2.2.1)
    f[38..42].copy_from_slice(&tpa);
    f
}

/// The fields of a received ARP frame that the claim needs: sender MAC, sender IP, target IP.
pub fn arp_parse(f: &[u8]) -> Option<([u8; 6], [u8; 4], [u8; 4])> {
    if f.len() < 42 || f[12..14] != ETH_ARP || f[14..20] != [0x00, 0x01, 0x08, 0x00, 6, 4] {
        return None;
    }
    let sha = f[22..28].try_into().ok()?;
    let spa = f[28..32].try_into().ok()?;
    let tpa = f[38..42].try_into().ok()?;
    Some((sha, spa, tpa))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// `sent` probes so far; the next at `next_ms`.
    Probe { sent: u8 },
    /// Waiting ANNOUNCE_WAIT after the last probe, then `sent` announcements.
    Announce { sent: u8 },
    Claimed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// Send this frame now.
    Send([u8; FRAME_LEN]),
    /// The address is claimed (probed and announced): configure it.
    Claimed(NetConfig),
}

pub struct Claim {
    mac: [u8; 6],
    attempt: u32,
    conflicts: u16,
    cand: NetConfig,
    phase: Phase,
    next_ms: u64,
    rng: u32,
}

impl Claim {
    /// Start with the first candidate of this MAC (2.1: the same address at every start, as long
    /// as nobody else holds it). The first probe comes after a random 0-1 s (PROBE_WAIT).
    pub fn new(mac: [u8; 6], now: u64) -> Self {
        let rng = u32::from_be_bytes([mac[2], mac[3], mac[4], mac[5]]) ^ 0x9E37_79B9;
        let mut c = Claim { mac, attempt: 0, conflicts: 0, cand: netcfg::link_local(mac, 0), phase: Phase::Probe { sent: 0 }, next_ms: 0, rng };
        c.next_ms = now + c.rand(0, PROBE_WAIT_MS);
        c
    }

    pub fn candidate(&self) -> NetConfig {
        self.cand
    }

    pub fn conflicts(&self) -> u16 {
        self.conflicts
    }

    fn rand(&mut self, lo: u64, hi: u64) -> u64 {
        // xorshift32: enough to spread hosts that start together (2.2.1, random delays).
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        lo + (self.rng as u64) % (hi - lo).max(1)
    }

    /// Call often (every few ms).
    pub fn poll(&mut self, now: u64) -> Action {
        if now < self.next_ms {
            return Action::None;
        }
        match self.phase {
            Phase::Probe { sent } => {
                let f = arp_request(self.mac, [0; 4], self.cand.ip);
                let sent = sent + 1;
                self.phase = Phase::Probe { sent };
                self.next_ms = if sent < PROBE_NUM { now + self.rand(PROBE_MIN_MS, PROBE_MAX_MS) } else { now + ANNOUNCE_WAIT_MS };
                if sent == PROBE_NUM {
                    self.phase = Phase::Announce { sent: 0 };
                }
                Action::Send(f)
            }
            Phase::Announce { sent } if sent < ANNOUNCE_NUM => {
                let f = arp_request(self.mac, self.cand.ip, self.cand.ip);
                self.phase = Phase::Announce { sent: sent + 1 };
                self.next_ms = if sent + 1 < ANNOUNCE_NUM { now + ANNOUNCE_INTERVAL_MS } else { now };
                Action::Send(f)
            }
            Phase::Announce { .. } => {
                self.phase = Phase::Claimed;
                Action::Claimed(self.cand)
            }
            Phase::Claimed => Action::None,
        }
    }

    /// A received frame (all of socket 0's MACRAW traffic). A conflict while probing (2.2.1:
    /// an ARP from another host with the candidate as sender address, or another host's probe
    /// for it) or while announcing (sender address = candidate) gives up the candidate and
    /// starts again with the next one; after MAX_CONFLICTS, one attempt per 60 s (2.2.1, 9).
    pub fn on_frame(&mut self, f: &[u8], now: u64) {
        let Some((sha, spa, tpa)) = arp_parse(f) else { return };
        if sha == self.mac || self.phase == Phase::Claimed {
            return;
        }
        let probing = matches!(self.phase, Phase::Probe { .. });
        let conflict = spa == self.cand.ip || (probing && spa == [0; 4] && tpa == self.cand.ip);
        if conflict {
            self.conflict(now);
        }
    }

    /// Probe the current candidate again (a link-local address chosen again without a conflict:
    /// after a lease end or a DHCPNAK; A1.11, 0049).
    pub fn restart(&mut self, now: u64) {
        self.phase = Phase::Probe { sent: 0 };
        self.next_ms = now + self.rand(0, PROBE_WAIT_MS);
    }

    /// A conflict on the candidate or on the claimed address (the chip's IP_CONFLI while in
    /// use, 2.5 option (a)): give it up, take the next candidate and probe it; after
    /// MAX_CONFLICTS, one attempt per RATE_LIMIT_INTERVAL (2.2.1, 9).
    pub fn conflict(&mut self, now: u64) {
        self.conflicts = self.conflicts.saturating_add(1);
        self.attempt += 1;
        self.cand = netcfg::link_local(self.mac, self.attempt);
        self.phase = Phase::Probe { sent: 0 };
        self.next_ms = if self.conflicts >= MAX_CONFLICTS { now + RATE_LIMIT_INTERVAL_MS } else { now + self.rand(0, PROBE_WAIT_MS) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::vec::Vec;

    const MAC: [u8; 6] = [2, 0, 0, 0, 0x72, 0x50];
    const OTHER: [u8; 6] = [2, 0, 0, 0, 0x11, 0x22];

    /// Run the claim 1 ms at a time; returns (time, frame) of every send and the claim time.
    fn run(c: &mut Claim, from: u64, to: u64, mut inject: impl FnMut(u64, &Claim) -> Option<[u8; FRAME_LEN]>) -> (Vec<(u64, [u8; FRAME_LEN])>, Option<(u64, NetConfig)>) {
        let mut sent = Vec::new();
        for t in from..to {
            if let Some(f) = inject(t, c) {
                c.on_frame(&f, t);
            }
            match c.poll(t) {
                Action::Send(f) => sent.push((t, f)),
                Action::Claimed(n) => return (sent, Some((t, n))),
                Action::None => {}
            }
        }
        (sent, None)
    }

    #[test]
    fn three_probes_then_two_announcements_within_the_section_9_times() {
        let mut c = Claim::new(MAC, 0);
        let cand = c.candidate().ip;
        let (sent, claimed) = run(&mut c, 0, 20_000, |_, _| None);
        assert_eq!(sent.len(), 5);
        // Probes: sender IP 0.0.0.0, target the candidate, from our MAC, broadcast.
        for (_, f) in &sent[..3] {
            assert_eq!(arp_parse(f), Some((MAC, [0; 4], cand)));
            assert_eq!(f[0..6], [0xFF; 6]);
        }
        // Announcements: sender = target = the candidate.
        for (_, f) in &sent[3..] {
            assert_eq!(arp_parse(f), Some((MAC, cand, cand)));
        }
        let t: Vec<u64> = sent.iter().map(|s| s.0).collect();
        assert!(t[0] < PROBE_WAIT_MS);
        assert!((1000..2000).contains(&(t[1] - t[0])) && (1000..2000).contains(&(t[2] - t[1])));
        assert_eq!(t[3] - t[2], ANNOUNCE_WAIT_MS);
        assert_eq!(t[4] - t[3], ANNOUNCE_INTERVAL_MS);
        let (tc, n) = claimed.unwrap();
        assert_eq!(n.ip, cand);
        assert!(tc <= 1000 + 2000 + 2000 + 2000 + 2000, "at most 9 s (0048: 15 s with 6 s of DHCP)");
        // 169.254/16, not the first or last 256 (2.1).
        assert!(cand[0] == 169 && cand[1] == 254 && (1..=254).contains(&cand[2]));
    }

    #[test]
    fn a_conflict_while_probing_or_announcing_takes_the_next_candidate() {
        // Another host answers for the candidate after the first probe.
        let mut c = Claim::new(MAC, 0);
        let first = c.candidate().ip;
        let mut hit = false;
        let (_, claimed) = run(&mut c, 0, 30_000, |t, c| {
            (!hit && t == 1500 && c.candidate().ip == first).then(|| {
                hit = true;
                let mut f = arp_request(OTHER, first, [169, 254, 0, 1]);
                f[21] = 2; // a reply
                f
            })
        });
        assert_eq!(c.conflicts(), 1);
        assert_ne!(claimed.unwrap().1.ip, first);
        // Another host probing for the same candidate counts too (2.2.1).
        let mut c = Claim::new(MAC, 0);
        let first = c.candidate().ip;
        c.on_frame(&arp_request(OTHER, [0; 4], first), 10);
        assert_ne!(c.candidate().ip, first);
        // Our own frames and other addresses do not.
        let mut c = Claim::new(MAC, 0);
        let first = c.candidate().ip;
        c.on_frame(&arp_request(MAC, [0; 4], first), 10);
        c.on_frame(&arp_request(OTHER, [169, 254, 9, 9], [169, 254, 9, 10]), 10);
        assert_eq!(c.candidate().ip, first);
        // While announcing: an ARP with the candidate as sender address.
        let mut c = Claim::new(MAC, 0);
        let first = c.candidate().ip;
        let (_, claimed) = run(&mut c, 0, 30_000, |t, c| (t == 7000 && c.candidate().ip == first).then(|| arp_request(OTHER, first, first)));
        assert_ne!(claimed.unwrap().1.ip, first);
    }

    #[test]
    fn after_ten_conflicts_one_attempt_per_minute() {
        let mut c = Claim::new(MAC, 0);
        for i in 0..MAX_CONFLICTS {
            let cand = c.candidate().ip;
            c.on_frame(&arp_request(OTHER, cand, cand), 100 + i as u64);
        }
        let t0 = 100 + MAX_CONFLICTS as u64 - 1;
        let (sent, _) = run(&mut c, t0, t0 + 70_000, |_, _| None);
        assert!(sent[0].0 >= t0 + RATE_LIMIT_INTERVAL_MS);
    }
}
