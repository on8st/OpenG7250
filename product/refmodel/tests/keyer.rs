//! The CWX keyer against B10.5a (0046) and the cases of T-B10-43..46.
//! GPL-3.0-or-later, (c) 2026 ON8ST.
use g7250_refmodel::keyer::{Keyer, Off, Out, Settings};

const MS: u64 = 1000;

fn set(speed: u8, weight: u8) -> Settings {
    Settings { iambic: true, mode_b: false, strict: false, speed, weight }
}

/// Feed (t_ms, dot, dash) changes, run to `end_ms`; return the marks as (on, off) in us.
fn run(k: &mut Keyer, inputs: &[(u64, bool, bool)], end_ms: u64) -> (Vec<(u64, u64)>, Vec<Out>) {
    let mut out = Vec::new();
    for &(t, d, a) in inputs {
        k.input(t * MS, d, a, &mut out);
    }
    k.advance(end_ms * MS, &mut out);
    let mut marks = Vec::new();
    let mut on = None;
    for o in &out {
        match *o {
            Out::On(t) => on = Some(t),
            Out::Off(t, _) => marks.push((on.take().unwrap(), t)),
            Out::Released(_) => {}
        }
    }
    (marks, out)
}

fn lens(marks: &[(u64, u64)]) -> (Vec<u64>, Vec<u64>) {
    let m = marks.iter().map(|(a, b)| b - a).collect();
    let s = marks.windows(2).map(|w| w[1].0 - w[0].1).collect();
    (m, s)
}

#[test]
fn unit_and_dots_t_b10_43() {
    let mut k = Keyer::new(set(20, 50));
    let (marks, _) = run(&mut k, &[(0, true, false), (690, false, false)], 2000);
    let (m, s) = lens(&marks);
    assert_eq!(marks.len(), 6);
    assert!(m.iter().all(|&x| x == 60 * MS) && s.iter().all(|&x| x == 60 * MS), "{m:?} {s:?}");
    let mut k = Keyer::new(set(20, 50));
    let (marks, _) = run(&mut k, &[(0, false, true), (850, false, false)], 2000);
    let (m, s) = lens(&marks);
    assert_eq!(marks.len(), 4);
    assert!(m.iter().all(|&x| x == 180 * MS) && s.iter().all(|&x| x == 60 * MS));
    for speed in [60, 70] {
        let mut k = Keyer::new(set(speed, 50));
        let (marks, _) = run(&mut k, &[(0, true, false), (190, false, false)], 1000);
        assert!(lens(&marks).0.iter().all(|&x| x == 20 * MS), "speed {speed}");
        assert_eq!(marks.len(), 5);
    }
}

#[test]
fn weight_t_b10_44() {
    for (w, mark, space) in [(66, 79_200, 40_800), (33, 39_600, 80_400), (80, 79_200, 40_800), (0, 60_000, 60_000)] {
        let mut k = Keyer::new(set(20, w));
        let (marks, _) = run(&mut k, &[(0, true, false), (500, false, false)], 2000);
        let (m, s) = lens(&marks);
        assert!(m.iter().all(|&x| x == mark) && s.iter().all(|&x| x == space), "W {w}: {m:?} {s:?}");
    }
}

#[test]
fn strict_spacing_t_b10_45() {
    // one dot (0-60), released in its mark; pressed again 1.5 T after its end (150) and held
    for (strict, start) in [(true, 240), (false, 150)] {
        let mut k = Keyer::new(Settings { strict, ..set(20, 50) });
        let (marks, _) = run(&mut k, &[(0, true, false), (30, false, false), (150, true, false), (280, false, false)], 1000);
        assert_eq!(marks[1].0, start * MS, "strict {strict}: {marks:?}");
    }
    // strict: pressed at 1.5 T, released at 2.5 T (inside the wait): nothing starts
    for mode_b in [false, true] {
        let mut k = Keyer::new(Settings { strict: true, mode_b, ..set(20, 50) });
        let (marks, _) = run(&mut k, &[(0, true, false), (30, false, false), (150, true, false), (210, false, false)], 1000);
        assert_eq!(marks.len(), 1, "mode B {mode_b}: {marks:?}");
    }
}

#[test]
fn iambic_a_b_t_b10_46() {
    // (a) squeeze during a dash, both released before its end (180)
    for (mode_b, n) in [(false, 1), (true, 2)] {
        let mut k = Keyer::new(Settings { mode_b, ..set(20, 50) });
        let (marks, _) = run(&mut k, &[(0, false, true), (30, true, true), (120, false, false)], 1000);
        assert_eq!(marks.len(), n, "(a) mode B {mode_b}: {marks:?}");
        if mode_b {
            assert_eq!(marks[1], (240 * MS, 300 * MS));
        }
    }
    // (b) dot pressed and released during the dash mark, dash held to 700 ms:
    // mode A dash dash dash (0, 240, 480); mode B dash dot dash dash (0, 240, 360, 600)
    let mut k = Keyer::new(set(20, 50));
    let (marks, _) = run(&mut k, &[(0, false, true), (60, true, true), (100, false, true), (700, false, false)], 2000);
    assert_eq!(marks, vec![(0, 180 * MS), (240 * MS, 420 * MS), (480 * MS, 660 * MS)], "(b) mode A");
    let mut k = Keyer::new(Settings { mode_b: true, ..set(20, 50) });
    let (marks, _) = run(&mut k, &[(0, false, true), (60, true, true), (100, false, true), (700, false, false)], 2000);
    assert_eq!(marks, vec![(0, 180 * MS), (240 * MS, 300 * MS), (360 * MS, 540 * MS), (600 * MS, 780 * MS)], "(b) mode B");
    // (c) dot pressed and released only in the space after the dash (190-220), dash released
    // in the mark (100): no dot in either mode
    for mode_b in [false, true] {
        let mut k = Keyer::new(Settings { mode_b, ..set(20, 50) });
        let (marks, _) = run(&mut k, &[(0, false, true), (100, false, false), (190, true, false), (220, false, false)], 1000);
        assert_eq!(marks, vec![(0, 180 * MS)], "(c) mode B {mode_b}");
    }
}

#[test]
fn straight_key_and_cap_t_b10_33() {
    let mut k = Keyer::new(Settings { iambic: false, ..set(20, 50) });
    let (marks, _) = run(&mut k, &[(0, true, false), (300, false, false)], 1000);
    assert_eq!(marks, vec![(0, 300 * MS)]);
    let mut k = Keyer::new(Settings { iambic: false, ..set(20, 50) });
    let (marks, out) = run(&mut k, &[(0, true, false)], 1500);
    assert_eq!(marks, vec![(0, 1000 * MS)]);
    assert!(out.contains(&Out::Off(1000 * MS, Off::Cap)));
    // stopped: a new edge needs both bits 0 first
    let mut more = Vec::new();
    k.input(1600 * MS, true, false, &mut more);
    assert!(more.is_empty());
    k.input(1700 * MS, false, false, &mut more);
    k.input(1800 * MS, true, false, &mut more);
    assert_eq!(more.first(), Some(&Out::On(1800 * MS)));
}
