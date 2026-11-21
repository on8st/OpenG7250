// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! What the management server needs, prepared once at start from flash and the store
//! (A1.5 identification, A1.6 configuration, A1.12 calibration set; Annex M).

use g7250_layout::header::Kind as ImageKind;
use g7250_layout::{BOOT_STAGE, HEADER_OFFSET, Range, Slot};
use g7250_proto::mgmt as m;
use g7250_proto::mgmt::payload::{Config, Identify, Version};
use g7250_store::{Kind, Store};

pub type NvStore = Store<crate::nv::BoardFlash>;

pub struct MgmtInit {
    /// Kept for the write commands (step 4).
    pub store: Option<NvStore>,
    pub secret: Option<[u8; 32]>,
    /// `GET_CONFIG` reply payload (Table M-3).
    pub config: heapless::Vec<u8, 256>,
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
}

fn version_of(v: u32) -> Version {
    Version { major: (v >> 24) as u8, minor: (v >> 16) as u8, build: v as u16 }
}

/// Integrity code of Table M-2 tag 9 (0 none: erased header, 1 good, 2 bad) and the version.
fn image_state(kind: ImageKind, r: Range) -> (u8, Version) {
    const RAM: Range = Range::new(0x2400_0000, 0x2402_0000);
    let h = HEADER_OFFSET as usize;
    // An unreadable (ECC) word anywhere in the area: integrity bad (R0), and no slice read.
    let Ok(image) = g7250_board::flash::mapped(r) else { return (2, Version::default()) };
    if image[h..h + 4].iter().all(|&b| b == 0xFF) {
        return (0, Version::default());
    }
    match g7250_layout::header::validate(image, kind, r, RAM) {
        Ok(hdr) => (1, version_of(hdr.fw_version)),
        Err(_) => (2, Version::default()),
    }
}

/// Factory configuration (Table A1-4; A1.6): DHCP with link-local fall-back, no static
/// address, default level 1, region 0 (Table A3-0), maximum continuous TX time 5 min (0007
/// Resolved 3), empty TCI allow-list. Not reported yet: per-band power limits (tag 8) and S2/T3
/// (tags 9, 10: they need the calibration set).
fn factory_config() -> Config {
    Config {
        address_mode: Some(0),
        address: Some([0; 4]),
        netmask: Some([0; 4]),
        gateway: Some([0; 4]),
        default_level: Some(1),
        region: Some(0),
        max_tx_time_ms: Some(300_000),
        allow_count: Some(0),
        ..Default::default()
    }
}

impl MgmtInit {
    pub fn new(mut store: Option<NvStore>, tx_arm_state: u32, slot: Slot) -> Self {
        let mut buf = [0u8; g7250_store::MAX_PAYLOAD];
        let store_unreadable = store.as_ref().map_or(0, |s| s.unreadable_at_open());
        let mut get = |k: Kind, buf: &mut [u8]| store.as_mut().and_then(|s| s.get(k, buf).ok().flatten()).map(|(_, n)| n);

        let secret = match get(Kind::Secret, &mut buf) {
            Some(32) => buf[..32].try_into().ok(),
            _ => None,
        };
        let mut fell_back = (false, false, secret.is_none());

        let mut config = heapless::Vec::new();
        match get(Kind::Config, &mut buf) {
            Some(n) if n <= 256 && Config::decode(&buf[..n]).is_ok() => {
                let _ = config.extend_from_slice(&buf[..n]);
            }
            _ => {
                fell_back.0 = true;
                let mut b = [0u8; 256];
                if let Some(n) = factory_config().encode(&mut b) {
                    let _ = config.extend_from_slice(&b[..n]);
                }
            }
        }

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

        let (_, boot_v) = image_state(ImageKind::BootStage, BOOT_STAGE);
        let (other_int, other_v) = image_state(ImageKind::Application, slot.other().range());
        let identify = Identify {
            model: b"G7250",
            mac: [0; 6],
            serial: None,
            running: Version { major: g7250_board::FW_MAJOR, minor: g7250_board::FW_MINOR, build: g7250_board::FW_BUILD },
            other_slot: other_v,
            boot_stage: boot_v,
            hw_revision: 0,
            stock_flags: 0,
            integrity: [1, other_int],
            update_state: 0,
            secret_set: secret.is_some(),
            window: (false, 0),
            active_level: 1,
            owner: ([0; 4], 0),
            tx_arm_state,
            uptime_ms: 0,
            reset_cause: 0, // P-SYS-2: the codes are pending hardware
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
        let update = crate::update::BoardUpdate::new(store.as_mut(), slot);
        let mut identify = identify;
        identify.update_state = update.state as u8;
        identify.running_slot = g7250_update::stage::slot_code(slot);
        MgmtInit { store, secret, config, calibration, identify, update, store_unreadable, fell_back }
    }
}

/// Carry out a store effect of a management write before the reply; returns the effects that
/// must wait for the reply (network change, reset).
pub fn apply_stored(m: &mut MgmtInit, e: g7250_radio::mgmt::Effect) -> Result<Option<g7250_radio::mgmt::Effect>, &'static str> {
    use g7250_radio::mgmt::Effect;
    match e {
        Effect::SetSecret(s) => {
            let st = m.store.as_mut().ok_or("no store")?;
            st.put(Kind::Secret, 1, &s).map_err(|_| "secret")?;
            m.secret = Some(s);
            m.identify.secret_set = true;
            Ok(None)
        }
        Effect::SetConfig(v) => {
            let st = m.store.as_mut().ok_or("no store")?;
            st.put(Kind::Config, 1, &v).map_err(|_| "configuration")?;
            m.config.clear();
            let _ = m.config.extend_from_slice(&v);
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
        later @ (Effect::ApplyNetwork | Effect::Restart | Effect::FactoryReset | Effect::UpdateStarted) => Ok(Some(later)),
        // Applied by the net loop before any store write (0055).
        Effect::DisarmForMaintenance => Ok(None),
    }
}

/// The network configuration C-1 from the stored Table M-3 items.
pub fn network_from_config(cfg: &[u8]) -> g7250_proto::netcfg::NetConfig {
    use g7250_proto::netcfg::{Mode, NetConfig};
    let c = Config::decode(cfg).unwrap_or_default();
    match (c.address_mode, c.address, c.netmask) {
        (Some(1), Some(ip), Some(mask)) => NetConfig { mode: Mode::Static, ip, mask, gateway: c.gateway.unwrap_or([0; 4]) },
        _ => NetConfig::FACTORY,
    }
}

/// The configured region C-6 from the stored Table M-3 items (factory: 0, Table A3-0).
pub fn region_from_config(cfg: &[u8]) -> u8 {
    Config::decode(cfg).ok().and_then(|c| c.region).unwrap_or(0)
}

/// Table A1-4 in the store: configuration (C-1, C-2, C-6, C-7) back to the factory values and the
/// secret cleared, by leaving them out of a compaction; twice, so that no copy of the secret is
/// left in the other sector. Calibration set and boot counter are kept.
pub fn factory_reset_store(m: &mut MgmtInit) -> Result<(), &'static str> {
    let st = m.store.as_mut().ok_or("no store")?;
    let drop = |k: Kind| k == Kind::Secret || k == Kind::Config;
    st.compact_without(drop).map_err(|_| "factory reset")?;
    st.compact_without(drop).map_err(|_| "factory reset")?;
    Ok(())
}
