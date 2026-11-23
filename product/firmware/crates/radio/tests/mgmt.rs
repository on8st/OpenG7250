// SPDX-License-Identifier: GPL-3.0-or-later
//! Radio end of the management channel, step 2 (A1.4, A1.10, Annex M).
use g7250_proto::mgmt::{self, payload::Identify, Command, Status};
use g7250_proto::mgmt::payload::Package;
use g7250_radio::mgmt::{Ctx, Effect, Reason, Server, Updater, FIRST_SECRET_WINDOW_MS};

/// A model of the update side: counts blocks, verifies once complete.
#[derive(Default)]
struct Model {
    began: bool,
    received: u32,
    count: u32,
    verified: bool,
    activated: bool,
    fail_writes: bool,
    trial: bool,
    /// The next block finds the store full (64, once).
    full: bool,
    room: bool,
}

impl Updater for Model {
    fn begin(&mut self, foreign: bool, _d: bool, pkg: &Package, _: u64) -> Result<u32, Status> {
        if self.trial {
            return Err(Status::UpdateInProgress);
        }
        if foreign {
            return Err(Status::NotSupported);
        }
        if pkg.target_slot == 1 {
            return Err(Status::RunningSlot); // running in slot A
        }
        self.began = true;
        self.count = pkg.length.div_ceil(512);
        self.received = 0;
        Ok(self.count)
    }
    fn block(&mut self, n: u32, crc_ok: bool, _: &[u8], _: u64) -> Result<u32, Status> {
        if !crc_ok {
            return Err(Status::BadBlockCrc);
        }
        if n != self.received {
            return Err(Status::BlockOutOfOrder);
        }
        if core::mem::take(&mut self.full) {
            self.room = true;
            return Err(Status::RoomMade);
        }
        if self.fail_writes {
            self.received = 0;
            return Err(Status::VerificationFailed);
        }
        self.received += 1;
        Ok(self.received)
    }
    fn verify(&mut self, _: u64) -> Result<[u8; 32], Status> {
        if self.received != self.count {
            return Err(Status::VerificationFailed);
        }
        self.verified = true;
        Ok([0x5A; 32])
    }
    fn room_made(&mut self) -> bool {
        core::mem::take(&mut self.room)
    }
    fn verify_note(&self) -> (u8, u32, Option<&'static str>) {
        (2, 0x0001_0002, (!self.verified).then_some("missing-blocks"))
    }
    fn activate(&mut self, _: u64) -> Result<(), Status> {
        if !self.verified {
            return Err(Status::VerificationFailed);
        }
        self.activated = true;
        Ok(())
    }
    fn abort(&mut self, _: u64) -> bool {
        self.received = 0;
        self.count = 0;
        self.verified = false;
        true
    }
    fn status(&self) -> (u8, u32, u32) {
        (if self.verified { 2 } else if self.began { 1 } else { 0 }, self.received, self.count)
    }
    fn rollback(&mut self, _: u64) -> Result<g7250_radio::mgmt::Rollback, Status> {
        if self.trial || self.began {
            return Err(Status::UpdateInProgress);
        }
        Ok(g7250_radio::mgmt::Rollback { from: (1, 0x0001_0002), to: (2, 0x0001_0001) })
    }
    fn in_progress(&self) -> bool {
        self.began || self.trial
    }
}

const OWN: [u8; 4] = [192, 168, 1, 20];
const MASK: [u8; 4] = [255, 255, 255, 0];
const HOST: ([u8; 4], u16) = ([192, 168, 1, 5], 50000);

struct World {
    id: Identify<'static>,
    n: u8,
    secret: Option<[u8; 32]>,
    p2: bool,
    armed: bool,
    c3_lost: bool,
    up: Model,
    /// The stored configuration (Table M-3 TLVs).
    config: Vec<u8>,
}

impl World {
    fn new() -> Self {
        World { id: Identify { model: b"G7250", mac: [2, 0, 0, 0, 0x72, 0x50], versions: [1, 0, 44, 1, 1], active_level: 1, ..Default::default() }, n: 0, secret: None, p2: false, armed: false, c3_lost: false, up: Model::default(), config: vec![1, 1, 0] }
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
            config: &self.config,
            calibration: &[0u8; 37],
            rng: &mut rng,
            secret: self.secret.as_ref(),
            p2_session: self.p2,
            armed: self.armed,
            c3_lost: self.c3_lost,
            update: &mut self.up,
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
    let c = mgmt::payload::Config::decode(&w.call(&mut s, HOST, &req(1, Command::GetConfig, &[]), 0).unwrap().2).unwrap();
    assert_eq!(c.address_mode, Some(0));
    assert_eq!(w.call(&mut s, HOST, &req(2, Command::GetCalibration, &[]), 0).unwrap().2.len(), 37);
    let st = w.call(&mut s, HOST, &req(3, Command::GetStats, &[]), 0).unwrap();
    let names: Vec<_> = mgmt::payload::stats(&st.2).map(|e| e.unwrap().0.to_vec()).collect();
    assert!(names.contains(&b"mgmt_requests".to_vec()));
    // Not built yet: answered with 56; framing errors with their codes.
    assert_eq!(w.call(&mut s, HOST, &req(4, Command::ReadLog, &[0, 0, 0, 0]), 0).unwrap().1, Status::Ok);
    assert_eq!(w.call(&mut s, HOST, &req(6, Command::ReadLog, &[]), 0).unwrap().1, Status::Malformed);
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

fn auth_required_count(w: &mut World, s: &mut Server, id: u32, now: u64) -> u64 {
    let r = w.call(s, HOST, &req(id, Command::GetStats, &[]), now).unwrap();
    mgmt::payload::stats(&r.2).map(|e| e.unwrap()).find(|(n, _)| *n == b"mg_auth_required").unwrap().1
}

#[test]
fn authenticated_requests_before_a_secret_exists_t_a1_99() {
    use g7250_radio::mgmt::Effect;
    let mut s = Server::new();
    let mut w = World::new();
    // No secret set: 32 with and without a trailer, not executed; ten in 10 s never give 34
    // (0080). Commands whose empty payload is well formed (a malformed one gets 17 first, 0056).
    // The trailer is made with a challenge the radio gave, which stays unused.
    let ch: [u8; 16] = w.call(&mut s, HOST, &req(1, Command::GetChallenge, &[]), 0).unwrap().2.try_into().unwrap();
    let mut b = [0u8; 1000];
    for i in 0..10u32 {
        let cmd = [Command::SetConfig, Command::FactoryReset, Command::Restart, Command::UpdateAbort][i as usize % 4];
        let trailer = (i % 2 == 0).then_some((&[1u8; 32], &ch));
        let n = mgmt::build_request(&mut b, 100 + i, cmd, &[], trailer).unwrap();
        assert_eq!(w.call(&mut s, HOST, &b[..n], 100 + 900 * i as u64).unwrap().1, Status::AuthRequired, "{cmd:?} trailer {}", trailer.is_some());
        assert!(s.take_effects().is_empty(), "not executed");
    }
    assert_eq!(auth_required_count(&mut w, &mut s, 200, 9_500), 10);
    assert_eq!(s.stats.mgmt_auth_failures, 0, "a 32 is no authentication failure");
    assert_eq!(w.call(&mut s, HOST, &req(201, Command::GetChallenge, &[]), 9_600).unwrap().1, Status::Ok);
    // Reads stay answered.
    assert_eq!(w.call(&mut s, HOST, &req(202, Command::Identify, &[]), 9_700).unwrap().1, Status::Ok);
}

#[test]
fn five_failures_in_10_s_give_34_and_a_32_does_not_count() {
    let mut s = Server::new();
    let mut w = World::new();
    w.secret = Some([7; 32]);
    let mut b = [0u8; 1000];
    // Four wrong HMACs (33), then ten requests without a trailer (32): still no 34.
    for i in 0..4u32 {
        let n = mgmt::build_request(&mut b, 100 + i, Command::SetConfig, &[], Some((&[1; 32], &[2; 16]))).unwrap();
        assert_eq!(w.call(&mut s, HOST, &b[..n], 100 * i as u64).unwrap().1, Status::AuthFailed);
    }
    for i in 0..10u32 {
        assert_eq!(w.call(&mut s, HOST, &req(120 + i, Command::SetConfig, &[]), 400 + 60 * i as u64).unwrap().1, Status::AuthRequired);
    }
    // The fifth 33 within 10 s: from then on 34, also for GET_CHALLENGE.
    let n = mgmt::build_request(&mut b, 140, Command::SetConfig, &[], Some((&[1; 32], &[2; 16]))).unwrap();
    assert_eq!(w.call(&mut s, HOST, &b[..n], 1_100).unwrap().1, Status::AuthFailed);
    let n = mgmt::build_request(&mut b, 200, Command::SetConfig, &[], Some((&[1; 32], &[2; 16]))).unwrap();
    assert_eq!(w.call(&mut s, HOST, &b[..n], 1_200).unwrap().1, Status::RateLimited);
    assert_eq!(w.call(&mut s, HOST, &req(201, Command::GetChallenge, &[]), 1_300).unwrap().1, Status::RateLimited);
    // Reads stay answered; 10 s after the last failure the limit lifts.
    assert_eq!(w.call(&mut s, HOST, &req(202, Command::Identify, &[]), 1_400).unwrap().1, Status::Ok);
    assert_eq!(w.call(&mut s, HOST, &req(203, Command::GetChallenge, &[]), 11_099).unwrap().1, Status::RateLimited);
    assert_eq!(w.call(&mut s, HOST, &req(204, Command::GetChallenge, &[]), 11_100).unwrap().1, Status::Ok);
}

#[test]
fn a_32_with_a_secret_set_leaves_the_challenge_usable_t_a1_99() {
    let mut s = Server::new();
    let mut w = World::new();
    w.secret = Some([7; 32]);
    let ch: [u8; 16] = w.call(&mut s, HOST, &req(1, Command::GetChallenge, &[]), 0).unwrap().2.try_into().unwrap();
    // No trailer: 32, the challenge still there.
    assert_eq!(w.call(&mut s, HOST, &req(2, Command::SetConfig, &[]), 100).unwrap().1, Status::AuthRequired);
    let mut b = [0u8; 1000];
    let n = mgmt::build_request(&mut b, 4, Command::SetConfig, &[], Some((&[7; 32], &ch))).unwrap();
    assert_eq!(w.call(&mut s, HOST, &b[..n], 200).unwrap().1, Status::Ok);
    // A wrong HMAC: 33, counted as a failure, not as a 32.
    let ch: [u8; 16] = w.call(&mut s, HOST, &req(6, Command::GetChallenge, &[]), 300).unwrap().2.try_into().unwrap();
    let n = mgmt::build_request(&mut b, 8, Command::SetConfig, &[], Some((&[8; 32], &ch))).unwrap();
    assert_eq!(w.call(&mut s, HOST, &b[..n], 400).unwrap().1, Status::AuthFailed);
    assert_eq!(s.stats.mgmt_auth_failures, 1);
    assert_eq!(auth_required_count(&mut w, &mut s, 10, 500), 1);
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
    // Verified; an empty SET_CONFIG changes nothing and answers every item.
    assert_eq!(w.call(&mut s, HOST, &b[..n], 100).unwrap().1, Status::Ok);
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

fn authed(w: &mut World, s: &mut Server, src: ([u8; 4], u16), id: u32, cmd: Command, payload: &[u8], now: u64) -> (Status, Vec<u8>) {
    let ch = w.call(s, src, &req(id, Command::GetChallenge, &[]), now).unwrap().2;
    let ch: [u8; 16] = ch.try_into().unwrap();
    let mut b = [0u8; 1000];
    let n = mgmt::build_request(&mut b, id + 1, cmd, payload, Some((w.secret.as_ref().unwrap(), &ch))).unwrap();
    let r = w.call(s, src, &b[..n], now + 1).unwrap();
    (r.1, r.2)
}

#[test]
fn first_secret_only_in_the_window() {
    use g7250_radio::mgmt::Effect;
    let mut s = Server::new();
    let mut w = World::new();
    // Network not up: window closed.
    assert_eq!(w.call(&mut s, HOST, &req(1, Command::SetFirstSecret, &[7; 32]), 0).unwrap().1, Status::FirstSecretClosed);
    s.network_up(0);
    s.take_effects();
    assert_eq!(w.call(&mut s, HOST, &req(2, Command::SetFirstSecret, &[7; 31]), 10).unwrap().1, Status::Malformed);
    assert_eq!(w.call(&mut s, HOST, &req(3, Command::SetFirstSecret, &[7; 32]), 20).unwrap().1, Status::Ok);
    assert_eq!(s.take_effects().as_slice(), &[Effect::SetSecret([7; 32])]);
    // Used up: a second claim is refused even before the secret is stored.
    assert_eq!(w.call(&mut s, HOST, &req(4, Command::SetFirstSecret, &[8; 32]), 30).unwrap().1, Status::FirstSecretClosed);
    // The claim is logged with its source.
    let log = w.call(&mut s, HOST, &req(5, Command::ReadLog, &[0, 0, 0, 0]), 40).unwrap().2;
    let e: Vec<_> = mgmt::payload::log_entries(&log).map(|e| String::from_utf8(e.unwrap().text.to_vec()).unwrap()).collect();
    assert!(e.iter().any(|t| t.contains("first secret set by 192.168.1.5:50000")), "{e:?}");
}

#[test]
fn write_session_one_holder_restart_exempt() {
    let mut s = Server::new();
    let mut w = World::new();
    w.secret = Some([7; 32]);
    let other = ([192, 168, 1, 9], 40000);
    assert_eq!(authed(&mut w, &mut s, HOST, 10, Command::ClearLog, &[], 0).0, Status::Ok);
    // Another host's write within 60 s: busy.
    assert_eq!(authed(&mut w, &mut s, other, 20, Command::ClearLog, &[], 1000).0, Status::Busy);
    // RESTART is exempt.
    assert_eq!(authed(&mut w, &mut s, other, 30, Command::Restart, &[], 2000).0, Status::Ok);
    // One commanded reset at a time: a second RESTART gets 1, a FACTORY_RESET 35.
    assert_eq!(authed(&mut w, &mut s, HOST, 40, Command::Restart, &[], 3000).0, Status::InProgress);
    assert_eq!(authed(&mut w, &mut s, HOST, 50, Command::FactoryReset, &[], 4000).0, Status::Busy);
    // After 60 s idle the other host may write.
    let mut s = Server::new();
    assert_eq!(authed(&mut w, &mut s, HOST, 60, Command::ClearLog, &[], 0).0, Status::Ok);
    assert_eq!(authed(&mut w, &mut s, other, 70, Command::ClearLog, &[], 61_000).0, Status::Ok);
    // END_WRITE_SESSION releases it.
    assert_eq!(authed(&mut w, &mut s, other, 80, Command::EndWriteSession, &[], 62_000).0, Status::Ok);
    assert_eq!(authed(&mut w, &mut s, HOST, 90, Command::ClearLog, &[], 62_100).0, Status::Ok);
}

#[test]
fn change_secret_unmasks_with_the_request_challenge() {
    use g7250_radio::mgmt::Effect;
    let mut s = Server::new();
    let mut w = World::new();
    w.secret = Some([7; 32]);
    let ch = w.call(&mut s, HOST, &req(1, Command::GetChallenge, &[]), 0).unwrap().2;
    let ch: [u8; 16] = ch.try_into().unwrap();
    let wire = mgmt::auth::change_secret_mask(&[7; 32], &ch, &[9; 32]);
    let mut b = [0u8; 1000];
    let n = mgmt::build_request(&mut b, 2, Command::ChangeSecret, &wire, Some((&[7; 32], &ch))).unwrap();
    s.take_effects();
    assert_eq!(w.call(&mut s, HOST, &b[..n], 1).unwrap().1, Status::Ok);
    assert_eq!(s.take_effects().as_slice(), &[Effect::SetSecret([9; 32])]);
}

#[test]
fn set_config_validated_as_a_whole() {
    use g7250_proto::mgmt::payload::Config;
    use g7250_radio::mgmt::Effect;
    let mut s = Server::new();
    let mut w = World::new();
    w.secret = Some([7; 32]);
    let enc = |c: &Config| {
        let mut b = [0u8; 256];
        let n = c.encode(&mut b).unwrap();
        b[..n].to_vec()
    };
    // Static without an address: invalid.
    let c = Config { address_mode: Some(1), ..Default::default() };
    assert_eq!(authed(&mut w, &mut s, HOST, 10, Command::SetConfig, &enc(&c), 0).0, Status::InvalidConfig);
    // Looser than built-in: 49.
    let c = Config { max_tx_time_ms: Some(400_000), ..Default::default() };
    assert_eq!(authed(&mut w, &mut s, HOST, 20, Command::SetConfig, &enc(&c), 10).0, Status::NotAllowed);
    // S2 needs the calibration set: 62.
    let c = Config { swr_s2_x100: Some(250), ..Default::default() };
    assert_eq!(authed(&mut w, &mut s, HOST, 30, Command::SetConfig, &enc(&c), 20).0, Status::NeedsCalibration);
    // Level 2 and region 1 are not in this version.
    let c = Config { default_level: Some(2), ..Default::default() };
    assert_eq!(authed(&mut w, &mut s, HOST, 40, Command::SetConfig, &enc(&c), 30).0, Status::InvalidConfig);
    // A valid static address: stored, applied after the reply, every item answered.
    s.take_effects();
    let c = Config { address_mode: Some(1), address: Some([192, 168, 1, 50]), netmask: Some([255, 255, 255, 0]), gateway: Some([192, 168, 1, 1]), ..Default::default() };
    let (st, p) = authed(&mut w, &mut s, HOST, 50, Command::SetConfig, &enc(&c), 40);
    assert_eq!(st, Status::Ok);
    let back = Config::decode(&p).unwrap();
    assert_eq!(back.address, Some([192, 168, 1, 50]));
    assert_eq!(back.address_mode, Some(1));
    assert_eq!(back.netmask, Some([255, 255, 255, 0]), "every item after the change");
    let fx = s.take_effects();
    assert!(matches!(fx[0], Effect::SetConfig(..)));
    assert_eq!(fx[1], Effect::ApplyNetwork);
    // A band limit against the built-in value of its band (0079): 100 W on HF, 0 W at 50 MHz
    // until P-PWR-2; an index with no band is invalid.
    let band = |i: usize, l: u16| Config { band_limit: core::array::from_fn(|j| (j == i).then_some(l)), ..Default::default() };
    let cases = [(0, 1001, Status::NotAllowed), (8, 1001, Status::NotAllowed), (9, 1, Status::NotAllowed), (9, 500, Status::NotAllowed), (10, 0, Status::InvalidConfig), (8, 1000, Status::Ok), (9, 0, Status::Ok)];
    for (k, (i, l, want)) in cases.into_iter().enumerate() {
        let id = 60 + 2 * k as u32;
        assert_eq!(authed(&mut w, &mut s, HOST, id, Command::SetConfig, &enc(&band(i, l)), 1000 + 1000 * k as u64).0, want, "band {i} limit {l}");
        s.take_effects();
    }
}

/// T-A1-28: GET_CONFIG and SET_CONFIG answer every item of Table M-3, so tag 8 holds one TLV
/// per `tx_band` with its persistent value: the built-in maximum where nothing tightens it (A3.4
/// rule 1), the tightened value where something does. The stored record is not changed by this.
#[test]
fn every_band_limit_is_answered_also_where_none_is_stored() {
    use g7250_proto::mgmt::payload::{Config, MAX_BANDS};
    use g7250_radio::record::TX_BANDS;
    let enc = |c: &Config| {
        let mut b = [0u8; 256];
        let n = c.encode(&mut b).unwrap();
        b[..n].to_vec()
    };
    let builtin: Vec<Option<u16>> = (0..MAX_BANDS).map(|i| TX_BANDS.get(i).map(|b| b.2)).collect();
    let mut s = Server::new();
    let mut w = World::new();
    w.config = enc(&Config::factory());
    let tags8 = |p: &[u8]| g7250_proto::mgmt::tlv::iter(p).filter(|t| t.unwrap().0 == 8).count();
    let p = w.call(&mut s, HOST, &req(1, Command::GetConfig, &[]), 0).unwrap().2;
    let c = Config::decode(&p).unwrap();
    assert_eq!(c.band_limit.to_vec(), builtin, "factory configuration: every band, built-in");
    assert_eq!(tags8(&p), TX_BANDS.len(), "one TLV per band");
    assert_eq!((c.max_tx_time_ms, c.address_mode), (Some(300_000), Some(0)), "the other items as stored");
    // A tightened band keeps its value; the rest stay built-in.
    let mut t = Config::factory();
    t.band_limit[4] = Some(500);
    w.config = enc(&t);
    let c = Config::decode(&w.call(&mut s, HOST, &req(2, Command::GetConfig, &[]), 0).unwrap().2).unwrap();
    let mut want = builtin.clone();
    want[4] = Some(500);
    assert_eq!(c.band_limit.to_vec(), want);
    // SET_CONFIG of another item answers the same; what it stores has only the tightened band.
    w.secret = Some([7; 32]);
    s.take_effects();
    let (st, p) = authed(&mut w, &mut s, HOST, 10, Command::SetConfig, &enc(&Config { default_level: Some(1), ..Default::default() }), 0);
    assert_eq!(st, Status::Ok);
    assert_eq!(Config::decode(&p).unwrap().band_limit.to_vec(), want);
    let fx = s.take_effects();
    let Effect::SetConfig(v, _) = &fx[0] else { panic!("{fx:?}") };
    let stored = Config::decode(v).unwrap();
    assert_eq!(stored.band_limit.iter().filter(|l| l.is_some()).count(), 1, "the store holds what was written");
    assert_eq!(stored.band_limit[4], Some(500));
    // A stored record that does not decode is answered as it is.
    w.config = vec![8, 2, 0, 0];
    assert_eq!(w.call(&mut s, HOST, &req(3, Command::GetConfig, &[]), 0).unwrap().2, vec![8, 2, 0, 0]);
}

#[test]
fn calibration_refused_during_a_p2_session() {
    let mut s = Server::new();
    let mut w = World::new();
    w.secret = Some([7; 32]);
    w.p2 = true;
    let set = [0, 0, 0, 1, 0];
    assert_eq!(authed(&mut w, &mut s, HOST, 10, Command::SetCalibration, &set, 0).0, Status::P2SessionRuns);
    w.p2 = false;
    let (st, p) = authed(&mut w, &mut s, HOST, 20, Command::SetCalibration, &set, 10);
    assert_eq!((st, p.len(), p[32]), (Status::Ok, 33, 0));
    // Return to stock and vendor recovery: always 56.
    assert_eq!(authed(&mut w, &mut s, HOST, 30, Command::ReturnToStock, &[], 20).0, Status::NotSupported);
}

#[test]
fn p_rx_2_that_does_not_decode_gets_48() {
    // A1.12 item 1: a value that does not decode gets 48 and changes nothing; P-RX-2 per Table
    // P-1 (0060).
    let mut s = Server::new();
    let mut w = World::new();
    w.secret = Some([7; 32]);
    let entry = |v: &[u8]| {
        let mut e = vec![0, 0, 0, 1, 0, 6];
        e.extend_from_slice(b"P-RX-2");
        e.extend_from_slice(&(v.len() as u16).to_be_bytes());
        e.extend_from_slice(v);
        e
    };
    let bad = entry(&[0, 3]); // offset 3, no rate
    assert_eq!(authed(&mut w, &mut s, HOST, 10, Command::SetCalibration, &bad, 0).0, Status::InvalidConfig);
    assert!(s.take_effects().is_empty(), "nothing stored");
    let mut good = vec![0x01, 1, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0x40];
    good.extend_from_slice(&[0; 10]);
    let (st, _) = authed(&mut w, &mut s, HOST, 20, Command::SetCalibration, &entry(&good), 10);
    assert_eq!(st, Status::Ok);
    assert!(matches!(s.take_effects()[0], Effect::SetCalibration(_)));
}

fn package(len: u32, slot: u8) -> Vec<u8> {
    let pkg = Package {
        header_version: 1,
        component: 1,
        hardware_id: 0x00C8,
        min_hw_revision: 0,
        version: mgmt::payload::Version { major: 0, minor: 1, build: 2 },
        length: len,
        sha256: [0x5A; 32],
        signature_len: 0,
        target_slot: slot,
    };
    mgmt::payload::update_begin(0, &pkg).to_vec()
}

#[test]
fn update_begin_with_an_unreadable_header_is_17() {
    // T-A1-81 (0063): a wrong magic or header version 2 cannot be read: 17 in the framing group,
    // nothing begun. (Component 2: 56, in the stager's order; update crate tests.)
    let mut s = Server::new();
    let mut w = World::new();
    w.secret = Some([7; 32]);
    let mut magic = package(1300, 2);
    magic[1] ^= 0xFF;
    assert_eq!(authed(&mut w, &mut s, HOST, 10, Command::UpdateBegin, &magic, 0).0, Status::Malformed);
    let mut ver = package(1300, 2);
    ver[1 + 4] = 2;
    assert_eq!(authed(&mut w, &mut s, HOST, 20, Command::UpdateBegin, &ver, 10).0, Status::Malformed);
    assert!(!w.up.began);
    assert!(s.take_effects().is_empty());
}

fn block(n: u32, data: &[u8]) -> Vec<u8> {
    let mut b = [0u8; 600];
    let k = mgmt::payload::Block::encode(n, data, &mut b).unwrap();
    b[..k].to_vec()
}

#[test]
fn update_commands_through_the_updater() {
    let mut s = Server::new();
    let mut w = World::new();
    w.secret = Some([7; 32]);
    // UPDATE_STATUS is a read: idle.
    let (_, st, p) = w.call(&mut s, HOST, &req(1, Command::UpdateStatus, &[]), 0).unwrap();
    assert_eq!((st, p[0]), (Status::Ok, 0));
    // Armed: every update command 50 (T-A1-33), ROLLBACK too (T-A1-27).
    w.armed = true;
    assert_eq!(authed(&mut w, &mut s, HOST, 100, Command::UpdateBegin, &package(1300, 2), 10000).0, Status::Armed);
    assert_eq!(authed(&mut w, &mut s, HOST, 110, Command::Rollback, &[], 11000).0, Status::Armed);
    assert_eq!(w.call(&mut s, HOST, &req(125, Command::UpdateStatus, &[]), 12_500).unwrap().1, Status::Ok);
    w.armed = false;
    // An image for the running slot: 63, nothing begun (T-A1-67).
    assert_eq!(authed(&mut w, &mut s, HOST, 200, Command::UpdateBegin, &package(1300, 1), 20000).0, Status::RunningSlot);
    assert!(!w.up.began);
    let (st, p) = authed(&mut w, &mut s, HOST, 210, Command::UpdateBegin, &package(1300, 2), 21000);
    assert_eq!((st, &p[..]), (Status::Ok, &[2, 0, 0, 0, 0, 3, 0][..]));
    assert_eq!(s.take_effects().as_slice(), &[Effect::UpdateStarted]);
    // Blocks; a bad CRC is 53.
    let mut bad = block(0, &[1; 512]);
    bad[4] ^= 1;
    assert_eq!(authed(&mut w, &mut s, HOST, 300, Command::UpdateBlock, &bad, 30000).0, Status::BadBlockCrc);
    for n in 0..3u32 {
        let len = if n == 2 { 276 } else { 512 };
        let (st, p) = authed(&mut w, &mut s, HOST, 310 + 10 * n, Command::UpdateBlock, &block(n, &vec![n as u8; len]), 31_000 + 1000 * n as u64);
        assert_eq!((st, &p[4..8]), (Status::Ok, &(n + 1).to_be_bytes()[..]));
    }
    // SWITCH_LEVEL during an update: 58 (T-A1-31).
    assert_eq!(authed(&mut w, &mut s, HOST, 400, Command::SwitchLevel, &[1], 40000).0, Status::UpdateInProgress);
    let (st, p) = authed(&mut w, &mut s, HOST, 410, Command::UpdateVerify, &[], 41000);
    assert_eq!((st, p), (Status::Ok, vec![0x5A; 32]));
    let (st, p) = authed(&mut w, &mut s, HOST, 420, Command::UpdateActivate, &[0], 42000);
    assert_eq!((st, p), (Status::Ok, vec![0]));
    assert_eq!(s.take_effects().as_slice(), &[Effect::Restart(Reason::Activated { slot: 2, version: 0x0001_0002 })]);
    assert!(w.up.activated);
    // One commanded reset at a time.
    assert_eq!(authed(&mut w, &mut s, HOST, 430, Command::Rollback, &[], 43000).0, Status::Busy);
}

#[test]
fn a_store_write_that_fails_turns_the_reply_into_55_also_for_a_repeat() {
    // B2.3 "flash work that fails" (0065 point 2; T-A1-85): the board's store write fails after
    // the command was accepted; the reply, and a repeat of the request, are 55 with no payload.
    let mut s = Server::new();
    let mut w = World::new();
    w.secret = Some([7; 32]);
    let set = [0, 0, 0, 1, 0];
    let mut out = [0u8; mgmt::MAX_REPLY];
    let (st, _) = authed(&mut w, &mut s, HOST, 10, Command::SetCalibration, &set, 0);
    assert_eq!(st, Status::Ok);
    let n = s.fail_reply(HOST, Status::VerificationFailed, &mut out).unwrap();
    let r = mgmt::parse_reply(&out[..n]).unwrap();
    assert_eq!((r.status, r.payload.len(), r.header.request_id), (Some(Status::VerificationFailed), 0, 11));
}

#[test]
fn update_verify_is_logged_good_and_failed() {
    // A1.7 "every update step": a failed verify names its check, a good one the slot and version.
    let mut s = Server::new();
    let mut w = World::new();
    w.secret = Some([7; 32]);
    assert_eq!(authed(&mut w, &mut s, HOST, 10, Command::UpdateBegin, &package(1300, 2), 0).0, Status::Ok);
    assert_eq!(authed(&mut w, &mut s, HOST, 20, Command::UpdateVerify, &[], 1000).0, Status::VerificationFailed);
    for n in 0..3u32 {
        let len = if n == 2 { 276 } else { 512 };
        authed(&mut w, &mut s, HOST, 30 + 10 * n, Command::UpdateBlock, &block(n, &vec![n as u8; len]), 2000 + n as u64);
    }
    assert_eq!(authed(&mut w, &mut s, HOST, 70, Command::UpdateVerify, &[], 3000).0, Status::Ok);
    let log = w.call(&mut s, HOST, &req(80, Command::ReadLog, &[0, 0, 0, 0]), 4000).unwrap().2;
    let e: Vec<_> = mgmt::payload::log_entries(&log).map(|e| String::from_utf8(e.unwrap().text.to_vec()).unwrap()).collect();
    assert!(e.iter().any(|t| t == "update verify failed: missing-blocks"), "{e:?}");
    assert!(e.iter().any(|t| t == "update verified: slot B 0.1.2"), "{e:?}");
}

#[test]
fn a_retransmission_read_after_slow_flash_work_is_a_repeat() {
    // 0069 point 3 (A1.4): a request whose flash work takes 9 s; its retransmission, read only
    // after the reply (it waited in the socket buffer), is answered from the stored reply and
    // not executed again, because the repeat window counts from the reply.
    let mut s = Server::new();
    let mut w = World::new();
    w.secret = Some([7; 32]);
    let ch = w.call(&mut s, HOST, &req(1, Command::GetChallenge, &[]), 0).unwrap().2;
    let ch: [u8; 16] = ch.try_into().unwrap();
    let mut b = [0u8; 1000];
    let n = mgmt::build_request(&mut b, 2, Command::SetCalibration, &[0, 0, 0, 1, 0], Some((w.secret.as_ref().unwrap(), &ch))).unwrap();
    let first = w.call(&mut s, HOST, &b[..n], 10).unwrap();
    assert_eq!(first.1, Status::Ok);
    assert_eq!(s.take_effects().len(), 1, "executed once");
    // The store write took 9 s; the reply goes out then.
    s.reply_sent(HOST, 9_010);
    // The retransmission sent during the work, read at 12 s: a repeat.
    let again = w.call(&mut s, HOST, &b[..n], 12_000).unwrap();
    assert_eq!((again.0, again.1, &again.2), (first.0, first.1, &first.2), "the stored reply");
    assert!(s.take_effects().is_empty(), "not executed again");
    // Without the reply stamp it would have been more than 5 s after the receipt: a new request.
}

#[test]
fn rollback_is_logged_with_slots_and_versions() {
    // T-A1-84 (0064 point 3.3): reply 0, the log line, then a commanded reset; 58 during a
    // trial boot.
    let mut s = Server::new();
    let mut w = World::new();
    w.secret = Some([7; 32]);
    w.up.trial = true;
    assert_eq!(authed(&mut w, &mut s, HOST, 10, Command::Rollback, &[], 0).0, Status::UpdateInProgress);
    assert!(s.take_effects().is_empty());
    w.up.trial = false;
    assert_eq!(authed(&mut w, &mut s, HOST, 20, Command::Rollback, &[], 1000).0, Status::Ok);
    let fx = s.take_effects();
    assert!(matches!(fx.as_slice(), [Effect::Restart(Reason::Rollback(_))]));
    let Effect::Restart(reason) = fx[0].clone() else { unreachable!() };
    s.log.push(1_500, format_args!("something in between"));
    reason.log(&mut s.log.0, 1_600);
    let log = w.call(&mut s, HOST, &req(30, Command::ReadLog, &[0, 0, 0, 0]), 2000).unwrap().2;
    let e: Vec<_> = mgmt::payload::log_entries(&log).map(|e| String::from_utf8(e.unwrap().text.to_vec()).unwrap()).collect();
    assert_eq!(e.last().unwrap(), "rollback commanded: slot A 0.1.2 to slot B 0.1.1");
    assert_eq!(e.iter().filter(|t| t.contains("rollback commanded")).count(), 1, "once, as the last entry");
}

#[test]
fn update_write_failure_is_55_and_logged_and_begin_in_trial_is_58() {
    let mut s = Server::new();
    let mut w = World::new();
    w.secret = Some([7; 32]);
    w.up.trial = true;
    assert_eq!(authed(&mut w, &mut s, HOST, 100, Command::UpdateBegin, &package(1300, 2), 10000).0, Status::UpdateInProgress);
    w.up.trial = false;
    assert_eq!(authed(&mut w, &mut s, HOST, 110, Command::UpdateBegin, &package(1300, 2), 11000).0, Status::Ok);
    w.up.fail_writes = true;
    assert_eq!(authed(&mut w, &mut s, HOST, 120, Command::UpdateBlock, &block(0, &[0; 512]), 12000).0, Status::VerificationFailed);
    let (st, p) = authed(&mut w, &mut s, HOST, 130, Command::ReadLog, &[0, 0, 0, 0], 13000);
    assert_eq!(st, Status::Ok);
    let text = String::from_utf8_lossy(&p);
    assert!(text.contains("write of block 0 to slot B failed"), "{text}");
    // Abort says a restart is needed.
    let (st, p) = authed(&mut w, &mut s, HOST, 140, Command::UpdateAbort, &[], 14000);
    assert_eq!((st, p), (Status::Ok, vec![1]));
}

#[test]
fn t_b5_21_no_flash_work_while_armed() {
    use g7250_radio::mgmt::Effect;
    // B2.3 "Flash work" (0055): while armed or keyed, SET_FIRST_SECRET, CHANGE_SECRET and
    // SET_CALIBRATION get status 50 and write nothing; SET_CONFIG disarms first, then writes.
    let mut s = Server::new();
    let mut w = World::new();
    s.network_up(0);
    s.take_effects();
    w.armed = true;
    assert_eq!(w.call(&mut s, HOST, &req(1, Command::SetFirstSecret, &[7; 32]), 10).unwrap().1, Status::Armed);
    assert!(s.take_effects().is_empty());
    w.armed = false;
    assert_eq!(w.call(&mut s, HOST, &req(2, Command::SetFirstSecret, &[7; 32]), 20).unwrap().1, Status::Ok);
    w.secret = Some([7; 32]);
    s.take_effects();
    w.armed = true;
    assert_eq!(authed(&mut w, &mut s, HOST, 10, Command::ChangeSecret, &[9; 32], 100).0, Status::Armed);
    assert_eq!(authed(&mut w, &mut s, HOST, 20, Command::SetCalibration, &[0, 0, 0, 1, 0], 200).0, Status::Armed);
    assert!(s.take_effects().is_empty(), "no flash work while armed");
    // SET_CONFIG while armed: the disarm comes before the store write.
    use g7250_proto::mgmt::payload::Config;
    let mut b = [0u8; 256];
    let n = Config { max_tx_time_ms: Some(60_000), ..Default::default() }.encode(&mut b).unwrap();
    assert_eq!(authed(&mut w, &mut s, HOST, 30, Command::SetConfig, &b[..n], 300).0, Status::Ok);
    let e = s.take_effects();
    assert_eq!(e[0], Effect::DisarmForMaintenance);
    assert!(matches!(e[1], Effect::SetConfig(..)));
    // Unarmed: no disarm.
    w.armed = false;
    assert_eq!(authed(&mut w, &mut s, HOST, 40, Command::SetConfig, &b[..n], 400).0, Status::Ok);
    assert!(matches!(s.take_effects().as_slice(), [Effect::SetConfig(..)]));
}

#[test]
fn t_b5_21_status_50_takes_its_place_in_the_a1_3_order() {
    // Table A1-3: ... 36, 35, 58, 57, 50, then the command-specific codes; 17 before all of them.
    let mut s = Server::new();
    let mut w = World::new();
    s.network_up(0);
    w.armed = true;
    // SET_FIRST_SECRET while armed: malformed 17 before 50.
    assert_eq!(w.call(&mut s, HOST, &req(1, Command::SetFirstSecret, &[7; 31]), 10).unwrap().1, Status::Malformed);
    // With a secret set (window closed): 36, not 50.
    w.secret = Some([7; 32]);
    assert_eq!(w.call(&mut s, HOST, &req(2, Command::SetFirstSecret, &[7; 32]), 20).unwrap().1, Status::FirstSecretClosed);
    // CHANGE_SECRET malformed while armed: 17.
    assert_eq!(authed(&mut w, &mut s, HOST, 10, Command::ChangeSecret, &[9; 31], 100).0, Status::Malformed);
    // SET_CALIBRATION during a P2 session while armed: 57, not 50; malformed: 17 first.
    w.p2 = true;
    assert_eq!(authed(&mut w, &mut s, HOST, 20, Command::SetCalibration, &[0, 0, 0, 1, 0], 200).0, Status::P2SessionRuns);
    assert_eq!(authed(&mut w, &mut s, HOST, 30, Command::SetCalibration, &[], 300).0, Status::Malformed);
    assert!(s.take_effects().iter().all(|e| !matches!(e, g7250_radio::mgmt::Effect::SetSecret(_) | g7250_radio::mgmt::Effect::SetCalibration(_))));
}

/// A SET_CALIBRATION payload: date, empty source, one entry (id, value of `vlen` declared, `v`).
fn cal(id: &[u8], vlen: u16, v: &[u8]) -> Vec<u8> {
    let mut p = vec![0, 0, 0, 1, 0, id.len() as u8];
    p.extend_from_slice(id);
    p.extend_from_slice(&vlen.to_be_bytes());
    p.extend_from_slice(v);
    p
}

#[test]
fn t_a1_75_malformed_payload_is_17_before_every_state_check_values_after() {
    // A1.4 "Malformed payload" (0056): no size or structure gives 17 in the framing group, before
    // authentication and every state check; a value not allowed is command-specific, after them.
    let mut s = Server::new();
    let mut w = World::new();
    s.network_up(0);
    w.armed = true;
    w.p2 = true;
    // SET_FIRST_SECRET of 31 bytes: 17, armed and in a session, window open or closed.
    assert_eq!(w.call(&mut s, HOST, &req(1, Command::SetFirstSecret, &[7; 31]), 10).unwrap().1, Status::Malformed);
    w.secret = Some([7; 32]);
    assert_eq!(w.call(&mut s, HOST, &req(2, Command::SetFirstSecret, &[7; 31]), 20).unwrap().1, Status::Malformed);
    // SET_CALIBRATION with a TLV past the end: 17, also before authentication (no HMAC: 17, not 32).
    let past = cal(b"P-RF-2", 10, &[1]);
    assert_eq!(authed(&mut w, &mut s, HOST, 10, Command::SetCalibration, &past, 100).0, Status::Malformed);
    assert_eq!(w.call(&mut s, HOST, &req(3, Command::SetCalibration, &past), 150).unwrap().1, Status::Malformed);
    // Well formed, unknown parameter id: 57 in a session, 50 armed outside one, 48 otherwise.
    let unknown = cal(b"P-XX-9", 1, &[1]);
    assert_eq!(authed(&mut w, &mut s, HOST, 20, Command::SetCalibration, &unknown, 200).0, Status::P2SessionRuns);
    w.p2 = false;
    assert_eq!(authed(&mut w, &mut s, HOST, 30, Command::SetCalibration, &unknown, 300).0, Status::Armed);
    w.armed = false;
    assert_eq!(authed(&mut w, &mut s, HOST, 40, Command::SetCalibration, &unknown, 400).0, Status::InvalidConfig);
    // A known id is accepted (its value encoding is not fixed yet, Table P-1).
    assert_eq!(authed(&mut w, &mut s, HOST, 50, Command::SetCalibration, &cal(b"P-RF-2", 1, &[1]), 500).0, Status::Ok);
    // Values that decode but are not allowed: SWITCH_LEVEL 3 is 48, UPDATE_ACTIVATE with the
    // foreign flag 56; a wrong size of either is 17.
    assert_eq!(authed(&mut w, &mut s, HOST, 60, Command::SwitchLevel, &[3], 600).0, Status::InvalidConfig);
    assert_eq!(authed(&mut w, &mut s, HOST, 70, Command::SwitchLevel, &[1, 1], 700).0, Status::Malformed);
    assert_eq!(authed(&mut w, &mut s, HOST, 80, Command::UpdateActivate, &[], 800).0, Status::Malformed);
}

#[test]
fn t_a1_76_get_stats_has_ll_unprobed() {
    // 0057: the counter of link-local addresses used without a complete probe.
    let mut s = Server::new();
    let mut w = World::new();
    s.stats.ll_unprobed = 2;
    let r = w.call(&mut s, HOST, &req(1, Command::GetStats, &[]), 0).unwrap();
    let got: Vec<(String, u64)> = mgmt::payload::stats(&r.2).map(|e| e.unwrap()).map(|(n, v)| (String::from_utf8(n.to_vec()).unwrap(), v)).collect();
    assert!(got.contains(&("ll_unprobed".to_string(), 2)), "{got:?}");
}

#[test]
fn reasons_before_a_commanded_reset_0068() {
    let last = |r: Reason| {
        let mut s = Server::new();
        s.log.push(1, format_args!("before"));
        r.log(&mut s.log.0, 77);
        let (a, b) = s.log.0.range();
        assert_eq!(a, 0, "sequence numbers start at 0");
        let e = s.log.0.entry(b - 1).unwrap();
        String::from_utf8(e.text().to_vec()).unwrap()
    };
    assert_eq!(last(Reason::Restart), "restart: restart");
    assert_eq!(last(Reason::FactoryReset), "restart: factory-reset");
    assert_eq!(last(Reason::Activated { slot: 2, version: 0x0001_0002 }), "restart: update-activated slot B 0.1.2");
    assert_eq!(last(Reason::UpdateAbandoned), "restart: update-abandoned");
    assert_eq!(last(Reason::TrialUnconfirmed), "restart: trial-unconfirmed");
    assert_eq!(last(Reason::Rollback(g7250_radio::mgmt::Rollback { from: (2, 0x0001_0002), to: (1, 0x0001_0001) })), "rollback commanded: slot B 0.1.2 to slot A 0.1.1");
}

#[test]
fn read_log_goes_on_over_a_reset_t_a1_34() {
    // The old run's last entry, the boot stage's lines, the new run: one sequence, no gap.
    use g7250_safety::logring::Ring;
    let mut s = Server::new();
    s.log.push(600_000, format_args!("update aborted by 192.168.1.5:50000"));
    Reason::UpdateAbandoned.log(&mut s.log.0, 600_700);
    let mem = s.log.0.into_memory();
    let (mut r, kept) = Ring::open(mem, false);
    assert!(kept);
    r.log(0, format_args!("start after software: log kept"));
    let mut s = Server::with_log(Ring::open(r.into_memory(), false).0);
    let mut w = World::new();
    s.log.push(6_000, format_args!("new run"));
    let log = w.call(&mut s, HOST, &req(1, Command::ReadLog, &[0, 0, 0, 0]), 7_000).unwrap().2;
    let e: Vec<_> = mgmt::payload::log_entries(&log).map(|e| e.unwrap()).map(|e| (e.seq, e.uptime_ms, String::from_utf8(e.text.to_vec()).unwrap())).collect();
    assert_eq!(
        e,
        [
            (0, 600_000, "update aborted by 192.168.1.5:50000".to_string()),
            (1, 600_700, "restart: update-abandoned".to_string()),
            (2, 0, "start after software: log kept".to_string()),
            (3, 6_000, "new run".to_string()),
        ]
    );
    // From a sequence number on.
    let log = w.call(&mut s, HOST, &req(2, Command::ReadLog, &[0, 0, 0, 3]), 8_000).unwrap().2;
    assert_eq!(mgmt::payload::log_entries(&log).count(), 1);
}

#[test]
fn store_full_64_is_logged_and_a_repeat_gets_it_again() {
    // 0067 addendum, T-A1-88: 64 from the updater; a repeat with the same id is answered from the
    // stored reply; a new request is a new try.
    assert_eq!(Status::from_u16(64), Some(Status::RoomMade));
    let mut s = Server::new();
    let mut w = World::new();
    w.secret = Some([7; 32]);
    assert_eq!(authed(&mut w, &mut s, HOST, 10, Command::UpdateBegin, &package(1300, 2), 0).0, Status::Ok);
    w.up.full = true;
    let ch = w.call(&mut s, HOST, &req(20, Command::GetChallenge, &[]), 1000).unwrap().2;
    let ch: [u8; 16] = ch.try_into().unwrap();
    let mut b = [0u8; 1000];
    let n = mgmt::build_request(&mut b, 21, Command::UpdateBlock, &block(0, &[0; 512]), Some((w.secret.as_ref().unwrap(), &ch))).unwrap();
    assert_eq!(w.call(&mut s, HOST, &b[..n], 1001).unwrap().1, Status::RoomMade);
    assert_eq!(w.call(&mut s, HOST, &b[..n], 1500).unwrap().1, Status::RoomMade, "a repeat: the stored 64");
    assert_eq!(authed(&mut w, &mut s, HOST, 30, Command::UpdateBlock, &block(0, &[0; 512]), 2000).0, Status::Ok, "a new request");
    let log = w.call(&mut s, HOST, &req(40, Command::ReadLog, &[0, 0, 0, 0]), 3000).unwrap().2;
    let e: Vec<_> = mgmt::payload::log_entries(&log).map(|e| String::from_utf8(e.unwrap().text.to_vec()).unwrap()).collect();
    assert_eq!(e.iter().filter(|t| *t == "store: room made (update)").count(), 1, "{e:?}");
}

/// T-A1-87 (radio part): while C-3 is lost, a SET_CONFIG of some C-3 items gets 48 and changes
/// nothing; with no calibration set a write of every C-3 item gets 62, one without tags 9 and 10
/// gets 48; a write with no C-3 item is stored as usual. The effect says whether a C-3 item was
/// written, for the compaction of 0072 point 1.
#[test]
fn c3_lost_ends_only_with_every_item_0072() {
    use g7250_radio::mgmt::Effect;
    use g7250_proto::mgmt::payload::Config;
    let mut s = Server::new();
    let mut w = World::new();
    w.secret = Some([7; 32]);
    let enc = |c: &Config| {
        let mut b = [0u8; 256];
        let n = c.encode(&mut b).unwrap();
        b[..n].to_vec()
    };
    let bands = g7250_radio::record::TX_BANDS.len();
    let all_but_calibrated = Config { max_tx_time_ms: Some(60_000), band_limit: core::array::from_fn(|i| (i < bands).then_some(0)), ..Default::default() };
    let every = Config { swr_s2_x100: Some(200), pa_temp_t3: Some(700), ..all_but_calibrated };
    let one_band = Config { band_limit: core::array::from_fn(|i| (i == 0).then_some(500)), ..Default::default() };
    let c1 = Config { default_level: Some(1), ..Default::default() };
    // Not lost: a C-3 item is flagged, a C-1 write is not.
    let mut k = 0u32;
    let mut send = |w: &mut World, s: &mut Server, c: &Config| {
        k += 1;
        let r = authed(w, s, HOST, 10 * k, Command::SetConfig, &enc(c), 1000 * k as u64).0;
        (r, s.take_effects())
    };
    let (st, fx) = send(&mut w, &mut s, &one_band);
    assert_eq!(st, Status::Ok);
    assert!(matches!(fx.as_slice(), [Effect::SetConfig(_, true)]), "{fx:?}");
    let (st, fx) = send(&mut w, &mut s, &c1);
    assert_eq!(st, Status::Ok);
    assert!(matches!(fx.as_slice(), [Effect::SetConfig(_, false)]), "{fx:?}");
    // Lost.
    w.c3_lost = true;
    for (c, want) in [(&one_band, Status::InvalidConfig), (&all_but_calibrated, Status::InvalidConfig), (&every, Status::NeedsCalibration)] {
        let (st, fx) = send(&mut w, &mut s, c);
        assert_eq!(st, want, "{c:?}");
        assert!(fx.is_empty(), "changes nothing");
    }
    let (st, fx) = send(&mut w, &mut s, &c1);
    assert_eq!(st, Status::Ok);
    assert!(matches!(fx.as_slice(), [Effect::SetConfig(_, false)]), "{fx:?}");
}

/// T-A1-95 (management part): GET_STATS lists every name of Table M-6 first, in table order,
/// each at 0 on a fresh start, then the firmware's own, all in one reply of at most 1400 bytes;
/// the excess of 25 requests in one second from one source is counted in mg_rate_drop and
/// drop_7251, not in mg_cap_drop (T-A1-26; 0074).
#[test]
fn get_stats_lists_table_m6_first_t_a1_95() {
    const M6: [&str; 46] = [
        "mg_not_request", "mg_rate_drop", "mg_cap_drop", "mg_auth_required", "disc_rate_drop", "disc_src1024", "setip_ignored", "gen1024_ignored", "cap_rate_drop", "notowner_drop", "reg7250_drop", "takeovers", "icmp_unreach", "hp_rx", "hp_stale", "hp_resync", "hp_release", "gap_1027", "gap_1029", "duc_foreign", "duc_not_tx", "duc_cw_drop", "duc_fifo_drop", "cw_speed_clamped", "cw_hang_clamped", "ll_unprobed", "drop_1024", "drop_1025", "drop_1026", "drop_1027", "drop_1029", "drop_7250", "drop_7251", "fault_2", "fault_3", "fault_9", "fault_10", "fault_16", "fault_17", "fault_18", "fault_19", "reset_unknown", "reset_software", "reset_watchdog", "reset_pin", "trip_count",
    ];
    let stats = |w: &mut World, s: &mut Server, id: u32, now: u64| {
        let r = w.call(s, HOST, &req(id, Command::GetStats, &[]), now).unwrap();
        assert!(mgmt::HEADER_LEN + r.2.len() <= 1400);
        mgmt::payload::stats(&r.2).map(|e| e.unwrap()).map(|(n, v)| (String::from_utf8(n.to_vec()).unwrap(), v)).collect::<Vec<_>>()
    };
    let mut s = Server::new();
    let mut w = World::new();
    let got = stats(&mut w, &mut s, 1, 0);
    let names: Vec<&str> = got.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(&names[..46], &M6[..], "Table M-6 first, in its order");
    assert!(got[..46].iter().all(|(n, v)| *v == 0 && n.len() <= 16), "{got:?}");
    assert!(names.len() > 46, "the firmware's own after them");
    assert!(!names[46..].iter().any(|n| M6.contains(n)), "no name twice");
    let answered = (0..25u32).filter(|i| w.call(&mut s, HOST, &req(100 + i, Command::Identify, &[]), 2000 + 10 * *i as u64).is_some()).count() as u64;
    assert!(answered < 25);
    let got = stats(&mut w, &mut s, 200, 5000);
    let v = |n: &str| got.iter().find(|(k, _)| k == n).unwrap().1;
    assert_eq!((v("mg_rate_drop"), v("drop_7251"), v("mg_cap_drop")), (25 - answered, 25 - answered, 0));
}
