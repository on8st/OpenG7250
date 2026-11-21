// SPDX-License-Identifier: GPL-3.0-or-later
//! DHCP client (A1.11, A1.11a; RFC 2131).
use g7250_proto::dhcp;
use g7250_proto::netcfg::{self, Mode, NetConfig};
use g7250_radio::dhcp_client::{Action, Client, Msg, State, WAIT_MS};

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
        t += 100;
    }
    // 4, then +3 wait +8, +3 +16, +3 +32, +3 +64, +3 +64 ...
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
    assert!(matches!(c.on_reply(&reply(dhcp::OFFER, x, [192, 168, 1, 77], 3600), 5_900), Action::Send { .. }));
    assert!(c.listening());
    assert_eq!(c.poll(6_000), Action::None);
    assert!(!c.listening(), "abandoned 2 s after the first send");
    assert!(matches!(c.state, State::Init { .. }), "retried at the next interval");
    assert_eq!(WAIT_MS, 2_000);
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
