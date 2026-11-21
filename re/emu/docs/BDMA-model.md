# BDMA and DMAMUX2 (Main, D3 domain)

`models/csharp/G7250Bdma.cs` and Renode's `STM32_DMAMUX` as DMAMUX2, loaded
with `platforms/g7250_main_spi6_dma.repl` (step 1g of
`docs/PRODUCT-TESTBENCH-PLAN.md`). The BDMA replaces Renode 1.17's
`STM32LDMA`, whose channels were wired only to EXTI lines; DMAMUX2 was not in
the base platform (only a tag).

Sources: ST RM0468 (public), BDMA and DMAMUX chapters; BDMA channel
interrupts 129–136. No firmware-specific behaviour.

## What it models

- 8 channels: ISR (GIF, TCIF, HTIF, TEIF per channel), IFCR, CCR, CNDTR, CPAR,
  CM0AR, CM1AR at 0x08 + 0x14·x.
- Peripheral-to-memory (DIR = 0) and memory-to-peripheral (DIR = 1): one item
  per request edge from DMAMUX2 (output n → channel n); edges during a
  transfer are queued; a request already pending when the channel is enabled
  is served. Memory-to-memory: the whole transfer at enable.
- PSIZE and MSIZE separately: read at the source size, written at the
  destination size (zero-extended or truncated); PINC and MINC.
- Circular mode, double-buffer mode (CT toggles), HTIF and TCIF, interrupt per
  channel.
- **D3 only:** the BDMA reaches SRAM4 (0x38000000), the backup SRAM
  (0x38800000) and the D3 peripherals (0x58000000–0x5FFFFFFF); any other
  address gives TEIF and disables the channel, as on the hardware. A product
  image with an SPI6 DMA buffer in AXI or D2 SRAM fails here as it would on
  the radio. For the product side: SPI6 DMA buffers go in SRAM4; the backup
  SRAM stays unused (his/15), although the hardware would reach it.

DMAMUX2 is Renode's DMAMUX model with 32 request inputs and 8 outputs; the
SPI6 requests are inputs 11 (RX) and 12 (TX). Request generators and
synchronisation are not exercised.

## Not modelled

Transfer timing, channel priorities, secure and privileged attributes.

## Tests

`firmware-tests/bench/spi6dma_test.c` (see `docs/SPI-model.md`), 6 of 6.
