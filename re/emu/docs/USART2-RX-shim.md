# USART2 RX shim (Main) — removed

Superseded by the generic USART2 and DMA models (`docs/USART-model.md`,
`docs/DMA-model.md`, step 1e of `docs/PRODUCT-TESTBENCH-PLAN.md`).

History: Renode 1.17's DMA kept no circular mode and its USART raised no DMA
requests, so the Main received no SLIP bytes on USART2. From 2026-11-20 a shim
(`models/csharp/G7250Usart2RxShim.cs`, now deleted) wrote each received byte
into the DMA1 Stream 2 ring and updated NDTR, for the HIS-02 probe (run 5).
It depended on the firmware's own ring set-up and was therefore stock-only.
Since step 1e the bytes reach the ring through the real USART2 request and the
DMA stream, with no knowledge of the firmware.
