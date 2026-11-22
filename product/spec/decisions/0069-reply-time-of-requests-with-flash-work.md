# 0069 — Reply time of requests that do flash work

- Status: proposed (2026-11-22); answers a spec gap found by the emulator's flash model
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
   store sector holds the old secret, and the reset comes within 17 s of the reply.

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
