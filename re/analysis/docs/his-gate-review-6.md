# HIS gate review 6 — his/next ec929f2 (F1/F2 answers)

Scope: `git diff e2725fe ec929f2 -- his` (his/10, his/40, his/85, his/90),
the answers to the firmware's hardware questions F1-Q1..Q6 and F2-Q1..Q5.
RE background read: `re/analysis/docs/his-f1-f2-answers.md`. I judged it
against the gate rule (hardware facts only; boot order, timeouts and register
values that stock chose stay on the RE side), decision D1 from review 1
(crystal frequencies only from the board or a schematic, never derived from
stock PLL settings), and decisions 0023 and 0025.

Note: `CLEANROOM.md` on this tree only has its section headings. I took the
rule's wording from the request and the gate brief.

## The four points raised by the RE session

### 1. HIS 10: "factory PWR_CR3 = 0x2E" — the value goes, the wiring stays

`0x2E` and the bit list are what stock wrote. The product needs two facts:
the supply configuration the board is wired for, and the RM0468 constraint
that the supply configuration can be written only once per power-on, so a
mismatched value can stop the MCU. Those two facts are safety content and
must stay. The register value is one encoding of them and has to go.

The SMPS output level is a separate question. If the SMPS output also feeds
circuits outside the MCU ("supplying external circuits"), then 2.5 V is an
electrical value on the board and must be stated as such. Whether anything
outside the MCU actually uses that output has not been observed: stock may
have picked that mode without needing it. So 2.5 V on an external rail is
inferred, not shown.

The tag has to match the part (see point 4). If the part were an H723/H733,
which has no SMPS, the board could not be wired for an SMPS. The supply
statement and the part statement rest on the same evidence, so the supply
statement cannot be HIGH while the part is MEDIUM.

**P1 (required).** Replace the PWR_CR3 bullet with roughly:
"**Core supply:** the board is wired for the RM0468 supply configuration
*SMPS supplies the LDO, SMPS output 2.5 V also available to external
circuits* (MEDIUM: inferred from the configuration the factory image selects;
tied to the part, see above). Whether any board circuit uses the 2.5 V SMPS
output is not known. The supply configuration can be written only once per
power-on (RM0468, PWR section; cite the paragraph). A configuration that does
not match the wiring can stop the MCU, so new firmware selects this
configuration and no other until the wiring is checked. Whether the vendor
bootloader has already written it is not known: read it at application start."
No hex value, no bit names taken from stock. Bit names from RM0468 are
acceptable if the product needs them.

The 90 row "F1-Q3, F1-Q4" must also drop "PWR_CR3 as the factory image sets
it is known (HIGH)". It should say "supply configuration (MEDIUM, `10`)", and
add a check of the SMPS/LDO pins and external use from board photos.

### 2. HIS 40: CH394 reset timing and SCK — only the datasheet limits stay

The 12–19 ms no-access window from CH394DS1 V1.2 §10.4.3 is a datasheet fact,
and VERIFIED is right for it. Three values are stock timing:

- "RST# held low for 10 ms",
- "the first access 100 ms after it goes high",
- "RST# low for more than 2 s at power-up".

They are a timeout and a boot sequence that stock chose, which is exactly
what the rule keeps out. The product needs the datasheet's minimum RST# low
time and the datasheet's window, nothing else.

"SCK about 17.19 MHz (PCLK4 137.5 MHz / 8)" goes further: it exposes stock's
clock tree (the PCLK4 frequency and a prescaler). Review 2 let
"about 17 MHz is known to work" through. Strictly, that is still a stock
operating point, not a limit. What the product needs is the manual's maximum
SCK.

**P2 (required).**
- Remove the "Known to work: …10 ms…100 ms…2 s" sentence.
- Remove "PCLK4 137.5 MHz / 8" and "17.19".
- Add to the 90 table (or the 40 Open list): "minimum RST# low time and
  maximum SPI clock: from CH394DS1 (not yet taken)".

**A1 (advisory).** "About 17 MHz has been used on the board" may stay only
until the manual's maximum is recorded, and only as a description of the
board link, not of stock code. It should be tagged MEDIUM and say "upper
value seen in use, not a datasheet limit". I would rather see it replaced by
the datasheet figure.

### 3. HIS 85: "ATU strobes idle high (HIGH)" — idle level kept, as MEDIUM

"The factory image drives PD13, PG2 and PG5 push-pull high from its GPIO
set-up" describes stock initialisation. The interface fact is the strobe
polarity: which level leaves the latch untouched and which edge or level
latches. That fact belongs to the latch part, and the part is not identified
(74HC595 kind, HYPOTHESIS). So "idle high" is inferred from the level stock
starts with, which could also be a choice stock made. It is not observed on
the hardware, and it is not in a datasheet.

The level is worth keeping because it is safety content. A wrong idle level
or a spurious edge could latch random data into the ATU relays. But HIGH is
not justified.

**P3 (required).** Reword as:
"**ATU strobe polarity:** the latch strobes (PD13, PG2, PG5) are inactive
high (MEDIUM: inferred from the level the factory image starts them at; the
latch part and its active edge are not identified)."
Keep the existing sentences about board pulls (not known) and the high
impedance between reset and GPIO set-up, which are correct (the STM32H7
reset state is analog mode, no pull).

In the 90 row F1-Q5, change "Idle level high (HIGH)" to MEDIUM. Then, as
advisory **A2**, make the safe-state table row agree with it. The row
currently says "leave unwritten; no safe value is known". It should say
"drive the strobes to the inactive level (high, MEDIUM) and never pulse them;
no safe latch value is known".

The same pattern appears in HIS 40 (F2-Q4): "Enabling the MCU's internal
pull-up on PC13 is the safe choice and is known to work (HIGH: the factory
image does so)". The table already says INT# is active low and needs a
pull-up (VERIFIED). **P4 (required):** drop "(HIGH: the factory image does
so)" and "known to work", and write "a board pull-up is not known; use the
MCU's internal pull-up unless one is found (`90`)".

### 4. HIS 10: part MEDIUM vs AXI SRAM 320 KiB HIGH — consistent, with one fix

- **AXI SRAM.** The 128 KiB AXI SRAM plus 192 KiB shared with the ITCM through
  the TCM/AXI option bytes is the same on H723/H725/H733/H735 (RM0468). The
  size therefore does not depend on which H72x/H73x part it is, and is
  consistent with a MEDIUM part. The evidence is an initial stack pointer at
  the top of 320 KiB in an image that runs on shipped units; with a smaller
  AXI SRAM the first stack push would fault. That supports HIGH for "the
  option bytes on the units as shipped leave 320 KiB to the AXI SRAM". It is
  a statement about option-byte state on shipped units, not about wiring, and
  the HIS words it that way ("on the units as shipped"). OK. **A3
  (advisory):** add "a unit whose option bytes were changed may differ; read
  them in step 2", which is already implied by the 90 row.
- **Part.** MEDIUM is right. The HIS states "H723/H733 have no SMPS" flatly,
  and as a datasheet fact that is correct. But the RE background itself calls
  the exclusion LOW confidence, because SMPS code in the image does not prove
  that the device has an SMPS. MEDIUM for the part covers that. The
  inconsistency is in the supply bullet (HIGH), which rests on the same
  evidence: fixed by P1.
- "Expected DEV_ID 0x483" and the port-H package argument are hardware facts
  with sources. OK.

## Further findings

**P5 (required): HSE 25 MHz breaks decision D1.** "HSE: a 25 MHz crystal …
(HIGH: the factory image runs HSE on with HSEBYP clear and derives all PLLs
from 25 MHz)" is exactly what D1 (review 1, decided by ON8ST 2026-11-20) rules
out: crystal frequencies are recorded "only once read from the board (crystal
marking) or a schematic; do not derive them from stock PLL settings". It also
leaks stock's clock tree ("derives all PLLs"). Replace it with:
"**HSE:** frequency and crystal vs external clock not recorded until read from
the board (decision D1); see `90`". Add a 90 row: crystal or oscillator
marking near OSC_IN/OSC_OUT from a board photo. If the RE side thinks the
inference is strong enough, that is a question for ON8ST to revisit D1. It is
not a call the HIS can make.

**N1 (nit, 40).** The new "MAC address (F2-Q3)" bullet repeats the existing
"MAC address" bullet just above it. Merge the two.

**A4 (advisory, 40, PHY status).** "the register offsets, including 0x002E,
match the W5500 map (HIGH, firmware analysis)" uses the stock driver as the
source for a chip register map. The CH394 manual (table 6-1) is cited
elsewhere in the same section. Check 0x002E there and tag it VERIFIED
(datasheet), or keep HIGH but name the manual check as open. The bit layout
at MEDIUM is fine.

**Checked, no change needed:**
- F1-Q4, the unknown state at application start, and the 90 row that
  measures it with a test image after F1 (read-only; reports on USART1).
- CHIPV at 0x0039 (VERIFIED, manual), the W5500 0x04 caveat, MODE bit 7 as
  software reset, CHIP_CONFIG at 0x003E.
- F2-Q5, the destination address, as unknown.
- The high-impedance note in 85.
- None of the new 90 steps writes option bytes or sector 0. All are reads,
  resistance checks with power off, or photos. This is consistent with
  0023/0025 and with the safe step order.

## Verdict

**CHANGES.** Required: P1 (PWR_CR3 value out, supply configuration as wiring,
MEDIUM), P2 (CH394 stock reset timing and SCK clock tree out; datasheet limits
open), P3 (ATU strobe polarity MEDIUM, worded as inference), P4 (PC13 pull-up
without "the factory image does so"), P5 (HSE removed per D1, 90 row added).
Advisory: A1–A4, N1. With P1–P5 applied, ec929f2's content is clearable on a
diff check.

## Final check (diff ec929f2..b7df142, his/)

| Item | Result |
| --- | --- |
| P1 (10, 90) | Applied. No hex value and no stock bit list; the RM0468 configuration is named as wiring, MEDIUM and tied to the part; the write-once constraint and "select this configuration and no other" are stated; the 90 row adds the SMPS/LDO pin check from photos. |
| P2 (40, 90) | Applied. The 10 ms / 100 ms / 2 s values and the PCLK4 / prescaler are gone; the minimum RST# low time and the maximum SCK are open, with a 90 row (datasheet, not a radio measurement). |
| P3 (85, 90) | Applied, using the wording proposed; the 90 row is MEDIUM. |
| P4 (40) | Applied in 40, but **not in 90**: the F2-Q4 row still says "internal pull-up known to work", which is the stock-behaviour phrase P4 removed. See Q1. |
| P5 (10, 90) | Applied. HSE is not recorded (D1), and a 90 row reads the crystal marking. |
| A1 (40) | Applied; judged below. |
| A2 (85 table) | Applied, and consistent with P3. |
| A3 (10) | Applied. |
| A4 (40) | Applied: the check against table 6-1 is named as open. |
| N1 (40) | Applied: one MAC bullet. |

### A1: "about 17 MHz, upper value seen in use, not a datasheet limit (MEDIUM)"

Acceptable as worded, for the time being.

- It does rest on the factory image's operating point, but the form no longer
  carries stock detail: no clock tree, no prescaler, a rounded value.
- What remains says something about the board, not the code: the SPI6 link on
  shipped units carries about 17 MHz. The datasheet cannot give that, because
  the chip maximum says nothing about traces or the connector. Once the
  datasheet maximum is taken, both values are useful: the lower of the two is
  the safe bound until the link is measured.
- The tag MEDIUM and "not a datasheet limit" stop the product side from
  reading it as a limit.

An "open until the datasheet maximum" wording would also pass. I don't
require it.

### Q1 (required)

In `his/90-needs-hardware.md`, row "F2-Q4: INT# board pull-up", change
"Unknown; internal pull-up known to work" to
"Unknown; use the MCU's internal pull-up unless one is found (`40-ethernet.md`)".

The rest of the diff is clean. It adds no stock behaviour and no untagged
fact, and every new 90 step is a photo, a read or a datasheet look-up.

### Verdict (final check)

**CHANGES**: Q1 only, a one-row fix in 90. With Q1 applied, b7df142 is
cleared on a diff check of that row.
