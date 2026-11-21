// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Update, trial boot, confirmation and rollback with power cut at every flash step.

use crate::boot::{self, UpdateState};
use crate::sim::Sim;
use crate::stage::{self, Offer, Phase, Refusal, Running, Stager};
use g7250_layout::header::{self, Header, Kind as ImgKind};
use g7250_layout::select::{self, Choice};
use g7250_layout::{BOOT_STAGE, HEADER_OFFSET, Range, STORE, Slot, VENDOR_BOOTLOADER, Writer};
use g7250_store::{Kind, Store};
use sha2::{Digest, Sha256};

const RAM: Range = Range::new(0x2400_0000, 0x2402_0000);
const V1: u32 = header::version(1, 0, 1);
const V2: u32 = header::version(1, 0, 2);

/// A sealed application image for `slot`, `len` bytes, filled from `seed`.
fn image(slot: Slot, version: u32, len: usize, seed: u8) -> Vec<u8> {
    let mut v: Vec<u8> = (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect();
    v[0..4].copy_from_slice(&0x2401_0000u32.to_le_bytes());
    v[4..8].copy_from_slice(&((slot.range().start + 0x501) as u32).to_le_bytes());
    let h = header::to_bytes(&Header::unsealed(ImgKind::Application, slot.range().start, version, false, "test"));
    let o = HEADER_OFFSET as usize;
    v[o..o + h.len()].copy_from_slice(&h);
    header::seal(&mut v).unwrap();
    v
}

fn offer(img: &[u8], version: u32) -> Offer {
    Offer {
        foreign: false,
        downgrade: false,
        header_version: 1,
        component: 1,
        hardware_id: stage::HARDWARE_ID,
        min_hw_revision: 0,
        version,
        length: img.len() as u32,
        sha256: Sha256::digest(img).into(),
        signature_len: 0,
        target_slot: 2,
    }
}

fn running(slot: Slot, version: u32) -> Running {
    Running { slot, version, hw_revision: 0, trial: false, app_ram: RAM }
}

fn valid(s: &Sim, slot: Slot) -> bool {
    let r = slot.range();
    header::validate(s.bytes(r.start, r.len()), ImgKind::Application, r, RAM).is_ok()
}

/// The boot stage at a start: its choice (writes the store as the boot stage).
fn boot(s: &mut Sim) -> Choice {
    let (va, vb) = (valid(s, Slot::A), valid(s, Slot::B));
    let Ok(mut st) = Store::open(&mut *s, Writer::BootStage) else { return Choice::NoImage };
    boot::boot_stage(&mut st, |x| if x == Slot::A { va } else { vb })
}

fn record(s: &mut Sim) -> Option<select::BootRecord> {
    let mut st = Store::open(&mut *s, Writer::BootStage).unwrap();
    boot::load(&mut st).unwrap()
}

/// A whole update from the running slot, as the management server drives it. Stops at the first
/// error (power lost).
fn update(s: &mut Sim, run: &Running, img: &[u8], version: u32) -> Result<(), Refusal> {
    let mut g = Stager::new();
    let o = offer(img, version);
    let n = g.begin(&o, run, 0)?;
    for b in 0..n {
        let d = &img[(b * stage::BLOCK) as usize..((b + 1) * stage::BLOCK).min(img.len() as u32) as usize];
        g.block(&mut *s, run, b, true, d, 1)?;
    }
    g.verify(&mut *s, run, 2)?;
    let mut st = Store::open(&mut *s, Writer::Application { running: run.slot }).map_err(|_| Refusal::VerifyFailed)?;
    g.activate(&mut st, run, 3)
}

/// Slot A holds V1 (first install), nothing else. `fill`: the store's live sector nearly full, so
/// that the next record write compacts.
fn installed(fill: bool) -> (Sim, Vec<u8>) {
    let mut s = Sim::new();
    let a = image(Slot::A, V1, 3000, 1);
    s.install(Slot::A.range().start, &a);
    if fill {
        let mut st = Store::open(&mut s, Writer::BootStage).unwrap();
        while st.free() >= 64 {
            st.put(Kind::Config, 1, &[7u8; 40]).unwrap();
        }
    }
    s.steps = 0;
    (s, a)
}

fn steps_of(s: &Sim, f: impl Fn(&mut Sim)) -> usize {
    let mut c = s.clone();
    c.steps = 0;
    f(&mut c);
    assert!(!c.lost);
    c.steps
}

#[test]
fn power_loss_at_every_step_of_an_update_leaves_the_old_image_running() {
    for fill in [false, true] {
        let (base, a) = installed(fill);
        let b = image(Slot::B, V2, 2600, 9);
        let run = running(Slot::A, V1);
        // With the store full the activation record compacts first (a sector erase in the cut).
        let generation = |s: &mut Sim| Store::open(&mut *s, Writer::BootStage).unwrap().generation();
        let mut c = base.clone();
        let g0 = generation(&mut c);
        update(&mut c, &run, &b, V2).unwrap();
        assert_eq!(generation(&mut c) != g0, fill);
        let total = steps_of(&base, |s| update(s, &run, &b, V2).unwrap());
        assert!(total > 80, "{total}");
        for cut in 0..=total {
            let mut s = base.clone();
            s.budget = Some(cut);
            let done = update(&mut s, &run, &b, V2).is_ok() && !s.lost;
            s.power_on();
            // The running image is never touched; sector 0 and the boot stage are outside the
            // model (any write there fails the test).
            assert_eq!(s.bytes(Slot::A.range().start, a.len() as u32), &a[..]);
            let c = boot(&mut s);
            if done {
                assert_eq!(cut, total);
                assert_eq!(c, Choice::Trial(Slot::B), "fill {fill}");
            } else {
                assert_eq!(c, Choice::Boot(Slot::A), "fill {fill} cut {cut}/{total}");
                // And a new update from there completes (resumable from scratch).
                assert!(update(&mut s, &run, &b, V2).is_ok());
                assert_eq!(boot(&mut s), Choice::Trial(Slot::B));
            }
        }
    }
}

/// After an activation: the store holds the trial record.
fn activated(fill: bool) -> (Sim, Vec<u8>, Vec<u8>) {
    let (mut s, a) = installed(fill);
    let b = image(Slot::B, V2, 2600, 9);
    update(&mut s, &running(Slot::A, V1), &b, V2).unwrap();
    if fill {
        // Fill again so that the boot stage's and the confirmation's records compact.
        let mut st = Store::open(&mut s, Writer::BootStage).unwrap();
        while st.free() >= 64 {
            st.put(Kind::Config, 1, &[7u8; 40]).unwrap();
        }
    }
    s.steps = 0;
    (s, a, b)
}

fn confirm(s: &mut Sim, running: Slot) -> bool {
    let Ok(mut st) = Store::open(&mut *s, Writer::Application { running }) else { return false };
    boot::confirm(&mut st, running).unwrap_or(false)
}

#[test]
fn power_loss_at_every_step_of_a_trial_start_and_confirmation() {
    for fill in [false, true] {
        let (base, a, b) = activated(fill);
        let n_boot = steps_of(&base, |s| assert_eq!(boot(s), Choice::Trial(Slot::B)));
        let n_conf = steps_of(&base, |s| {
            boot(s);
            assert!(confirm(s, Slot::B));
        }) - n_boot;
        for cut in 0..=n_boot + n_conf {
            let mut s = base.clone();
            s.budget = Some(cut);
            let c = boot(&mut s);
            if !s.lost {
                assert_eq!(c, Choice::Trial(Slot::B));
                // The trial runs: an unconfirmed trial is its update state (arm MAINTENANCE).
                let r = record(&mut s);
                if !s.lost {
                    assert_eq!(boot::state(r, Slot::B), UpdateState::Trial);
                }
                confirm(&mut s, Slot::B);
            }
            let lost = s.lost;
            s.power_on();
            let next = boot(&mut s);
            let want = if cut < n_boot {
                Choice::Trial(Slot::B) // the attempt was not recorded: tried again
            } else if lost {
                Choice::Boot(Slot::A) // attempt used, no confirmation: rolled back (U-6)
            } else {
                Choice::Boot(Slot::B) // confirmed
            };
            assert_eq!(next, want, "fill {fill} cut {cut} of {n_boot}+{n_conf}");
            if want == Choice::Boot(Slot::A) {
                assert_eq!(boot::state(record(&mut s), Slot::A), UpdateState::RolledBack);
            }
            // Neither image is ever damaged.
            assert_eq!(s.bytes(Slot::A.range().start, a.len() as u32), &a[..]);
            assert_eq!(s.bytes(Slot::B.range().start, b.len() as u32), &b[..]);
            // The choice is stable from then on.
            assert_eq!(boot(&mut s), if want == Choice::Trial(Slot::B) { Choice::Boot(Slot::A) } else { want });
        }
    }
}

#[test]
fn trial_without_confirmation_rolls_back_with_power_loss_at_every_step_of_the_rollback() {
    for fill in [false, true] {
        let (mut base, a, _) = activated(fill);
        assert_eq!(boot(&mut base), Choice::Trial(Slot::B)); // attempt used; the trial hangs
        base.steps = 0;
        let n = steps_of(&base, |s| assert_eq!(boot(s), Choice::Boot(Slot::A)));
        assert!(n > 0);
        for cut in 0..=n {
            let mut s = base.clone();
            s.budget = Some(cut);
            let _ = boot(&mut s);
            s.power_on();
            // Whatever the cut: slot A starts, and the record says rolled back.
            assert_eq!(boot(&mut s), Choice::Boot(Slot::A), "fill {fill} cut {cut}/{n}");
            let r = record(&mut s).unwrap();
            assert!(r.rolled_back && r.trial.is_none() && r.active == Slot::A);
            assert_eq!(s.bytes(Slot::A.range().start, a.len() as u32), &a[..]);
        }
    }
}

#[test]
fn power_loss_at_every_step_of_a_rollback_command() {
    let (mut base, _, _) = activated(true);
    boot(&mut base);
    assert!(confirm(&mut base, Slot::B));
    assert_eq!(boot(&mut base), Choice::Boot(Slot::B));
    let rb = |s: &mut Sim| {
        if let Ok(mut st) = Store::open(&mut *s, Writer::Application { running: Slot::B }) {
            let _ = boot::rollback(&mut st, Slot::B);
        }
    };
    let n = steps_of(&base, rb);
    for cut in 0..=n {
        let mut s = base.clone();
        s.budget = Some(cut);
        rb(&mut s);
        let lost = s.lost;
        s.power_on();
        assert_eq!(boot(&mut s), if lost { Choice::Boot(Slot::B) } else { Choice::Boot(Slot::A) }, "cut {cut}/{n}");
    }
}

#[test]
fn begin_checks_in_order_and_writes_nothing() {
    let (mut s, _) = installed(false);
    let b = image(Slot::B, V2, 2600, 9);
    let run = running(Slot::A, V1);
    let mut g = Stager::new();
    let before = s.clone().mem;
    let o = offer(&b, V2);
    assert_eq!(g.begin(&Offer { hardware_id: 0x00C9, ..o }, &run, 0), Err(Refusal::WrongHardware));
    assert_eq!(g.begin(&Offer { min_hw_revision: 3, ..o }, &run, 0), Err(Refusal::WrongHardware));
    assert_eq!(g.begin(&Offer { signature_len: 64, ..o }, &run, 0), Err(Refusal::Malformed));
    assert_eq!(g.begin(&Offer { foreign: true, ..o }, &run, 0), Err(Refusal::NotSupported));
    assert_eq!(g.begin(&Offer { length: Slot::B.range().len() + 1, ..o }, &run, 0), Err(Refusal::TooLarge));
    assert_eq!(g.begin(&Offer { version: header::version(1, 0, 0), ..o }, &run, 0), Err(Refusal::Older));
    assert_eq!(g.begin(&o, &Running { trial: true, ..run }, 0), Err(Refusal::UpdateInProgress));
    // 0043, T-A1-67: a package for the running slot (A) gets 63; byte 52 above 2 is malformed.
    assert_eq!(g.begin(&Offer { target_slot: 1, ..o }, &run, 0), Err(Refusal::RunningSlot));
    assert_eq!(g.begin(&Offer { target_slot: 3, ..o }, &run, 0), Err(Refusal::Malformed));
    assert!(!g.in_progress());
    assert_eq!(s.mem, before);
    // A downgrade with the flag is accepted (U-8); the same version is not a downgrade.
    assert!(g.begin(&Offer { version: header::version(1, 0, 0), downgrade: true, ..o }, &run, 0).is_ok());
    assert!(g.begin(&Offer { version: V1, ..o }, &run, 0).is_ok());
    assert!(g.in_progress());
    let _ = &mut s;
}

#[test]
fn blocks_crc_order_repeat_and_verify() {
    let (mut s, _) = installed(false);
    let b = image(Slot::B, V2, 1300, 9); // three blocks, the last 276 bytes
    let run = running(Slot::A, V1);
    let mut g = Stager::new();
    let o = offer(&b, V2);
    assert_eq!(g.block(&mut s, &run, 0, true, &b[..512], 0), Err(Refusal::OutOfOrder)); // no begin
    assert_eq!(g.begin(&o, &run, 0), Ok(3));
    assert_eq!(g.block(&mut s, &run, 0, false, &b[..512], 0), Err(Refusal::BadCrc));
    assert_eq!(g.block(&mut s, &run, 1, true, &b[512..1024], 0), Err(Refusal::OutOfOrder));
    assert_eq!(g.block(&mut s, &run, 0, true, &b[..500], 0), Err(Refusal::Malformed));
    assert_eq!(g.block(&mut s, &run, 0, true, &b[..512], 0), Ok(1));
    assert_eq!(g.block(&mut s, &run, 0, true, &b[..512], 0), Ok(1)); // repeat: no second write
    assert_eq!(g.verify(&mut s, &run, 0), Err(Refusal::VerifyFailed)); // not complete
    // The same package again resumes; the status shows where.
    assert_eq!(g.begin(&o, &run, 0), Ok(3));
    assert_eq!(g.status(), (1, 1, 3));
    assert_eq!(g.block(&mut s, &run, 1, true, &b[512..1024], 0), Ok(2));
    assert_eq!(g.block(&mut s, &run, 2, true, &b[1024..1200], 0), Err(Refusal::Malformed));
    assert_eq!(g.block(&mut s, &run, 2, true, &b[1024..], 0), Ok(3));
    let mut copy = s.clone();
    let mut st = Store::open(&mut copy, Writer::Application { running: Slot::A }).unwrap();
    assert_eq!(g.activate(&mut st, &run, 0), Err(Refusal::VerifyFailed)); // before verify
    drop(st);
    assert_eq!(g.verify(&mut s, &run, 0), Ok(o.sha256));
    assert_eq!(g.status(), (2, 3, 3));
}

#[test]
fn verify_refuses_a_wrong_hash_and_an_image_for_the_wrong_slot() {
    let run = running(Slot::A, V1);
    let push = |img: &[u8], o: Offer| {
        let (mut s, _) = installed(false);
        let mut g = Stager::new();
        let n = g.begin(&o, &run, 0).unwrap();
        for k in 0..n {
            let d = &img[(k * 512) as usize..((k + 1) * 512).min(img.len() as u32) as usize];
            g.block(&mut s, &run, k, true, d, 0).unwrap();
        }
        g.verify(&mut s, &run, 0)
    };
    let b = image(Slot::B, V2, 2000, 3);
    let mut bad = offer(&b, V2);
    bad.sha256[0] ^= 1;
    assert_eq!(push(&b, bad), Err(Refusal::VerifyFailed));
    // An image linked for slot A, with its right hash: refused before activation.
    let wrong = image(Slot::A, V2, 2000, 3);
    assert_eq!(push(&wrong, offer(&wrong, V2)), Err(Refusal::VerifyFailed));
    // The package version must be the image's.
    assert_eq!(push(&b, offer(&b, header::version(1, 0, 3))), Err(Refusal::VerifyFailed));
    assert!(push(&b, offer(&b, V2)).is_ok());
}

#[test]
fn abort_and_abandonment() {
    let (mut s, _) = installed(false);
    let b = image(Slot::B, V2, 1300, 9);
    let run = running(Slot::A, V1);
    let mut g = Stager::new();
    g.begin(&offer(&b, V2), &run, 1000).unwrap();
    g.block(&mut s, &run, 0, true, &b[..512], 2000).unwrap();
    assert!(!g.abandoned(2000 + stage::ABANDON_MS - 1));
    assert!(g.abandoned(2000 + stage::ABANDON_MS));
    g.abort(5000);
    assert_eq!(g.phase(), Phase::Discarded);
    assert!(g.in_progress()); // the update state lasts until a restart
    assert_eq!(g.status(), (0, 0, 0));
    assert_eq!(g.block(&mut s, &run, 1, true, &b[512..1024], 0), Err(Refusal::OutOfOrder));
    // The 10-minute rule also runs after an abort (T-A1-46).
    assert!(g.abandoned(5000 + stage::ABANDON_MS));
    // A discarded transfer is never activated; slot B is not bootable.
    assert!(!valid(&s, Slot::B));
    assert_eq!(boot(&mut s), Choice::Boot(Slot::A));
}

#[test]
fn the_stager_writes_only_the_other_slot() {
    // Every flash operation of a full update lies in slot B or the store: never sector 0, the
    // boot stage or the running slot.
    let (mut s, _) = installed(false);
    let before = s.clone();
    let b = image(Slot::B, V2, 2600, 9);
    update(&mut s, &running(Slot::A, V1), &b, V2).unwrap();
    for (i, (x, y)) in before.mem.iter().zip(s.mem.iter()).enumerate() {
        if x != y {
            let addr = g7250_layout::APPLICATION_AREA.start + i as u32;
            assert!(Slot::B.range().contains_span(addr, 1) || STORE.contains_span(addr, 1), "{addr:#x}");
            assert!(!BOOT_STAGE.contains_span(addr, 1) && !VENDOR_BOOTLOADER.contains_span(addr, 1));
        }
    }
}
