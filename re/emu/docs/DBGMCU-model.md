# DBGMCU model (Main)

`models/csharp/G7250Dbgmcu.cs` at 0x5C001000, loaded by `product_run.py`.
Renode 1.17's STM32H7 platform has nothing at this address, so a product
image that touched it got "not modelled: DBGMCU/offset ...".

Source: ST RM0468 (public), debug support chapter; DEV_ID from his/10.

## What it models

- IDCODE (0x000): DEV_ID 0x483 (STM32H72x/H73x, his/10) and a REV_ID. The
  silicon revision on the radio is not known (his/90, step 2), so REV_ID is a
  parameter, default 0x1001 (HYPOTHESIS). IDCODE is read-only.
- CR (0x004), APB3FZ1 (0x034), APB1LFZ1 (0x03C), APB2FZ1 (0x04C) and APB4FZ1
  (0x054): storage of the writable bits.

## Not modelled

The effect of the freeze and low-power debug bits (they act only while a
debugger halts the core, and no debugger is attached), the trace and debug
clock enables. Other offsets log "not modelled: DBGMCU/offset ...".

## Self-test

`firmware-tests/bench/dbgmcu_test.c`: DEV_ID, IDCODE read-only, CR and
APB4FZ1 read back. 4 of 4 pass.
