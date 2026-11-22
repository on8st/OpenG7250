# 0069 — Reply time of requests that do flash work

- Status: accepted (ON8ST, 2026-11-22, relayed by the openg7250 session, as at `f3c15c1`);
  addendum on the start check accepted (ON8ST, 2026-11-22, as at `78aa233`); proposed
  2026-11-22; answers a spec gap found by the emulator's flash model
  (relayed by the openg7250 session, 2026-11-22)
- Date: 2026-11-22
- Builds on: 0020 §24 (retransmission), §27 (stored reply), 0055 (flash work), 0063, 0065
  (`t_erase_max`, failure bound), 0067 (blocks sent again); [STM32H72x].

## Context

A host retransmits after 500 ms without a reply, at most 5 times (A1.4), so it gives up after
about 2.5 s. A request that erases a 128 KiB sector stalls the processor for up to
`t_erase_max` = 4 s, and an erase that does not end is only declared failed after
2 × `t_erase_max` = 8 s (B2.3, 0065); the network loop does not run meanwhile. A correct radio
at the datasheet's maximum erase time therefore makes a correct host give up. This applies to
an `UPDATE_BLOCK` or `UPDATE_BEGIN` that erases, to a management write whose store write needs
a compaction, and to the streaming installer, which erases and programs a whole sector at the
last block of it (A1.8a item 3).

The emulator named three routes: (a) answer the block before its erase; (b) erase the whole
staging slot at `UPDATE_BEGIN`, with a longer host wait for it; (c) a longer retransmission
window for every management request that may do flash work.

## Decision

1. **Route (c), with a bound on the work per request.** A request MUST do at most one sector
   erase and program at most one sector (128 KiB). Store compactions, a staging slot erased
   sector by sector as its blocks arrive, and the installer's sectors all fit this. Erasing a
   whole slot in one request is not allowed. The bound counts the flash work before the reply.
   - **Work after the reply, before a reset** (firmware question, 2026-11-22). A request that
     ends in a commanded reset (A1.9) MAY do further flash work after its reply (step 3) and
     before the reset (step 5): at most two sector erases with what they need programmed, the
     reset following within 2 × (2 × `t_erase_max`) + 1 s = 17 s of the reply. The rules of B2.3
     "during flash work" apply; RF is gone and nothing is armed (A1.9), so 0055 holds. This is
     what `FACTORY_RESET` needs: a first compaction moves the store into the other sector, a
     second erases the sector that still holds the old secret, so that the secret is gone from
     both sectors before the reset (A1.6a). If an erase after the reply fails
     (0065), it is logged (kept over the reset, 0068) and the next start finishes it before the
     management socket opens. A host that waits for the radio after such a reset allows 17 s
     more than after a `RESTART`.
   - **`CHANGE_SECRET`** (openg7250, 2026-11-22: the old secret MUST NOT stay readable in flash
     longer than until the next start). `CHANGE_SECRET` does not end in a reset, so it gets
     one erase, before its reply: the new secret is always written by a compaction into the
     other store sector, which erases that sector and writes the live items there with the new
     secret, so the sector with the old secret is no longer live (it is "stale"). Because
     `CHANGE_SECRET` always compacts, the live sector never holds an older secret record
     (firmware input, 2026-11-22: an appended record would need a second erase). The stale
     sector is erased by whichever comes first: the next compaction (it is then the sector
     that compaction erases), or the start check below. It is not erased on its own at any
     other time, so that no flash work starts outside a request or a start (B2.3).
   - **Start check** (firmware proposal, 2026-11-22). At every start, before the management
     socket opens, the firmware checks the non-live store sector; if it holds a secret record
     other than the live secret (or any secret record while the live generation has none, as
     after a factory reset), it erases that sector once and logs `store: stale secret
     erased`. This is safe because the live sector holds a complete generation. It covers a
     secret left by `CHANGE_SECRET` and an erase left unfinished by a `FACTORY_RESET`. A
     non-live sector that holds only the live secret is left as it is. Exception: if the store
     has to fall back to the non-live sector's generation at that start (0065 point 5, newest
     commit word unreadable), that sector is the live one and is kept; this is logged. `SET_FIRST_SECRET` never replaces a secret: the
     first-secret window opens only while no secret is set (A1.10), and a factory reset erases
     the old secret from both sectors (above).
2. **Reply bound for the radio.** The G7250 MUST send the reply to such a request within
   `t_flash_reply` = 15 s of receiving it, whether the flash work succeeds or fails (0065).
   Derivation: an erase of at most `t_erase_max` (4 s), programming one sector of 4096 words
   of 266 bits at the datasheet maximum of 580 µs each (2.4 s, [STM32H72x] Table 48, x8), a
   failure declared at the latest 2 × `t_erase_max` (8 s) into the operation that hangs, and
   0.5 s for the rest: 14.9 s, rounded up. Requests without flash work keep their usual
   reply times.
3. **Requests that may do flash work** (the list of B2.3 "Flash work", made explicit):
   `SET_CONFIG`, `SET_FIRST_SECRET`, `CHANGE_SECRET`, `FACTORY_RESET`, `SET_CALIBRATION`,
   `CLEAR_CALIBRATION`, `UPDATE_BEGIN`, `UPDATE_BLOCK`, `UPDATE_ACTIVATE`, `ROLLBACK`.
4. **Host rule** (A1.4). For these requests a host SHOULD keep retransmitting every 500 ms for
   at least 16 s before it gives up, instead of 5 times. For every other request the 5 times of
   A1.4 stay. Retransmissions that arrive during the flash work are repeats: they are
   answered from the stored reply once the work has ended and are not executed again (A1.4);
   a host may then get several copies of the reply and uses the first.
5. **Other requests during flash work.** A request from any host that arrives during flash
   work is answered after it ends, if the 5 s stored-reply time and the receive buffer allow;
   a host that gives up after 2.5 s retries later. B2.3 already allows this for P2.
6. **Tests.** T-B5-22: with every erase stretched to `t_erase_max` and every word program to
   its maximum, a host that retransmits every 500 ms for 16 s completes an update, a
   `SET_CONFIG` with a compaction and an installer sector; each reply comes within 15 s; each
   request is executed once; flash.log shows at most one erase per request. T-A1-85: an erase
   that does not end gets its 55 reply within 15 s. T-A1-39: after a `FACTORY_RESET` neither
   store sector holds the old secret, and the reset comes within 17 s of the reply. T-A1-92
   (new): the old secret after `CHANGE_SECRET`.

## Addendum: the start check after a fall-back, and torn records (accepted by ON8ST, 2026-11-22)

Two firmware readings, relayed by the openg7250 session, 2026-11-22.

1. **After a fall-back** (newest commit word unreadable, 0065 point 5). The live generation
   is then the older one, and the other sector holds the failed newer generation, with the
   newer secret if the failed write was a `CHANGE_SECRET`. That sector is erased by the start
   check, as its wording says: this is meant. The failed generation cannot be used again (its
   commit word is unreadable and is never programmed again, 0065), no host can read it for
   diagnosis, and keeping it would keep a secret that is not in use readable in flash. The
   start logs the fall-back line and `store: stale secret erased`. The radio is then back on
   the older secret: a request authenticated with the newer one gets 33, and the log tells
   the owner why.
2. **Torn records** (a stale-sector erase or a compaction cut by a power loss). The rule "not
   readable from flash after the next start" covers part of a secret too. The start check
   therefore MUST erase the non-live sector also when it holds any word the store cannot place
   in a complete, checked record, unless the sector is fully erased; a non-live sector that
   holds only complete records with no secret other than the live one is left as before.
   Such an erase is logged `store: non-live sector erased (unrecognised data)`.
   - **Only with a live generation** (openg7250, 2026-11-22). This erase, and the start check
     as a whole, is done only when the live sector holds a complete generation of this
     firmware's store. With none in either sector (the first start after the install, or both
     sectors holding data that is not this firmware's) the start check does nothing: the store
     starts from the Table A1-4 values and its first write erases the sector it writes into,
     as before.
   - **"Complete generation"** (firmware finding, 2026-11-22). The live sector holds a complete
     generation when either (a) its generation has a readable commit word, or (b) it holds the
     store's first generation, which by the firmware's design carries no commit word, and at
     least one record of it passes its check that is one of this closed list: the management
     secret (A1.10), a configuration item (A1.6, C-1 to C-7), a calibration entry (A1.12). Other
     records do not count: the boot counter (B10.2 item 5), the boot record (A1.8), the staging
     mark (0067 point 4), the factory-reset mark, and any record a later version adds unless
     it is added to this list. Nothing a start writes before the start check (the boot
     record, the boot counter, a compaction that finishes a factory reset) is a new record of
     this list, so the boot counter of this start cannot make a sector count (correction
     after review, 2026-11-22). An erased sector, or one holding only data it cannot place in a checked
     record, never holds a complete generation, so the start check never erases the other
     sector on its behalf. The firmware's proposal (a commit, or a first generation that holds
     a secret) is too narrow: a first generation with only configuration or calibration, fallen
     back to from a failed newer generation that carries a secret, would leave that secret in
     flash. Cases:
     - the only committed generation is in sector 6, its commit word becomes unreadable, and
       sector 7 is erased: sector 7 holds no complete generation, the start check does
       nothing, and sector 6 is not erased (the store then reads as 0065 point 5 says). This
       holds although the boot counter of this start lands in sector 7, and at every later
       start until a record of the closed list is written there;
     - fall-back after the first `CHANGE_SECRET` to the first generation, which holds the
       first secret: (b) holds, and the failed newer generation is erased (point 1);
     - fall-back to a first generation holding only configuration or calibration: (b) holds as
       well, and the failed newer generation is erased.
   - **Only the start check is limited** (firmware reading of its start order, 2026-11-22).
     Other store writes at a start erase as A1.6 "Store writes" says: the boot counter or the
     boot record compacts when the live sector is full, and finishing a factory reset compacts;
     each erases the other sector first. These rules say only when the start check erases.
   - **Accepted consequence.** In the first case above, the owner's first `SET_CONFIG` or
     `SET_CALIBRATION` after it writes a record of the list into sector 7, and the next start
     check erases sector 6. That is meant: sector 6's generation has not been in use since its
     commit word became unreadable.
   - **Assumption: a first generation is recognised by having no commit word.** A compaction
     into a sector cut before its commit, with the other sector unreadable as well, looks the
     same and may be partial. Treating it as complete needs two faults at once (a cut
     compaction and an unreadable generation) and is accepted; no mark of a first generation
     is required.
   - **Sector 6 and the vendor bootloader** (0061, P-SYS-5). One store sector is sector 6,
     where the vendor bootloader may stage ([HIS] HIS 15 rule 5). If it writes there at every
     start, every start would erase that sector, and the store would lose its spare sector.
     The drill F1 therefore counts this erase: after the install and one store write, a
     `RESTART` and a power cycle MUST each start with no `store: non-live sector erased
     (unrecognised data)` line. If such a line appears, the store location of 0061 is decided
     again before any further install.

Tests: T-A1-92 (a fall-back after a `CHANGE_SECRET`; a stale-sector erase cut by a power loss;
no erase without a live generation); the drill F1 (P-SYS-5).

## Alternatives considered

- (a) Answer a block before its erase: the next block then waits for the erase, so the gap
  moves to the next request and nothing is gained; an erase failure would also be reported on
  a request that did not cause it.
- (b) Erase the whole staging slot at `UPDATE_BEGIN`: one request then holds several sector
  erases (16 s and more with failure bounds), so it needs an even longer host wait, and it
  still leaves store compactions and the installer uncovered.
- A reply "in progress" before the erase, then the final reply: a new message type for one
  case, while a longer retransmission window needs nothing new.

## Consequences

- Spec A1.4 (retransmission bullet), B2.3 "Flash work" (one erase per request,
  `t_flash_reply`); tests T-B5-22 and T-A1-85 extended.

## Sources

- [STM32H72x] DS13311 Rev 5 Table 48; decisions 0020, 0055, 0063, 0065, 0067; the emulator's
  flash-model finding (relayed by the openg7250 session, 2026-11-22).
