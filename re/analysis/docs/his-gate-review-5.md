# HIS gate review 5 (his/next 10dcaa7)

**Scope.** `git diff his/integrate his/next -- his` (his/integrate holds the
cleared b95f519 content; his/next is at 10dcaa7): the roles of the three NCO
words in `80-rf-fpga.md` and `90-needs-hardware.md`. RE background:
`re/analysis/docs/his-02-nco-probe.md`, run 5. Reviewer: the gate session.

## Leak check

`his/` contains no VFO, RIT, split or XIT wording, no frequency values, nothing
about byte 18 bit 5, and no byte-0 values (checked by grep). The text keeps to
interface terms: "receive-path tuning frequency", "third tuning word". The
VFO/RIT/split table and the reading of the run stay in the RE note. **No leak.**

## The confidence tag of offsets 1 and 5 (the point raised)

What run 5 actually shows:
1. **The bytes on the bus.** The fused GPIOE+FPGA model only records what the
   MCU writes, so the logged words are real output of the factory image, not
   values the model chose. For the *offsets* (review 4) this is good evidence,
   and HIGH stands.
2. **The roles.** The model does nothing with the words: no NCO, no I/Q that
   depends on them. So the run cannot show what the FPGA does with offsets 1
   and 5. It shows which values the factory image puts there when its settings
   change. Calling them "receive-path tuning frequencies" is an RE inference
   from those values: they carry receive-only offsets, and offset 19 does not.
   That inference is reasonable, but it rests on how stock treats its
   settings, i.e. on stock behaviour, with no observation of the hardware.
3. **"Independently of each other"** is observed in the same sense: the image
   writes the two words separately. That the FPGA has two independent receive
   NCOs is again the inference.

CLEANROOM treats emulator-only facts as consistency, not hardware evidence.
Here the evidence is better than circular, because it is the image's own
output, but it says nothing about the FPGA side. **MEDIUM is the right tag**
for the roles of offsets 1 and 5 (two receive tuning words, independent). HIGH
stays for the offsets and the word format. HYPOTHESIS for offset 19 as the
transmit word is correct.

The source wording is not clear enough: the evidence line says "the 0x01 byte
offsets and the roles of the three words are what the factory image writes over
this bus in the emulator". The image writes *values*; the roles are read from
them.

## Required changes

| # | File | Change |
| --- | --- | --- |
| K1 | 80, 90 | Offsets 1 and 5 as receive-path tuning words, independent of each other: HIGH → **MEDIUM**. |
| K2 | 80 evidence line | Separate observation from inference, for example: "The 0x01 byte offsets are the bytes the factory image writes over this bus in the emulator (re/emu HIS-02 probe), which agree with the disassembly. The roles of the words are inferred from the values the image writes when its settings change; the emulator's FPGA model only records the bytes, so the FPGA's use of the words is not observed." |
| K3 | 80 Open | Replace "(HIGH, emulator run of the factory image)" with "(MEDIUM: inferred from the values the factory image writes in the emulator; FPGA use not observed)". |

Everything else in the diff is fine: offset 19 HYPOTHESIS with "transmit side not
exercised", and the open item "which physical receive path, and the transmit
role of offset 19, need a capture on the radio including transmit".

## Verdict

**CHANGES** (not cleared): K1–K3, all of them tag and wording only. A check of
that diff alone is enough to clear.

---

## Final check (diff f4538be..9d10e72, his/)

| Item | Result |
| --- | --- |
| K1 (80, 90) | applied: the roles of offsets 1 and 5 are MEDIUM in both files; offsets and word format stay HIGH. |
| K2 (80 evidence) | applied: the offsets are "observed"; the roles are "inferred from the values written; the FPGA's use of the words is not observed". |
| K3 (80 Open) | applied, using the wording proposed. |

The diff adds no stock behaviour and no untagged fact.

### Verdict (final)

**CLEARED** at 9d10e72 (his/next). Follow-up for the emu session: update the
status in `his/README.md` to the new cleared commit and review 5.
