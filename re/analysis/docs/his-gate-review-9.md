# HIS gate review 9 — his/next 0677c2d (AXI SRAM over a reset)

Scope: `git diff 766213a 0677c2d -- his`, one commit that changes his/10 and
his/90: "AXI SRAM over a reset without power loss (0x2401FFE0–0x2401FFFF):
UNKNOWN", with a measurement.

## Findings

- **Hardware fact or stock behaviour.** The question covers two things: what
  the MCU keeps in SRAM over a system reset, which is hardware, and whether
  the stage before 0x08020000 (the vendor bootloader) writes or clears that
  range. The second is how the bootloader behaves. The bootloader stays
  installed and runs before the product on every reset (decisions 0023 and
  0025, `15-stock-recovery.md`), so its effect on RAM is a platform constraint
  for the product. This is the same kind of constraint as "state at
  application start" (F1-Q4, cleared in review 6). Stated as unknown, it
  passes.
- **No RE detail, nothing that points at the stock firmware.** The range
  0x2401FFE0–0x2401FFFF (the top 32 bytes of the first 128 KiB of AXI SRAM)
  appears in none of the RE sources (re/analysis, re/emu, g7250-base). It is
  therefore not a location taken from the stock images, and naming it reveals
  nothing about them. The wording "the stage that runs before 0x08020000" is
  neutral. The measurement writes a pattern and reads it back, so it depends
  on no stock knowledge. OK.
- **Tag.** UNKNOWN is honest: there is no source either way.

## Advisories (do not block)

- **C1 (10).** Separate the hardware part from the unknown part, so the
  product side does not read "unknown" as "the MCU may clear SRAM". RM0468
  says that the SRAMs are not cleared by a system reset; only a power loss,
  an RDP regression or a software erase changes them. Add that with its RM
  section cited (VERIFIED, datasheet), and keep UNKNOWN for the bootloader's
  effect. Without the citation, leave it out rather than state it from
  memory.
- **C2 (10).** The AXI SRAM has ECC. Writing whole 64-bit words of the pattern
  before the first read keeps a partial-word write or a read of unwritten
  words from raising an ECC error. This is useful for the measurement as well
  as for the product's later use of the range. Cite RM0468.
- **C3 (90).** Order the step: "after F1", like the F1-Q3/Q4 row, because the
  product firmware must be installed by the proven path first. The step also
  falls under the general step rule (RX, dummy loads on both ports). The
  watchdog reset should use the IWDG as the option bytes leave it (read in
  step 2).
- **N1 (nit).** Elsewhere the HIS writes "**not known**" for open facts and
  keeps the tag vocabulary to VERIFIED, HIGH, MEDIUM and HYPOTHESIS.
  "UNKNOWN" in capitals reads like a new tag. Use the usual wording.

## Verdict

**CLEARED** at 0677c2d (his/next). C1–C3 and N1 are advisory.

Follow-up for the emu session: update the status in `his/README.md` to
0677c2d and review 9.
