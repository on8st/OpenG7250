# Open questions for the hardware-interface spec (HIS)

Questions that only the hardware-interface spec (`g7250-his`, now `his/` in the umbrella repository) can answer. The HIS was cleared by gate review 2 on 2026-11-20 and answered at commit `2a38b7f`; the answers to the follow-up questions passed gate review 3 at `d0a39b0`, the HIS-02 frame layout gate review 4 at `b95f519`, the HIS-02 word roles gate review 5 at `9d10e72`, the answers to the firmware's questions gate review 6 at `d199d33`, the CH394 SPI maximum gate review 7 at `6e2208a`, the FPGA bus cycle gate review 8 at `5292ba8`, and the AXI SRAM reset item gate review 9 at `0677c2d`. Each question below carries its status; "pending hardware" means only the radio can settle it (HIS 90). This repository
does not look for the answers itself (AGENTS.md rules 1 and 7). Each question says why we
need it, which decision or milestone waits for it, and what public sources already say.

Hardware observations by ON8ST are noted where he gave them. They count as provisional until
the HIS confirms them.

Priority: **M1** = needed for milestone M1 (discovery, capability block, RX-IQ);
**M2–M5** = needed for that milestone; **G6** = needed for the level-2 feasibility gate.

## Receiver and sampling

### HIS-01 — ADC and sample clock (M1)

- **HIS answer** ([HIS] `2a38b7f`): partly (HIS 80); see `analysis/his-2a38b7f-impact.md` §2.
- Question: which ADC is used, at what sample clock, and which decimated IQ sample rates can
  the main unit produce?
- Why: P2 DDC sample rates are 48/96/192/384/768/1536 ksps [P2]. The network side limits
  realistic rates to 384 ksps or lower (`analysis/ch394-network-budget.md`). Decision 0004
  (streams and sample rates) needs the hardware side.
- Known: "RF direct sampling" [G7250-MAN]. No ADC type or rate is published.
- Added by 0006: can the ADC chain give a peak magnitude per period (for P2 ADC max
  magnitude)?

### HIS-02 — Number of receivers (M1)

- **HIS answer** ([HIS] `9d10e72`, gate review 5): partly (HIS 80, 90): offsets 1 and 5 each set a receive-path tuning frequency, independently (MEDIUM); offset 19 is a third tuning word, most likely transmit (HYPOTHESIS); offsets HIGH. Which physical receive path offsets 1/5 drive, and the transmit role of offset 19, need the radio. The spec offers one DDC in this version (0004); a second DDC is a later decision. Pending hardware.
- **Update** ([HIS] `5292ba8`, gate review 8): bus cycle (STB/CMD active-high, MEDIUM; read turnaround rule), 0x04 samples big-endian 24-bit, six strobes per sample (MEDIUM), I/Q order unknown; which word is DDC0 unknown; byte 0 and bytes 9–18 of 0x01 unknown, so DDC0 cannot run until they are understood (P-RX-2; 0042).
- Question: how many independent receive channels (DDCs) can the main unit run at the same
  time?
- Added by 0060: does the factory calibration (24Cxx EEPROM at I²C1 0x50, HIS 15 rule 3) hold
  the values the factory firmware writes into bytes 0 and 9–18 of the 0x01 frame, and if so
  where and in what layout? Until answered, P-RX-2 is taken from a capture of the frames on
  PE0–PE10 (HIS 90).
- Why: P2 reports the number of DDCs in the discovery reply [P2]. Each DDC adds one I&Q
  socket and its bandwidth (`analysis/ch394-network-budget.md`).
- Known: "one receiver" comes from the start brief (0001) and is not stated in
  [G7250-MAN].

### HIS-03 — Second ADC or TX feedback path (M4)

- **HIS answer** ([HIS] `2a38b7f`): partly (HIS 85); see `analysis/his-2a38b7f-impact.md` §2.
- Question: is there a second ADC, or a path that feeds the transmitted signal back to the
  receiver?
- Why: only then are P2 features such as synchronous DDCs and PureSignal of any use
  (`analysis/p1-vs-p2.md`).

### HIS-04 — DSP clock and frequency word (M1)

- **HIS answer** ([HIS] `2a38b7f`): answered (MEDIUM) (HIS 80); see `analysis/his-2a38b7f-impact.md` §2.
- Question: what is the DSP clock from which receive and transmit frequencies are derived,
  and can the hardware take a frequency in Hz or does it need a phase word?
- Why: P2 lets the radio declare "frequency or phase word" and its DSP clock in the
  discovery reply [P2].
- Updated by 0022 §81: the wire format is fixed (phase word on a virtual 122.88 MHz clock);
  this question now covers only the internal DSP clock the firmware converts from.

### HIS-05 — Frequency reference (M2)

- **HIS answer** ([HIS] `2a38b7f`): partly (HIS 30); see `analysis/his-2a38b7f-impact.md` §2.
- Question: what is the reference oscillator, and can it be calibrated or locked to an
  external reference?
- Why: P2 has a "10 MHz reference PLL locked" status bit and reference-source settings [P2].
- Known: ±1.5 ppm 10 minutes after power-on, 1 ppm/hour at 25 °C [G7250-MAN].

## RF control

### HIS-06 — Antenna switching (M2)

- **HIS answer** ([HIS] `d0a39b0`): partly (HIS 85): two rear ports ANT1/ANT2 (SO-239, 50 Ω, VERIFIED); selection most likely bits 4–5 of the PD13 ATU latch (MEDIUM), no known safe value, latch left unwritten until confirmed. Pending hardware.
- Question: how are ANT1 and ANT2 selected, and what is the state after power-on and after
  a fault?
- Why: G7250 register map (0005); safe receive default state for a host that has not armed
  TX (0015).
- Known: ANT1 and ANT2 on the main unit, selectable in the menu "ANT SELECT"
  [G7250-MAN].

### HIS-07 — Filter switching and front end (M2)

- **HIS answer** ([HIS] `2a38b7f`): partly (HIS 85); see `analysis/his-2a38b7f-impact.md` §2.
- Question: which band-pass and low-pass filters, preamplifier and attenuator exist, how are
  they switched, and what does the main unit do automatically per frequency?
- Why: G7250 register map (0005); which standard P2 fields (step attenuator, Alex words)
  might fit with identical meaning (0005); safe RX default (0015).
- Added by 0022 §1: the filter plan: which TX frequencies share one low-pass filter and relay
  state (decides which retunes while keyed are allowed without unkeying).
- Added by 0022 §23: relay settle time `t_settle` and the RF-gone guard time `t_rf_off`
  (or forward-power threshold `P_rf_off`) before any relay moves after unkey.
- Added by 0022 §57: `t_rf_off` + `t_settle` must not exceed 98.75 ms, so that the DUC
  underrun time fits between the key latency + 1.25 ms and the 100 ms minimum keyed timeout.
- Added by 0022 §83 item 2: may a relay be commanded again before it has settled (needed
  to go straight from one TX state to another without the return to RX)?

### HIS-08 — Internal ATU (M2)

- **HIS answer** ([HIS] `2a38b7f`): partly (HIS 85); see `analysis/his-2a38b7f-impact.md` §2.
- **Update** ([HIS] `d199d33`): ATU strobes PD13, PG2, PG5 inactive high (MEDIUM): drive to the inactive level, never pulse; board pulls unknown.
- Question: how is the internal tuner switched in and out, how is tuning started, what does
  it report (busy, success, failure, SWR), and where is the per-band tuning data stored?
- Why: G7250 register map (0005); interaction with TX arm and PTT (0015, 0007).
- Known: 20–150 Ω, initial tuning ≤ 10 s, memory recall ≤ 0.1 s, data stored per band;
  after tuning the radio "automatically returns to receive status", so tuning transmits
  [G7250-MAN].
- Added by 0022 §22, §28: the tune power of an ATU tune cycle, and whether ATU memory recall
  moves L/C relays (it is treated as an RF-path change).

### HIS-09 — External interfaces on the main unit (M2)

- **HIS answer** ([HIS] `2a38b7f`): partly (HIS 85); see `analysis/his-2a38b7f-impact.md` §2.
- Question: can the firmware control or read the TUNER (13.8 V out, return, trigger), SEND
  (transmit control) and ALC (output) interfaces, and how?
- Why: candidates for the G7250 register map (0005) and the safety core (0007): SEND keys
  external equipment, so it belongs under the TX arm.
- Known: these connectors and their pins [G7250-MAN].

## Transmit and safety
- Added by 0007: is SEND an output only (keying external equipment) or also an input?

### HIS-10 — PTT and TX interlock (M3, M4)

- **HIS answer** ([HIS] `2a38b7f`): partly (HIS 85); see `analysis/his-2a38b7f-impact.md` §2.
- Question: where do PTT and the TX interlock sit in hardware, which sources can key the
  transmitter (network, controller microphone PTT, KEY input, SEND), and can the firmware
  block each of them?
- Why: TX arm (0015), watchdog and safety (0007), single owner (0011). The arm only works if
  the firmware is the only path to the PA, or can block every other path.
- Added by 0007: can the controller key the PA on its own, without the main-unit firmware?

### HIS-11 — Sensors (M2, M3)

- **HIS answer** ([HIS] `2a38b7f`): partly (HIS 85); see `analysis/his-2a38b7f-impact.md` §2.
- Question: which sensors exist and how are they read: forward and reflected power, SWR,
  PA temperature, supply voltage and current, PA current? With what resolution and update
  rate?
- Why: P2 High Priority status carries forward/reverse power and supply voltage, sent every
  1 ms during TX [P2]; telemetry (0006) and SWR/temperature limits (0007).
- Known: supply 13.8 V ±15 %, TX current ≤ 23 A [G7250-MAN].
- Added by 0006: where does the power coupler sit relative to the ATU; is there a separate
  exciter detector; does per-unit calibration data exist in the radio, and where; what is the
  fastest rate at which each sensor can be read?
- Added by 0007: what plausibility check is possible per sensor (to detect a failed sensor)?
- Added by 0022 §7, §17: is there a PEP detector, or only an average detector? What filter
  time constant does the PA temperature sensor need?
- Added by 0022 §23: blanking time `t_blank` after unkey for the "RF while not keyed" check.
- Added by 0022 §43: if the detector gives no PEP, the conservative SSB mean limit
  `P_ssb_mean` (start value 25 W); whether envelope samples exist for the AM carrier estimate.
- Added by 0022 §62, §68: the margin needed on the digital crest-factor PEP estimate
  (analog PA compression and detector errors), and that the "RF while not keyed" threshold
  can be set at or below `P_rf_off` with `t_blank` < `t_reset_wait`.
- Added by 0022 §80: detector and sensor response times, which add to the reaction times
  in Table A3-2.
- Added by 0022 §84: the shortest CW element gap in which the "RF while not keyed" check
  can still run, given `t_blank`.
- Added by 0022 §88: the forward power with all-zero DUC samples (carrier leakage); the
  "RF while not keyed" threshold must lie above it.

### HIS-12 — Power control and limits (M4)

- **HIS answer** ([HIS] `2a38b7f`): partly (HIS 85); see `analysis/his-2a38b7f-impact.md` §2.
- Question: how is output power set (drive level, PA bias), what are the limits per band and
  per mode, and is there a hardware power or SWR fold-back?
- Why: P2 drive level 0–255 [P2]; limits in the safety core (0011, 0007).
- Known: SSB/CW/NFM 100 W, AM 25 W [G7250-MAN]; 50 MHz power not stated in the manual.
- Added by 0022 §7: the fold-back power caps `P_swr` (high SWR) and `P_temp` (high
  temperature), the minimum forward power `P_min` for SWR evaluation, and the hystereses.
- Added by 0022 §23: the forward-power threshold `P_rf_off` below which RF counts as gone.
- Added by 0022 §59: `t_reset_wait` (2 s) must exceed `t_rf_off`.
- Added by 0022 §80: whether the PA tolerates the Table A3-2 reaction times (≤ 10 ms for
  SWR, voltage and current).

### HIS-13 — Fault and reset behaviour (M3)

- **HIS answer** ([HIS] `2a38b7f`): partly (HIS 85); see `analysis/his-2a38b7f-impact.md` §2.
- **Update** ([HIS] `0677c2d`, gate review 9): whether AXI SRAM content (the firmware's region, then 0x2401FFE0–0x2401FFFF; since 2026-11-21 the persist block is 0x2401FF80–0x2401FFFF, 128 bytes, HIS update to follow) survives a software or watchdog reset through the stage before 0x08020000 is UNKNOWN; measurement in HIS 90. Spec: P-SYS-9 (0050).
- **Update** ([HIS] `72591aa`): a system reset does not clear SRAM; AXI SRAM has a 64-bit ECC word, so whole aligned 64-bit words are written (both MEDIUM, ST citations not yet checked); whether the earlier stage writes the region stays not known. A surviving pattern shows both retention and that the earlier stage leaves the range alone; a lost one does not tell which. P-SYS-9 unchanged.
- Question: what does the hardware do on a processor reset, brown-out or watchdog reset:
  does the PA drop out by itself, and in which state do relays and switches come up?
- Why: safe state after reset or fault (0001 spec section 9, 0007).

## Network
- Added by 0008: can the firmware read the cause of the last reset?
- Added by 0022 §27: can fault and reset counters be kept across a reset without power loss
  (RAM that survives a warm reset, or a reset-cause register)?

### HIS-14 — Ethernet controller and its connection (M1)

- **HIS answer** ([HIS] `2a38b7f`): partly (HIS 40); see `analysis/his-2a38b7f-impact.md` §2.
- **Update** ([HIS] `d199d33`, gate review 6): CH394 reset window 12–19 ms after RST# high (VERIFIED, CH394DS1); minimum RST# low and maximum SCK still to be taken from the datasheet; about 17 MHz seen in use, not a datasheet limit (MEDIUM); CHIPV at 0x0039; whether the chip reports a datagram's destination address is not known (P-NET-3).
- **Update** ([HIS] `6e2208a`, gate review 7): maximum SPI clock 62 MHz in mode 0 or 3 (CH394DS1 V1.2, VERIFIED); about 17 MHz seen in use stays MEDIUM and the safe bound until measured (P-NET-1). Open from F2-Q1: only the minimum RST# low time (firmware detail).
- Question: please confirm the WCH CH394Q. Which interface connects it to the processor
  (SPI), at what clock, and does the existing design use the chip's sockets or MACRAW mode?
- Why: the SPI clock caps all network throughput (at most 62 MHz [CH394]); the socket mode
  decides the 8-socket budget (`analysis/ch394-network-budget.md`, 0016, 0017).
- Known: LAN port on the main unit [G7250-MAN]; ON8ST saw a CH394Q during his own hardware
  inspection (2026-11-19).
- Added by 0007: does the CH394Q report link status (cable unplugged) to the processor?
- Added by 0008: is a DHCP client present in the current design? (The CH394 has none built
  in, per 0008.)
- Added by 0022 §60: the factory-default network configuration (C-1: DHCP or a fixed address).

### HIS-15 — MAC address (M1)

- **HIS answer** ([HIS] `d0a39b0`): partly (HIS 40): the CH394 provides the MAC address in its own register set; the factory firmware stores none in the EEPROM (HIGH). Uniqueness per unit not verified. Pending hardware.
- Question: where is the MAC address stored, and is it unique per unit?
- Why: the P2 discovery reply carries the MAC address [P2].
- Added by 0008: is there a unique serial number besides the MAC address?

### HIS-16 — Public sources on the network interface (M1)

- **HIS answer** ([HIS] `d0a39b0`): partly (HIS 15): the public vendor update guide is Radioddity's "Extended manual for Xiegu G7250 v1.0.1" in the V1.0.1 release package on radioddity.com/pages/xiegu-download. The network-reception sources are not answered.
- Question: which public sources document the network interface of the G7250? ON8ST
  mentions a video by the YouTube channel "Temporarily Offline", and [G7250-MAN] refers to a
  "G7250 HDSDR Network Reception Tutorial". Links, please.
- Why: to cite them in `sources.md` ([G7250-TOFF]).

## Main unit and controller

### HIS-17 — The CTRL link (M2, M4)

- **HIS answer** ([HIS] `2a38b7f`): answered (HIS 20, 50, 70); see `analysis/his-2a38b7f-impact.md` §2.
- **Update** ([HIS] `d0a39b0`): power-up without the Control needs hardware (HIS 20, HIS 90 observation T0-P2). Pending hardware.
- Question: what does the 15-pin CTRL cable between main unit and controller carry? In
  particular: can the main unit send audio to the controller's speaker, and can it receive
  microphone audio, PTT and Morse key from the controller? What happens on the main unit when
  the controller is disconnected?
- Why: P2 microphone data (port 1026) for TX from the host, a possible audio path later
  (0017), PTT sources (HIS-10), and whether the firmware must keep the controller working.
- Known: speaker, microphone, key, USB and Bluetooth are all on the controller; the main unit
  has no audio connector [G7250-MAN], (ON8ST, 0017).

## Compute and firmware
- Added by 0008: can the main unit read the controller's TF card or USB, show a confirmation
  on the controller's screen, or receive a button press from it (first-secret set-up,
  physical-presence confirmation, local factory reset)? Does the controller's NETWORK page
  write the main unit's network settings?
- Added by 0023: does the stock controller work at all with a different main-unit firmware
  (and if so, what does it need from the main unit)? What must the main unit do, or not do,
  on CTRL so that the controller is not left in an error state, and so that nothing it sends
  can damage the controller or keep it from starting with the stock main-unit firmware? How
  can the main unit detect that a controller is connected?
- Added on 2026-11-20 (question passed on by the openg7250 session for the emulator): what
  does the stock controller do when the main unit stays silent on CTRL?
- Added on 2026-11-20 (same source): does powering the radio on and off depend on the
  controller?

### HIS-18 — Processor, memory and logic (G6, M1)

- **HIS answer** ([HIS] `2a38b7f`): answered (HIS 10, 15); see `analysis/his-2a38b7f-impact.md` §2.
- **Update** ([HIS] `d199d33`): part STM32H725xG/H735xG (MEDIUM), package open; AXI SRAM 320 KiB as shipped (HIGH); core supply SMPS feeding the LDO (MEDIUM, write-once); HSE not recorded; state at application start unknown (HIS 10, 90).
- Question: which processor(s), DSP or FPGA are in the main unit, at what clock, with how
  much RAM and flash?
- Why: P2 is the heavier protocol (several ports, 24-bit TX IQ at 192 ksps [P2]); the
  WebSocket layer and TCI commands for level 2 run on the processor, not in the CH394Q
  (`analysis/ch394-network-budget.md`); gate G6 (0011).
- Added by 0008: flash size and layout (room for two images, or one plus staging), RAM for a
  log buffer, and whether an FPGA image is loaded by the main-unit firmware.
- Added by 0022 §62, §63: the delay from DUC samples to the coupler (for the crest-factor
  window), and the DUC FIFO capacity while keyed.
- Added by 0022 §88: the DUC-to-coupler delay, which `t_blank` must cover for the zero-sample
  gap check.
- Added by 0024: is there flash room for a retained stock image (main unit, and FPGA if
  separate) beside our two slots or slot plus staging? Is the FPGA image a separate image,
  and does the main-unit firmware load it? Where does the stock firmware keep its own
  settings, so that our firmware can leave them untouched?

### HIS-19 — Firmware loading and recovery (M5)

- **HIS answer** ([HIS] `2a38b7f`): answered, with unknowns (HIS 15); see `analysis/his-2a38b7f-impact.md` §2.
- **Update** ([HIS] `d0a39b0`): the F1 drill and its safe step order are in HIS 90 (SWD read-only first, backup only at RDP level 0). Pending hardware.
- Question: how is firmware loaded into the main unit, is there a boot loader that survives
  a failed update, and can an update be done over the network?
- Why: firmware update over the management channel (0001 spec section 10, 0008); P2 also has
  program packets on port 1024 [P2].
- Added by 0008: can the boot choice be switched atomically, and can the boot loader detect
  an unconfirmed trial boot?
- Added by 0024: is there a vendor update or recovery path (vendor boot loader, vendor
  update tool), how is it entered, can software select it, and does it survive the
  installation of our firmware? How is the first install (stock to new) done, and how the
  return (new to stock)? Can the boot choice select a stock image, and where must a stock
  image be written so that the radio starts it?
- Added by 0025: which interface does the stock boot loader, or the vendor's update, use to
  update the main unit? Does that path need the controller (TF card, USB, display, button)?
  What does the main unit need in order to accept a stock image? (Asked only as that; no
  detail of the vendor format is recorded on the product side.) Flash room for images and
  staging is HIS-18 (already asked by 0008 and 0024).

### HIS-20 — Source of randomness (M5)

- **HIS answer** ([HIS] `d0a39b0`): answered (HIS 10): both candidate Main parts (STM32H725xG, H735xG) have a true random-number generator per the ST datasheet (HIGH for present).
- Question: is there a hardware random-number generator or an equivalent entropy source?
- Why: challenges for the management channel (0008) and the per-session TX arm token (0007).

## Sources

- [P2], [CH394], [G7250-MAN] and the other keys: see `sources.md`.
- Decisions 0001, 0004–0008 (reserved where not yet written), 0011, 0015, 0016, 0017.
- `analysis/p1-vs-p2.md`, `analysis/ch394-network-budget.md`.
