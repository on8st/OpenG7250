# 0007 — The emulator as test bench for the product firmware

- Status: proposed
- Date: 2026-11-20
- Decided by: ON8ST (proposal to the repository owner)
- Affects: re/emu | product/firmware | testbench | project

## Context

The radio has not been delivered yet, and every product firmware image must be
tested before it runs on the hardware. The emulator in `re/emu/` runs the stock
Main and Control images on Renode, but its models are built from the stock
firmware's disassembly, so the product side may not read them
([0002](0002-three-sides-and-gate.md)). Some of the models are also tied to the
stock image itself: shims that write into the stock firmware's buffers stand in
for peripheral behaviour Renode 1.17 lacks, such as circular DMA.

## Decision

Every product firmware image is first tested in full on the emulator, as a
black box, before it runs on the radio.

- The product side delivers the image (ELF) and the tests, written from the
  spec only (`testbench/`).
- The RE side runs them with one command and returns a result file: pass or
  fail per test, plus the image's own console and network output. Nothing from
  the emulator's internals or models goes back.
- A failure that comes from a missing or wrong model is reported as such
  ("not modelled: …"), without describing how the stock firmware uses that
  part. A hardware question that comes out of it goes through the HIS and the
  gate as usual.
- The emulator models peripherals as the hardware behaves (reference manual,
  datasheets, the HIS), not as the stock firmware uses them: no fixed stock
  buffer addresses or stock-only shortcuts in the models that a product image
  depends on.
- The stock acceptance run (A–E) stays as a regression for the models.
- Later the run can become a CI job that the product side starts and reads
  results from, without access to `re/`.

## Consequences

The emulator needs generic peripheral models first (circular DMA, ADC, MDMA,
the FPGA and PA models at bus and pin level) and a runner for product images.
Emulator results stay model consistency, not hardware evidence: passing on the
emulator is required before the radio, but does not replace the hardware tests
in `his/90-needs-hardware.md`. Transmit stays disabled until it is measured on
the radio.
