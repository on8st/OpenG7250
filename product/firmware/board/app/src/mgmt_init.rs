// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! What the management server needs, prepared once at start from flash and the store
//! (A1.5 identification, A1.6 configuration, A1.12 calibration set; Annex M).

use g7250_layout::header::Kind as ImageKind;
use g7250_layout::{BOOT_STAGE, Range, Slot};
use g7250_proto::mgmt as m;
use g7250_proto::mgmt::payload::{Config, Identify, Version};
use g7250_store::{Judged, Kind, Limits, Stale, Store};

pub type NvStore = Store<crate::nv::BoardFlash>;

pub struct MgmtInit {
    /// Kept for the write commands (step 4).
    pub store: Option<NvStore>,
    pub secret: Option<[u8; 32]>,
    /// `GET_CONFIG` reply payload (Table M-3).
    pub config: heapless::Vec<u8, { m::payload::MAX_CONFIG }>,
    /// `GET_CALIBRATION` reply payload: set id (SHA-256 of the stored set), then the set.
    pub calibration: heapless::Vec<u8, 1100>,
    pub identify: Identify<'static>,
    /// The update path and the trial state (A1.8).
    pub update: crate::update::BoardUpdate,
    /// At start: flash words the store found unreadable (double ECC error), and which items then
    /// had no readable record (configuration factory values, no calibration set, no secret),
    /// for the net loop to log once (0065 point 5; T-A1-87).
    pub store_unreadable: u32,
    pub fell_back: (bool, bool, bool),
    /// What the store did at this start, for the log.
    pub store_notes: heapless::Vec<&'static str, 6>,
    /// C-3 is lost: TX closed on every band until a `SET_CONFIG` of every C-3 item (0072).
    pub c3_lost: bool,
}

const _: () = assert!(m::payload::MAX_BANDS == g7250_store::LIMIT_BANDS);

/// The configuration as the management server sees it (Table M-3): the stored record's items
/// other than C-3, and C-3 from the store's own record (A1.6 "C-3 at a start"; 0072). Lost: 0 W on
/// every band, the other C-3 items at their built-in values (A1.6 "Lost").
pub fn merged_config(record: Option<&[u8]>, l: &Limits) -> Config {
    let f = Config::factory();
    let c = Config::stored(record).unwrap_or(f);
    let bands = g7250_radio::record::TX_BANDS.len();
    if l.lost {
        return Config { max_tx_time_ms: f.max_tx_time_ms, band_limit: core::array::from_fn(|i| (i < bands).then_some(0)), swr_s2_x100: None, pa_temp_t3: None, ..c };
    }
    Config { max_tx_time_ms: l.max_tx_time_ms.or(f.max_tx_time_ms), band_limit: l.band, swr_s2_x100: l.swr_s2_x100, pa_temp_t3: l.pa_temp_t3, ..c }
}

/// The C-3 items of a configuration, as the store keeps them.
fn limits_of(c: &Config) -> Limits {
    Limits { lost: false, max_tx_time_ms: c.max_tx_time_ms, band: c.band_limit, swr_s2_x100: c.swr_s2_x100, pa_temp_t3: c.pa_temp_t3 }
}

/// The configuration record without C-3, which the store keeps in its own record (0072).
fn record_of(c: &Config) -> Config {
    Config { max_tx_time_ms: None, band_limit: [None; m::payload::MAX_BANDS], swr_s2_x100: None, pa_temp_t3: None, ..*c }
}

fn version_of(v: u32) -> Version {
    Version { major: (v >> 24) as u8, minor: (v >> 16) as u8, build: v as u16 }
}

/// The boot stage's version, if its image checks good.
fn boot_stage_version() -> Version {
    const RAM: Range = Range::new(0x2400_0000, 0x2402_0000);
    let mut io = g7250_board::flash::RawFlash::new(g7250_layout::Writer::Application { running: Slot::A });
    g7250_update::image::check(&mut io, ImageKind::BootStage, BOOT_STAGE, RAM, &mut || {}).map_or(Version::default(), |h| version_of(h.fw_version))
}

impl MgmtInit {
    pub fn new(mut store: Option<NvStore>, tx_arm_state: u32, slot: Slot, cause: g7250_safety::reset::Cause) -> Self {
        let mut buf = [0u8; g7250_store::MAX_PAYLOAD];
        let mut store_notes: heapless::Vec<&'static str, 6> = heapless::Vec::new();
        // A factory reset that did not end (a failed erase, a power loss): finished now, before
        // anything is read from the store and before the management socket opens (0069).
        if let Some(s) = store.as_mut() {
            if s.fell_back() {
                let _ = store_notes.push("store: newest generation not readable; the previous one is kept in use");
            }
            if s.wipe_pending().unwrap_or(false) {
                let _ = store_notes.push(if s.finish_wipe(factory_drops, Kind::Config, Config::after_factory_reset).is_ok() { "store: factory reset finished" } else { "store: factory reset not finished; erase failed" });
            }
        }
        let store_unreadable = store.as_ref().map_or(0, |s| s.unreadable_at_open());
        let mut get = |k: Kind, buf: &mut [u8]| store.as_mut().and_then(|s| s.get(k, buf).ok().flatten()).map(|(_, n)| n);

        let secret = match get(Kind::Secret, &mut buf) {
            Some(32) => buf[..32].try_into().ok(),
            _ => None,
        };
        let mut fell_back = (false, false, secret.is_none());
        // The start check (A1.10 "The old secret in flash"; 0069 and its addendum): another secret
        // than the live one, or data that is no complete record, in the sector that is not live.
        if let Some(s) = store.as_mut() {
            let (done, failed) = match s.stale(Kind::Secret, secret.as_ref().map(|s: &[u8; 32]| &s[..])) {
                Ok(Stale::Other) => ("store: stale secret erased", "store: stale secret not erased; erase failed"),
                Ok(Stale::Unrecognised) => ("store: non-live sector erased (unrecognised data)", "store: non-live sector not erased (unrecognised data); erase failed"),
                Ok(Stale::Nothing) | Err(_) => ("", ""),
            };
            if !done.is_empty() {
                let _ = store_notes.push(if s.erase_stale().is_ok() { done } else { failed });
            }
        }
        // C-3 from the store's own record, as judged at open (0072): logged once per start when
        // lost or taken from a failed generation.
        let limits = store.as_ref().map_or(Limits::BUILTIN, |s| s.limits());
        if limits.lost {
            let _ = store_notes.push("config: C-3 lost, TX closed until rewritten");
        } else if store.as_ref().is_some_and(|s| s.judged() == Judged::FailedGeneration) {
            let _ = store_notes.push("config: C-3 of a failed generation kept: each item the stricter");
        }
        let mut get = |k: Kind, buf: &mut [u8]| store.as_mut().and_then(|s| s.get(k, buf).ok().flatten()).map(|(_, n)| n);
        let record = match get(Kind::Config, &mut buf) {
            Some(n) if Config::stored(Some(&buf[..n])).is_some() => Some(&buf[..n]),
            _ => {
                fell_back.0 = true;
                None
            }
        };
        let mut config = heapless::Vec::new();
        let mut b = [0u8; m::payload::MAX_CONFIG];
        if let Some(n) = merged_config(record, &limits).encode(&mut b) {
            let _ = config.extend_from_slice(&b[..n]);
        }
        let mut get = |k: Kind, buf: &mut [u8]| store.as_mut().and_then(|s| s.get(k, buf).ok().flatten()).map(|(_, n)| n);

        let mut calibration = heapless::Vec::new();
        let (cal_id, cal_date) = match get(Kind::Calibration, &mut buf) {
            Some(n) if n >= 5 => {
                let id = m::auth::sha256(&buf[..n]);
                let _ = calibration.extend_from_slice(&id);
                let _ = calibration.extend_from_slice(&buf[..n]);
                (id, u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]))
            }
            _ => {
                fell_back.1 = true;
                // No set: id zeros, date 0, empty source, no entries.
                let _ = calibration.extend_from_slice(&[0u8; 37]);
                ([0u8; 32], 0)
            }
        };

        let update = crate::update::BoardUpdate::new(store.as_mut(), slot);
        let boot_v = boot_stage_version();
        let identify = Identify {
            model: b"G7250",
            mac: [0; 6],
            serial: None,
            running: Version { major: g7250_board::FW_MAJOR, minor: g7250_board::FW_MINOR, build: g7250_board::FW_BUILD },
            other_slot: Version::default(), // per request: the last judgement (other_slot)
            boot_stage: boot_v,
            hw_revision: 0,
            stock_flags: 0,
            integrity: [1, 0], // the running image: 1 while it runs (0067); the other: other_slot
            update_state: 0,
            secret_set: secret.is_some(),
            window: (false, 0),
            active_level: 1,
            owner: ([0; 4], 0),
            tx_arm_state,
            uptime_ms: 0,
            reset_cause: cause as u8, // tag 17, from the boot stage's one reading (0068)
            versions: [m::VERSION_MAJOR, m::VERSION_MINOR, 44, 1, 1],
            controller: 0,
            tx_calibrated: false,
            calibration_id: cal_id,
            calibration_date: cal_date,
            sectors_written: 0,
            running_slot: 0, // set below
            layout: g7250_layout::IDENTIFY_LAYOUT, // tag 25 (0062): the layout the link uses
            ll_unprobed: false, // set by the net loop (0057)
        };
        let mut identify = identify;
        identify.update_state = update.state as u8;
        identify.running_slot = g7250_update::stage::slot_code(slot);
        MgmtInit { store, secret, config, calibration, identify, update, store_unreadable, fell_back, store_notes, c3_lost: limits.lost }
    }
}

/// `IDENTIFY` tags 5 and 9 for the other slot: its last judgement (A1.8; 0067).
pub fn other_slot(id: &mut Identify<'_>, u: &crate::update::BoardUpdate) {
    id.integrity[1] = u.other;
    id.other_slot = if u.other == 1 { version_of(u.other_version) } else { Version::default() };
}

/// Carry out a store effect of a management write before the reply; returns the effects that
/// must wait for the reply (network change, reset).
pub fn apply_stored(m: &mut MgmtInit, e: g7250_radio::mgmt::Effect) -> Result<Option<g7250_radio::mgmt::Effect>, &'static str> {
    use g7250_radio::mgmt::Effect;
    match e {
        Effect::SetSecret(s) => {
            let st = m.store.as_mut().ok_or("no store")?;
            // A new secret over an old one: by a compaction, so that the old one is in a sector that
            // is no longer live; the next start, or the next compaction, erases it (0069).
            let r = if m.secret.is_some() { st.replace(Kind::Secret, 1, &s) } else { st.put(Kind::Secret, 1, &s) };
            r.map_err(|_| "secret")?;
            m.secret = Some(s);
            m.identify.secret_set = true;
            Ok(None)
        }
        Effect::SetConfig(v, c3) => {
            // C-3 in its own record, written by a compaction whenever the request held a C-3 item
            // (A1.6 C-3; 0072 point 1); the other items in the configuration record, in the same
            // generation then.
            let st = m.store.as_mut().ok_or("no store")?;
            let c = Config::decode(&v).map_err(|_| "configuration")?;
            let mut b = [0u8; m::payload::MAX_CONFIG];
            let n = record_of(&c).encode(&mut b).ok_or("configuration")?;
            if c3 {
                st.write_limits(Some((Kind::Config, 1, &b[..n])), limits_of(&c)).map_err(|_| "configuration")?;
            } else {
                st.put(Kind::Config, 1, &b[..n]).map_err(|_| "configuration")?;
            }
            let l = st.limits();
            m.c3_lost = l.lost;
            m.config.clear();
            let mut out = [0u8; m::payload::MAX_CONFIG];
            if let Some(k) = merged_config(Some(&b[..n]), &l).encode(&mut out) {
                let _ = m.config.extend_from_slice(&out[..k]);
            }
            Ok(None)
        }
        Effect::SetCalibration(v) => {
            let st = m.store.as_mut().ok_or("no store")?;
            st.put(Kind::Calibration, 1, &v).map_err(|_| "calibration")?;
            let id = m::auth::sha256(&v);
            m.calibration.clear();
            let _ = m.calibration.extend_from_slice(&id);
            let _ = m.calibration.extend_from_slice(&v);
            m.identify.calibration_id = id;
            m.identify.calibration_date = u32::from_be_bytes([v[0], v[1], v[2], v[3]]);
            Ok(None)
        }
        Effect::ClearCalibration => {
            let st = m.store.as_mut().ok_or("no store")?;
            st.compact_without(|k| k == Kind::Calibration).map_err(|_| "calibration")?;
            m.calibration.clear();
            let _ = m.calibration.extend_from_slice(&[0u8; 37]);
            m.identify.calibration_id = [0; 32];
            m.identify.calibration_date = 0;
            Ok(None)
        }
        later @ (Effect::ApplyNetwork | Effect::Restart(_) | Effect::FactoryReset | Effect::UpdateStarted) => Ok(Some(later)),
        // Applied by the net loop before any store write (0055).
        Effect::DisarmForMaintenance => Ok(None),
    }
}

/// The network configuration C-1 from the stored Table M-3 items; the factory one when they are
/// not valid (netcfg::from_items).
pub fn network_from_config(cfg: &[u8]) -> g7250_proto::netcfg::NetConfig {
    network_items(cfg).unwrap_or(g7250_proto::netcfg::NetConfig::FACTORY)
}

fn network_items(cfg: &[u8]) -> Option<g7250_proto::netcfg::NetConfig> {
    let c = Config::decode(cfg).ok()?;
    g7250_proto::netcfg::from_items(c.address_mode, c.address, c.netmask, c.gateway)
}

/// The C-1 configuration a start brings up (A1.6: the configuration survives a restart), and
/// the log line that names it. The factory configuration only when the store holds no readable
/// configuration record or its C-1 items are not valid.
pub fn network_at_start(m: &MgmtInit) -> (g7250_proto::netcfg::NetConfig, &'static str) {
    use g7250_proto::netcfg::{Mode, NetConfig};
    if m.fell_back.0 {
        return (NetConfig::FACTORY, "network at start: factory configuration (no stored configuration)");
    }
    match network_items(&m.config) {
        Some(c) if c.mode == Mode::Static => (c, "network at start: stored configuration (static)"),
        Some(c) => (c, "network at start: stored configuration (DHCP)"),
        None => (NetConfig::FACTORY, "network at start: factory configuration (stored C-1 items not valid)"),
    }
}

/// The fall-back of A1.6 C-4 after an address conflict on a static address: the C-1 items of
/// `prev` (the configuration before the change) back in the store, every other item kept.
pub fn restore_network(m: &mut MgmtInit, prev: &[u8]) -> Result<(), &'static str> {
    let cur = Config::decode(&m.config).unwrap_or_default();
    let back = cur.with_network_of(&Config::decode(prev).unwrap_or_default());
    let mut b = [0u8; m::payload::MAX_CONFIG];
    let n = back.encode(&mut b).ok_or("configuration")?;
    let mut v = heapless::Vec::new();
    let _ = v.extend_from_slice(&b[..n]);
    apply_stored(m, g7250_radio::mgmt::Effect::SetConfig(v, false)).map(|_| ())
}

/// A Set IP's result stored as the C-1 items (B3.9; T-B3-25): a static `want` that is a usable
/// host as static, anything else as DHCP. Returns whether the store was written (no write when
/// the items are already stored). The caller drops the arm first (B2.3; 0055).
pub fn store_set_ip(m: &mut MgmtInit, want: &g7250_proto::netcfg::NetConfig) -> Result<bool, &'static str> {
    use g7250_proto::netcfg::{valid_host, Mode};
    let cur = Config::decode(&m.config).unwrap_or_default();
    let usable = want.mode == Mode::Static && valid_host(want.ip, want.mask);
    let new = cur.with_set_ip(usable.then_some((want.ip, want.mask, want.gateway)));
    if (new.address_mode, new.address, new.netmask, new.gateway) == (cur.address_mode, cur.address, cur.netmask, cur.gateway) {
        return Ok(false);
    }
    let mut b = [0u8; m::payload::MAX_CONFIG];
    let n = new.encode(&mut b).ok_or("configuration")?;
    let mut v = heapless::Vec::new();
    let _ = v.extend_from_slice(&b[..n]);
    apply_stored(m, g7250_radio::mgmt::Effect::SetConfig(v, false)).map(|_| true)
}

/// The configured region C-6 from the stored Table M-3 items (factory: 0, Table A3-0).
pub fn region_from_config(cfg: &[u8]) -> u8 {
    Config::decode(cfg).ok().and_then(|c| c.region).unwrap_or(0)
}

/// Table A1-4 in the store: configuration (C-1, C-2, C-6, C-7) back to the factory values with the
/// C-3 limits kept, and the secret cleared, in one compaction; then a second one, so that no copy
/// of the secret is left in the other sector. Calibration set and boot counter are kept.
pub fn factory_reset_store(m: &mut MgmtInit) -> Result<(), &'static str> {
    let st = m.store.as_mut().ok_or("no store")?;
    st.wipe(factory_drops, Kind::Config, Config::after_factory_reset).map_err(|_| "factory reset")
}

/// The store item a factory reset drops (Table A1-4; the configuration is written new).
fn factory_drops(k: Kind) -> bool {
    k == Kind::Secret
}
