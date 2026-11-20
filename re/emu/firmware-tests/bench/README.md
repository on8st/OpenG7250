# Bench self-test programs

Own test scaffolding for the product test bench (decision 0007,
`docs/PRODUCT-TESTBENCH-PLAN.md`), written from ST RM0468. Not product code and
not derived from any vendor image.

- `smoke.c`: console banner and a GPIO blink (the runner's smoke target).
- `dma_test.c`: DMA1 + DMAMUX1 self-test (`docs/DMA-model.md`).

Build: `make -C firmware-tests/bench` (arm-none-eabi-gcc) into `.private/bench/`.
Run: `python3 harness/g7250emu/product_run.py .private/bench/<prog>.elf
firmware-tests/bench/tests-bench.json --selftest`. Each program prints
`TEST <name> PASS|FAIL ...` lines and `BENCH DONE` on USART1. `m1_smoke.sh`
runs them (step [6]).
