# Clean-room rules

Rules for every part of OpenG7250 and every contributor, human or agent.
Each rule is backed by a decision in [`decisions/`](decisions/).

## Why

The new firmware replaces the stock firmware on the same hardware. To keep an
open-source release defensible, the project keeps a verifiable record that the
product was built from public protocols, own design choices and reviewed
hardware facts only, never from how the stock firmware does things.

## Separation

There are three sides ([0002](decisions/0002-three-sides-and-gate.md)):

- **RE** (`re/`) studies the stock firmware and hardware.
- **Gate** (`his/`) holds hardware facts only. It is written by the RE side and
  reaches the product side only after a gate review marks it cleared.
- **Product** (`product/`, `testbench/`) is what is being built. It never reads
  `re/`.

A contributor who has read `re/` or stock firmware does not write product
material. The product side works in its own sparse clone with a read-only,
cleared copy of `his/` ([0003](decisions/0003-product-work-tree.md)).

## Material

| May pass to the product side (through the HIS) | Stays on the RE side |
|---|---|
| Parts, buses, addresses, pins, wiring | Boot order, timeouts, stock register values |
| Electrical values and datasheet behaviour | Stock data formats and defaults |
| The FPGA's bus interface | Stock protocols the product does not need |
| Safety constraints (e.g. keep the factory firmware reinstallable) | Decompiler output, listings, dumps, traces |

Every HIS fact carries its source and a confidence tag (VERIFIED, HIGH,
MEDIUM, HYPOTHESIS). Facts that rest only on our own emulator count as model
consistency, not as hardware evidence.

Never in the repository at all: see [SECURITY.md](SECURITY.md).

## Way of working

1. The product side asks hardware questions as HIS questions.
2. The RE side answers with a change to `his/`, and puts RE detail in
   `re/analysis/docs/`.
3. An independent reviewer checks the change against this page and records the
   verdict in a gate review in `re/analysis/docs/`.
4. Only a **cleared** commit of `his/` is given to the product side.
5. Values that need the real radio stay "pending hardware" in the spec, with a
   safe placeholder; transmit stays disabled until they are measured.
