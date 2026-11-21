# SPI model (Main SPI6)

`models/csharp/G7250Spi.cs`, loaded for SPI6 with
`platforms/g7250_main_spi6_dma.repl` by `m1_run.py` and `product_run.py`
(step 1g of `docs/PRODUCT-TESTBENCH-PLAN.md`). It replaces Renode 1.17's
`STM32H7_SPI` on SPI6, which raised no TX DMA request, and SPI6 had no DMA
wiring at all. SPI1 and SPI2 still use Renode's model (polled; their DMA is
not needed by any test yet).

Sources: ST RM0468 (public), SPI chapter; SPI6 interrupt 86; DMAMUX2 requests
spi6_rx_dma = 11 and spi6_tx_dma = 12 (the stock image configures the same
numbers, a consistency check).

## What it models (master)

- CR1 (SPE, CSTART, SSI), CR2 (TSIZE), CFG1 (DSIZE, FTHLV, RXDMAEN, TXDMAEN,
  MBR), CFG2 (COMM, MASTER, LSBFRST, CPOL/CPHA, SSM), IER, SR, IFCR, TXDR,
  RXDR; CRCPOLY, UDRDR, I2SCFGR stored.
- Each frame written to TXDR goes to the attached SPI device byte by byte (MSB
  first, LSB first with LSBFRST); 8-, 16- and 32-bit accesses pack 1 to 4
  frames. Full duplex: the reply goes to the RX FIFO (depth per instance, 8
  for SPI6; OVR beyond it). Simplex transmit discards the reply; simplex
  receive clocks in 0x00 frames on demand.
- TSIZE > 0: EOT, TXTF and TXC after TSIZE frames, CSTART cleared, the
  device's FinishTransmission called. TSIZE = 0: endless.
- SR: RXP (FIFO at or above FTHLV + 1 frames), TXP, DXP, EOT, TXTF, OVR, TXC,
  RXPLVL, RXWNE, CTSIZE; interrupt line from IER.
- DMA: an RX request edge per received frame while RXDMAEN; a TX request while
  TXDMAEN and frames remain (after CSTART and after each TXDR write). Both
  requests drop when the SPI cannot move data (SPE cleared, transfer ended,
  FIFO empty), so a DMA channel enabled before the next CSTART does not
  transfer early.

## Timing (SPI6 in product runs: `sysbus.spi6 Timed true`)

- f_SCK = SPI6 kernel clock / 2^(MBR+1) (CFG1 MBR). The kernel clock comes from
  the clock model (RM0468 RCC_D3CCIPR SPI6SEL: rcc_pclk4, the reset choice, =
  HCLK / D3PPRE from RCC_D3CFGR; hsi_ker; the PLL, CSI and HSE choices are not
  modelled, pclk4 is used with a warning).
- Every data frame takes its bit count / f_SCK of emulated time, back to back on
  the bus. A received frame is visible (RXP, RXDR, an RX DMA request) only from
  its completion; EOT, TXTF and TXC only from the last frame's completion; TXP
  only while fewer than the FIFO depth of frames are still in flight. The device
  (CH394) sees each frame when it is written; only its visibility to the core
  and the DMA is timed. Inter-frame gaps (MIDI, MSSI) are not modelled.
- Time is read from a free-running timer of the emulated clock, so it advances
  inside a quantum while the core polls.
- Other SPI instances, and stock runs, stay untimed.
- Self-test `firmware-tests/bench/spi6time_test.c` (m1_smoke step [6]): 1003
  frames at 4 MHz, by polling and by BDMA, measured with TIM2 at 1 MHz.

## Not modelled

Slave mode, timing and baud rate of the untimed instances, CRC calculation, underrun, mode fault, TI
mode, I2S, suspend (CSUSP, MASRX), SS pin management.

## Self-test

`firmware-tests/bench/spi6dma_test.c`: CH394 register reads by full-duplex
DMA (version, MAC), CNDTR at 0 after the transfers, and a buffer outside D3
giving a BDMA transfer error. 6 of 6 pass. `net_test` (polled SPI6) passes the
runner self-check on this model. The stock Main's own Ethernet driver now
reads the chip through SPI6 + BDMA (chip version, MAC, UDP socket opened).
