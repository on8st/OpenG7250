# DMA1/DMA2 model (Main)

`models/csharp/G7250Dma.cs` with `platforms/g7250_main_dma.repl`, loaded by
`m1_run.py` (every stock Main run) and by `product_run.py` (every product
run). It replaces Renode 1.17's `STM32DMA`, which keeps no circular mode
(after one transfer the CIRC bit is gone and NDTR stays 0). Step 1a of
`docs/PRODUCT-TESTBENCH-PLAN.md`.

Sources: ST RM0468 (public), DMA controller chapter. No firmware-specific
behaviour.

## What it models

- 8 streams per controller; LISR/HISR, LIFCR/HIFCR; per stream CR, NDTR, PAR,
  M0AR, M1AR, FCR (reset value 0x21).
- Directions: peripheral-to-memory and memory-to-peripheral, one data item per
  request edge from DMAMUX1 (input number = stream); memory-to-memory, the
  whole transfer when the stream is enabled (source PAR, destination M0AR).
- PSIZE, PINC, MINC, PINCOS; NDTR counts PSIZE items.
- Circular mode: NDTR and both pointers reload at the end of a block, the
  stream stays enabled.
- Double-buffer mode: CT toggles at the end of each block; the memory address
  not in use may be written while the stream is enabled.
- Flags TCIF, HTIF (half of the block done) and TEIF (source or destination
  not mapped: the stream is disabled); one interrupt line per stream from
  TCIE/HTIE/TEIE/DMEIE/FEIE, to the NVIC lines of the base platform.
- While EN = 1, NDTR, PAR and the configuration bits of CR are protected
  (writes ignored), only the interrupt enables change. Clearing EN stops the
  stream; NDTR then shows the items left.

## Not modelled (documented assumptions)

Transfer timing (every beat is instantaneous in virtual time), FIFO
thresholds and burst beats (FIFO mode is done as direct beats; the memory
contents are the same as with packing), stream priorities, peripheral flow
control (PFCTRL), FIFO and direct-mode error conditions. A register offset
outside the map logs "not modelled: DMAx/offset ...".

## DMAMUX1 (step 1b)

Renode 1.17's `STM32_DMAMUX` is kept. The bench self-test shows that a request
on a DMAMUX input reaches the stream whose channel selects that request ID
(DMAMUX channel n → DMA1 stream n for n < 8, DMA2 stream n − 8 above), one
item per request edge. Request generators and synchronisation are not
exercised yet.

## Self-test

`firmware-tests/bench/dma_test.c` (own test program), run with
`python3 harness/g7250emu/product_run.py .private/bench/dma_test.elf
firmware-tests/bench/tests-bench.json --selftest`. Requests come from a
bench-only request source (`G7250BenchReq`, DMAMUX input 113,
`platforms/g7250_bench.repl`), never loaded for stock or product images.
Checks: memory-to-memory copy and TCIF; request routing through DMAMUX;
NDTR protected while enabled; circular reload; HT and TC interrupts; wrap
with new data; disable; double-buffer CT toggling with two TC interrupts;
transfer error on an unmapped address.

## Effect on the stock image

The stock Main runs on this model from step 1a on (test A, acceptance).
Since step 1e the USART2 RX ring of the stock Main fills through the generic
USART2 model and this DMA (`docs/USART-model.md`); the RX ring shim is gone.
