# re/emu — handover

The G7250 emulator and test harness. Imported with its full history from the
stand-alone `g7250-emu` repository (historical name) at tag **emu-v1** (commit
d64f0d4) into `re/emu/`, then extended on branch `emu/integrate` (PR #3).
Historical repository names are explained in `docs/REPO_LAYOUT.md`, "Historical
names".

## State (2026-11-20)

Consolidated gate `tests/baseline/acceptance_ae.sh`, expected result:
**A–E ACCEPTANCE: ALL PASS (with 1 known failure(s))**, the known failure
being D2b. One targeted check per item:

- **A** the stock Main reaches its **superloop**: "Init Radio Server" plus at
  least 128 IWDG refreshes after it (`harness/g7250emu/superloop_run.py`). This
  needs the MDMA model and the PA bias plant (below); without them the test
  fails. A1–A4 are the earlier boot milestones (System started .. RX mode);
  A5 the forced default IP 192.168.3.199:50001.
- **B1** functional EEPROM (settings load); **B2** functional CH394 on SPI6,
  chip-side SIPR = 192.168.3.199 (A1.12).
- **C/C2** the Control boots to its GUI service loop (GT911 touch, codec,
  DMA2D) via the LTDC CDSR stub.
- **D1** two-machine SLIP (USART2↔USART2) transmits 0xC0 frames and the Main
  receives them through its DMA ring.
- **D2a** FPGA boot bus check: "Check complete!", version 1.0.0, no bus errors
  before it (`tests/baseline/fpga_boot.sh`).
- **D2b** FPGA runtime health in the superloop: **known failure** (KFAIL, with
  counts; XPASS if it ever passes). See "Open items".
- **E1** dual-signal hang verdict; **E2** PW-1 PowerModel confirmation
  (`r0_confirm.py`); **E3** boot-milestone test; **E4** HIS 00–80 present.

## Models added after emu-v1

| Model | File | Doc | Why |
| --- | --- | --- | --- |
| MDMA (software request, polled) | `models/csharp/G7250Mdma.cs` | `docs/MDMA-model.md` | Renode has no MDMA; the DSP's buffer copies timed out and the boot crawled |
| PA bias plant and ADC1 metering feed | `models/csharp/G7250PaPlant.cs`, `platforms/g7250_main_pa_plant.repl` | `docs/PA-plant-model.md` | The boot-time bias trim needs a drain current that responds to the bias DAC; Renode's ADC/circular DMA path returns no usable values |
| DMA1/DMA2 (generic, circular) | `models/csharp/G7250Dma.cs`, `platforms/g7250_main_dma.repl` | `docs/DMA-model.md` | Renode 1.17's DMA has no circular mode |
| USART2 (generic, DMA requests) | `models/csharp/G7250Usart.cs`, `platforms/g7250_main_usart2.repl` | `docs/USART-model.md` | Renode's USART raises no DMA requests, so the Main received no SLIP bytes; replaces the earlier RX ring shim (removed) |

`m1_run.py` loads the MDMA, PA, DMA and USART2 models for every Main boot.
The product test bench (`docs/PRODUCT-TESTBENCH-PLAN.md`, branch `emu/bench`)
uses the generic ones only: `harness/g7250emu/product_run.py`, own self-test
programs in `firmware-tests/bench/`.

## HIS work done with the emulator

- **HIS-02, FPGA 0x01 frame** (`harness/g7250emu/fpga_nco_probe.py`): run 5,
  with the RX shim, told the three tuning words apart (offsets 1 and 5:
  receive-path tuning; offset 19: a third tuning word, most likely transmit).
  RE note: `re/analysis/docs/his-02-nco-probe.md`.
- The HIS (`his/`, PR #4, branch `his/integrate`) is **cleared at 9d10e72 by
  gate review 5** (`re/analysis/docs/his-gate-review-5.md`). Changes not yet
  checked by the gate stay on the local branch `his/next`.

## How to run the tests (from `re/emu/`)

Prereqs (all local and git-ignored, see "Clean-room / policy"):
- `tools/renode-1.17.0/`: the pinned Renode portable (osx-arm64).
- Decrypted stock images outside any repository; point to them with
  `image_map = /abs/dir` in `local.config`, and put their hashes in
  `local.config` as `image_sha256.main = <hex>` / `image_sha256.control = <hex>`.
  The tracked `images/manifest.yaml` stays blank. **Never commit that hash.**
- `his_dir = /abs/path/to/his` in `local.config` (or env `G7250_HIS`) for E4,
  pointing at the `his/` folder of a worktree that has it. Not needed once
  `his/` is in the same tree.

Commands:
- Fast gate, no image needed:            `bash tests/baseline/m1_smoke.sh`
- Full A–E acceptance (needs image-map): `bash tests/baseline/acceptance_ae.sh`
- Superloop test A:                      `python3 harness/g7250emu/superloop_run.py`
- HIS-02 probe:                          `python3 harness/g7250emu/fpga_nco_probe.py`
- Main boot:                             `python3 harness/g7250emu/m1_run.py` (`--fpga` for the FPGA model)
- Control boot:                          `python3 harness/g7250emu/control_run.py`
- Two-machine SLIP:                      `python3 harness/g7250emu/slip_run.py`

All image-driven scripts refuse cleanly (exit 2) without an image-map. Output
goes to `.private/`.

## Open items

- **D2b FPGA runtime health — known failure.** In the superloop the firmware
  logs "[ERR] FPGA bus error" and "FPGA buffer underrun", because the fused FPGA
  model delivers counter data, not realistic RX I/Q at the real block rate. It
  becomes a normal check once the model delivers realistic RX data.
- **Hardware-only questions.** Everything the emulator cannot answer (real bias
  codes and plant values, ADC scan rate, the FPGA's use of the tuning words, TX
  and XIT behaviour, and the items in `his/90-needs-hardware.md`) needs
  measurements on a radio.
- The model and platform source comments still cite "g7250-base `<path>`": these
  refer to the former data pin and are explained in `deps/re-analysis.lock` and
  `docs/REPO_LAYOUT.md`.

## Clean-room / policy (must stay true)

- The emulator does no decryption; decrypted images live outside any repository.
- `images/manifest.yaml` never carries a decrypted-image hash; that hash lives
  only in the git-ignored `local.config`.
- `tools/leak-check.sh` (umbrella) must pass; exceptions go in
  `tools/leak-allow.txt` with a reason.
- Local commits only; no push, PR, issue or mail without an explicit go from the
  repository owner (umbrella AGENTS.md rule 4).
- Hardware facts reach the product side only through the HIS after review at
  the gate.

## Key artifacts

- `harness/g7250emu`: bootcore, m1_run (`--fpga`), superloop_run,
  fpga_nco_probe, control_run, slip_run, r0_confirm, observe (strict / hang /
  heatmap), imagemap, platform_extras.
- `models/csharp`: G7250Slaves (EEPROM, CH394), G7250Gpioe (fused GPIOE+FPGA),
  G7250Mdma, G7250PaPlant, G7250Dma, G7250Usart; bench only: G7250BenchReq.
- `models/devices`, `models/system`: host-tested device, plant and power logic.
- `platforms/*.repl`: g7250_main, g7250_control, the I2C/CH394/FPGA/PA
  overlays, SLIP.
- `tests/baseline`: m1_smoke (incl. the bench self-tests), boot_milestone,
  fpga_boot (D2a), acceptance_ae.
- `docs`: EMU-JOB-STATUS, M1-status, M2-control-status, M8c-R0-report,
  D2-action-plan, FPGA-model-notes, the three model docs above.

## Continuation

Maintained by the `openg7250-emu` session on request of the RE session
(`openg7250`).
