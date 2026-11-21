# MDMA model (Main)

> Sources cited as "g7250-base `<path>`" refer to the former data pin
> (historical name; see `docs/REPO_LAYOUT.md`, "Historical names").

`models/csharp/G7250Mdma.cs`, registered at 0x52000000 by `m1_run.py` for every
Main boot (it replaces the earlier `MDMA_NO_MODEL_TODO` tag).

## Why

With the fused FPGA model, RX sample blocks reach the Main's DSP. The DSP clears
and copies buffers with the MDMA and polls for completion. With only a tag at
0x52000000, every poll read 0 and ran into its timeout, so the boot crawled after
the FPGA bus check and never reached the superloop (diagnosed 2026-11-20: PC
held in the tick-wait helper `FUN_0803AD78`, called through `FUN_0803AAA8` from
the MDMA fill helper `FUN_0803AEE8`, read from the stack in the emulator).

## What the firmware does (from the analysis)

g7250-base `docs/dma.md` and `main_unit/docs/memory_map.md` (VERIFIED):
- five channels configured, all with software request, full-transfer trigger,
  packing and a 32-byte buffer length; no MDMA interrupt (the vector is the
  default handler, never enabled); the helpers start a transfer and poll;
- channel 0–2: word-to-word copies, both sides incrementing (RX/TX DSP, the wfview
  TX-audio ring);
- channel 4: fill, fixed source, incrementing word destination (buffer clears);
- the copy helper falls back to a CPU copy when the MDMA is busy and does its own
  D-cache maintenance.

## What the model does (since step 1d, 2026-11-21)

Generic, per ST RM0468 (public); step 1d of `docs/PRODUCT-TESTBENCH-PLAN.md`.
Loaded with `platforms/g7250_main_mdma.repl` (interrupt 122) by `m1_run.py`
and `product_run.py`.

- 16 channels at 0x40 + 0x40·x (CISR, CIFCR, CESR, CCR, CTCR, CBNDTR, CSAR,
  CDAR, CBRUR, CLAR, CTBR, CMAR, CMDR) and GISR0.
- Requests: software (CTCR.SWRM with a CCR write that sets SWRQ while EN) or a
  hardware trigger on GPIO input TSEL (CTBR.TSEL). No trigger source is
  connected yet; a product image that uses one gets "not modelled".
- Each request transfers one unit of CTCR.TRGM: a buffer (TLEN + 1 bytes), a
  block (BNDT bytes), a repeated block (BRC + 1 blocks, with the CBRUR source
  and destination updates and BRSUM/BRDUM) or the whole channel transfer
  including linked-list nodes.
- Increment, decrement or fixed addresses with SINCOS/DINCOS; different source
  and destination element sizes are packed through a byte stream.
- Flags TCIF, BTIF, BRTIF, CTCIF and TEIF (with CESR) on an unmapped address;
  EN cleared at channel completion; one interrupt line from the enables.
- Linked list: at the end of a node with CLAR ≠ 0 the next node is loaded from
  memory (CTCR, CBNDTR, CSAR, CDAR, CBRUR, CLAR, CTBR, reserved, CMAR, CMDR).
- Configuration registers are writable only while the channel is disabled.

## Not modelled (assumptions)

Transfer timing (each unit is instantaneous in virtual time), bursts, bus
selection, priorities, mask writes (CMAR/CMDR), endianness exchange
(BEX/HEX/WEX), padding and alignment modes beyond equal sizes or byte packing,
buffer-wise hardware request handshakes (BWM), secure and privileged
attributes.

## Self-test

`firmware-tests/bench/mdma_test.c`: buffer mode (one request per buffer),
block mode with word-to-half-word packing, repeated block with a destination
update, fill with a fixed source, a two-node linked list, the channel
interrupt, and a transfer error. 10 of 10 pass. The stock Main still reaches
its superloop on this model.

## History

The first version (2026-11-20) performed the whole transfer on a software
request and was written for the stock image's use (polled, full-transfer
trigger); it is replaced by the model above.
