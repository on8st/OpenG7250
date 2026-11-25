// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Update, trial boot, confirmation and rollback with power cut at every flash step.

use crate::boot::{self, UpdateState};
use crate::sim::{Shared, Sim};
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

/// As the boot stage: the image check of the slot (A1.8; 0067).
fn valid(s: &mut Sim, slot: Slot) -> bool {
    crate::image::check(s, ImgKind::Application, slot.range(), RAM, &mut || {}).is_ok()
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
    for (fill, ecc) in [(false, false), (true, false), (false, true), (true, true)] {
        let (mut base, a) = installed(fill);
        base.ecc_on_cut = ecc;
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
    for (fill, ecc) in [(false, false), (true, false), (false, true), (true, true)] {
        let (mut base, a, b) = activated(fill);
        base.ecc_on_cut = ecc;
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
    for (fill, ecc) in [(false, false), (true, false), (false, true), (true, true)] {
        let (mut base, a, _) = activated(fill);
        base.ecc_on_cut = ecc;
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
  for ecc in [false, true] {
    let (mut base, _, _) = activated(true);
    base.ecc_on_cut = ecc;
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
        assert_eq!(boot(&mut s), if lost { Choice::Boot(Slot::B) } else { Choice::Trial(Slot::A) }, "ecc {ecc} cut {cut}/{n}");
    }
  }
}

#[test]
fn a_rollback_target_starts_as_a_trial_and_a_failed_one_returns() {
    // T-A1-84 (0064 point 3.4): the image started by ROLLBACK is a trial; unconfirmed, the next
    // start returns to the image that sent ROLLBACK; confirmed, it stays.
    let (mut base, _, _) = activated(false);
    boot(&mut base);
    assert!(confirm(&mut base, Slot::B));
    assert_eq!(boot(&mut base), Choice::Boot(Slot::B));
    {
        let mut st = Store::open(&mut base, Writer::Application { running: Slot::B }).unwrap();
        boot::rollback(&mut st, Slot::B).unwrap();
    }
    let mut failed = base.clone();
    assert_eq!(boot(&mut failed), Choice::Trial(Slot::A), "the target starts as a trial");
    assert_eq!(boot(&mut failed), Choice::Boot(Slot::B), "not confirmed: back to the image that sent ROLLBACK");
    let mut good = base.clone();
    assert_eq!(boot(&mut good), Choice::Trial(Slot::A));
    assert!(confirm(&mut good, Slot::A));
    assert_eq!(boot(&mut good), Choice::Boot(Slot::A), "confirmed: it stays");
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
    // T-A1-78 (0061 addendum, 0062): slot size + 1 gets 61 and nothing is written; the order is
    // 51, 63, then 61, whatever the length; the slot size is IDENTIFY tag 25's count x 128 KiB.
    let slot = g7250_layout::IDENTIFY_LAYOUT[3] as u32 * g7250_layout::SECTOR_SIZE;
    let big = Offer { length: slot + 1, target_slot: 2, ..o };
    assert_eq!(g.begin(&big, &run, 0), Err(Refusal::TooLarge));
    assert_eq!(g.begin(&Offer { hardware_id: 0x00C9, ..big }, &run, 0), Err(Refusal::WrongHardware));
    assert_eq!(g.begin(&Offer { target_slot: 1, ..big }, &run, 0), Err(Refusal::RunningSlot));
    assert_eq!(g.begin(&Offer { length: u32::MAX, target_slot: 1, ..o }, &run, 0), Err(Refusal::RunningSlot));
    // 56 (foreign) before 63 and 61; a too-large older image gets 61, not 52 (Table A1-3;
    // 0061 addendum item 4).
    assert_eq!(g.begin(&Offer { foreign: true, target_slot: 1, ..big }, &run, 0), Err(Refusal::NotSupported));
    assert_eq!(g.begin(&Offer { version: header::version(1, 0, 0), ..big }, &run, 0), Err(Refusal::TooLarge));
    assert!(!g.in_progress());
    assert_eq!(s.mem, before);
    assert!(g.begin(&Offer { length: slot, target_slot: 2, ..o }, &run, 0).is_ok(), "exactly the slot size");
    let mut g = Stager::new();
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
    assert_eq!(g.abandon_due(), Some(2000 + stage::ABANDON_MS), "the wake time (0073)");
    g.abort(5000);
    assert_eq!(g.phase(), Phase::Discarded);
    assert!(g.in_progress()); // the update state lasts until a restart
    assert_eq!(g.status(), (0, 0, 0));
    assert_eq!(g.block(&mut s, &run, 1, true, &b[512..1024], 0), Err(Refusal::OutOfOrder));
    // The 10-minute rule also runs after an abort (T-A1-46).
    assert!(g.abandoned(5000 + stage::ABANDON_MS));
    assert_eq!(g.abandon_due(), Some(5000 + stage::ABANDON_MS));
    assert_eq!(Stager::new().abandon_due(), None, "no update in progress: no wake");
    // A discarded transfer is never activated; slot B is not bootable.
    assert!(!valid(&mut s, Slot::B));
    assert_eq!(boot(&mut s), Choice::Boot(Slot::A));
}

#[test]
fn after_the_blocks_the_staging_slot_holds_the_image_t_a1_83_84() {
    // The coordinator's T-A1-84 finding (fw-6ca08d1): every block answered 0, then the verify 55.
    // Here the staging slot is read back after each block: the bytes received so far are there
    // (the erase and the programs happened; the last block's partial flash word padded with 0xFF
    // and programmed), and the rest of the slot is erased. Lengths: 2148 (the run: 67 whole flash
    // words and 4 bytes), a multiple of 32 (2144), a multiple of 512 (2048), and 1 byte over.
    for len in [2148usize, 2144, 2048, 2049] {
        staging_slot_holds(len);
    }
}

#[test]
fn a_failed_verify_records_its_check() {
    // For the log (A1.7): missing-blocks, then a wrong hash, then an image that is not runnable.
    let (mut s, _) = installed(false);
    let run = running(Slot::A, V1);
    let b = image(Slot::B, V2, 2148, 9);
    let mut g = Stager::new();
    let n = g.begin(&offer(&b, V2), &run, 0).unwrap();
    assert!(g.verify(&mut s, &run, 1).is_err());
    assert_eq!(g.verify_failure(), Some("missing-blocks"));
    for k in 0..n {
        let d = &b[(k * stage::BLOCK) as usize..((k + 1) * stage::BLOCK).min(b.len() as u32) as usize];
        g.block(&mut s, &run, k, true, d, 1).unwrap();
    }
    let mut g2 = Stager::new();
    let mut bad = offer(&b, V2);
    bad.sha256[0] ^= 1;
    g2.begin(&bad, &run, 0).unwrap();
    for k in 0..n {
        let d = &b[(k * stage::BLOCK) as usize..((k + 1) * stage::BLOCK).min(b.len() as u32) as usize];
        let _ = g2.block(&mut s, &run, k, true, d, 1);
    }
    assert!(g2.verify(&mut s, &run, 2).is_err());
    assert_eq!(g2.verify_failure(), Some("hash"));
    // An image linked for slot A staged in B: the hash holds, the image check fails.
    let wrong = image(Slot::A, V2, 2148, 9);
    let mut g3 = Stager::new();
    g3.begin(&offer(&wrong, V2), &run, 0).unwrap();
    for k in 0..n {
        let d = &wrong[(k * stage::BLOCK) as usize..((k + 1) * stage::BLOCK).min(wrong.len() as u32) as usize];
        g3.block(&mut s, &run, k, true, d, 1).unwrap();
    }
    assert!(g3.verify(&mut s, &run, 2).is_err());
    assert_eq!(g3.verify_failure(), Some("slot"));
    assert_eq!(g3.staged_version(), Some(V2));
}

fn staging_slot_holds(len: usize) {
    let (mut s, _) = installed(false);
    let run = running(Slot::A, V1);
    let b = image(Slot::B, V2, len, 9);
    let mut g = Stager::new();
    let n = g.begin(&offer(&b, V2), &run, 0).unwrap();
    assert_eq!(n as usize, len.div_ceil(512));
    let base = (Slot::B.range().start - g7250_layout::APPLICATION_AREA.start) as usize;
    for k in 0..n {
        let d = &b[(k * stage::BLOCK) as usize..((k + 1) * stage::BLOCK).min(b.len() as u32) as usize];
        assert_eq!(g.block(&mut s, &run, k, true, d, 1), Ok(k + 1));
        let upto = ((k + 1) * stage::BLOCK).min(b.len() as u32) as usize;
        assert_eq!(&s.mem[base..base + upto], &b[..upto], "len {len}: after block {k}");
    }
    assert!(s.mem[base + b.len()..base + Slot::B.range().len() as usize].iter().all(|&x| x == 0xFF), "len {len}: the rest erased");
    assert!(g.verify(&mut s, &run, 2).is_ok(), "len {len}");
    assert_eq!(g.verify_failure(), None);
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

#[test]
fn a_trial_reset_before_confirmation_is_never_tried_again() {
    // T-A1-68: one attempt only. Whatever ends the trial before its confirmation (crash, watchdog,
    // power loss, the 60 s reset), every later start is the previous image.
    for fill in [false, true] {
        let (mut s, a, _) = activated(fill);
        assert_eq!(boot(&mut s), Choice::Trial(Slot::B));
        for _ in 0..3 {
            assert_eq!(boot(&mut s), Choice::Boot(Slot::A));
        }
        assert_eq!(boot::state(record(&mut s), Slot::A), UpdateState::RolledBack);
        // A late confirmation from the old trial image cannot revive it (not the trial any more).
        assert!(!confirm(&mut s, Slot::B));
        assert_eq!(boot(&mut s), Choice::Boot(Slot::A));
        assert_eq!(s.bytes(Slot::A.range().start, a.len() as u32), &a[..]);
    }
}

#[test]
fn begin_component_other_than_main_is_56_in_the_order() {
    // T-A1-81 (0063): a readable header with component 2 gets 56; with the wrong hardware id 51
    // first; 56 before 63, 61 and 52 (A1.8, Table A1-3).
    let (s, _) = installed(false);
    let b = image(Slot::B, V2, 2600, 9);
    let run = running(Slot::A, V1);
    let mut g = Stager::new();
    let before = s.mem.clone();
    let o = Offer { component: 2, ..offer(&b, V2) };
    assert_eq!(g.begin(&o, &run, 0), Err(Refusal::NotSupported));
    assert_eq!(g.begin(&Offer { hardware_id: 0x00C9, ..o }, &run, 0), Err(Refusal::WrongHardware));
    assert_eq!(g.begin(&Offer { target_slot: 1, ..o }, &run, 0), Err(Refusal::NotSupported));
    assert_eq!(g.begin(&Offer { length: Slot::B.range().len() + 1, ..o }, &run, 0), Err(Refusal::NotSupported));
    assert_eq!(g.begin(&Offer { version: header::version(1, 0, 0), ..o }, &run, 0), Err(Refusal::NotSupported));
    assert!(!g.in_progress());
    assert_eq!(s.mem, before);
}

#[test]
fn no_transfer_in_progress_block_54_verify_and_activate_55() {
    // T-A1-80 (0063): after a restart, after UPDATE_ABORT and after an activation, a block gets
    // 54 (also with a bad CRC), verify and activate 55; nothing is written, the status stays.
    let (mut s, _) = installed(false);
    let run = running(Slot::A, V1);
    let check = |g: &mut Stager, s: &mut Sim, idle: (u8, u32, u32)| {
        let before = s.mem.clone();
        assert_eq!(g.block(&mut *s, &run, 0, true, &[1; 512], 5), Err(Refusal::OutOfOrder));
        assert_eq!(g.block(&mut *s, &run, 0, false, &[1; 512], 5), Err(Refusal::OutOfOrder), "54 before 53");
        assert_eq!(g.verify(&mut *s, &run, 5).err(), Some(Refusal::VerifyFailed));
        {
            let mut st = Store::open(&mut *s, Writer::Application { running: run.slot }).unwrap();
            assert_eq!(g.activate(&mut st, &run, 5), Err(Refusal::VerifyFailed));
        }
        assert_eq!(s.mem, before, "nothing written");
        assert_eq!(g.status(), idle);
    };
    // After a restart: a new stager.
    let mut g = Stager::new();
    check(&mut g, &mut s, (0, 0, 0));
    // After UPDATE_ABORT.
    let b = image(Slot::B, V2, 2600, 9);
    let n = g.begin(&offer(&b, V2), &run, 0).unwrap();
    g.block(&mut s, &run, 0, true, &b[..stage::BLOCK as usize], 1).unwrap();
    g.abort(2);
    let st = g.status();
    check(&mut g, &mut s, st);
    assert!(n > 1);
    // After an activation.
    let (mut s, _, _) = activated(false);
    let mut g = Stager::new();
    let b2 = image(Slot::B, V2, 2600, 3);
    let m = g.begin(&offer(&b2, V2), &run, 0).unwrap();
    for k in 0..m {
        let d = &b2[(k * stage::BLOCK) as usize..((k + 1) * stage::BLOCK).min(b2.len() as u32) as usize];
        g.block(&mut s, &run, k, true, d, 1).unwrap();
    }
    g.verify(&mut s, &run, 2).unwrap();
    {
        let mut st = Store::open(&mut s, Writer::Application { running: run.slot }).unwrap();
        g.activate(&mut st, &run, 3).unwrap();
    }
    let st = g.status();
    let before = s.mem.clone();
    assert_eq!(g.block(&mut s, &run, 0, false, &[1; 512], 5), Err(Refusal::OutOfOrder));
    assert_eq!(g.verify(&mut s, &run, 5).err(), Some(Refusal::VerifyFailed));
    {
        let mut stw = Store::open(&mut s, Writer::Application { running: run.slot }).unwrap();
        assert_eq!(g.activate(&mut stw, &run, 5), Err(Refusal::VerifyFailed));
    }
    assert_eq!((s.mem == before, g.status()), (true, st));
}

#[test]
fn refused_commands_and_a_second_abort_never_delay_the_restart() {
    // T-A1-82 (0063 addendum): after UPDATE_ABORT, stray commands every 60 s are refused (block
    // 54, verify and activate 55, begin and ROLLBACK 58, a second abort 0 with no effect), and
    // the restart into the running image still comes 10 min after the reply to the first abort.
    let (mut s, _) = installed(false);
    let run = running(Slot::A, V1);
    let b = image(Slot::B, V2, 2600, 9);
    let mut g = Stager::new();
    g.begin(&offer(&b, V2), &run, 0).unwrap();
    assert!(g.abort(1_000));
    let t0 = 1_000u64;
    for k in 1..10u64 {
        let now = t0 + 60_000 * k;
        assert_eq!(g.block(&mut s, &run, 0, true, &b[..stage::BLOCK as usize], now), Err(Refusal::OutOfOrder));
        assert_eq!(g.verify(&mut s, &run, now).err(), Some(Refusal::VerifyFailed));
        {
            let mut st = Store::open(&mut s, Writer::Application { running: run.slot }).unwrap();
            assert_eq!(g.activate(&mut st, &run, now), Err(Refusal::VerifyFailed));
        }
        assert_eq!(g.begin(&offer(&b, V2), &run, now), Err(Refusal::UpdateInProgress));
        assert_eq!(g.rollback_allowed(&run), Err(Refusal::UpdateInProgress));
        assert!(!g.abort(now), "a second abort has no effect");
        assert_eq!(g.status().0, 0, "UPDATE_STATUS idle");
        assert!(g.in_progress(), "the update state remains until a restart");
    }
    assert!(!g.abandoned(t0 + stage::ABANDON_MS - 1));
    assert!(g.abandoned(t0 + stage::ABANDON_MS), "restart 10 min after the first abort");

    // During a transfer: a block refused with 53 every 60 s, and a verify of an incomplete
    // transfer (55), do not delay the abandonment; a block answered 0 does.
    let mut g = Stager::new();
    g.begin(&offer(&b, V2), &run, 0).unwrap();
    for k in 1..10u64 {
        let now = 60_000 * k;
        assert_eq!(g.block(&mut s, &run, 0, false, &b[..stage::BLOCK as usize], now), Err(Refusal::BadCrc));
        assert_eq!(g.verify(&mut s, &run, now).err(), Some(Refusal::VerifyFailed));
    }
    assert!(g.abandoned(stage::ABANDON_MS));
    g.block(&mut s, &run, 0, true, &b[..stage::BLOCK as usize], 590_000).unwrap();
    assert!(!g.abandoned(stage::ABANDON_MS));
    assert!(g.abandoned(590_000 + stage::ABANDON_MS));
    // With no update state and a confirmed image, ROLLBACK is not refused by the stager; during
    // a trial boot it gets 58 (0064 point 3.1).
    assert_eq!(Stager::new().rollback_allowed(&run), Ok(()));
    assert_eq!(Stager::new().rollback_allowed(&Running { trial: true, ..run }), Err(Refusal::UpdateInProgress));
}

/// `img` transferred whole into slot B with `o`; the stager and the flash after it.
fn transferred(img: &[u8], o: Offer) -> (Sim, Stager) {
    let (mut s, _) = installed(false);
    let run = running(Slot::A, V1);
    let mut g = Stager::new();
    let n = g.begin(&o, &run, 0).unwrap();
    for k in 0..n {
        g.block(&mut s, &run, k, true, block_of(img, k), 1).unwrap();
    }
    (s, g)
}

fn block_of(img: &[u8], k: u32) -> &[u8] {
    &img[(k * stage::BLOCK) as usize..((k + 1) * stage::BLOCK).min(img.len() as u32) as usize]
}

#[test]
fn verify_names_the_first_failed_check_t_a1_88() {
    // 0067: packages whose SHA-256 is right but whose image is not runnable.
    let run = running(Slot::A, V1);
    let o = HEADER_OFFSET as usize;
    let good = image(Slot::B, V2, 3000, 7);
    let sealed = |f: &dyn Fn(&mut Vec<u8>)| {
        let mut v = good.clone();
        f(&mut v);
        header::seal(&mut v).unwrap();
        v
    };
    let mut integrity = good.clone();
    integrity[2000] ^= 1;
    let mut appended = good.clone();
    appended.extend_from_slice(&[0x5A; 32]);
    let mut long = good.clone();
    long[o + header::LENGTH_FIELD..o + header::LENGTH_FIELD + 4].copy_from_slice(&(Slot::B.range().len() + 1).to_le_bytes());
    let cases: Vec<(Vec<u8>, u32, &str)> = vec![
        (sealed(&|v| v[o..o + 4].copy_from_slice(b"NOPE")), V2, "no-record"),
        (sealed(&|v| v[o + 20] ^= 1), V2, "hardware"),
        (sealed(&|v| v[o + 28] = ImgKind::BootStage as u8), V2, "kind"),
        (long, V2, "length"),
        (image(Slot::A, V2, 3000, 7), V2, "slot"),
        (integrity, V2, "integrity"),
        (sealed(&|v| v[4..8].copy_from_slice(&(Slot::A.range().start + 0x501).to_le_bytes())), V2, "vectors"),
        (sealed(&|v| v[0..4].copy_from_slice(&RAM.start.to_le_bytes())), V2, "vectors"),
        (appended, V2, "header"),
        (good.clone(), header::version(1, 0, 3), "header"),
    ];
    for (img, version, word) in cases {
        let (mut s, mut g) = transferred(&img, offer(&img, version));
        let before = s.mem.clone();
        assert_eq!(g.verify(&mut s, &run, 2), Err(Refusal::VerifyFailed), "{word}");
        assert_eq!(g.verify_failure(), Some(word));
        assert_eq!(g.status().0, 1, "{word}: the transfer stays in progress");
        let mut st = Store::open(&mut s, Writer::Application { running: Slot::A }).unwrap();
        assert_eq!(g.activate(&mut st, &run, 3), Err(Refusal::VerifyFailed), "{word}");
        drop(st);
        assert_eq!(s.mem, before, "{word}: nothing written");
        // The boot stage reaches the same result from the slot alone, except for the header
        // check, which needs the package: there the image itself is good.
        let r = image_word(&mut s, Slot::B);
        assert_eq!(r, if word == "header" { None } else { Some(word) });
    }
    // Hash before everything else: a wrong hash of an image that also fails the image check.
    let wrong = image(Slot::A, V2, 3000, 7);
    let mut of = offer(&wrong, V2);
    of.sha256[0] ^= 1;
    let (mut s, mut g) = transferred(&wrong, of);
    assert!(g.verify(&mut s, &run, 2).is_err());
    assert_eq!(g.verify_failure(), Some("hash"));
}

/// The word of the image check for a slot, `None` if it passes.
fn image_word(s: &mut Sim, slot: Slot) -> Option<&'static str> {
    crate::image::check(s, ImgKind::Application, slot.range(), RAM, &mut || {}).err().map(|c| c.word())
}

#[test]
fn the_image_check_is_judged_over_the_recorded_length() {
    // A torn word inside the image: unreadable. The same beyond the recorded length: no matter.
    let img = image(Slot::B, V2, 3000, 7);
    let (mut s, _) = transferred(&img, offer(&img, V2));
    let mut t = s.clone();
    t.tear(Slot::B.range().start + 1024);
    assert_eq!(image_word(&mut t, Slot::B), Some("unreadable"));
    s.tear(Slot::B.range().start + 4096);
    assert_eq!(image_word(&mut s, Slot::B), None);
}

#[test]
fn a_block_after_a_good_verify_cancels_it() {
    let run = running(Slot::A, V1);
    let img = image(Slot::B, V2, 3000, 7);
    let (mut s, mut g) = transferred(&img, offer(&img, V2));
    assert!(g.verify(&mut s, &run, 2).is_ok());
    // A refused block leaves the verify; an accepted one cancels it.
    assert_eq!(g.block(&mut s, &run, 1, false, block_of(&img, 1), 3), Err(Refusal::BadCrc));
    assert_eq!(g.status().0, 2);
    assert_eq!(g.block(&mut s, &run, 1, true, block_of(&img, 1), 3), Ok(6));
    assert_eq!(g.status(), (1, 6, 6));
    {
        let mut st = Store::open(&mut s, Writer::Application { running: Slot::A }).unwrap();
        assert_eq!(g.activate(&mut st, &run, 4), Err(Refusal::VerifyFailed));
    }
    assert!(g.verify(&mut s, &run, 5).is_ok());
    let mut st = Store::open(&mut s, Writer::Application { running: Slot::A }).unwrap();
    assert_eq!(g.activate(&mut st, &run, 6), Ok(()));
}

#[test]
fn blocks_sent_again_replace_what_was_received() {
    // 0067 "Blocks sent again". Two sectors: 260 blocks, 256 of them in the first.
    let run = running(Slot::A, V1);
    let good = image(Slot::B, V2, 260 * 512, 7);
    let mut bad = good.clone();
    bad[600] ^= 1; // block 1
    bad[258 * 512] ^= 1; // block 258, second sector
    let (mut s, mut g) = transferred(&bad, offer(&good, V2));
    assert!(g.verify(&mut s, &run, 2).is_err());
    assert_eq!(g.verify_failure(), Some("hash"));
    // The same bytes again: accepted, nothing to write, no block lost.
    let steps = s.steps;
    assert_eq!(g.block(&mut s, &run, 5, true, block_of(&good, 5), 3), Ok(260));
    assert_eq!(s.steps, steps);
    // Block 258 with other bytes: its sector is erased (one erase, 16 words), the other three
    // blocks of it count as not received; the first sector keeps its 256.
    assert_eq!(g.block(&mut s, &run, 258, true, block_of(&good, 258), 4), Ok(257));
    assert_eq!(s.steps, steps + 1 + 16);
    assert_eq!(g.status(), (1, 257, 260));
    assert!(g.verify(&mut s, &run, 5).is_err());
    assert_eq!(g.verify_failure(), Some("missing-blocks"));
    // Every block again, in order: the last data of each block is what is verified.
    for k in 0..260 {
        g.block(&mut s, &run, k, true, block_of(&good, k), 6).unwrap();
    }
    assert_eq!(g.status(), (1, 260, 260));
    assert!(g.verify(&mut s, &run, 7).is_ok());
    assert_eq!(s.bytes(Slot::B.range().start, good.len() as u32), &good[..]);
    // A block the transfer has not reached stays out of order, also with holes before it.
    let mut g = Stager::new();
    g.begin(&offer(&good, V2), &run, 0).unwrap();
    for k in 0..3 {
        g.block(&mut s, &run, k, true, block_of(&good, k), 1).unwrap();
    }
    assert_eq!(g.block(&mut s, &run, 1, true, &[0x11; 512], 1), Ok(1));
    assert_eq!(g.block(&mut s, &run, 4, true, block_of(&good, 4), 1), Err(Refusal::OutOfOrder));
    assert_eq!(g.block(&mut s, &run, 3, true, block_of(&good, 3), 1), Ok(2));
    assert_eq!(g.block(&mut s, &run, 0, true, block_of(&good, 0), 1), Ok(3));
}

/// Tag 9 of a slot as the application judges it.
fn tag9(s: &mut Sim, slot: Slot) -> u8 {
    let open = {
        let mut st = Store::open(&mut *s, Writer::BootStage).unwrap();
        boot::staging_open(&mut st).unwrap() == Some(slot)
    };
    let c = crate::image::check(s, ImgKind::Application, slot.range(), RAM, &mut || {});
    boot::integrity(open, &c)
}

/// A whole transfer and verify with the staging mark, over one flash, as the board does it.
fn marked_update(s: &core::cell::RefCell<Sim>, g: &mut Stager, img: &[u8], o: Offer) -> Result<(), Refusal> {
    let run = running(Slot::A, V1);
    let n = g.begin(&o, &run, 0)?;
    let mut io = Shared(s);
    let mut st = Store::open(Shared(s), Writer::Application { running: Slot::A }).map_err(|_| Refusal::VerifyFailed)?;
    st.reserve(4096).map_err(|_| Refusal::VerifyFailed)?;
    for k in 0..n {
        g.block_marked(Some(&mut st), &mut io, &run, k, true, block_of(img, k), 1)?;
    }
    g.verify_marked(Some(&mut st), &mut io, &run, 2)?;
    Ok(())
}

#[test]
fn tag_9_through_an_update_t_a1_89() {
    let run = running(Slot::A, V1);
    let img = image(Slot::B, V2, 2000, 7);
    let (s, _) = installed(false);
    let s = core::cell::RefCell::new(s);
    assert_eq!(tag9(&mut s.borrow_mut(), Slot::B), 0, "erased");
    let mut g = Stager::new();
    marked_update(&s, &mut g, &img, offer(&img, V2)).unwrap();
    assert_eq!(tag9(&mut s.borrow_mut(), Slot::B), 1, "good verify");
    // A new transfer: BEGIN alone changes nothing; the first block (the first erase) gives 0,
    // also after a restart (a new stager), although the slot's old image was good.
    let img2 = image(Slot::B, header::version(1, 0, 3), 2000, 9);
    let mut g = Stager::new();
    g.begin(&offer(&img2, header::version(1, 0, 3)), &run, 0).unwrap();
    assert_eq!(tag9(&mut s.borrow_mut(), Slot::B), 1);
    let mut io = Shared(&s);
    let mut st = Store::open(Shared(&s), Writer::Application { running: Slot::A }).unwrap();
    // A refused block sets no mark.
    assert_eq!(g.block_marked(Some(&mut st), &mut io, &run, 0, false, block_of(&img2, 0), 1), Err(Refusal::BadCrc));
    assert_eq!(tag9(&mut s.borrow_mut(), Slot::B), 1);
    g.block_marked(Some(&mut st), &mut io, &run, 0, true, block_of(&img2, 0), 1).unwrap();
    assert_eq!(tag9(&mut s.borrow_mut(), Slot::B), 0);
    g.abort(2);
    assert_eq!(tag9(&mut s.borrow_mut(), Slot::B), 0, "after UPDATE_ABORT");
    // Every block but no verify, then a failed verify: 0, whatever the bytes.
    let mut g = Stager::new();
    let mut o = offer(&img, V2);
    o.sha256[0] ^= 1;
    assert!(marked_update(&s, &mut g, &img, o).is_err());
    assert_eq!(tag9(&mut s.borrow_mut(), Slot::B), 0, "failed verify of a good image");
    // A good verify later: 1; a block that cancels it: 0; verified again: 1.
    let mut g = Stager::new();
    marked_update(&s, &mut g, &img, offer(&img, V2)).unwrap();
    assert_eq!(tag9(&mut s.borrow_mut(), Slot::B), 1);
    let mut st = Store::open(Shared(&s), Writer::Application { running: Slot::A }).unwrap();
    g.block_marked(Some(&mut st), &mut io, &run, 1, true, block_of(&img, 1), 3).unwrap();
    assert_eq!(tag9(&mut s.borrow_mut(), Slot::B), 0);
    g.verify_marked(Some(&mut st), &mut io, &run, 4).unwrap();
    assert_eq!(tag9(&mut s.borrow_mut(), Slot::B), 1);
    // The boot record was never written by all this.
    assert_eq!(record(&mut s.borrow_mut()), None);
    // One byte changed later: 2.
    let i = (Slot::B.range().start - g7250_layout::APPLICATION_AREA.start) as usize + 1800;
    s.borrow_mut().mem[i] ^= 1;
    assert_eq!(tag9(&mut s.borrow_mut(), Slot::B), 2);
    // No store: the block is a write failure, nothing is erased.
    let mut g = Stager::new();
    g.begin(&offer(&img, V2), &run, 0).unwrap();
    let before = s.borrow().mem.clone();
    let none: Option<&mut Store<Shared>> = None;
    assert_eq!(g.block_marked(none, &mut io, &run, 0, true, block_of(&img, 0), 1), Err(Refusal::VerifyFailed));
    assert_eq!(s.borrow().mem, before);
}

#[test]
fn a_slot_whose_trial_failed_stays_good_and_is_a_rollback_target_t_a1_89() {
    // 0067 point d: tag 9 reports integrity; the failed trial shows in tag 10.
    let (s, a) = installed(false);
    let img = image(Slot::B, V2, 2000, 7);
    let s = core::cell::RefCell::new(s);
    let mut g = Stager::new();
    marked_update(&s, &mut g, &img, offer(&img, V2)).unwrap();
    let mut s = s.into_inner();
    {
        let mut st = Store::open(&mut s, Writer::Application { running: Slot::A }).unwrap();
        g.activate(&mut st, &running(Slot::A, V1), 3).unwrap();
    }
    assert_eq!(boot(&mut s), Choice::Trial(Slot::B));
    assert_eq!(boot(&mut s), Choice::Boot(Slot::A), "not confirmed: back");
    assert_eq!(boot::state(record(&mut s), Slot::A), UpdateState::RolledBack);
    assert_eq!(tag9(&mut s, Slot::B), 1);
    {
        let mut st = Store::open(&mut s, Writer::Application { running: Slot::A }).unwrap();
        boot::rollback(&mut st, Slot::A).unwrap();
    }
    assert_eq!(boot(&mut s), Choice::Trial(Slot::B), "a trial again");
    assert_eq!(s.bytes(Slot::A.range().start, a.len() as u32), &a[..]);
}

#[test]
fn boot_refusals_t_a1_90() {
    // The trial slot changed after the verify: the previous image, rolled back.
    let (mut s, _, _) = activated(false);
    let at = |slot: Slot| (slot.range().start - g7250_layout::APPLICATION_AREA.start) as usize + 2000;
    s.mem[at(Slot::B)] ^= 1;
    assert_eq!(image_word(&mut s, Slot::B), Some("integrity"));
    assert_eq!(boot(&mut s), Choice::Boot(Slot::A));
    assert_eq!(boot::state(record(&mut s), Slot::A), UpdateState::RolledBack);
    // The confirmed active image changed, the other slot good: the other one, no trial, recorded.
    let (mut s, _, _) = activated(false);
    boot(&mut s);
    assert!(confirm(&mut s, Slot::B));
    s.mem[at(Slot::B)] ^= 1;
    assert_eq!(boot(&mut s), Choice::Boot(Slot::A));
    assert_eq!(boot::state(record(&mut s), Slot::A), UpdateState::RolledBack);
    assert_eq!(record(&mut s).map(|r| (r.active, r.trial, r.confirmed)), Some((Slot::A, None, true)));
    assert_eq!(boot(&mut s), Choice::Boot(Slot::A), "an ordinary start from then on");
    // Both changed: no image.
    s.mem[at(Slot::A)] ^= 1;
    assert_eq!(boot(&mut s), Choice::NoImage);
}

#[test]
fn power_loss_at_every_step_of_a_marked_transfer() {
    // The slot held a good image (tag 9 = 1). Whatever the cut, the running image starts, and a
    // slot whose bytes changed never counts as good unless the verify ended.
    let old = image(Slot::B, V2, 2000, 7);
    let new = image(Slot::B, header::version(1, 0, 3), 1500, 9);
    for ecc in [false, true] {
        let (s, _) = installed(true); // the store nearly full: BEGIN makes room
        let s = core::cell::RefCell::new(s);
        marked_update(&s, &mut Stager::new(), &old, offer(&old, V2)).unwrap();
        let mut base = s.into_inner();
        base.ecc_on_cut = ecc;
        base.steps = 0;
        let go = |s: Sim| {
            let s = core::cell::RefCell::new(s);
            let r = marked_update(&s, &mut Stager::new(), &new, offer(&new, header::version(1, 0, 3)));
            (s.into_inner(), r)
        };
        let n = go(base.clone()).0.steps;
        assert!(n > 40);
        for cut in 0..=n {
            let mut c = base.clone();
            c.budget = Some(cut);
            let (mut c, r) = go(c);
            c.power_on();
            assert_eq!(boot(&mut c), Choice::Boot(Slot::A), "ecc {ecc} cut {cut}");
            let t = tag9(&mut c, Slot::B);
            let is = |img: &[u8]| c.bytes(Slot::B.range().start, img.len() as u32) == img;
            if r.is_ok() {
                assert_eq!(t, 1);
            } else if t == 1 {
                assert!(is(&old) || is(&new), "ecc {ecc} cut {cut}: good but neither image");
            } else {
                assert_eq!(t, 0, "ecc {ecc} cut {cut}");
            }
        }
    }
}

#[test]
fn one_erase_per_update_request_0069() {
    // With the store nearly full: BEGIN's reserve compacts (one erase); then the first block of a
    // sector does its one erase, the others and the verify none.
    let (s, _) = installed(true);
    let img = image(Slot::B, V2, 260 * 512, 7);
    let run = running(Slot::A, V1);
    let s = core::cell::RefCell::new(s);
    let mut g = Stager::new();
    let mut io = Shared(&s);
    let mut st = Store::open(Shared(&s), Writer::Application { running: Slot::A }).unwrap();
    let erases = |s: &core::cell::RefCell<Sim>| s.borrow().erases;
    let e0 = erases(&s);
    g.begin(&offer(&img, V2), &run, 0).unwrap();
    st.reserve(4096).unwrap();
    assert_eq!(erases(&s), e0 + 1, "BEGIN");
    for k in 0..260 {
        let e = erases(&s);
        g.block_marked(Some(&mut st), &mut io, &run, k, true, block_of(&img, k), 1).unwrap();
        assert_eq!(erases(&s) - e, (k % 256 == 0) as usize, "block {k}");
    }
    let e = erases(&s);
    g.verify_marked(Some(&mut st), &mut io, &run, 2).unwrap();
    assert_eq!(erases(&s), e, "VERIFY");
}

#[test]
fn store_full_during_a_transfer_64_and_a_verify_that_makes_room_t_a1_88() {
    let (s, _) = installed(true); // the store full but for less than 64 bytes
    let img = image(Slot::B, V2, 2000, 7);
    let run = running(Slot::A, V1);
    let s = core::cell::RefCell::new(s);
    let mut io = Shared(&s);
    let mut st = Store::open(Shared(&s), Writer::Application { running: Slot::A }).unwrap();
    let fill = |st: &mut Store<Shared>, left: u32| {
        while st.free() >= left {
            st.put(Kind::Config, 1, &[7u8; 8]).unwrap();
        }
    };
    fill(&mut st, 64);
    let mut g = Stager::new();
    let n = g.begin(&offer(&img, V2), &run, 0).unwrap(); // no reserve here
    let e = s.borrow().erases;
    assert_eq!(g.block_marked(Some(&mut st), &mut io, &run, 0, true, block_of(&img, 0), 400_000), Err(Refusal::RoomMade));
    assert_eq!(Refusal::RoomMade as u16, 64);
    assert!(g.take_room_made() && !g.take_room_made());
    assert_eq!((s.borrow().erases - e, g.status()), (1, (1, 0, 4)), "one erase, nothing taken, the transfer goes on");
    // A 64 restarts the timer.
    assert!(!g.abandoned(400_000 + stage::ABANDON_MS - 1) && g.abandoned(400_000 + stage::ABANDON_MS));
    for k in 0..n {
        g.block_marked(Some(&mut st), &mut io, &run, k, true, block_of(&img, k), 400_001).unwrap();
    }
    // The store filled up meanwhile: a block gets 64 and loses nothing; a verify makes the room
    // itself (its one erase) and completes.
    fill(&mut st, 32);
    let e = s.borrow().erases;
    assert_eq!(g.block_marked(Some(&mut st), &mut io, &run, 1, true, block_of(&img, 1), 400_002), Err(Refusal::RoomMade));
    assert_eq!((s.borrow().erases - e, g.status()), (1, (1, 4, 4)));
    fill(&mut st, 32);
    assert!(g.take_room_made());
    let e = s.borrow().erases;
    assert!(g.verify_marked(Some(&mut st), &mut io, &run, 400_003).is_ok());
    assert_eq!((s.borrow().erases - e, g.take_room_made(), g.verify_failure()), (1, true, None));
    drop(st);
    assert_eq!(tag9(&mut s.borrow_mut(), Slot::B), 1);
}

#[test]
fn a_verify_whose_mark_cannot_be_written_discards_the_transfer_0043() {
    let (s, _) = installed(false);
    let img = image(Slot::B, V2, 2000, 7);
    let run = running(Slot::A, V1);
    let s = core::cell::RefCell::new(s);
    let mut io = Shared(&s);
    let mut st = Store::open(Shared(&s), Writer::Application { running: Slot::A }).unwrap();
    let mut g = Stager::new();
    let n = g.begin(&offer(&img, V2), &run, 0).unwrap();
    for k in 0..n {
        g.block_marked(Some(&mut st), &mut io, &run, k, true, block_of(&img, k), 1).unwrap();
    }
    s.borrow_mut().budget = Some(0);
    assert_eq!(g.verify_marked(Some(&mut st), &mut io, &run, 2), Err(Refusal::VerifyFailed));
    // A write failure: discarded, a new UPDATE_BEGIN accepted (A1.8 "Write failure").
    assert_eq!((g.phase(), g.verify_failure()), (Phase::Failed, None));
    drop(st);
    s.borrow_mut().power_on();
    assert_eq!(tag9(&mut s.borrow_mut(), Slot::B), 0);
}

#[test]
fn after_a_write_failure_a_new_begin_is_accepted_t_a1_85() {
    // A1.8 "Write failure" (0043): the transfer is discarded (UPDATE_STATUS idle), the update
    // state remains until a restart and its 10-minute rule runs from the last command answered
    // 0 (the 55 does not restart it); unlike after UPDATE_ABORT, a new UPDATE_BEGIN is accepted.
    let run = running(Slot::A, V1);
    let img = image(Slot::B, V2, 2000, 7);
    let (s, _) = installed(false);
    let s = core::cell::RefCell::new(s);
    let mut io = Shared(&s);
    let mut g = Stager::new();
    g.begin(&offer(&img, V2), &run, 1_000).unwrap();
    let none: Option<&mut Store<Shared>> = None;
    assert_eq!(g.block_marked(none, &mut io, &run, 0, true, block_of(&img, 0), 2_000), Err(Refusal::VerifyFailed));
    assert_eq!(g.status().0, 0, "no transfer in progress");
    assert!(g.in_progress(), "the update state remains");
    assert!(!g.abandoned(1_000 + stage::ABANDON_MS - 1) && g.abandoned(1_000 + stage::ABANDON_MS), "10 min from the BEGIN answered 0");
    assert_eq!(g.begin(&offer(&img, V2), &run, 3_000), Ok(offer(&img, V2).length.div_ceil(stage::BLOCK)), "a new BEGIN");
    // After UPDATE_ABORT, by contrast: 58.
    g.abort(4_000);
    assert_eq!(g.begin(&offer(&img, V2), &run, 5_000), Err(Refusal::UpdateInProgress));
    // A write failure after an abort's discard cannot happen (no transfer), and one after the
    // new BEGIN is again followed by an accepted BEGIN.
    let mut g = Stager::new();
    g.begin(&offer(&img, V2), &run, 0).unwrap();
    let none: Option<&mut Store<Shared>> = None;
    assert!(g.block_marked(none, &mut io, &run, 0, true, block_of(&img, 0), 1).is_err());
    assert!(g.begin(&offer(&img, V2), &run, 2).is_ok());
}
