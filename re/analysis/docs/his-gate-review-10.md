# HIS gate review 10 — his/next 069ea0b (AXI SRAM range widened)

Scope: `git diff 30152c6 069ea0b -- his`, one commit that changes his/10 and
his/90: the AXI SRAM range in the "reset without power loss" item widens from
0x2401FFE0–0x2401FFFF (32 bytes) to 0x2401FF80–0x2401FFFF (128 bytes).

## Findings

- Only the address range changes, in both places. "Not known", the MEDIUM
  tags with their unchecked citations, and the measurement are unchanged. 10
  and 90 agree.
- **No RE detail.** Neither range appears in any RE source (re/analysis,
  re/emu, g7250-base). The range is a product-side choice and does not point
  at a location used by the stock images. OK.
- **Hardware consistency.** 0x2401FF80 is aligned to 64 bits (and to
  128 bytes), so the range is 16 whole ECC words. That fits the 64-bit-word
  rule in the same item. It lies inside the AXI SRAM, under the cleared
  320 KiB, and within its first 128 KiB, which does not depend on the
  TCM/AXI option bytes. OK.
- **Safety.** No new step. The measurement stays after F1, under the step
  rule. OK.

Wording remark, no change needed: the commit title says "range kept over a
reset", while the HIS text correctly leaves open whether it is kept.

## Verdict

**CLEARED** at 069ea0b (his/next).

Follow-up for the emu session: update the status in `his/README.md`,
`README.md` and `ROADMAP.md` to 069ea0b and review 10.
