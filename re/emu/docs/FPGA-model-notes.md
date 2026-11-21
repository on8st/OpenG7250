# FPGA model notes (D2 / M3)

## Since step 2 (FPGA), 2026-11-21: behaviour from his/80 only

`models/csharp/G7250Gpioe.cs` (`G7250GpioeFpga`) was rewritten so that what it
does follows `his/80-rf-fpga.md`, not a particular firmware:

- full GPIOE port (MODER, OTYPER, OSPEEDR, PUPDR, IDR, ODR, BSRR with set
  winning, LCKR, AFRL/AFRH); pin levels to EXTI;
- bus: D0–D7 = PE0–PE7 (direction from MODER), CMD = PE8, STB = PE9; a rising
  STB with CMD high latches a command, with CMD low it moves one data byte;
- 0x00 loopback; 0x01 24-byte frame recorded (`FrameHex`, `NcoWord(offset)`,
  `RateIndex` from byte 23) and logged; 0x02 seven status bytes (settable);
  0x03 TX I/Q bytes counted (`TxBytes`); 0x04 RX I/Q, two signed 24-bit values
  per sample, 8/4/2/1 samples per DRDY by rate index, from a complex tone
  (settable); 0x08 version (settable, default 1.0.0); 0x05/0x06/0x09/0x0A
  counted (`ForbiddenCommands`) and logged as a warning;
- DRDY (PE10): a rising edge per sample block at `drdyRate` in virtual time,
  default 48 kHz (his/80). The stock regression (`platforms/g7250_main_fpga.repl`)
  uses 4 kHz for host speed; bench and product runs use
  `platforms/g7250_main_fpga_his.repl` with the his/80 rate.

HYPOTHESIS, documented: the byte order of the 24-bit values (big-endian, as
the 0x01 frame), the status byte layout, the RX source. Not modelled: strobe
timing and bus turnaround, the FPGA's use of the frame fields, TX.

Self-test: `firmware-tests/bench/fpga_test.c` (loopback, version, status,
frame, DRDY on EXTI10 with RX I/Q reads), 8 of 8 pass. With the stock image:
"Check complete!" and the superloop as before; D2b (runtime health) still
known-fail, with far fewer errors (1 bus error, 4 underruns in one superloop
run against 10–20 and 60–110 before).

The sections below are the history of the earlier fused model.

Moved from the HIS (`his/80-rf-fpga.md`, tag `his-draft0`) at gate review 1:
these notes describe the emulator, not the hardware. Text unchanged below;
paths are relative to `re/emu/`. Current state: `docs/EMU-JOB-STATUS.md` (D2)
and `docs/D2-action-plan.md`.

## Emulator status (g7250-emu, now `re/emu/`) — MODELLED (2026-11-19)

> `g7250-emu` and `g7250-base` below are historical names; see `docs/REPO_LAYOUT.md`, "Historical names".

A fused GPIOE+FPGA port (g7250-emu models/csharp/G7250Gpioe.cs) now implements
this interface: the loopback test register (0x00), the 3-byte version (0x08 ->
1.0.0), and DRDY (PE10) pulsing a changing 24-bit RX I/Q. The stock Main boots
to "Check FPGA BUS... Check complete!" with no FPGA errors (m1_run --fpga). The
remaining realism (true 48 kHz NCO-accurate I/Q, TX path) can build on this.

### Original note
Not yet modelled: the base GPIOE absorbs the Main's init bit-bang, so the stock
Main boots to its service loop but logs "[ERR] FPGA BUS Error" (the read-back
loopback has no FPGA driving the data lines) and no live I/Q flows. A **fused
GPIOE+FPGA model** (M3) must: (a) echo the 0x00 loopback for the bus test, (b)
return the 3-byte version, (c) drive PE0–7 on reads and PE10/DRDY at the rate
index, and (d) source/sink 24-bit I/Q. This needs a C# peripheral wired to the
GPIOE pins (both the MCU-output→FPGA and FPGA→MCU-input directions) plus the
exact STB/CMD handshake — the clean-room firmware target for the RF path.

**M3 attempt finding (g7250-emu):** a C# `G7250Fpga` model (loopback 0x00 +
version 0x08 + DRDY output) is written and compiles, but naively wiring all of
PE0–PE10 to it breaks the boot right after "System started." — GPIOE is heavily
multiplexed (EXTI lines, RF band switching), so an FPGA driving those pins
outside a bus transaction triggers spurious EXTI/behaviour. The correct model
must drive the data lines only during an active FPGA read strobe and must not
disturb the other GPIOE uses (EXTI-aware, transaction-scoped). That is the M3
work; the stock boot is unaffected because the errors are log-only.

**M3 approach pinned (g7250-emu, 6 attempts):** (1) wiring a separate C# FPGA to
the GPIOE pins breaks the boot — Renode GPIO is unidirectional and never
high-impedance, so the FPGA forces pins that are also EXTI lines (PE3/4/6/10).
(2) Subclassing Renode's STM32_GPIOPort to intercept IDR fails: its
ReadDoubleWord/WriteDoubleWord are not virtual. (3) The viable path is a
from-scratch FUSED GPIOE+FPGA port (one C# peripheral that is the GPIO port and
computes IDR itself, returning FPGA data on reads with MODER giving direction).
A first such port (`G7250GpioeFpga`, models/csharp/G7250Gpioe.cs) compiles and
replaces gpioPortE, but stalls the boot just after PH9 — reimplementing the
STM32H7 GPIO-port semantics the firmware's early init depends on must be exact
(MODER/IDR/ODR/BSRR/AFR/PUPDR + the AF-timer and EXTI/syscfg routing). Finishing
that, plus the STB/CMD read/write handshake and DRDY-paced 24-bit I/Q, is the M3
deliverable.
