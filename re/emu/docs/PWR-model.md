# PWR model (Main)

`models/csharp/G7250Pwr.cs` at 0x58024800, loaded by `product_run.py` and
`m1_run.py`. It replaces Renode 1.17's Python stub, which read PWR_CR3 as a
constant (only USB33RDY set), had no write-once supply configuration and
always reported ACTVOSRDY.

Sources: ST RM0468 (public), PWR chapter; the board's supply wiring from
his/10 ("SMPS supplies the LDO", MEDIUM).

## What it models

- PWR_CR3 reset value 0x00000006 (SMPSEN, LDOEN: the SMPS supplies the LDO).
- The supply configuration (SMPSLEVEL, SMPSEXTHP, SMPSEN, LDOEN, BYPASS) is
  write-once after power-on: the first write sets it, later writes leave it
  unchanged. A system reset (watchdog, software) does not unlock it; only a
  power-on does (a new emulator boot, `power.cycle`, `power.cut`). The USB
  regulator bits of CR3 stay writable; USB33RDY follows USB33DEN.
- PWR_CR3.SMPSEXTRDY is set while SMPSEXTHP is set in a valid configuration
  (the SMPS external supply is ready).
- PWR_CSR1.ACTVOSRDY is set while the active supply configuration matches the
  board wiring. Per his/10, a configuration with SMPSEN and LDOEN and without
  BYPASS matches; LDO alone, SMPS direct and bypass do not (HYPOTHESIS, tied to
  his/10). A configuration that does not match leaves ACTVOSRDY clear until the
  next power-on, as the MCU would not reach a valid core voltage.
- ACTVOS follows D3CR.VOS; D3CR.VOSRDY is set while ACTVOSRDY is.
- CR1 (reset 0xF000C000), CR2, CPUCR, WKUPCR, WKUPEPR are stored.

## Stock image (regression)

The stock Main's supply routine writes its configuration and then waits for a
PWR_CR3 condition that the configuration it wrote can never meet; on the
hardware it leaves the loop by its own 1 s timeout, and so it does here. In
the fully logged stock boot (`m1_run.py`) PWR accesses are therefore not
logged, so that the 1 s poll does not flood the log.

## Not modelled

Voltage levels and timing, PVD/AVD thresholds and flags, low-power modes
(Stop, Standby), wake-up pins, the backup-domain regulator.

## Self-tests

`firmware-tests/bench/pwr_test.c`: reset value, ACTVOSRDY at reset, a valid
configuration written, a second write ignored, VOSRDY; after a watchdog
reset the configuration and its lock are still there (7 checks).
`firmware-tests/bench/pwr_bad_test.c`: LDO alone gives no ACTVOSRDY. All pass.
