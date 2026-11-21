// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Network configuration C-1 (spec A1.6): static or DHCP; address, netmask, gateway.
//!
//! The factory default is TBD in the spec (Table A1-4 C-1, HIS-14). Until it is fixed, this
//! firmware uses DHCP at power-on (A1.11: "the DHCP exchange at power-on runs before the P2
//! sockets are opened") and, without a lease, an IPv4 link-local address (RFC 3927) derived from
//! the MAC. Spec question SQ-F2-1.

pub type Ip = [u8; 4];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Static,
    Dhcp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetConfig {
    pub mode: Mode,
    pub ip: Ip,
    pub mask: Ip,
    pub gateway: Ip,
}

/// Where the address in use came from (logged; A1.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Static,
    Dhcp,
    LinkLocal,
    SetIp,
}

impl NetConfig {
    /// This firmware's interim factory default (see module doc).
    pub const FACTORY: NetConfig = NetConfig { mode: Mode::Dhcp, ip: [0; 4], mask: [0; 4], gateway: [0; 4] };
}

pub fn u32_of(ip: Ip) -> u32 {
    u32::from_be_bytes(ip)
}

pub fn ip_of(v: u32) -> Ip {
    v.to_be_bytes()
}

/// Directed broadcast address of the subnet (B3.2: discovery to it MUST be answered).
pub fn directed_broadcast(ip: Ip, mask: Ip) -> Ip {
    ip_of(u32_of(ip) | !u32_of(mask))
}

/// True if `other` lies in the subnet of `ip`/`mask` (B3.9 Set IP to a unicast or directed
/// broadcast address is accepted only from inside the subnet).
pub fn in_subnet(ip: Ip, mask: Ip, other: Ip) -> bool {
    let m = u32_of(mask);
    u32_of(ip) & m == u32_of(other) & m
}

/// A usable host netmask: contiguous ones from the top, not /0, /31 or /32.
pub fn valid_mask(mask: Ip) -> bool {
    let m = u32_of(mask);
    let ones = m.leading_ones();
    ones >= 1 && ones <= 30 && m.count_ones() == ones
}

/// A usable unicast host address in its subnet.
pub fn valid_host(ip: Ip, mask: Ip) -> bool {
    if !valid_mask(mask) {
        return false;
    }
    let v = u32_of(ip);
    let host = v & !u32_of(mask);
    let first = ip[0];
    host != 0 && host != !u32_of(mask) && first != 0 && first != 127 && first < 224
}

/// RFC 3927 link-local address 169.254.1.0-169.254.254.255 from the MAC (section 2.1: a
/// pseudo-random number generator seeded from the MAC), mask 255.255.0.0. `attempt` selects the
/// next candidate after a conflict.
pub fn link_local(mac: [u8; 6], attempt: u32) -> NetConfig {
    // FNV-1a over the MAC and the attempt number.
    let mut h: u32 = 0x811C_9DC5;
    for b in mac.iter().chain(attempt.to_le_bytes().iter()) {
        h ^= *b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    let n = h % (254 * 256 - 256); // 0 .. 65023
    let v = 0xA9FE_0100 + n; // 169.254.1.0 + n, at most 169.254.254.255
    NetConfig { mode: Mode::Static, ip: ip_of(v), mask: [255, 255, 0, 0], gateway: [0; 4] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broadcast_and_subnet() {
        assert_eq!(directed_broadcast([192, 168, 1, 20], [255, 255, 255, 0]), [192, 168, 1, 255]);
        assert!(in_subnet([192, 168, 1, 20], [255, 255, 255, 0], [192, 168, 1, 7]));
        assert!(!in_subnet([192, 168, 1, 20], [255, 255, 255, 0], [192, 168, 2, 7]));
    }

    #[test]
    fn masks_and_hosts() {
        assert!(valid_mask([255, 255, 255, 0]));
        assert!(!valid_mask([255, 0, 255, 0]));
        assert!(!valid_mask([255, 255, 255, 255]));
        assert!(valid_host([192, 168, 1, 20], [255, 255, 255, 0]));
        assert!(!valid_host([192, 168, 1, 0], [255, 255, 255, 0]));
        assert!(!valid_host([192, 168, 1, 255], [255, 255, 255, 0]));
        assert!(!valid_host([127, 0, 0, 1], [255, 0, 0, 0]));
    }

    #[test]
    fn link_local_range() {
        for a in 0..2000 {
            let c = link_local([0x02, 1, 2, 3, 4, 5], a);
            assert_eq!(&c.ip[..2], &[169, 254]);
            assert!(c.ip[2] >= 1 && c.ip[2] <= 254, "{:?}", c.ip);
            assert!(valid_host(c.ip, c.mask));
        }
        assert_ne!(link_local([2, 1, 2, 3, 4, 5], 0).ip, link_local([2, 1, 2, 3, 4, 6], 0).ip);
    }
}
