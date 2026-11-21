# Plan: the emulator as test bench for the product firmware

Status: in progress on branch `emu/bench` (see "Progress" at the end). Implements proposed decision 0007
(`decisions/0007-emulator-as-test-bench.md`, on the umbrella branch).

## Goal

Every product firmware image (Rust, Embassy / `embassy-stm32`) is tested in
full on the emulator, as a black box, before it runs on the radio. One command
takes the image (ELF) and a test list and writes a result file: pass/fail per
test, plus the image's own console and network output. Nothing from the
emulator's internals goes back. A failure caused by a missing model reads
"not modelled: `<peripheral>/<register>`", with no stock usage detail.

## Rules for the models

- Models follow the hardware: ST RM0468, the device datasheets and the cleared
  HIS (`his/`). They do not follow how the stock image uses a block. No fixed
  stock buffer addresses, no stock-only sequences in anything a product image
  depends on.
- Shims that exist only for the stock image (the metering feed into a fixed
  RAM buffer, the USART2 RX ring shim, the polled-only MDMA shortcut) are
  removed, or kept only behind an explicit stock-only switch that the product
  runner never sets.
- Regression after every step: `tests/baseline/m1_smoke.sh` and
  `tests/baseline/acceptance_ae.sh` (expected "ALL PASS (with 1 known
  failure(s))", D2b) stay green. The stock image is the model regression.
- Emulator passes are model consistency, not hardware evidence (0007,
  `his/90-needs-hardware.md`). Transmit stays disabled.

## What Renode 1.17 lacks (seen in this emulator)

| Block | Renode 1.17 model | Gap |
| --- | --- | --- |
| DMA1/DMA2 | `DMA.STM32DMA` | No circular mode: after one transfer the CIRC bit is gone (CR read 0x2C01) and NDTR stays 0; no NDTR reload, so no ring buffer; half-transfer behaviour unverified. Peripheral requests only arrive where a source model raises them. |
| DMAMUX1 | `Miscellaneous.STM32_DMAMUX` (16 outputs to DMA1/DMA2 streams) | Present; routing to be verified per request line with the new DMA model. Request generators unverified. |
| ADC1/2 | `Analog.STM32H7_ADC` | Wrong raw values (DR read 2 for a set channel voltage); sequence, scan and DMA request behaviour not usable as is. |
| MDMA | none | Not modelled at all (our model only does polled software-request transfers). |
| USART2 RX | `UART.STM32F7_USART` | Raises no DMA request on RXNE with DMAR set, so a DMA ring never fills. |
| Timers | `Timers.STM32_Timer` | Not yet exercised the way an Embassy time driver uses them (free-running counter + compare interrupts + overflow); to be checked in step 3. |
| PWR | Python peripheral | Enough for the stock boot; the supply-configuration and voltage-scaling handshakes `embassy-stm32` waits on must be checked. |

## Steps

Effort is in working sessions of this emulator session (rough, from the work
so far: the MDMA, PA and shim models took about one session each).

### 1. Generic DMA, DMAMUX, ADC, MDMA, USART2 RX — about 5–7 sessions

1a. **DMA1/DMA2** (own C# model replacing `STM32DMA` via `sysbus Unregister`,
the pattern already used for the LTDC and GPIOE): 8 streams each, all stream
registers per RM0468; request input per stream (GPIO from DMAMUX); peripheral-
to-memory, memory-to-peripheral, memory-to-memory; data sizes and increments;
**circular mode with NDTR reload**; double buffer (DBM/CT); HTIF/TCIF/TEIF and
the interrupt enables to the NVIC lines from the base platform; FIFO mode
modelled as direct transfers (timing not modelled, documented). Unit test: a
small own test ELF (C, arm-none-eabi-gcc) that sets up a circular transfer and
checks NDTR, HT/TC flags and interrupts.

1b. **DMAMUX1**: verify the existing model routes request inputs to the
right stream for the requests we need (USART2 RX/TX, ADC1, SPI); replace it
only if it does not.

1c. **ADC1** (own model): channels as analog inputs settable from the harness
or a plant model (volts → 12/16-bit raw per resolution and reference),
regular sequence (SQR), scan and continuous modes, software and timer
triggers, EOC/EOS flags and interrupt, DMA request in one-shot and circular
mode (DMNGT). Pins from `his/85-rf-control.md` ("ADC1 metering inputs").

1d. **MDMA**: extend the model to the request types the hardware has
(buffer, block, repeated block, linked list), with CTCIF/BTIF/BRTIF/CTEIF
interrupts. Software request stays the first case; hardware triggers are
added when a product image uses them ("not modelled" until then).

1e. **USART2 RX through the DMA**: a USART wrapper or own model that raises
the DMA request on RXNE when DMAR is set (and TXE for DMAT). Then the RX ring
shim goes, and D1 (two-machine SLIP) receives on the Main for real — which
also closes the D1 open item.

1f. **Remove stock-only shims**: the stock image then gets its metering from
the ADC1 model + PA plant (step 2) through the real DMA, and its SLIP RX
through 1e. Whatever cannot be made generic stays behind `--stock-shims`
(stock runs only), documented.

Regression after each of 1a–1f.

### 2. FPGA and PA models at bus and pin level — about 3–4 sessions

- **FPGA**: the fused GPIOE+FPGA model reworked so its behaviour follows
  `his/80-rf-fpga.md` only (bus pins, command bytes, frame lengths, DRDY):
  command decode, version and status reads, and an RX I/Q source with a
  configurable block rate. Tuning words are recorded and exposed to the test
  harness; nothing depends on stock sequences. This is also the path to D2b
  (realistic RX data).
- **PA**: the bias DAC as a generic SPI device per the BU2506 datasheet and
  `his/85-rf-control.md` (word format, LD, RESET); a plant that turns bias
  codes into a drain current on the Idd ADC pin, plus supply voltage and NTC
  pins, all through the ADC model of step 1c. The plant values stay
  HYPOTHESIS placeholders (documented in `docs/PA-plant-model.md`) until
  measured. TX and PTT outputs are observed only; any TX keying by an image
  is reported as a test failure.

### 3. The product runner — about 3–4 sessions

- `harness/g7250emu/product_run.py`: `product_run.py <image.elf> <tests>
  -o <result>`. Loads the ELF on the Main platform with generic models only
  (no stock image map, no stock shims), runs each test with its own time
  budget, writes one result file (JSON + a short text summary): per test
  pass/fail/"not modelled: …", the image's USART1 console, and its network
  output (CH394 / SLIP frames as bytes).
- Access to an unmodelled or tagged register is caught (Renode's unhandled-
  access log) and mapped to "not modelled: `<peripheral>/<register>`" from the
  RM0468 names, never to a stock address or stock usage.
- The test list format is defined with the product side later
  (`testbench/`); until then a minimal list: boot within N s, console line
  matches, no hard fault, no TX keying.
- **Smoke target**: `firmware-tests/bench-smoke/`, an own C program
  (arm-none-eabi-gcc, as `firmware-tests/m0/`): clock setup, a GPIO blink,
  "hello" on USART1, then a circular DMA RX echo on USART2 once step 1 is in.
  Test scaffolding, not product code.
- Embassy readiness: the first real product images will exercise the RCC/PWR
  start-up handshakes, the time driver (a timer with compare interrupts) and
  the DMA ring-buffered UART. These are checked with a small own test
  program in the same scaffold; a Rust toolchain is not installed on this
  machine (not needed for the bench itself, the product side delivers ELFs).

### 4. Regression and documentation — continuous

`m1_smoke.sh` + `acceptance_ae.sh` after every step; the bench smoke run is
added to `m1_smoke.sh` once the runner exists. Each model gets a doc with its
sources, what it models and what it does not.

## Order and total

1a → 1b → 1e (unblocks D1) → 1c → 2 (PA) → 1f → 1d → 2 (FPGA) → 3, with the
runner skeleton and the smoke ELF started early (after 1a) so every model step
also gets a product-style test. Total about 11–15 sessions.

## Open points for the RE session / ON8ST

- The test list format, together with the product side (`testbench/`).
- Whether a Rust toolchain may be installed here for own Embassy-shaped test
  programs (scaffolding only), or whether the product side provides small
  Embassy test ELFs.
- Where the result file goes for the product side (a path in `testbench/` or a
  CI artifact later, per 0007).

## Progress (branch emu/bench)

| Step | State | Commit |
| --- | --- | --- |
| 1a DMA1/DMA2 | done | e9b71e8 |
| 1b DMAMUX1 | done (Renode's model, routing verified) | e9b71e8 |
| Runner skeleton + smoke ELF | done (`--selftest` lists) | e9b71e8 |
| 1e USART2 RX/TX through the DMA | done; RX ring shim removed | 91de8ef |
| 1c ADC1 | done | fc15adf |
| 2 PA (bus/pin level) | done; stock metering feed removed | 9e53770 |
| 1f stock-only shims | done: none left in the models; the fused GPIOE+FPGA port is loaded only for stock runs (`--fpga`) and is replaced in step 2 FPGA | this table |
| 1d MDMA | done | e2b86f4 |
| 2 FPGA | done (his/80 only; stock regression at a reduced DRDY rate) | 594f400 |
| 3 product runner (g7250-testlist v1.3) | core, network bridge, fixtures hw.observe/trace/input, power.cycle, ctrl.set: done (`docs/PRODUCT-RUNNER.md`) | c1cacd7 |
| 3 reset.wdt (IWDG1 + RCC_RSR reset flags) | done | e059b2c |
| Boot path for product runs (bootloader stand-in, `--boot none`) | done | 95cfbe8 |
| 1g BDMA + DMAMUX2 + SPI DMA requests | done (SPI6; `docs/SPI-model.md`, `docs/BDMA-model.md`) | (this commit) |
| link.set, power.cut | after 1g | |
