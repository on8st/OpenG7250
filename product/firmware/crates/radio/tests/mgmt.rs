// SPDX-License-Identifier: GPL-3.0-or-later
//! Radio end of the management channel, step 2 (A1.4, A1.10, Annex M).
use g7250_proto::mgmt::{self, payload::Identify, Command, Status};
use g7250_radio::mgmt::{Ctx, Server, FIRST_SECRET_WINDOW_MS};

const OWN: [u8; 4] = [192, 168, 1, 20];
const MASK: [u8; 4] = [255, 255, 255, 0];
const HOST: ([u8; 4], u16) = ([192, 168, 1, 5], 50000);

struct World {
    id: Identify<'static>,
    n: u8,
    secret: Option<[u8; 32]>,
}

impl World {
    fn new() -> Self {
        World { id: Identify { model: b"G7250", mac: [2, 0, 0, 0, 0x72, 0x50], versions: [1, 0, 44, 1, 1], active_level: 1, ..Default::default() }, n: 0, secret: None }
    }
    fn call(&mut self, srv: &mut Server, src: ([u8; 4], u16), d: &[u8], now: u64) -> Option<(u32, Status, Vec<u8>)> {
        let mut n = self.n;
        let mut rng = |c: &mut [u8; 16]| {
            n = n.wrapping_add(1);
            *c = [n; 16];
            true
        };
        let mut ctx = Ctx {
            now_ms: now,
            own_ip: OWN,
            own_mask: MASK,
            identify: &self.id,
            config: &[1, 1, 0],
            calibration: &[0u8; 37],
            rng: &mut rng,
            secret: self.secret.as_ref(),
        };
        let mut out = [0u8; mgmt::MAX_REPLY];
        let r = srv.handle(src, d, &mut ctx, &mut out).map(|len| {
            let p = mgmt::parse_reply(&out[..len]).unwrap();
            (p.header.request_id, p.status.unwrap(), p.payload.to_vec())
        });
        self.n = n;
        r
    }
}

fn req(id: u32, cmd: Command, payload: &[u8]) -> Vec<u8> {
    let mut b = [0u8; 1000];
    let n = mgmt::build_request(&mut b, id, cmd, payload, None).unwrap();
    b[..n].to_vec()
}

#[test]
fn identify_from_anywhere_reads_from_own_subnet_only() {
    let mut s = Server::new();
    let mut w = World::new();
    let (id, st, p) = w.call(&mut s, HOST, &req(7, Command::Identify, &[]), 0).unwrap();
    assert_eq!((id, st), (7, Status::Ok));
    let i = Identify::decode(&p).unwrap();
    assert_eq!((i.model, i.mac), (&b"G7250"[..], [2, 0, 0, 0, 0x72, 0x50]));
    // IDENTIFY from another subnet: answered (A1.5, A1.10).
    assert!(w.call(&mut s, ([10, 0, 0, 9], 4000), &req(1, Command::Identify, &[]), 0).is_some());
    // Other reads from another subnet, and anything from a broadcast source: no reply.
    assert!(w.call(&mut s, ([10, 0, 0, 9], 4000), &req(2, Command::GetConfig, &[]), 0).is_none());
    assert!(w.call(&mut s, ([192, 168, 1, 255], 4000), &req(3, Command::Identify, &[]), 0).is_none());
    assert!(s.stats.mgmt_dropped_source >= 2);
}

#[test]
fn reads_return_their_payloads() {
    let mut s = Server::new();
    let mut w = World::new();
    assert_eq!(w.call(&mut s, HOST, &req(1, Command::GetConfig, &[]), 0).unwrap().2, vec![1, 1, 0]);
    assert_eq!(w.call(&mut s, HOST, &req(2, Command::GetCalibration, &[]), 0).unwrap().2.len(), 37);
    let st = w.call(&mut s, HOST, &req(3, Command::GetStats, &[]), 0).unwrap();
    let names: Vec<_> = mgmt::payload::stats(&st.2).map(|e| e.unwrap().0.to_vec()).collect();
    assert!(names.contains(&b"mgmt_requests".to_vec()));
    // Not built yet: answered with 56; framing errors with their codes.
    assert_eq!(w.call(&mut s, HOST, &req(4, Command::ReadLog, &[0, 0, 0, 0]), 0).unwrap().1, Status::NotSupported);
    let mut bad = req(5, Command::Identify, &[]);
    bad[4] = 9;
    assert_eq!(w.call(&mut s, HOST, &bad, 0).unwrap().1, Status::UnsupportedVersion);
}

#[test]
fn repeat_is_answered_from_the_cache() {
    let mut s = Server::new();
    let mut w = World::new();
    let a = w.call(&mut s, HOST, &req(9, Command::GetChallenge, &[]), 0).unwrap();
    // Same id, command and payload within 5 s: the same challenge comes back, not a new one.
    let b = w.call(&mut s, HOST, &req(9, Command::GetChallenge, &[]), 1000).unwrap();
    assert_eq!(a.2, b.2);
    assert_eq!(s.stats.mgmt_repeats, 1);
    // After 5 s, or with a new id, it is executed again.
    let c = w.call(&mut s, HOST, &req(9, Command::GetChallenge, &[]), 6000).unwrap();
    assert_ne!(a.2, c.2);
    let d = w.call(&mut s, HOST, &req(10, Command::GetChallenge, &[]), 6000).unwrap();
    assert_ne!(c.2, d.2);
}

#[test]
fn authenticated_requests_before_a_secret_exists() {
    let mut s = Server::new();
    let mut w = World::new();
    // No trailer: 32.
    assert_eq!(w.call(&mut s, HOST, &req(1, Command::SetConfig, &[]), 0).unwrap().1, Status::AuthRequired);
    // With a trailer but no secret stored: 33; five of them in 10 s: 34, also for GET_CHALLENGE.
    for i in 0..5u32 {
        let mut b = [0u8; 1000];
        let n = mgmt::build_request(&mut b, 100 + i, Command::SetConfig, &[], Some((&[1; 32], &[2; 16]))).unwrap();
        assert_eq!(w.call(&mut s, HOST, &b[..n], 100 * i as u64).unwrap().1, Status::AuthFailed);
    }
    let mut b = [0u8; 1000];
    let n = mgmt::build_request(&mut b, 200, Command::SetConfig, &[], Some((&[1; 32], &[2; 16]))).unwrap();
    assert_eq!(w.call(&mut s, HOST, &b[..n], 600).unwrap().1, Status::RateLimited);
    assert_eq!(w.call(&mut s, HOST, &req(201, Command::GetChallenge, &[]), 700).unwrap().1, Status::RateLimited);
    // Reads stay answered; after 10 s without a failure the limit lifts.
    assert_eq!(w.call(&mut s, HOST, &req(202, Command::Identify, &[]), 800).unwrap().1, Status::Ok);
    assert_eq!(w.call(&mut s, HOST, &req(203, Command::GetChallenge, &[]), 11_000).unwrap().1, Status::Ok);
}

#[test]
fn authenticated_with_secret_and_challenge_reaches_the_command() {
    let mut s = Server::new();
    let mut w = World::new();
    w.secret = Some([7; 32]);
    let ch = w.call(&mut s, HOST, &req(1, Command::GetChallenge, &[]), 0).unwrap().2;
    let ch: [u8; 16] = ch.try_into().unwrap();
    let mut b = [0u8; 1000];
    let n = mgmt::build_request(&mut b, 2, Command::SetConfig, &[], Some((&[7; 32], &ch))).unwrap();
    // Verified; writes are not built yet: 56.
    assert_eq!(w.call(&mut s, HOST, &b[..n], 100).unwrap().1, Status::NotSupported);
    // The challenge is used up: the same request with a new id fails.
    let n = mgmt::build_request(&mut b, 3, Command::SetConfig, &[], Some((&[7; 32], &ch))).unwrap();
    assert_eq!(w.call(&mut s, HOST, &b[..n], 200).unwrap().1, Status::AuthFailed);
    // A challenge older than 10 s is no good.
    let ch = w.call(&mut s, HOST, &req(4, Command::GetChallenge, &[]), 1000).unwrap().2;
    let ch: [u8; 16] = ch.try_into().unwrap();
    let n = mgmt::build_request(&mut b, 5, Command::SetConfig, &[], Some((&[7; 32], &ch))).unwrap();
    assert_eq!(w.call(&mut s, HOST, &b[..n], 12_000).unwrap().1, Status::AuthFailed);
}

#[test]
fn first_secret_window() {
    let mut s = Server::new();
    assert_eq!(s.window(0, false), (false, 0)); // network not up yet
    s.network_up(1000);
    assert_eq!(s.window(1000, false), (true, 300));
    assert_eq!(s.window(1000 + FIRST_SECRET_WINDOW_MS - 1, false), (true, 1));
    assert_eq!(s.window(1000 + FIRST_SECRET_WINDOW_MS, false), (false, 0));
    assert_eq!(s.window(2000, true), (false, 0)); // secret set
    let mut w = World::new();
    let p = w.call(&mut s, HOST, &req(1, Command::Identify, &[]), 61_000).unwrap().2;
    assert_eq!(Identify::decode(&p).unwrap().window, (true, 240));
}

#[test]
fn reply_caps() {
    let mut s = Server::new();
    let mut w = World::new();
    // 20 per second per source.
    let ok = (0..30u32).filter(|i| w.call(&mut s, HOST, &req(1000 + i, Command::GetStats, &[]), 0).is_some()).count();
    assert_eq!(ok, 20);
    // 10 IDENTIFY per second from outside the subnet, whatever the source.
    let mut s = Server::new();
    let ok = (0..30u16).filter(|i| w.call(&mut s, ([10, 0, 1, *i as u8], 5000), &req(*i as u32, Command::Identify, &[]), 0).is_some()).count();
    assert_eq!(ok, 10);
    // Non-requests are dropped and counted.
    let mut r = req(1, Command::Identify, &[]);
    r[6] = 2;
    assert!(w.call(&mut s, HOST, &r, 0).is_none());
    assert_eq!(s.stats.mgmt_dropped_not_request, 1);
}
