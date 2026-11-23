# 0072 — C-3 limits that may be lost

- Status: accepted (ON8ST, 2026-11-23, relayed by the openg7250 session: reading (b), points
  A to I, option (i) with "or leave it lost"); accepted in principle and held on 2026-11-22
  until the point with Table M-3 was decided; proposed 2026-11-22 and revised in five rounds
  with the openg7250 session. The text is now applied to the spec.
- Date: 2026-11-23
- Builds on: 0022 §31 (C-3 only stricter; looser back toward the built-in value), 0036 (TX
  armable once calibrated), 0039 (calibration set, status 62), 0065 (store writes, power loss,
  ECC errors), 0069 (one erase per request, the start check and its addendum), 0074 (tags 9
  and 10 absent without a set).

## Context

0065 lets an item with no readable record take its Table A1-4 factory value. Table A1-4 keeps
C-3 over a factory reset and gives it no factory value, and the only values the firmware could
fall back to are the built-in limits, the loosest allowed. A tightening the owner made (a lower
power limit for a band the licence restricts, a shorter TX time) could therefore come back
looser after a torn write or a flash read error, with nothing in the log or in `GET_CONFIG` to
show it. The rule of this decision: a lost tightening must never come back as a looser limit.

Closing TX was chosen over "the strictest value" because the spec defines no strictest value
per C-3 item, while 0 W per band already closes a band (A1.6 C-6).

## Decision

1. **A C-3 write always compacts** (A1.6 C-3). A `SET_CONFIG` that holds any C-3 item is
   written by a compaction into a new store generation, never appended, whether or not the
   values change. Every committed generation then carries the C-3 items in force and no older
   C-3 record, and an appended write that is torn can never be a C-3 record. The other items of
   the request go into the same generation, all or none, except after a fall-back from that
   generation: the other items are then old and each C-3 item the stricter of old and new. A
   C-3 write answered 55 because its commit word could not be programmed may act in the same
   way from the next start (point H).
2. **Generation numbers** (points F and I). Each generation carries its number in two places,
   at least one outside the commit word and programmed before it; a number is readable only
   where it passes its check. The first generation has 0; a new generation takes a number
   higher than every number readable, C-3 state records included. In judging C-3, a live
   generation that is not committed has the number 0 and no C-3 record of its own.
3. **C-3 at a start** (A1.6 "Store writes"). A failed generation (commit word not erased and
   not valid) counts only if its number can be read and is higher than the live generation's
   and than the number in every readable C-3 state record of the live sector. Foreign data and
   a half-erased older generation do not count. Cases in order: a failed generation that counts
   gives the stricter of its values and the live values, or lost if its C-3 record is
   unreadable; else the newest C-3 state record; else the C-3 record of the committed live
   generation, or lost if unreadable; else the built-in values, the factory state. "Lost" is
   stricter than any value; an S2 or T3 never written is "not tightened", and the stricter of
   "not tightened" and a value is the value.
4. **Keeping the judgment.** Before a sector is erased that holds a generation without a valid
   commit and with a readable number above the live one's and the state records' (a failed
   generation that counts, or what a cut compaction left), the store writes a C-3 state record
   into the live sector with C-3 as judged or set since and that number, and reads it back.
   The record is not a C-3 change and not on the closed list of A1.10; it stays until a
   committed generation carries the state. Ordinary erases need no record.
5. **Lost.** TX is closed on every band (persistent power limit 0 W), the other C-3 items take
   their built-in values, `config: C-3 lost, TX closed until rewritten` is logged once per
   start and `GET_CONFIG` reports 0 W. Only an authenticated `SET_CONFIG` of every C-3 item
   ends it (reading (b): before the calibration set holds S2 and T3 it cannot end, and TX is
   not calibrated then anyway). A partial C-3 write while lost gets 48 and changes nothing
   (point A); with no set, a request of every C-3 item gets 62 and any other with a C-3 item
   48 (point G, Table A1-3 order). Every compaction carries the lost state or the judged
   values, the one of a factory reset included. A power loss during a write that holds no C-3
   item never changes C-3.
6. **Vendor re-install** (option (i)). A re-install through the vendor path may return C-3 to
   the built-in values or leave it lost; the owner then sets the limits again (Table A1-4,
   A1.8a item 9, P-SYS-5).
7. **Tests.** T-A1-86 (C-3 change, mixed request, cut erases), T-A1-87 (lost state, its end,
   the no-set case of point B, generation numbers, point I), T-A1-92 (state record and power
   loss at each step, newer and older failed generations, foreign data; point C), T-A1-39
   (lost over a factory reset), T-A3-47 (point D).

## Accepted points from the reviews

- A: a partial C-3 write while lost is refused with 48 (A1), not stored (A2, which needed an
  exception to 0065).
- B: the T-A1-87 case uses a complete simulated set; A1.12 item 3 opens TX only with every
  parameter of the freeze rule.
- C: T-A1-92 and the 0069 addendum's accepted consequence apply to a `SET_CONFIG` with no C-3
  item; sector 7 may hold a C-3 state record besides boot counters.
- D: T-A3-47 allows the session to end by its watchdog over the flash work of a C-3 change.
- E: the transition, in Consequences.
- F: a sector erase cut by a power loss, and foreign data, never count as a failed generation;
  the number in two places; the state record also before the erase of what a cut compaction
  left.
- G: 48 before 62.
- H: a C-3 write answered 55 at its commit word may act as the stricter value.
- I: a live generation that is not committed has the number 0 for C-3, so a cut compaction
  cannot hide a committed generation whose commit word decayed.

## Alternatives considered

- The built-in values for a C-3 item with no readable record (0065 as it stood): a lost
  tightening comes back looser.
- "The strictest value" per item: the spec defines none.
- (a) for the exit from the lost state: only the items writable at the time. S2 and T3 would
  come back at their built-in values after a later calibration, looser than the owner's
  tightening, which this decision exists to prevent.
- A release record before every erase of the other sector: covers one more case (a generation
  in a blank sector losing both its commit word and its number, two faults), at the cost of a
  record before every ordinary erase.
- (ii) for the vendor re-install: keep a C-3 copy outside the store; it needs a third flash
  location for one item.

## Cost

- Operator: TX closes only after a flash fault or a vendor re-install, not after a power cut;
  recovery is one `SET_CONFIG` of every C-3 item; while lost, a partial C-3 write gets 48; a
  fall-back or a failed commit may leave a tightening the owner was not told is stored, which
  is logged and can be loosened back; every C-3 change may interrupt a running P2 session, RX
  included (one erase, B2.3).
- Firmware: a C-3 write always compacts; the C-3 state record and room for it; "record, then
  erase" where a newer uncommitted or failed generation is erased; the generation number in two
  places, compared before a failed generation counts.
- Host: none new. A band at 0 W refuses arming as any closed band; g7250ctl should show the log
  line and offer to write C-3.

## Consequences

- Spec A1.6 C-3 bullet and "Store writes" ("C-3 at a start", "Keeping the judgment", "Lost").
- Spec Table A1-4, C-3 row; A1.8a item 9.
- Tests T-A1-86, T-A1-87, T-A1-92, T-A1-39, T-A3-47 extended.
- The 0069 addendum, "Accepted consequence": the owner's first `SET_CONFIG` with no C-3 item.
- 0072 MUST be in force in the boot stage, and in every image of this firmware that a radio
  with such a build can start, before the first build in which TX can be armed (0036). A store
  written by an image from before 0072 is not covered: the owner sets the C-3 limits again
  after the first start of a build with 0072.

## Sources

- Decisions 0022, 0036, 0039, 0065, 0069, 0074; the openg7250 session's reviews of 2026-11-22
  and 2026-11-23.
