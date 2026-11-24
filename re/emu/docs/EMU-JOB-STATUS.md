# Emulator job — action-list status (A–E)

Status of the A–E action list after the C#-peripheral unlock (runtime-compiled
custom slaves register when referenced by fully-qualified type name; inherited
C# models can be swapped via `sysbus Unregister`). ✅ done · ◐ partial · ⏳ later.

## A. Main boot to the superloop — ✅ DONE (M1, superloop 2026-11-20)
Test A now requires the superloop (see "Update 2026-11-20"); the milestones
below are kept as A1–A4. Boots `System started` → AXP PSU → EEPROM → VersaClock → LAN default IP
192.168.3.199:50001 → codec → TRX RX-mode → service loop; 60 s VT, no hang.
A1 PWR 0x08 ready ✅ · A2 chain chased ✅ · A3 FPGA errors tolerated (→ D2) ·
A4 60 s no hang ✅ · A5 A1.12 default IP ✅. (docs/M1-status.md,
tests/baseline/boot_milestone.sh.)

## B. Realistic I/O — ✅ DONE (B1 + B2)
- B1 ✅ functional EEPROM (models/csharp/G7250Slaves.cs `G7250Eeprom24C`,
  compiled in at runtime): the Main logs "[OK] Settings data successfully loaded
  from EEPROM". Wired by default (g7250_main_i2c1_func.repl).
- B2 ✅ functional CH394 (`G7250Ch394`, ISPIPeripheral) on spi6 with its CS#
  (PG15) wired so W5500 register frames are delimited by the chip-select edge.
  The model logs **"source IP set to 192.168.3.199"** when the firmware programs
  SIPR — the chip-side A1.12 confirmation. Wired into m1_run by default.

## C. Control boot — ✅ DONE (M2/C2)
Boots through SLIP client, AIC3204 codec, FatFS mount attempt, **GT911 touch at
0x14**, all service timers, then — via the `G7250LtdcStub` CDSR VSYNC shim
swapped in for the headless-silent C# LTDC — PAST the display wait into its
**GUI service loop** (DMA2D rendering + panel GPIO + SDMMC + input), verdict
"running". The SDMMC IntStatus polling is the normal no-card path (no SD card
inserted is a valid real state); it does not block the GUI loop. CLOSED.
(docs/M2-control-status.md.)

## D. Two machines — ✅ D1, ✅ D2a, ◐ D2b known-fail (see Open items)
- D1 ✅ harness: harness/g7250emu/slip_run.py boots both MCUs in one emulation,
  wires USART2↔USART2 through a UART hub, and SLIP bytes (0xC0-delimited) are
  transmitted over the link. RX on the real firmware is by circular DMA1 S3 (by
  design), so it is not register-visible to the access log — that is a property
  of the observe tool, not of the link; the SLIP harness itself is complete. CLOSED.
- D2 ✅ FPGA fused model: models/csharp/G7250Gpioe.cs (`G7250GpioeFpga`) is a
  from-scratch GPIOE port that also answers the bit-banged FPGA bus internally
  (D0-7/CMD/STB on PE0-9, DRDY on PE10 via a managed thread). Handles the loopback
  bus-test (cmd 0x00), the version read (0x08 -> 1.0.0) and DRDY-paced 24-bit RX
  I/Q. The stock Main now logs "Check FPGA BUS... Check complete!" with ZERO FPGA
  bus errors and reaches RX mode. Wired as `m1_run.py --fpga`; regression
  tests/baseline/fpga_boot.sh. Root-cause of the long struggle: a BSRR set/reset
  ordering bug (SET must win) + classify_tail mis-reading the DRDY EXTI servicing
  as a poll-hang; both fixed.

## E. Finishing / hardening
- E1 ✅ dual-signal hang verdict (MMIO poll-hang + executed-frozen WFI idle).
- E2 ✅ PW-1 emulator confirmation (r0_confirm.py): Main matches the prediction.
- E3 ✅ boot-milestone regression (tests/baseline/boot_milestone.sh).
  It runs until all 7 milestones are seen, then 10 s more (`m1_run.py --until-milestones`);
  120 s is the hard limit, so a hang still fails. It prints each milestone's wall time and the
  margin, and keeps the step's files (log gzipped) in `.private/m1/boot-runs/<UTC>`. Until
  2026-11-24 it had a fixed 35 s budget, which a slower host could run out of at a clean load.
- E4 ✅ HIS started, then as the separate `g7250-his` repo (historical; now
  `his/` in the umbrella, PR #4): README (role +
  clean-room process) and HIS sections 00–80 populated from the emulator-proven
  behaviour + public RE (reset/clock, power, I2C, ethernet, audio, display/input,
  SLIP interlink, RF/FPGA), behaviour-only with evidence cited. All eight HIS
  sections written (not a stub). CLOSED.

## Update 2026-11-20 (after emu-v1)
- A now means the superloop: with the MDMA model and the PA bias plant
  (docs/MDMA-model.md, docs/PA-plant-model.md) the stock Main completes its bias
  trim and reaches its superloop. Test: harness/g7250emu/superloop_run.py
  ("Init Radio Server" plus >= 128 IWDG refreshes after it). The earlier A check
  only reached "Start RX mode", which comes before the FPGA init.
- D2 is split: D2a FPGA boot bus check (normal check, tests/baseline/fpga_boot.sh)
  and D2b FPGA runtime health (known failure, see below). acceptance_ae.sh ends
  with "ALL PASS (with 1 known failure(s))".
- USART2 RX shim (docs/USART2-RX-shim.md): the Main receives SLIP bytes from a
  socket terminal and applies every settings image.
- HIS-02 probe (harness/g7250emu/fpga_nco_probe.py), run 5 with the shim: the
  three tuning words of the FPGA 0x01 frame told apart (RE note
  re/analysis/docs/his-02-nco-probe.md). The HIS (his/, PR #4) is cleared at
  9d10e72 by gate review 5.
- PR #1 scaffold merged into emu/integrate; tools/leak-check.sh passes.
- Historical repository names marked; see docs/REPO_LAYOUT.md, "Historical
  names".

## Update 2026-11-21 (product test bench, branch emu/bench)
- Step 1a: generic DMA1/DMA2 with circular mode (docs/DMA-model.md) for every
  Main run. Step 1b: Renode's DMAMUX1 kept, routing verified by own self-test.
- Step 1e: generic USART2 with DMA requests (docs/USART-model.md); the RX ring
  shim is removed. The HIS-02 probe applies its settings through the real path
  (same word roles as run 5); D1 now shows the Main receiving.
- Step 1c: generic ADC1 (docs/ADC-model.md) for bench and product runs.
- Step 2 (PA): the PA plant works at bus and pin level and drives the ADC1
  inputs; the stock metering feed into a fixed RAM buffer is removed. The stock
  Main still trims all three bias stages and reaches its superloop.
- Step 1d: generic MDMA. Step 2 (FPGA): GPIOE + FPGA from his/80 only.
- Step 3: product runner against g7250-testlist v1.3 (docs/PRODUCT-RUNNER.md):
  CH394 with UDP sockets and a network bridge (address mapping, pcapng), bench
  agent (hw.observe, hw.trace, hw.input), power.cycle, ctrl.set; self-check with
  a stand-in bench in m1_smoke [7]. Found: SPI6 data by BDMA is not delivered
  (step 1g).
- Runner skeleton harness/g7250emu/product_run.py and own self-test programs in
  firmware-tests/bench (smoke, dma_test, usart_test, adc_test), run in m1_smoke [6].

## Open items (PR #3, before leaving draft)
- **D2b FPGA runtime health — KNOWN-FAIL.** In the superloop the firmware's
  runtime health check logs "[ERR] FPGA bus error" (first RX sample unchanged)
  and "FPGA buffer underrun" (unconsumed RX block). Cause: the fused FPGA model
  delivers counter data, not realistic RX I/Q at the right block rate.
  acceptance_ae.sh reports it as KFAIL with the counts, without failing the run,
  and as XPASS if it passes. It becomes a normal check once the model delivers
  realistic RX data.
- ~~USART2 RX on the Main~~: closed by step 1e (generic USART2 + DMA); D1 now
  shows the Main receiving.
- **Hardware-only questions.** Real bias codes and plant values, the ADC scan
  rate, what the FPGA does with the tuning words, TX/XIT behaviour, and the items
  in his/90-needs-hardware.md cannot be answered in the emulator.

## Not in scope (policy)
No decryption; no push/PR/mail without an explicit go from the repository owner
(umbrella AGENTS.md rule 4). Decrypted images stay outside any repo; the
manifest hash stays local/untracked.
