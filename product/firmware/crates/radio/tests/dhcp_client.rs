// SPDX-License-Identifier: GPL-3.0-or-later
//! DHCP client (A1.11, A1.11a; RFC 2131).
use g7250_proto::dhcp;
use g7250_proto::netcfg::{self, Mode, NetConfig};
use g7250_radio::dhcp_client::{Action, BORROW_MS, BORROW_TARGET_MS, Client, KEEP_STATIC_MS, Msg, RETURN_MS, State, WAIT_MS};

const MAC: [u8; 6] = [2, 0, 0, 0, 0x72, 0x50];
const SERVER: [u8; 4] = [192, 168, 1, 1];

fn reply(t: u8, xid: u32, ip: [u8; 4], lease_s: u32) -> Vec<u8> {
    let mut b = vec![0u8; 240];
    b[0] = 2;
    b[1] = 1;
    b[2] = 6;
    b[4..8].copy_from_slice(&xid.to_be_bytes());
    b[16..20].copy_from_slice(&ip);
    b[28..34].copy_from_slice(&MAC);
    b[236..240].copy_from_slice(&[99, 130, 83, 99]);
    let l = lease_s.to_be_bytes();
    b.extend_from_slice(&[53, 1, t, 1, 4, 255, 255, 255, 0, 3, 4, 192, 168, 1, 1, 51, 4, l[0], l[1], l[2], l[3], 54, 4, 192, 168, 1, 1, 255]);
    b
}

fn ll() -> NetConfig {
    netcfg::link_local(MAC, 0)
}

#[test]
fn no_server_at_power_on_keeps_trying_with_backoff() {
    let mut c = Client::new(MAC, 100, ll(), None, 0);
    assert_eq!(c.poll(3_999), Action::None);
    let mut sends = vec![];
    let mut t = 0;
    while t < 200_000 {
        if let Action::Send { msg: Msg::Discover, to } = c.poll(t) {
            assert_eq!(to, [255; 4]);
            sends.push(t);
        }
        t += 20;
    }
    // 4, then the wait +8, the wait +16, the wait +32, the wait +64, the wait +64 ...
    assert_eq!(sends[0], 4_000);
    assert_eq!(sends[1], 4_000 + WAIT_MS + 8_000);
    assert_eq!(sends[2], sends[1] + WAIT_MS + 16_000);
    let gaps: Vec<u64> = sends.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps.iter().all(|&g| g <= WAIT_MS + 64_000));
    // A server appears: OFFER, REQUEST, ACK -> the lease is used.
    let mut c = Client::new(MAC, 100, ll(), None, 0);
    assert!(matches!(c.poll(4_000), Action::Send { .. }));
    assert!(c.listening());
    let x = c.xid;
    let a = c.on_reply(&reply(dhcp::OFFER, x, [192, 168, 1, 77], 3600), 4_100);
    assert_eq!(a, Action::Send { to: [255; 4], msg: Msg::Request { offered: [192, 168, 1, 77], server: SERVER } });
    let a = c.on_reply(&reply(dhcp::ACK, x, [192, 168, 1, 77], 3600), 4_200);
    assert_eq!(a, Action::Use(NetConfig { mode: Mode::Dhcp, ip: [192, 168, 1, 77], mask: [255, 255, 255, 0], gateway: SERVER }));
    assert_eq!(c.state, State::Bound);
    assert!(!c.listening());
}

fn bound(lease: u32) -> Client {
    let cfg = NetConfig { mode: Mode::Dhcp, ip: [192, 168, 1, 77], mask: [255, 255, 255, 0], gateway: SERVER };
    Client::new(MAC, 5, ll(), Some((cfg, SERVER, lease)), 0)
}

#[test]
fn renewal_at_t1_unicast_then_rebinding_at_t2_broadcast() {
    let mut c = bound(1000);
    assert_eq!(c.poll(499_999), Action::None);
    assert_eq!(c.poll(500_000), Action::Send { to: SERVER, msg: Msg::Renew { ciaddr: [192, 168, 1, 77] } });
    // No answer: pause, then try again; at T2 a broadcast.
    assert_eq!(c.poll(500_000 + WAIT_MS), Action::None);
    assert_eq!(c.state, State::Bound);
    let mut t = 500_000 + WAIT_MS;
    let mut got = None;
    while t < 1_000_000 {
        if let Action::Send { to, .. } = c.poll(t) {
            if to == [255; 4] {
                got = Some(t);
                break;
            }
            // Renewing again: no answer.
        }
        t += 1000;
    }
    assert!(got.unwrap() >= 875_000, "rebinding only from T2");
}

#[test]
fn silent_server_short_lease_renewing_rebinding_then_lease_end_dh_03() {
    // Lease 120 s: T1 60 s, T2 105 s, end 120 s; the server never answers (emulator DH-03).
    let mut c = bound(120);
    let mut sends: Vec<(u64, [u8; 4])> = vec![];
    let mut used = None;
    let mut t = 0;
    while t <= 200_000 {
        match c.poll(t) {
            Action::Send { to, .. } => sends.push((t, to)),
            Action::Use(cfg) => {
                used.get_or_insert((t, cfg));
            }
            Action::None => {}
        }
        t += 10;
    }
    // One unicast renewal at T1, a broadcast REQUEST at T2, none in between.
    assert_eq!(sends[0], (60_000, SERVER));
    assert_eq!(sends[1], (105_000, [255; 4]), "rebinding at T2: {sends:?}");
    assert!(sends.iter().filter(|s| s.0 < 120_000).all(|s| s.0 + WAIT_MS <= 120_000 || s.1 == [255; 4]));
    // Lease end: link-local, and the A1.11 retries start (a DISCOVER at once, then back-off).
    assert_eq!(used, Some((120_000, ll())));
    let d: Vec<u64> = sends.iter().filter(|s| s.0 >= 120_000).map(|s| s.0).collect();
    assert!(d[0] <= 120_010, "DISCOVER at the next poll after the lease end");
    // The DISCOVER at once is borrow 0, as the power-on exchange: free gaps 4, 8, 16 s (DH-01).
    let gaps: Vec<u64> = d.windows(2).map(|w| w[1] - w[0] - WAIT_MS).take(3).collect();
    assert_eq!(gaps, [4_000, 8_000, 16_000]);
    assert!(matches!(c.state, State::Init { .. } | State::Selecting { .. }));
    // A rebinding ACK before the end keeps or moves the address (A1.11a).
    let mut c = bound(120);
    c.poll(60_000);
    c.poll(60_000 + WAIT_MS);
    assert!(matches!(c.poll(105_000), Action::Send { to: [255, 255, 255, 255], .. }));
    let x = c.xid;
    let a = c.on_reply(&reply(dhcp::ACK, x, [192, 168, 1, 91], 120), 105_100);
    assert!(matches!(a, Action::Use(cfg) if cfg.ip == [192, 168, 1, 91]));
    assert_eq!(c.state, State::Bound);
}

#[test]
fn renewal_that_keeps_the_address_and_one_that_moves_it() {
    let mut c = bound(1000);
    c.poll(500_000);
    let x = c.xid;
    let a = c.on_reply(&reply(dhcp::ACK, x, [192, 168, 1, 77], 1000), 500_100);
    assert!(matches!(a, Action::Use(cfg) if cfg.ip == [192, 168, 1, 77]));
    // Next renewal is relative to the new lease.
    assert_eq!(c.poll(900_000), Action::None);
    assert!(matches!(c.poll(1_000_100), Action::Send { .. }));
    let x = c.xid;
    let a = c.on_reply(&reply(dhcp::ACK, x, [192, 168, 1, 90], 1000), 1_000_200);
    assert!(matches!(a, Action::Use(cfg) if cfg.ip == [192, 168, 1, 90]), "address change under A1.11a");
}

#[test]
fn nak_to_renewal_and_end_of_lease_go_back_to_link_local() {
    let mut c = bound(1000);
    c.poll(500_000);
    let x = c.xid;
    assert_eq!(c.on_reply(&reply(dhcp::NAK, x, [0; 4], 0), 500_100), Action::Use(ll()));
    assert!(matches!(c.state, State::Init { .. }));
    // Keeps trying at once.
    assert!(matches!(c.poll(500_100), Action::Send { msg: Msg::Discover, .. }));
    let mut c = bound(1000);
    assert_eq!(c.poll(1_000_000), Action::Use(ll()));
    assert!(matches!(c.state, State::Init { .. }));
}

#[test]
fn foreign_replies_are_ignored() {
    let mut c = Client::new(MAC, 100, ll(), None, 0);
    c.poll(4_000);
    let x = c.xid;
    assert_eq!(c.on_reply(&reply(dhcp::OFFER, x.wrapping_add(9), [192, 168, 1, 77], 3600), 4_100), Action::None);
    assert_eq!(c.on_reply(&reply(dhcp::ACK, x, [192, 168, 1, 77], 3600), 4_100), Action::None, "ACK while selecting");
}

#[test]
fn one_borrow_lasts_at_most_2_s_decision_0041() {
    // OFFER late in the window: the REQUEST does not get a new 2 s.
    let mut c = Client::new(MAC, 100, ll(), None, 0);
    assert!(matches!(c.poll(4_000), Action::Send { .. }));
    let x = c.xid;
    assert!(matches!(c.on_reply(&reply(dhcp::OFFER, x, [192, 168, 1, 77], 3600), 5_700), Action::Send { .. }));
    assert!(c.listening());
    assert_eq!(c.poll(4_000 + WAIT_MS), Action::None);
    assert!(!c.listening(), "abandoned 1.78 s after the first send");
    assert!(matches!(c.state, State::Init { .. }), "retried at the next interval");
    assert_eq!(WAIT_MS + RETURN_MS, BORROW_TARGET_MS);
    assert!(BORROW_TARGET_MS + 200 <= BORROW_MS, "200 ms under the bound of 0041");
}

#[test]
fn the_whole_borrow_close_and_reopen_included_is_at_most_2_s() {
    // T-A1-66 (a), 0041: the net loop takes socket 7 at `t0` and sends at once; it polls the
    // client every 2 ms while it borrows (the borrowed path's sleep) and gives the socket back
    // at the first poll that finds the client no longer listening; the reopen then takes at
    // most a few ms of CH394 commands.
    const LOOP_MS: u64 = 2;
    const REOPEN_MAX_MS: u64 = 5;
    for t0 in [4_000u64, 4_001] {
        let mut c = Client::new(MAC, 100, ll(), None, 0);
        assert!(matches!(c.poll(t0), Action::Send { .. }));
        let mut t = t0;
        while c.listening() {
            t += LOOP_MS;
            c.poll(t);
        }
        let end = t + REOPEN_MAX_MS;
        assert!(end - t0 <= BORROW_TARGET_MS && BORROW_TARGET_MS < BORROW_MS, "borrow {} ms", end - t0);
        assert!(end - t0 > BORROW_TARGET_MS - 50, "not much shorter either: {} ms", end - t0);
    }
}

#[test]
fn the_management_socket_is_free_at_least_4_s_between_borrows() {
    // No server: every borrow (first send until the client stops listening) is at most 2 s,
    // and the channel is free for at least 4 s before each one, also right after `new` (the
    // board starts the client after the power-on exchange: emulator run of dffed29, where the
    // first retry fell due at once and every management test met a borrow).
    let mut c = Client::new(MAC, 100, ll(), None, 10_000);
    let (mut borrowed_at, mut free_since) = (None::<u64>, 10_000u64);
    let mut t = 10_000;
    while t < 400_000 {
        if let Action::Send { .. } = c.poll(t) {
            if borrowed_at.is_none() {
                assert!(t - free_since >= 4_000, "borrow at {} only {} ms after the last", t, t - free_since);
                borrowed_at = Some(t);
            }
        }
        if let Some(b) = borrowed_at {
            if !c.listening() {
                assert!(t - b <= WAIT_MS, "borrow from {} to {}", b, t);
                borrowed_at = None;
                free_since = t;
            }
        }
        t += 10;
    }
}

#[test]
fn a_conflict_on_the_leased_address_declines_and_starts_again_after_10_s_0049() {
    let mut c = bound(3600);
    assert_eq!(c.decline(1_000), Some(([192, 168, 1, 77], SERVER)));
    assert!(c.lease().is_none());
    assert_eq!(c.poll(10_999), Action::None);
    assert!(matches!(c.poll(11_000), Action::Send { msg: Msg::Discover, .. }));
    // No lease: nothing to decline.
    let mut c = Client::new(MAC, 1, ll(), None, 0);
    assert_eq!(c.decline(5), None);
}

/// The actions of a client polled every millisecond, and of the same client polled only at its
/// `next_due` (the net loop's idle sleep): the same actions at the same times.
fn run_both(mut fine: Client, end_ms: u64, start: u64) {
    let mut coarse = fine.clone();
    let mut a = vec![];
    for t in start..end_ms {
        let act = fine.poll(t);
        if act != Action::None {
            a.push((t, act));
        }
    }
    let mut b = vec![];
    let mut t = start;
    while t < end_ms {
        let act = coarse.poll(t);
        if act != Action::None {
            b.push((t, act));
        }
        let due = coarse.next_due();
        assert!(due >= t, "next_due {due} before now {t}");
        t = if due == t { t + 1 } else { due };
    }
    assert_eq!(a, b);
    assert!(!a.is_empty());
}

#[test]
fn next_due_is_when_poll_acts() {
    // No server: DISCOVER and the back-off gaps (free gaps 4, 8, 16, 32 s; T-A1-66).
    run_both(Client::new(MAC, 100, ll(), None, 0), 200_000, 0);
    // A silent server and a short lease: renewal at T1, rebinding at T2, the pauses, the end.
    run_both(bound(120), 200_000, 0);
}

#[test]
fn a_change_into_dhcp_mode_from_a_static_address_t_a1_100() {
    // A1.11 (0082): the static address is kept for at most 6 s of attempts, each exchange a
    // borrow within the bound (0041); no lease by then: the link-local fall-back (the board
    // probes it), and DHCP goes on trying.
    let mut c = Client::start(MAC, 7, ll(), true, 50_000);
    assert_eq!(c.next_due(), 50_000);
    assert!(matches!(c.poll(50_000), Action::Send { msg: Msg::Discover, .. }));
    let mut t = 50_000;
    let mut borrow_start = Some(50_000);
    let mut used = None;
    while t < 70_000 {
        t += 10;
        let listening = c.listening();
        match c.poll(t) {
            Action::Use(u) => {
                used = Some((u, t));
                break;
            }
            Action::Send { msg: Msg::Discover, .. } => borrow_start = Some(t),
            _ => {}
        }
        if listening && !c.listening() {
            let b = borrow_start.take().unwrap();
            assert!(t - b <= BORROW_TARGET_MS - RETURN_MS, "borrow {} ms", t - b);
        }
    }
    assert_eq!(used, Some((ll(), 50_000 + KEEP_STATIC_MS)), "link-local after 6 s with no lease");
    assert!(!c.listening(), "an exchange still running is abandoned");
    // DHCP keeps trying after the probe.
    assert!(matches!(c.poll(56_000 + 4_000), Action::Send { msg: Msg::Discover, .. }));
    // A lease within the 6 s: used, and no fall-back after it.
    let mut c = Client::start(MAC, 7, ll(), true, 0);
    assert!(matches!(c.poll(0), Action::Send { .. }));
    let x = c.xid;
    assert!(matches!(c.on_reply(&reply(dhcp::OFFER, x, [192, 168, 1, 77], 3600), 100), Action::Send { .. }));
    let a = c.on_reply(&reply(dhcp::ACK, x, [192, 168, 1, 77], 3600), 200);
    assert_eq!(a, Action::Use(NetConfig { mode: Mode::Dhcp, ip: [192, 168, 1, 77], mask: [255, 255, 255, 0], gateway: SERVER }));
    assert_eq!(c.poll(KEEP_STATIC_MS + 10), Action::None);
}

#[test]
fn a_change_into_dhcp_mode_from_a_link_local_address_keeps_it_t_a1_100() {
    // A1.11 (0082): from a link-local address (the fall-back of a static one after a
    // conflict): kept with no time limit and no probe; DHCP by borrows with the back-off.
    let held = netcfg::link_local(MAC, 3);
    let mut c = Client::start(MAC, 7, held, false, 0);
    let mut sends = 0;
    for t in (0..120_000).step_by(10) {
        match c.poll(t) {
            Action::Use(_) => panic!("no address change without a lease"),
            Action::Send { .. } => sends += 1,
            _ => {}
        }
    }
    assert!(sends >= 3, "DHCP keeps trying ({sends})");
    assert_eq!(c.link_local(), held);
}
