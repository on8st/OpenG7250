# IWDG1 and reset-flag model (Main)

`models/csharp/G7250Iwdg.cs`, loaded as `sysbus.watchdog` by `product_run.py`
(`platforms/g7250_main_iwdg.repl`, boot address 0x08000000) and `m1_run.py`
(`platforms/g7250_main_iwdg_stock.repl`, boot address 0x08020000: the stock
application, the emulator has no bootloader). It replaces Renode 1.17's
`STM32_IndependentWatchdog`, which cannot tell the firmware why it was reset.
Serves the test-bench fixture `reset.wdt` (`docs/PRODUCT-RUNNER.md`).

Source: ST RM0468 (public), IWDG and RCC chapters. No firmware-specific
behaviour.

## What it models

- IWDG1: KR (0xCCCC start, 0xAAAA reload, 0x5555 unlocks PR/RLR/WINR), PR
  (divider 4 << PR), RLR (12 bit, reset 0xFFF), SR (updates complete at once),
  WINR (a reload while the counter is above WINR resets the MCU). Counter clock
  LSI 32 kHz in emulated time; at 0 the MCU is reset.
- RCC_RSR (RCC + 0xD0) through bus hooks on Renode's RCC model:
  power-on value PORRSTF | BORRSTF | PINRSTF | D1RSTF (0x00E80000); an IWDG1
  reset adds IWDG1RSTF | PINRSTF; any other MCU reset (software, SYSRESETREQ)
  adds SFT1RSTF | PINRSTF; the flags are sticky until RMVF is written.
- After every MCU reset the core boots from the boot address (option byte
  BOOT_ADD0, default 0x08000000): VTOR, SP and PC from its vector table. RAM
  keeps its contents (a system reset does not clear SRAM).
- `WatchdogReset()` (fixture `reset.wdt`) resets the MCU as an IWDG1 timeout;
  `LastResetCause` (`power`, `wdt`, `software`) feeds the `reset` trace event.

## Not modelled

Hardware start of the watchdog by option byte, freeze in debug, Stop or
Standby, LSI tolerance, IWDG2, other reset sources' flags (brown-out during
run, low-power, the D1/D2 domain flags after a run-time reset).

## Self-test

`firmware-tests/bench/wdt_test.c`: power-on flags, RMVF clears, a real IWDG
timeout after the refreshes stop, after the reset exactly IWDG1RSTF | PINRSTF,
RAM kept, flags cleared again. 7 of 7 pass. Runner self-check T-EMU-10:
`reset.wdt`, the image boots again, the trace carries `reset` with cause
`wdt`. The stock Main refreshes this watchdog in its superloop (test A).
