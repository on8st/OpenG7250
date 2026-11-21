# 0046 — CW keyer timing, weight, strict spacing and iambic modes

- Status: proposed (to the repository owner, ON8ST); answers a spec gap found by the firmware
  (B10.5a, B10.5b, Table B10-1), 2026-11-21
- Date: 2026-11-21
- Builds on: 0007 Decision 6, 0022 §25, §36, §82; [P2] DUC specific packet bytes 5, 9, 10;
  [ITU-M1677].

## Context

[P2] gives the keyer speed ("0 to 60 WPM"), the weight ("33 to 66, nominal is 50") and the mode
bits (iambic, mode B, strict character spacing), but not what they do. The firmware chose
dit = 1200 / wpm ms and its own reading of the weight.

## Decision (proposed)

1. **Unit** *T* = 1200 / *S* ms, *S* the keyer speed in words per minute counted on the 50-unit
   word PARIS; *S* honoured 1–60, 0 does not key, above 60 used as 60 (logged, counted).
2. **Elements** as ITU-R M.1677-1: dot 1 *T*, dash 3 *T*, element space 1 *T*.
3. **Weight** changes the mark/space ratio, not the dot/dash ratio: each mark +Δ and the space
   after it −Δ, Δ = *T* (*W* − 50) / 50; 50 neutral; clamped to 33–66; 0 read as 50. A dot plus
   its space stays 2 *T*, so the speed does not change with the weight.
4. **Iambic mode A and B** as the Curtis definitions: mode A stops after the current element when
   both paddles are released; mode B, after a squeeze during an element, adds one opposite
   element. One element of memory per paddle in both modes.
5. **Strict character spacing**: once a character has ended (no paddle at the end of the 1 *T*
   space), the next element waits until 3 *T* of space since the last mark.
6. **Tolerance** for tests: each mark and space ±1 ms or ±2 %, whichever is larger; a character
   ±2 ms or ±2 %; on the radio between the 50 % points of the RF envelope, on the reference model
   and the emulator between the key-on and key-off events.

## Amendment after the testbench findings SF-19 to SF-23 (2026-11-21)

- SF-19: the memory belongs to mode B only. Mode A decides from the paddles held at the
  decision point (end of the space after an element); a squeeze released before the end of a
  dash ends after the dash. "Memory in both modes" is withdrawn.
- SF-20: the memory window of mode B is the mark only, not the space after it.
- SF-21: strict spacing: at 3 *T* the next element starts only if a paddle is held then.
- SF-22: latency from the High Priority packet with the paddle edge to key-on while the keyer is
  idle: at most 5 ms (after the RF-path wait of A3.9a item 3 if one is needed).
- SF-23: log text `CW speed <S> used as 60` and counter `cw_speed_clamped`; likewise
  `CW hang delay <ms> clamped to 2000` and `cw_hang_clamped` for the hang delay.
- T-B10-43, T-B10-45 and T-B10-46 rewritten accordingly.

## Consequences

- Spec B10.5a "Keyer timing", Table B10-1 bytes 9–10; tests T-B10-43 to T-B10-46. The existing
  T-B10-33 (iambic at 20 wpm) is checked against these rules.

## Sources

- [P2] DUC specific packet; [ITU-M1677]; firmware question via openg7250, 2026-11-21; decisions
  0007, 0022.
