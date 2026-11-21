# What the cleared HIS answers, and what it changes in the spec

- Status: analysis, 2026-11-20. Input to decisions; changes nothing normative by itself.
- Source: [HIS] at commit `2a38b7f` (content `89f5b5d`, cleared by gate review 2), read in the
  read-only HIS worktree. Sections are cited as "HIS 15" for `his/15-stock-recovery.md`, and
  so on. HIS confidence tags (VERIFIED, HIGH, MEDIUM, HYPOTHESIS) are kept where they matter.
- The HIS records hardware facts only (HIS README). Its evidence often "rests on disassembly";
  that is the RE side's concern. This analysis takes the facts and tags as given and never
  looks behind them (AGENTS.md rules 1, 2, 7).

## 1. Summary

| | Effect on the spec |
|---|---|
| **Firmware update and the way back** (HIS 10, 15) | Largest impact. One stock image is 453 152 bytes, the application area 896 KiB, so two stock-sized images do not fit and no RAM region holds an image. The only documented update path of the Main runs through the Control (USB drive and the Control's update menu). Risk R5 of 0025 materialises for the first install. A1.8 and A1.8a need a new decision. |
| **CTRL link and power** (HIS 20, 70) | Confirms 0023: CTRL stays idle, PD5 never low. New: switching off is the Main's job (watch the power key PH5, drop the power hold PH9). Power-up without the Control is still open. |
| **Audio** (HIS 50) | The Main has its own AIC3204 codec with I²S, and audio between Main and Control crosses the CTRL cable as analog. This reopens the premise of 0017 ("the main unit cannot play audio") and the microphone question of 0004. |
| **RF control and safety** (HIS 85) | A complete safe-state table and pin map for T/R, filters, PA bias DAC, ATU latches and metering. The ATU is driven by the Main directly; tuning is our firmware's job. Many values (relay timing, detector laws, safe bias) are to be measured. |
| **FPGA and sample rates** (HIS 80) | Consistent with 0004 and 0022 §81: rates 384/192/96/48 kS/s, 24-bit I/Q both ways, phase word on 122.88 MHz (MEDIUM). |
| **Ethernet** (HIS 40) | CH394 on SPI6, mode 0; "about 17 MHz is known to work". The budget's higher SPI clocks are still unproven. |
| **Not answered** | HIS-02 (number of DDCs, in part), HIS-06 antennas, HIS-15 MAC, HIS-16 sources, HIS-20 randomness, most of HIS-13 and the timing values in HIS-07/11/12. |

## 2. Per question

Status: **answered**, **partly** (answered in part, rest still open), **open** (not in the HIS).

| HIS | Status | What the HIS says | Spec consequence |
|---|---|---|---|
| 01 ADC, sample clock | partly | RX I/Q from the FPGA at 384/192/96/48 kS/s, 24-bit, rate index 0–3 (HIS 80, MEDIUM); status read 0x02 includes an ADC peak value. ADC part and clock: not stated. | 0004 rates confirmed. P2 ADC overload/peak (0006) can use the ADC peak. |
| 02 Number of receivers | partly | One RX I/Q read command (0x04); the config frame carries three NCO words (HIS 80). How many independent DDCs: not stated. | Keep one DDC (0004). Ask what the three NCO words are. |
| 03 Second ADC, feedback | partly | Forward and reflected detectors on ADC1 (HIS 85). No second RF ADC stated. | No PureSignal (as now). |
| 04 DSP clock, frequency word | answered (MEDIUM) | 32-bit phase word f × 2³²/f_clk, f_clk = 122.88 MHz, first Nyquist zone plus a spectral-inversion flag (HIS 80). | The virtual 122.88 MHz clock of 0022 §81 equals the FPGA clock: no conversion if confirmed. Above 61.44 MHz the firmware sets the inversion flag; the G7250 tunes to 54 MHz, so this matters only for the flag. |
| 05 Frequency reference | partly | VersaClock 6 on I²C1, crystal HYPOTHESIS 26 MHz, OUT1 likely the FPGA clock (HIS 30). External reference: not stated. | P2 "10 MHz PLL locked" stays 0. |
| 06 Antenna switching | open | Not in the HIS. | — |
| 07 Filters, front end | partly | SPI1 chain A: 7 TX low-pass bits plus bit 7 T/R or PA-enable; chain B 24 bit: RX band-pass, preamp, attenuator (HIS 85, HIGH to HYPOTHESIS). Relay timing: per datasheet once identified. | Register map filter fields can be defined per band. t_settle, t_rf_off still open. |
| 08 Internal ATU | partly | Three 8-bit latches (PD13 network/bypass, PG2, PG5 reactance banks); no safe value known (HIS 85). | There is no ATU controller: tuning, memory and reporting are our firmware's job. Keep ATU functions out of M1–M3; record as a later product item. |
| 09 TUNER, SEND, ALC | partly | ALC on PA2, EALC on PC5 (ADC1) (HIS 85). SEND, TUNER: not stated. | SEND stays TBD. |
| 10 PTT, interlock | partly | PTT input PH6, CW key PH3/PH4 on the Main (HIGH); RX-only firmware MUST NOT transmit on them (HIS 85). Whether the Control can key the PA by itself: not stated; T/R and PA-enable are Main pins. | Local PTT/key exist on the Main: the spec's single-owner and arm rules need a "local key" source (decision). |
| 11 Sensors | partly | ADC1: forward, reflected, PA current, PA temperature (NTC), supply voltage, EALC, ALC (HIS 85, VERIFIED pins). Constants by calibration on the radio. Rates, PEP: not stated. | Telemetry (0006) feasible; limits stay TBD until calibration. |
| 12 Power control | partly | 8-channel DAC: drive level (1), driver bias (2), power reference (6), final bias (7, 8), fan (3); RF-gain voltage on DAC1 OUT1 (HIS 85). Bias channels MUST be 0 when not transmitting. Safe TX bias: by measurement. | Fan control is the Main's job. TX needs a calibration step before M4. |
| 13 Reset, fault | partly | Pins are high-impedance until init; board pull-ups unknown; firmware SHOULD set the safe state first (HIS 85). Reset cause, counters: not stated. | A3 safe state can cite HIS 85. |
| 14 Ethernet | partly | CH394 on SPI6 (PG12–15), RST PB5, INT PC13, mode 0; max SCK per datasheet, about 17 MHz known to work (HIS 40). Sockets vs MACRAW, link status, DHCP: "defined by the product spec". | The spec's choice of sockets stands. 384 ksps (needs SPI ≥ ~50 MHz, 0004) remains to be proven on the radio. |
| 15 MAC address | open | Not in the HIS. | — |
| 16 Public sources | open | Not answered. HIS 15 cites a public "vendor update guide V1.0.1". | Ask for its location, to cite it ourselves. |
| 17 CTRL link | answered | USART2 PD5/PD6; MUST NOT transmit; PD5 never low; the stock Control runs on with a silent Main (MEDIUM); the Main powers the Control over CTRL; the power key most likely reaches PH5 over CTRL; audio between the units is analog over CTRL (HIS 20, 50, 70). The Main cannot use the Control's USB, screen or buttons while CTRL is idle. | 0023 confirmed. Any feature that needs the controller (TF card, physical-presence button) is out. Power-off handling is the Main's (decision). |
| 18 Processor, memory | answered | Main STM32H725/H735 class (MEDIUM). Flash: bootloader sector 0 (128 KiB), application 0x08020000–0x080FFFFF (7 × 128 KiB). AXI SRAM 320 KiB. Stock Main image 453 152 bytes. FPGA configures itself; the Main does not load it and MUST NOT touch its flash (HIS 10, 15). Stock data: EEPROM 0x50 read-only, Backup SRAM and RTC backup registers not to be used. | See section 3. The FPGA part of 0024/A1.8a (component "FPGA") is not needed. |
| 19 Loading, recovery | answered, with unknowns | Updates are done by the vendor bootloader after a reset. The only documented path runs through the Control (USB drive, update menu). Transport and entry condition unknown. Whether the vendor bootloader starts an application it did not write: unknown. A Main-side installer for an owner-supplied stock image MAY exist, writing sectors 1–7 from RAM, block by block (HIS 15). The rollback drill F1 gates the first flash. | See section 3. |
| 20 Randomness | open | Not in the HIS. | The STM32H7 family has a hardware RNG per ST's public datasheet; to confirm for the exact part. |

## 3. Firmware update and the way back

### 3.1 Facts (HIS 10, 15)

- Our image MUST start at 0x08020000 with its vector table there, and MUST NOT write sector 0
  or the option bytes.
- The application area is 896 KiB (sectors 1–7, 128 KiB erase unit). Our own persistent data
  must also live there.
- A stock image needs 4 sectors (453 152 bytes). Stock plus a full copy of ours does not fit
  unless ours is at most about 3 sectors, and then there is no room for two slots of ours.
- No RAM region holds a whole image: staging must be in flash.
- The vendor bootloader starts whatever is at 0x08020000 (implied by the placement rule; whether
  it accepts an application it did not write is unknown).
- The only documented path to (re)install stock firmware runs through the Control.

### 3.2 What no longer holds

1. **0025 R1/R4 for stock → new.** While the stock firmware runs, the only documented path
   needs the Control. "One path on the main unit, also for the first install" cannot be met
   unless F1 finds another bootloader entry. This is R5; it goes back to ON8ST.
2. **0024 Decision 3 / A1.8a item 2 (retained stock image, `RETURN_TO_STOCK`).** Possible only if
   our firmware, its config and the stock image fit together (stock 4 sectors leaves 3), and
   then without a second slot of ours. In practice: drop, or MAY with a size limit.
3. **0025 R3 "same staging" for a stock image.** A stock image is too large to stage beside the
   running firmware. HIS 15 describes the only workable form: a RAM-resident installer that
   receives and writes the image block by block over the running firmware. That has no
   staging, no full-image hash before the first erase, and no way back to our firmware after it
   starts. A broken transfer leaves no valid application; recovery is then the vendor path,
   through the Control.
4. **A1.8 U-5/U-6 (atomic switch, trial boot, rollback)** for our own updates. The vendor
   bootloader always starts 0x08020000, so A/B slots need a small boot stage of our own at
   0x08020000 that picks the slot. That stage is never updated over the network (U-7 then applies
   to it). Layout example: sector 1 our boot stage, sector 2 our configuration, sectors 3–4 slot A,
   sectors 5–6 slot B, sector 7 log or spare; each slot 256 KiB. Whether our firmware fits in
   256 KiB is a firmware design question.

### 3.3 Options for ON8ST

**R5 (first install, stock → new).** The outcomes listed in 0025 R5, now with the facts:

- **(a) One-time first install through the vendor path** (USB drive on the Control, the
  Control's update menu), with our image in the form the vendor path takes. After that, updates
  and the way back go through the Main (A1.8, A1.8a). After a return to stock, a re-install is
  again a first install through the Control. Depends on F1 showing the vendor bootloader
  accepts our image.
- **(b) A Main-side path found by F1** (another bootloader entry, for example on USART1).
  Keeps 0025 as written, but is only a hope until F1.
- **(c) Our own boot loader in sector 0.** Excluded by HIS 15 rule 1 and 0024 R2.

Recommendation: (a), with (b) kept as an option if F1 finds it.

**Way back (new → stock).** Options:

- **(i) Streaming installer** (HIS 15): the stock image is written block by block over our
  firmware, from RAM, without the Control. One way, no staging; a broken transfer needs the
  vendor path. The spec adds a per-block check and a check of the whole written image before the
  final reset, and the host keeps the transfer going (the CLI can resume where it stopped while
  the installer still runs).
- **(ii) Vendor path only:** the owner uses the Control's update menu with the stock files.
  Simple, but needs the Control, against 0025 R2.
- **(iii) Both:** (i) as the main way, (ii) as the recovery path.

Recommendation: (iii). Consequence: `RETURN_TO_STOCK` (retained image) is dropped or becomes
MAY; `ENTER_VENDOR_RECOVERY` is dropped (no software entry known, HIS 15).

**Our own updates (new → new).** A/B with our own boot stage (3.2 item 4), or a single slot
with the streaming installer and no rollback. Recommendation: A/B, which keeps U-5/U-6.

## 4. Other decisions the HIS raises

1. **Power key and power hold** (HIS 20). The Main must watch PH5 and drop PH9 on a key press,
   after the safe state. The spec needs a rule for this: the power key ends any TX
   immediately, and a network command to switch off (management channel) is optional.
   Power-up without the Control is still open (HIS 20 "Open").
2. **Local PTT and key** (HIS 85). PH6, PH3/PH4 are on the Main. Today the spec knows only
   network keying. Options: ignore local inputs, or treat them as a key source of the owner
   session under the same arm. Until decided: ignore (as HIS 85 requires for RX-only builds).
3. **Audio** (HIS 50). With a codec on the Main and analog audio to the Control, receive audio
   on the controller's speaker and microphone audio for TX may be possible without the serial
   link. This would revisit 0017 and the microphone part of 0004. Recommendation: note it as a
   later option; no change now.
4. **ATU** (HIS 85). Tuning is our firmware's job. Recommendation: ATU in bypass (latches
   unwritten) until a later milestone; register map keeps the ATU fields reserved.
5. **First secret** (A1.10, 0008). The controller is not usable, so the first-secret set-up
   stays "first host after factory reset" with its existing limits.

## 5. Questions back to the HIS

- HIS-02: what are the three NCO words in the 0x01 frame (RX, TX, a second RX)?
- HIS-06, HIS-15, HIS-20: antennas, MAC address, RNG of the exact part.
- HIS-16: where is the public "vendor update guide V1.0.1"?
- HIS-19: the result of F1 (does the vendor bootloader start an image it did not write; is there
  a bootloader entry without the Control).
- HIS-17 (power): power-up without the Control (HIS 20 "Open").
- HIS-07/11/12: timing and calibration values, once measured.

## 6. Addendum: follow-up answers at `d0a39b0` (gate review 3)

| HIS | Answer | Spec consequence |
|---|---|---|
| 02 | Two of the three NCO words tune the receive path (MEDIUM); third word and order need a bus capture (HIS 80, 90). | One DDC stays (0004). Pending hardware. |
| 06 | ANT1/ANT2 on the rear; selection most likely PD13 latch bits 4–5 (MEDIUM), no safe value; latch unwritten (HIS 85). | Antenna selection shares the ATU latch: like the ATU (0032), not selectable until confirmed; `ANT_STATE` reads 0 "unknown". Pending hardware. |
| 15 | MAC address from the CH394's own registers; uniqueness unverified (HIS 40). | Discovery and identification report the CH394's MAC. Pending hardware: uniqueness. |
| 16 | Public vendor update guide: Radioddity "Extended manual for Xiegu G7250 v1.0.1" (HIS 15). | Cite in `sources.md`; A1.8a item 9. |
| 17 | Power-up without the Control needs hardware (HIS 20, 90). | Pending hardware (0029). |
| 19 | F1 drill and safe step order in HIS 90. | Pending hardware (0028). |
| 20 | True RNG present on both candidate parts (HIS 10, HIGH). | A1.10 challenges and the arm token use the hardware RNG. |
| — | IWDG start mode open (HIS 10, 90); SWD on PA13/PA14. | Firmware only. |

## 7. Addendum: `b95f519` (gate review 4)

- HIS-02 (HIS 80): 0x01 frame layout, NCO words at byte offsets 1, 5 and 19 (HIGH). Firmware
  detail only; no change to the spec, which offers one DDC (0004). The role of each word stays
  pending hardware.
- HIS 90: USART1 on PB6/PB7 now VERIFIED (debug console for the F1 test image); no spec change.

## 8. Addendum: `9d10e72` (gate review 5)

- HIS-02 (HIS 80, 90): offsets 1 and 5 are two independent receive tuning words (MEDIUM);
  offset 19 most likely the transmit tuning word (HYPOTHESIS).
- Spec change: the three "TBD (HIS-02)" places for the number of DDCs (B-part port list,
  discovery byte 20, record field `num_ddc`) and part C `TRX_COUNT` now read "1 in this
  version (0004)". A second receiver is possible but unconfirmed; offering it would be a later
  decision (one more socket, 0004 "Against").
- Pending hardware: mapping of offsets 1/5 to physical receive paths; offset 19 as the transmit
  word. TX stays disabled until measured (unchanged).

## 9. Addendum: `d199d33` (gate review 6)

- HIS 40: about 17 MHz SPI is the value seen in use, not a datasheet limit (MEDIUM): spec B6
  and P-NET-1 reworded. Whether the CH394 reports a datagram's destination address is not
  known: new P-NET-3, cited in B3.9 and T-B3-32.
- HIS 85: ATU strobes inactive high (MEDIUM), driven inactive and never pulsed: A3.9b and the
  P-ATU-1 placeholder say so. The latches stay unwritten.
- HIS 10 (part, supply, AXI SRAM, HSE, state at start) and the CH394 reset window: firmware
  detail; no spec change.

## 10. Addendum: `6e2208a` (gate review 7)

- HIS 40 confirms the datasheet SPI maximum, 62 MHz in mode 0 or 3 (VERIFIED), which the spec
  already cited as [CH394]; 17 MHz seen in use stays the safe bound (P-NET-1). P-NET-1 now cites
  both. No other spec change.
