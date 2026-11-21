# 0065 — Flash work: erase timing, failure, power loss and ECC errors

- Status: proposed (to the repository owner, ON8ST); answers testbench run-sheet gaps 5–9 (tb
  1b93429), 2026-11-21
- Date: 2026-11-21
- Builds on: 0007 (watchdog, fault classes), 0022 §16 (A3.10), 0055 (flash work), 0061 (store
  in sectors 6 and 7); [HIS] HIS 10, HIS 80, HIS 85; [STM32H72x].

## Context

The spec bounds flash work only by "a few seconds" (B2.3, 0055) and says nothing about an
erase that fails or never ends, a power loss inside a store write, flash ECC errors, or the
FPGA bus during an erase. The firmware already implements a two-sector, append-only store
with torn-record and ECC handling (`product/firmware/docs/memory-layout.md`); the spec now
states what is required, so that the tests can check it.

## Decision (proposed)

1. **Erase time** (gap 5). `t_erase_max` is the maximum 128 KiB sector erase time in the
   datasheet of the MCU fitted ([STM32H72x]); the spec names it but does not repeat the
   value. During flash work up to `t_erase_max`:
   - the processor watchdog does not expire;
   - the RF control pins keep their A3.6 levels;
   - the radio goes on as before afterwards.
   A3.10 has nothing to cover, because flash work never overlaps an arm (0055). Test T-B5-22,
   with every erase stretched to `t_erase_max` on the emulator.
2. **Flash work that fails** (gap 6). An erase or program that ends with an error flag, or does
   not end within 2 × `t_erase_max`, has failed:
   - a firmware update is handled as a write failure (55, 0043);
   - a store write gets 55 and the store keeps its old values;
   - both are logged and raise `INTERNAL_ERROR` (class 2): TX stays locked until a reset, and
     RX goes on.
   If the processor stalls so that the watchdog resets it, the store and the boot record are
   as before the operation, and the start-up reports `WDT_RESET_AT_START`. Status 55 is
   widened in Table A1-2. Test T-A1-85.
3. **Power loss in a store write** (gap 7). At any point, the erase of the other sector
   included, each item afterwards holds its old or its new value:
   - never a mix, and never an older value;
   - a write answered 0 is kept;
   - the generation never goes back.
   Test T-A1-86 (flash.log).
4. **FPGA bus during flash work** (gap 8). Yes, a spec test covers it. No bus cycle is open
   across an erase or a program: STB and CMD are low, and the firmware ends any cycle first.
   This is part of T-B5-22. The testbench's own T-B6-L01 stays as it is.
5. **ECC read errors** (gap 9). A double ECC error in a store word is treated as a torn
   record:
   - the store uses the older record, or the previous generation if the newest commit word is
     unreadable;
   - it logs this once per start and never programs that word again;
   - it raises no fault and no reset.
   An item with no readable record takes its Table A1-4 factory value, and a calibration set
   with none is absent; each is logged. Corrected single-bit errors are not reported. Test
   T-A1-87, if the emulator models flash ECC.

## Alternatives considered

- A fault class for an ECC error: the two-sector store already recovers, and a fault would
  lock TX for a condition the store handles.
- No time bound on a failed erase: then a hung erase could only end through the watchdog; a
  bound lets firmware that does not stall report the failure.

## Consequences

- Spec A1.6 "Store writes", B2.3 "Flash work" (two new items), Table A1-2 status 55; source
  [STM32H72x]; tests T-A1-85, T-A1-86, T-A1-87, T-B5-22.

## Sources

- [HIS] HIS 10, HIS 80, HIS 85; [STM32H72x]; decisions 0007, 0022, 0043, 0055, 0061.
