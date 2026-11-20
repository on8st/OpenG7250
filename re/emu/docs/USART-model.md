# USART model (Main USART2)

`models/csharp/G7250Usart.cs` with `platforms/g7250_main_usart2.repl`, loaded
by `m1_run.py`, `slip_run.py` and `product_run.py`. It replaces Renode 1.17's
`STM32F7_USART` on USART2, which raises no DMA requests, so a DMA receive ring
never filled. Step 1e of `docs/PRODUCT-TESTBENCH-PLAN.md`; it supersedes the
earlier USART2 RX ring shim (`docs/USART2-RX-shim.md`, removed).

Sources: ST RM0468 (public), USART chapter and the DMAMUX1 request table
(usart2_rx_dma = 43, usart2_tx_dma = 44). No firmware-specific behaviour.

## What it models

- Registers CR1, CR2, CR3, BRR, GTPR, RTOR, RQR, ISR, ICR, RDR, TDR, PRESC;
  32-, 16- and 8-bit access (a byte access acts on the whole register).
- Transmit: a TDR write with UE and TE sends the byte at once; TXE and TC
  stay set.
- Receive: bytes from the connected terminal or UART hub are queued while UE
  and RE are set (lost otherwise); RXNE while the queue is not empty; an RDR
  read takes one byte; IDLE is set when the queue runs empty after reception
  (cleared by ICR.IDLECF or a new byte); RQR.RXFRQ flushes the queue.
- DMA: a request edge on `RxDma` per received byte while CR3.DMAR, and again
  after an RDR read while bytes wait; `TxDma` is raised while CR3.DMAT and
  TXE, and again after each TDR write. The DMA model serves a request that is
  already pending when its stream is enabled, and queues edges that arrive
  during a transfer.
- TEACK and REACK follow TE and RE while UE; one interrupt line from
  RXNEIE, TXEIE, TCIE, IDLEIE and EIE.

## Not modelled (documented assumptions)

Baud-rate timing (bytes move instantly), overrun (the receive queue has no
depth limit; real hardware would set ORE if RDR is not read in time), FIFO
thresholds, parity, framing and noise errors, LIN, smartcard, IrDA,
synchronous mode, auto-baud, receiver timeout, wake-up from Stop. A write to
an unknown offset logs "not modelled: USART2/offset ...".

## Tests

- Own self-test `firmware-tests/bench/usart_test.c` (USART2 and USART3 through
  a UART hub): TEACK/REACK; RX into a 16-byte circular DMA ring (NDTR, ring
  contents after a wrap, HT and TC interrupts); IDLE and its clear; TX of 5
  bytes by DMA, received on USART3. 9 of 9 pass.
- Stock regression: the HIS-02 probe applies every settings image through
  USART2 and the DMA ring (no shim), with the same word roles as run 5; D1
  (`slip_run.py`) shows the Main receiving the Control's bytes.
