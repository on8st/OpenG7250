//! Unit tests of the model core against spec statements (no network).

use std::net::{Ipv4Addr, SocketAddrV4};

use g7250_refmodel::codes::*;
use g7250_refmodel::radio::{Config, Radio, Rx, Tx, MS, S};

fn host(p: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), p)
}
fn rx<'a>(src: SocketAddrV4, port: u16, data: &'a [u8]) -> Rx<'a> {
    Rx { src, local_port: port, to_ip: None, data }
}
fn general(wd: bool, pa: bool) -> Vec<u8> {
    let mut g = vec![0u8; 60];
    g[37] = 0b1000; // phase word
    g[38] = wd as u8;
    g[58] = pa as u8;
    g
}
fn ddc(rate: u16) -> Vec<u8> {
    let mut d = vec![0u8; 1444];
    d[7] = 1;
    d[18..20].copy_from_slice(&rate.to_be_bytes());
    d
}
fn hp(run: bool, ptt: bool, duc_hz: f64) -> Vec<u8> {
    let mut h = vec![0u8; 1444];
    h[4] = run as u8 | (ptt as u8) << 1;
    let pw = (duc_hz * 4_294_967_296.0 / DSP_CLOCK_HZ as f64) as u32;
    h[329..333].copy_from_slice(&pw.to_be_bytes());
    h[345] = 255;
    h
}
fn regw(pairs: &[(u16, u32)]) -> Vec<u8> {
    let mut v = vec![0u8; 4];
    for (a, d) in pairs {
        v.extend_from_slice(&a.to_be_bytes());
        v.extend_from_slice(&d.to_be_bytes());
    }
    v
}
fn pairs(t: &Tx) -> Vec<(u16, u32)> {
    t.data[4..].chunks(6).map(|c| (u16::from_be_bytes([c[0], c[1]]), u32::from_be_bytes([c[2], c[3], c[4], c[5]]))).collect()
}
fn find(out: &[Tx], a: u16) -> Option<u32> {
    out.iter().filter(|t| t.src_port == port::REGMAP).flat_map(pairs).find(|(x, _)| *x == a).map(|(_, v)| v)
}

/// Start a full session for host A: General, DDC-specific, run.
fn session(r: &mut Radio, a: SocketAddrV4, t: u64) -> Vec<Tx> {
    let mut out = r.on_datagram(t, &rx(a, port::DISCOVERY, &general(true, true)));
    out.extend(r.on_datagram(t, &rx(a, port::DDC_SPECIFIC, &ddc(192))));
    out.extend(r.on_datagram(t, &rx(a, port::HIGH_PRIORITY, &hp(true, false, 14.1e6))));
    out
}

#[test]
fn discovery_reply_b3() {
    let mut r = Radio::new(Config::default(), 0);
    let mut q = vec![0u8; 60];
    q[4] = 2;
    let out = r.on_datagram(0, &rx(host(50000), port::DISCOVERY, &q));
    assert_eq!(out.len(), 1);
    let d = &out[0].data;
    assert_eq!((d.len(), d[4], d[11], d[12], d[20], d[21], d[22]), (60, 2, 200, 44, 1, 1, 0));
    assert_eq!(out[0].src_port, 1024);
    assert_eq!(out[0].dst, host(50000));
    // B3.2: from source port 1024: no reply
    assert!(r.on_datagram(0, &rx(host(1024), port::DISCOVERY, &q)).is_empty());
    // B3.4: running for A, discovery from B (other port) -> 0x03; from A -> 0x02
    session(&mut r, host(50000), 0);
    let out = r.on_datagram(MS, &rx(host(50001), port::DISCOVERY, &q));
    assert_eq!(out[0].data[4], 3);
    let out = r.on_datagram(2 * MS, &rx(host(50000), port::DISCOVERY, &q));
    assert_eq!(out[0].data[4], 2);
}

#[test]
fn record_on_request_b4() {
    let mut r = Radio::new(Config::default(), 0);
    let out = r.on_datagram(0, &rx(host(40000), port::REGMAP, &regw(&[(reg::CAP_REQUEST, 1)])));
    assert_eq!(out.len(), 1);
    let p = pairs(&out[0]);
    assert_eq!(p[0], (reg::CAP_REQUEST, 1));
    assert_eq!(p.len(), 1 + 207);
    assert_eq!(p[1], (0, 0x4737_3235));
    assert_eq!(p[1 + 0x52], (0x52, 0x1C53_0001));
    assert_eq!(p[1 + 0x20].1 & 0b1_0010, 0b10, "tx bit 1 = 1 (0036), bit 4 = 0 until calibrated");
    // B4.4 item 3: rate limit 1/s per non-owner source
    assert!(r.on_datagram(200 * MS, &rx(host(40000), port::REGMAP, &regw(&[(reg::CAP_REQUEST, 1)]))).is_empty());
    assert_eq!(r.on_datagram(1100 * MS, &rx(host(40000), port::REGMAP, &regw(&[(reg::CAP_REQUEST, 1)]))).len(), 1);
}

#[test]
fn locked_arm_rejected_and_atu_not_calibrated() {
    let mut r = Radio::new(Config::default(), 0);
    let a = host(50000);
    session(&mut r, a, 0);
    let out = r.on_datagram(MS, &rx(a, port::REGMAP, &regw(&[(reg::TX_MODE_CLASS, 1)])));
    assert_eq!(find(&out, reg::TX_MODE_CLASS), Some(1));
    let tok = r.token;
    let out = r.on_datagram(2 * MS, &rx(a, port::REGMAP, &regw(&[(reg::TX_ARM, tok ^ ARM_MAGIC)])));
    let st = find(&out, reg::TX_ARM_STATE).unwrap();
    assert_eq!(st & 0xF, 0, "not armed");
    assert_eq!((st >> 8) & 0xFF, arm::NOT_CALIBRATED as u32);
    // PTT edge does not key
    let out = r.on_datagram(30 * MS, &rx(a, port::HIGH_PRIORITY, &hp(true, true, 14.1e6)));
    assert_eq!(find(&out, reg::TX_STATE).unwrap_or(0) & 1, 0);
    assert!(!r.tx.keyed && !r.hw.tx_levels);
    // ATU_TUNE = 1: NOT_CALIBRATED; = 0 accepted
    let out = r.on_datagram(40 * MS, &rx(a, port::REGMAP, &regw(&[(reg::ATU_TUNE, 1)])));
    assert_eq!(find(&out, reg::WRITE_STATUS).unwrap() & 0xFF, wr::NOT_CALIBRATED as u32);
    let out = r.on_datagram(41 * MS, &rx(a, port::REGMAP, &regw(&[(reg::ATU_TUNE, 0)])));
    assert_eq!(find(&out, reg::WRITE_STATUS), None);
}

#[test]
fn unlocked_arm_and_key() {
    let cfg = Config { calibrated: true, ..Config::default() };
    let mut r = Radio::new(cfg, 0);
    let a = host(50000);
    session(&mut r, a, 0);
    r.on_datagram(MS, &rx(a, port::REGMAP, &regw(&[(reg::TX_MODE_CLASS, 1)])));
    let tok = r.token;
    let out = r.on_datagram(2 * MS, &rx(a, port::REGMAP, &regw(&[(reg::TX_ARM, tok ^ ARM_MAGIC)])));
    assert_eq!(find(&out, reg::TX_ARM_STATE).unwrap() & 0xF, 1);
    let out = r.on_datagram(30 * MS, &rx(a, port::HIGH_PRIORITY, &hp(true, true, 14.1e6)));
    assert_eq!(find(&out, reg::TX_STATE).unwrap() & 1, 1);
    // keyed timeout 500 ms: no HP packet -> unkey, arm kept
    let mut t = 30 * MS;
    let mut unkeyed = None;
    while t < 700 * MS {
        t += MS;
        r.on_datagram(t, &rx(a, port::DUC_SPECIFIC, &{ let mut d = vec![0u8; 60]; d[14] = 0; d })); // feeds watchdog
        if find(&r.tick(t), reg::TX_STATE).map_or(false, |w| w & 1 == 0) {
            unkeyed = Some(t);
            break;
        }
    }
    let u = unkeyed.expect("unkeyed by keyed timeout");
    assert!(u >= 530 * MS && u <= 541 * MS, "{u}");
    assert_eq!(r.arm.state, 1);
}

#[test]
fn takeover_three_notices_a2_4() {
    let mut r = Radio::new(Config::default(), 0);
    let (a, b) = (host(50000), host(50001));
    session(&mut r, a, 0);
    let out = r.on_datagram(10 * MS, &rx(b, port::HIGH_PRIORITY, &hp(true, false, 14.1e6)));
    let to_a: Vec<_> = out.iter().filter(|t| t.dst == a && t.src_port == port::REGMAP).collect();
    assert_eq!(to_a.len(), 1);
    let st = pairs(to_a[0]).iter().find(|(x, _)| *x == reg::TX_ARM_STATE).unwrap().1;
    assert_eq!((st >> 16) & 0xFF, drop::TAKEOVER as u32);
    let mut n = 1;
    for t in 11..40 {
        n += r.tick(t * MS).iter().filter(|t| t.dst == a && t.src_port == port::REGMAP).count();
    }
    assert_eq!(n, 3);
    assert_eq!(r.session.as_ref().unwrap().owner, b);
}

#[test]
fn watchdog_b5_6() {
    let mut r = Radio::new(Config::default(), 0);
    session(&mut r, host(50000), 0);
    let mut t = 0;
    while r.session.is_some() {
        t += MS;
        r.tick(t);
    }
    assert!(t > S && t <= 1100 * MS, "{t}");
}

#[test]
fn register_rejections_b8_4() {
    let mut r = Radio::new(Config::default(), 0);
    let a = host(50000);
    session(&mut r, a, 0);
    let w = |r: &mut Radio, t: u64, p: &[(u16, u32)]| r.on_datagram(t, &rx(a, port::REGMAP, &regw(p)));
    let reason = |out: &[Tx]| find(out, reg::WRITE_STATUS).map(|v| v & 0xFF);
    assert_eq!(reason(&w(&mut r, MS, &[(0x0001, 5)])), Some(wr::READ_ONLY as u32));
    assert_eq!(reason(&w(&mut r, MS, &[(0x00D0, 5)])), Some(wr::UNKNOWN_ADDRESS as u32));
    assert_eq!(reason(&w(&mut r, MS, &[(0x0300, 5)])), Some(wr::READ_ONLY as u32), "group 3 present (0035)");
    assert_eq!(reason(&w(&mut r, MS, &[(0x0305, 5)])), Some(wr::UNKNOWN_ADDRESS as u32), "no separate supply current");
    assert_eq!(reason(&w(&mut r, MS, &[(0x0600, 5)])), Some(wr::UNKNOWN_ADDRESS as u32), "group 6 absent");
    assert_eq!(reason(&w(&mut r, MS, &[(reg::KEYED_TIMEOUT, 99)])), Some(wr::BAD_VALUE as u32));
    assert_eq!(reason(&w(&mut r, MS, &[(reg::LIMIT_TX_TIME, 400_000)])), Some(wr::LOOSER_LIMIT as u32));
    let out = w(&mut r, MS, &[(reg::LIMIT_TX_TIME, 10_000)]);
    assert_eq!(find(&out, reg::LIMIT_TX_TIME), Some(10_000));
    let out = w(&mut r, MS, &[(reg::LIMIT_TX_TIME, 0)]);
    assert_eq!(find(&out, reg::LIMIT_TX_TIME), Some(300_000));
    // padding ignored, no answer
    assert!(w(&mut r, MS, &[(0xFFFF, 1), (0, 0)]).is_empty());
    // non-owner: NOT_OWNER, at most one per 100 ms
    let b = host(50001);
    let o = r.on_datagram(2 * MS, &rx(b, port::REGMAP, &regw(&[(reg::KEYED_TIMEOUT, 200)])));
    assert_eq!(reason(&o), Some(wr::NOT_OWNER as u32));
    assert!(r.on_datagram(3 * MS, &rx(b, port::REGMAP, &regw(&[(reg::KEYED_TIMEOUT, 200)]))).is_empty());
    // non-owner from source port 7250: nothing
    assert!(r.on_datagram(300 * MS, &rx(host(7250), port::REGMAP, &regw(&[(reg::CAP_REQUEST, 1)]))).is_empty());
}

#[test]
fn freeze_rule_annex_p() {
    // tx bit 4 = 0: arm NOT_CALIBRATED (last in the B10.2 order), DUC0 reason 14,
    // LIMIT_SWR NOT_CALIBRATED, ATU_STATE 5 (T-B10-41, T-B10-42, T-A1-64)
    let mut r = Radio::new(Config::default(), 0);
    let a = host(50000);
    session(&mut r, a, 0);
    let tok = r.token;
    let out = r.on_datagram(MS, &rx(a, port::REGMAP, &regw(&[(reg::TX_ARM, tok ^ ARM_MAGIC)])));
    assert_eq!((find(&out, reg::TX_ARM_STATE).unwrap() >> 8) & 0xFF, arm::NO_MODE_CLASS as u32, "order");
    r.on_datagram(2 * MS, &rx(a, port::REGMAP, &regw(&[(reg::TX_MODE_CLASS, 2)])));
    let out = r.on_datagram(3 * MS, &rx(a, port::REGMAP, &regw(&[(reg::TX_ARM, tok ^ ARM_MAGIC)])));
    assert_eq!((find(&out, reg::TX_ARM_STATE).unwrap() >> 8) & 0xFF, arm::NOT_CALIBRATED as u32);
    let out = r.on_datagram(4 * MS, &rx(a, port::REGMAP, &regw(&[(reg::STREAM_STATUS, 0), (reg::ATU_STATE, 0)])));
    assert_eq!((find(&out, reg::STREAM_STATUS).unwrap() >> 8) & 0x7F, sr::TX_NOT_CALIBRATED as u32);
    assert_eq!(find(&out, reg::ATU_STATE), Some(5));
    let out = r.on_datagram(5 * MS, &rx(a, port::REGMAP, &regw(&[(reg::LIMIT_SWR, 2000)])));
    assert_eq!(find(&out, reg::WRITE_STATUS).unwrap() & 0xFF, wr::NOT_CALIBRATED as u32);
    // with a complete set the arm is accepted (T-B10-41, reference model)
    let mut r = Radio::new(Config { calibrated: true, ..Config::default() }, 0);
    session(&mut r, a, 0);
    r.on_datagram(MS, &rx(a, port::REGMAP, &regw(&[(reg::TX_MODE_CLASS, 2)])));
    let tok = r.token;
    let out = r.on_datagram(2 * MS, &rx(a, port::REGMAP, &regw(&[(reg::TX_ARM, tok ^ ARM_MAGIC)])));
    assert_eq!(find(&out, reg::TX_ARM_STATE).unwrap() & 0xF, 1);
    assert_eq!(r.record[0x20] >> 4 & 1, 1);
}

#[test]
fn record_table_a3_0_and_groups() {
    let r = Radio::new(Config::default(), 0);
    assert_eq!(r.record[0x21], 10);
    assert_eq!((r.record[0x22], r.record[0x23]), (1_800_000, 2_000_000));
    assert_eq!((r.record[0x22 + 27], r.record[0x23 + 27]), (50_000_000, 54_000_000));
    assert_eq!(r.record[0x53], 0x3F);
    assert_eq!(r.record[0x10], 0x07, "192 ksps maximum until P-NET-1");
    assert_eq!(r.record[0x57] & 0x1F, 0, "cal_info bits 0-4 = 0 uncalibrated");
}
