// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! G7250-P2 session, ownership and watchdog (spec B5; A2.2-A2.4; 0007 sub-decisions 2, 3).
//!
//! C&C packets are General (port 1024, byte 4 = 0x00, at least 60 bytes), DDC-specific (1025,
//! 1444), DUC-specific (1026, 60) and High Priority (1027, 1444) (B5.6; B11.3). The session
//! starts with the run bit (High Priority byte 4 bit [0]) while none runs; the owner is the
//! source IP address and UDP port (B5.5). Memory-mapped packets are not C&C (B4.5, B5.6).

pub type Addr = ([u8; 4], u16);

/// Which C&C packet.
use g7250_safety::cw::{Clamped, DucSettings};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CnC {
    General,
    DdcSpecific,
    DucSpecific,
    HighPriority,
}

impl CnC {
    /// Minimum payload length (Table B11-1); shorter packets are discarded whole (B11.3).
    pub const fn min_len(self) -> usize {
        match self {
            CnC::General | CnC::DucSpecific => 60,
            CnC::DdcSpecific | CnC::HighPriority => 1444,
        }
    }
}

/// The General packet fields this firmware acts on now (Table B7-1; B5.1a item 2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct General {
    /// Bytes 31-32: host port for register packets; 0 = the owner's source port (B8.1).
    pub reg_port: u16,
    /// Byte 37 bit [3]: phase word (B3.8).
    pub phase_word: bool,
    /// Byte 37 bits [0], [1]: time stamp, VITA-49 requested (B6.6); bit [2]: VNA mode (Table B7-1).
    pub timestamp: bool,
    pub vita: bool,
    pub vna: bool,
    /// Byte 38 bit [0]: hardware timer (B5.6).
    pub timer: bool,
    /// Byte 39: data format; only 0 (Big-Endian, 3-byte I&Q) is offered (B3.7).
    pub format: u8,
    /// Byte 58 bit [0]: PA enable (B10.10).
    pub pa_enable: bool,
    /// A G7250 local port in bytes 5-12, 15-22 or 29-30 that is neither zero nor its default:
    /// not honoured, logged, `LINK_STATUS` bit 17 (B2.3, Table B7-1; 0020 §13).
    pub foreign_port: bool,
}

/// The G7250 local port fields of the General packet and their defaults (Table B2-1, B7-1).
pub const GENERAL_PORTS: [(usize, u16); 9] =
    [(5, 1025), (7, 1026), (9, 1027), (11, 1025), (15, 1029), (17, 1035), (19, 1026), (21, 1027), (29, 7250)];

impl General {
    pub fn parse(p: &[u8]) -> Self {
        General {
            reg_port: u16::from_be_bytes([p[31], p[32]]),
            phase_word: p[37] & 0x08 != 0,
            timestamp: p[37] & 0x01 != 0,
            vita: p[37] & 0x02 != 0,
            vna: p[37] & 0x04 != 0,
            timer: p[38] & 0x01 != 0,
            format: p[39],
            pa_enable: p[58] & 0x01 != 0,
            foreign_port: GENERAL_PORTS.iter().any(|&(at, def)| {
                let v = u16::from_be_bytes([p[at], p[at + 1]]);
                v != 0 && v != def
            }),
        }
    }
}

/// The DDC-specific packet fields for DDC0 (Table B6-2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ddc {
    /// Byte 7 bit [0].
    pub enable: bool,
    /// Bytes 18-19, ksps.
    pub rate_ksps: u16,
    /// Byte 22, bits per sample; 0 is read as 24.
    pub size: u8,
}

impl Ddc {
    pub fn parse(p: &[u8]) -> Self {
        Ddc { enable: p[7] & 1 != 0, rate_ksps: u16::from_be_bytes([p[18], p[19]]), size: p[22] }
    }
}

/// Why a session ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndReason {
    /// Run bit cleared by the owner (B5.4): counters back to 0.
    Stop,
    /// No C&C from the owner for 1 s (B5.6).
    Watchdog,
    /// C&C from another host (B5.7); the new owner is in `Event::Started`.
    Takeover,
    /// Link down or below 100 Mbit/s (B5.8).
    LinkDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A session started. `restart`: the same host as the last owner after a stop, watchdog or
    /// link loss, with its RX-side settings carried over (B5.4 item 1). `takeover`: it took the
    /// radio from a running owner (B5.7).
    Started { owner: Addr, restart: bool, takeover: bool },
    Ended { owner: Addr, reason: EndReason },
    /// A run bit on a link below 100 Mbit/s: no session; answer with `LINK_STATUS` and
    /// `STREAM_STATUS` (B5.8).
    Refused { from: Addr },
}

/// Packets kept in the set-up phase, per source (B5.1a item 1).
#[derive(Clone, Copy, Debug)]
struct Kept {
    from: Addr,
    heard_ms: u64,
    general: Option<General>,
    ddc: Option<Ddc>,
    /// The last DUC-specific packet, and whether its hang delay was clamped (B10.6).
    duc: Option<(DucSettings, Clamped)>,
}

const KEPT: usize = 4;
/// Watchdog: standby after 1 s without C&C from the owner; tolerance 1000-1100 ms (B5.6).
pub const WATCHDOG_MS: u64 = 1000;

pub struct Session {
    owner: Option<Addr>,
    last_cnc_ms: u64,
    kept: [Option<Kept>; KEPT],
    /// The last owner whose session ended by stop, watchdog or link loss, with its General
    /// fields (restart, B5.4 item 1). Cleared by a reset or a change of address.
    last_owner: Option<(Addr, Option<General>, Option<Ddc>)>,
    /// General fields in force for the current session.
    pub general: Option<General>,
    /// DDC0 fields in force for the current session; `None` = no DDC-specific packet (B5.2).
    pub ddc: Option<Ddc>,
    /// The new owner's DUC-specific packet from its set-up phase, for the key path to take at
    /// the session start (B5.1a item 2). Never carried over from an ended session (A3.6a).
    pub duc_setup: Option<(DucSettings, Clamped)>,
    /// The owner's High Priority byte 4 bit [1] (PTT) as last received (B10.2 item 3).
    pub ptt: bool,
    /// Count of C&C packets discarded as short (B11.3).
    pub short_discarded: u64,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub const fn new() -> Self {
        Session { owner: None, last_cnc_ms: 0, kept: [None; KEPT], last_owner: None, general: None, ddc: None, duc_setup: None, ptt: false, short_discarded: 0 }
    }

    pub fn owner(&self) -> Option<Addr> {
        self.owner
    }

    pub fn running(&self) -> bool {
        self.owner.is_some()
    }

    /// Discovery byte 4 (B3.4): 0x03 while running and the request is not from the owner.
    pub fn discovery_in_use(&self, from: Addr) -> bool {
        self.owner.is_some_and(|o| o != from)
    }

    /// Where register packets for the owner go (B8.1): General bytes 31-32 if non-zero, else
    /// the owner's source port.
    pub fn register_destination(&self) -> Option<Addr> {
        let o = self.owner?;
        match self.general {
            Some(g) if g.reg_port != 0 => Some((o.0, g.reg_port)),
            _ => Some(o),
        }
    }

    fn keep(&mut self, from: Addr, kind: CnC, p: &[u8], now: u64) {
        let i = self.kept.iter().position(|k| k.is_some_and(|k| k.from == from)).unwrap_or_else(|| {
            self.kept.iter().position(|k| k.is_none()).unwrap_or_else(|| {
                // Evict the least recently heard (B5.1a item 1).
                (0..KEPT).min_by_key(|&i| self.kept[i].map_or(0, |k| k.heard_ms)).unwrap()
            })
        });
        let mut k = match self.kept[i] {
            Some(k) if k.from == from => k,
            _ => Kept { from, heard_ms: now, general: None, ddc: None, duc: None },
        };
        k.heard_ms = now;
        match kind {
            CnC::General => k.general = Some(General::parse(p)),
            CnC::DdcSpecific => k.ddc = Some(Ddc::parse(p)),
            CnC::DucSpecific => k.duc = Some(DucSettings::parse(p)),
            CnC::HighPriority => {}
        }
        self.kept[i] = Some(k);
    }

    fn take_kept(&mut self, from: Addr) -> Option<Kept> {
        let k = self.kept.iter().flatten().find(|k| k.from == from).copied();
        // Kept packets of every other source are discarded when a session starts (item 3).
        self.kept = [None; KEPT];
        k
    }

    /// A C&C packet from `from`. `link_ok`: link up at 100 Mbit/s or more (B5.8). Returns the
    /// events in order (at most two: an end, then a start).
    pub fn on_cnc(&mut self, from: Addr, kind: CnC, p: &[u8], now: u64, link_ok: bool) -> heapless::Vec<Event, 2> {
        let mut ev = heapless::Vec::new();
        if p.len() < kind.min_len() {
            self.short_discarded += 1;
            return ev;
        }
        let run = kind == CnC::HighPriority && p[4] & 0x01 != 0;
        match self.owner {
            Some(o) if o == from => {
                self.last_cnc_ms = now;
                match kind {
                    CnC::General => self.general = Some(General::parse(p)),
                    CnC::DdcSpecific => self.ddc = Some(Ddc::parse(p)),
                    CnC::HighPriority if !run => {
                        self.end(EndReason::Stop, &mut ev);
                    }
                    CnC::HighPriority => self.ptt = p[4] & 0x02 != 0,
                    _ => {}
                }
            }
            Some(_) => {
                // Takeover by any C&C packet of another host (B5.7); the packet counts for the
                // new session (A2.4 step 5).
                self.end(EndReason::Takeover, &mut ev);
                self.start(from, kind, p, now, false, true, &mut ev);
            }
            None => {
                if run {
                    if !link_ok {
                        let _ = ev.push(Event::Refused { from });
                        return ev;
                    }
                    let restart = self.last_owner.is_some_and(|(a, _, _)| a == from);
                    self.start(from, kind, p, now, restart, false, &mut ev);
                } else {
                    // Set-up phase (B5.1a).
                    self.keep(from, kind, p, now);
                }
            }
        }
        ev
    }

    #[allow(clippy::too_many_arguments)]
    fn start(&mut self, from: Addr, kind: CnC, p: &[u8], now: u64, restart: bool, takeover: bool, ev: &mut heapless::Vec<Event, 2>) {
        let kept = self.take_kept(from);
        // Carried over from the same host's ended session (B5.4 item 1), except byte 38 bit [0]
        // and byte 58 bit [0]; then the kept set-up packets on top (B5.1a item 2).
        let mut general = None;
        let mut ddc = None;
        self.duc_setup = None;
        if restart {
            if let Some((_, g, d)) = self.last_owner {
                general = g.map(|g| General { timer: false, pa_enable: false, ..g });
                ddc = d;
            }
        }
        if let Some(k) = kept {
            if let Some(g) = k.general {
                general = Some(g);
            }
            if let Some(d) = k.ddc {
                ddc = Some(d);
            }
            self.duc_setup = k.duc;
        }
        match kind {
            CnC::General => general = Some(General::parse(p)),
            CnC::DdcSpecific => ddc = Some(Ddc::parse(p)),
            _ => {}
        }
        self.general = general;
        self.ddc = ddc;
        self.ptt = kind == CnC::HighPriority && p[4] & 0x02 != 0;
        self.owner = Some(from);
        self.last_cnc_ms = now;
        let _ = ev.push(Event::Started { owner: from, restart, takeover });
    }

    fn end(&mut self, reason: EndReason, ev: &mut heapless::Vec<Event, 2>) {
        if let Some(o) = self.owner.take() {
            self.last_owner = match reason {
                EndReason::Stop | EndReason::Watchdog | EndReason::LinkDown => Some((o, self.general, self.ddc)),
                EndReason::Takeover => None,
            };
            self.general = None;
            self.ddc = None;
            self.ptt = false;
            let _ = ev.push(Event::Ended { owner: o, reason });
        }
    }

    /// Call often (every few ms). `timer_effective`: General byte 38 bit [0] OR armed, enabled
    /// before any General packet (B5.6).
    pub fn tick(&mut self, now: u64, timer_effective: bool, link_ok: bool) -> Option<Event> {
        let mut ev = heapless::Vec::<Event, 2>::new();
        if self.owner.is_some() && !link_ok {
            self.end(EndReason::LinkDown, &mut ev);
        } else if self.owner.is_some() && timer_effective && now.saturating_sub(self.last_cnc_ms) >= WATCHDOG_MS {
            self.end(EndReason::Watchdog, &mut ev);
        }
        ev.first().copied()
    }

    /// The radio's address changed, or any other reason that makes the network session not the
    /// same (B5.4 item 3): nothing is carried over.
    pub fn forget(&mut self) {
        self.last_owner = None;
        self.kept = [None; KEPT];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Addr = ([192, 168, 1, 5], 50000);
    const B: Addr = ([192, 168, 1, 6], 50001);

    fn hp(run: bool) -> std::vec::Vec<u8> {
        let mut p = std::vec![0u8; 1444];
        p[4] = run as u8;
        p
    }
    fn general(timer: bool, reg_port: u16) -> std::vec::Vec<u8> {
        let mut p = std::vec![0u8; 60];
        p[38] = timer as u8;
        p[31..33].copy_from_slice(&reg_port.to_be_bytes());
        p
    }

    #[test]
    fn set_up_duc_specific_and_pa_enable_reach_the_start_and_are_never_carried_over() {
        let mut s = Session::new();
        let mut d = std::vec![0u8; 60];
        d[5] = 0x80;
        d[9] = 20;
        let mut g = general(true, 0);
        g[58] = 1;
        s.on_cnc(A, CnC::DucSpecific, &d, 0, true);
        s.on_cnc(A, CnC::General, &g, 0, true);
        s.on_cnc(A, CnC::HighPriority, &hp(true), 10, true);
        let (duc, clamped) = s.duc_setup.unwrap();
        assert!(duc.break_in && duc.speed_wpm == 20 && !clamped.any());
        assert!(s.general.unwrap().pa_enable, "set-up General of the new owner applies (B5.1a)");
        // Stop and restart by the same host: General carried over without PA enable, no DUC.
        s.on_cnc(A, CnC::HighPriority, &hp(false), 20, true);
        s.on_cnc(A, CnC::HighPriority, &hp(true), 30, true);
        assert!(s.duc_setup.is_none());
        assert!(!s.general.unwrap().pa_enable);
        // Another host starts with a High Priority packet alone: no General, so PA enable stays
        // cleared (A3.6a); a takeover takes the same path through start().
        s.on_cnc(A, CnC::HighPriority, &hp(false), 40, true);
        let e = s.on_cnc(B, CnC::HighPriority, &hp(true), 50, true);
        assert!(matches!(e.as_slice(), [Event::Started { owner: B, .. }]));
        assert!(s.general.is_none_or(|g| !g.pa_enable) && s.duc_setup.is_none());
    }

    #[test]
    fn a_non_default_radio_port_is_flagged_zero_and_the_default_are_not() {
        // Zero everywhere, then every field at its default: honoured.
        let mut p = general(false, 0);
        assert!(!General::parse(&p).foreign_port);
        for &(at, def) in GENERAL_PORTS.iter() {
            p[at..at + 2].copy_from_slice(&def.to_be_bytes());
        }
        assert!(!General::parse(&p).foreign_port);
        // Any one field moved: not honoured (B2.3; T-B7-03), the management port included.
        for &(at, _) in GENERAL_PORTS.iter() {
            for port in [1030u16, 7251] {
                let mut q = general(false, 0);
                q[at..at + 2].copy_from_slice(&port.to_be_bytes());
                assert!(General::parse(&q).foreign_port, "byte {at} port {port}");
            }
        }
        // Bytes 13-14 (DDC audio, ignored) and 31-32 (host destination port) are not radio ports.
        let mut q = general(false, 50002);
        q[13..15].copy_from_slice(&1234u16.to_be_bytes());
        assert!(!General::parse(&q).foreign_port);
    }

    #[test]
    fn run_starts_stop_ends_and_short_packets_are_discarded() {
        let mut s = Session::new();
        assert!(s.on_cnc(A, CnC::HighPriority, &hp(true)[..1443], 0, true).is_empty());
        assert_eq!(s.short_discarded, 1);
        assert!(!s.running());
        let e = s.on_cnc(A, CnC::HighPriority, &hp(true), 0, true);
        assert_eq!(e.as_slice(), &[Event::Started { owner: A, restart: false, takeover: false }]);
        assert!(!s.discovery_in_use(A));
        assert!(s.discovery_in_use(B));
        assert!(s.discovery_in_use((A.0, 1234))); // same IP, other port
        let e = s.on_cnc(A, CnC::HighPriority, &hp(false), 10, true);
        assert_eq!(e.as_slice(), &[Event::Ended { owner: A, reason: EndReason::Stop }]);
        assert!(!s.running());
    }

    #[test]
    fn setup_phase_counts_for_its_own_host_only() {
        let mut s = Session::new();
        s.on_cnc(A, CnC::General, &general(true, 7000), 0, true);
        s.on_cnc(B, CnC::General, &general(false, 9000), 0, true);
        assert!(!s.running());
        s.on_cnc(A, CnC::HighPriority, &hp(true), 5, true);
        assert_eq!(s.general.unwrap().reg_port, 7000);
        assert!(s.general.unwrap().timer);
        assert_eq!(s.register_destination(), Some((A.0, 7000)));
    }

    #[test]
    fn takeover_by_any_cnc_from_another_host() {
        let mut s = Session::new();
        s.on_cnc(A, CnC::HighPriority, &hp(true), 0, true);
        let e = s.on_cnc(B, CnC::DucSpecific, &[0u8; 60], 10, true);
        assert_eq!(
            e.as_slice(),
            &[Event::Ended { owner: A, reason: EndReason::Takeover }, Event::Started { owner: B, restart: false, takeover: true }]
        );
        assert_eq!(s.owner(), Some(B));
        // The owner's IP on a new port is a takeover too (0007 Resolved 4).
        let e = s.on_cnc((B.0, 1), CnC::General, &general(true, 0), 20, true);
        assert_eq!(e.len(), 2);
    }

    #[test]
    fn watchdog_1s_only_when_effective() {
        let mut s = Session::new();
        s.on_cnc(A, CnC::HighPriority, &hp(true), 0, true);
        assert_eq!(s.tick(999, true, true), None);
        s.on_cnc(A, CnC::General, &general(true, 0), 500, true); // feeds
        assert_eq!(s.tick(1499, true, true), None);
        assert_eq!(s.tick(1500, true, true), Some(Event::Ended { owner: A, reason: EndReason::Watchdog }));
        // Timer disabled by the host (not armed): no watchdog standby.
        let mut s = Session::new();
        s.on_cnc(A, CnC::HighPriority, &hp(true), 0, true);
        assert_eq!(s.tick(5000, false, true), None);
        // Packets of other hosts never feed it: they take over instead.
    }

    #[test]
    fn another_port_of_the_owner_ip_is_another_host() {
        // B3.4 and T-B8-06/11: the host is IP and port; the owner's IP from another port is
        // another host while the session runs.
        let mut s = Session::new();
        s.on_cnc(A, CnC::HighPriority, &hp(true), 0, true);
        assert!(!s.discovery_in_use(A));
        assert!(s.discovery_in_use((A.0, A.1 + 1)));
        // Still running 999 ms after the last C&C with the timer on; ended at 1000 ms.
        assert!(s.tick(999, true, true).is_none());
        assert!(s.discovery_in_use((A.0, A.1 + 1)));
        assert!(s.tick(1000, true, true).is_some());
        assert!(!s.discovery_in_use((A.0, A.1 + 1)));
    }

    #[test]
    fn restart_by_the_same_host_carries_rx_settings_not_the_timer() {
        let mut s = Session::new();
        s.on_cnc(A, CnC::General, &general(true, 7000), 0, true);
        s.on_cnc(A, CnC::HighPriority, &hp(true), 1, true);
        s.tick(2000, true, true); // watchdog standby
        let e = s.on_cnc(A, CnC::HighPriority, &hp(true), 3000, true);
        assert_eq!(e.as_slice(), &[Event::Started { owner: A, restart: true, takeover: false }]);
        let g = s.general.unwrap();
        assert_eq!(g.reg_port, 7000);
        assert!(!g.timer, "byte 38 bit 0 is not carried over (B5.4 item 2)");
        // Another host after that end gets nothing carried over.
        s.on_cnc(A, CnC::HighPriority, &hp(false), 3100, true);
        let e = s.on_cnc(B, CnC::HighPriority, &hp(true), 3200, true);
        assert_eq!(e.as_slice(), &[Event::Started { owner: B, restart: false, takeover: false }]);
        assert_eq!(s.general, None);
    }

    #[test]
    fn slow_link_refuses_and_link_loss_ends() {
        let mut s = Session::new();
        assert_eq!(s.on_cnc(A, CnC::HighPriority, &hp(true), 0, false).as_slice(), &[Event::Refused { from: A }]);
        assert!(!s.running());
        s.on_cnc(A, CnC::HighPriority, &hp(true), 0, true);
        assert_eq!(s.tick(10, true, false), Some(Event::Ended { owner: A, reason: EndReason::LinkDown }));
    }
}
