# Product runner (step 3)

`harness/g7250emu/product_run.py`, step 3 of `docs/PRODUCT-TESTBENCH-PLAN.md`
and decision 0007. It runs a product firmware image (ELF) against a test list
of the test bench and writes the result file. Format: `testbench/docs/testlist-format.md`
(g7250-testlist / g7250-endpoint / g7250-testresult, v1.3 at fork tb/v1.1
6db1746), product-side material that the emulator side may read.

```
python3 harness/g7250emu/product_run.py <image.elf> <tests.json> [-o result.json] [--run-id ID] [--scope v1.0]
```

The test bench directory comes from `testbench_dir = <dir>` in `local.config`
(git-ignored) or the env `G7250_TESTBENCH`: the directory where
`python3 -m testbench run ...` works. The runner starts it as a subprocess and
never imports it.

## What one run does

Per test (or once per list with `"boot": "per-list"`):

1. Skip without booting: `target` `host` or `needs-radio`, a `scope` other than
   the one selected, or a `needs` fixture the runner does not offer
   (`fixture:<name>`).
2. Boot the image on the Main platform with the generic models only (DMA,
   DMAMUX, USART2, ADC, MDMA, PA plant, GPIOE + FPGA at the his/80 rate, CH394,
   EEPROM and I2C devices), plus the network bridge and the bench agent. No
   stock image map, no stock-only shim.
3. Wait until the image has programmed an address into the Ethernet
   controller; else `error` "the image configured no IP address ...".
4. Write `endpoint.json` (radio address as the image set it, three hosts: two
   in its subnet and one outside, `radio.connect`, `broadcast.send_to`, the
   control socket, the offered fixtures) and run the test bench for that test.
5. Stop at `timeout_s` of emulated time (`error` "timeout: ... emulated"), or at
   a wall-clock cap.
6. Take the bench's JSON line, add `duration_s` (emulated), `wall_s` and
   `artifacts` (console, network.pcapng, trace files). If the test failed or
   broke and the emulator logged accesses to parts it does not model, the
   status becomes `error` with reason "not modelled: <peripheral>/<register>"
   (named by ST RM0468 peripheral and offset only).

Run directory: `.private/product-runs/<run-id>/` (git-ignored), with
`result.json` and one directory per test (or `list/`): `console.txt`,
`network.pcapng`, `trace-<n>.jsonl`, `endpoint.json`, plus the emulator's own
`renode.log` and generated platform files, which stay on the RE side. Only
pass/fail and the image's own output are relayed to the product side.

Exit code: 0 no fail and no error, 1 a fail, 2 an error.

## Network (address mapping, format section 3)

The CH394 model (`docs/CH394-model.md`) exchanges UDP datagrams with the
runner over a local TCP link (`harness/g7250emu/benchnet.py`, `Bridge`):

- `radio.connect[P]` = 127.0.0.1:(40000 + P) for the spec ports 1024–1035,
  7250, 7251. A datagram the bench sends there from local port p reaches the
  image from `hosts[i].ip`:p (host i owns p by its `ports` range).
- A datagram from the image to `hosts[i].ip`:p is sent to `hosts[i].bind`:p
  from `radio.connect[<radio source port>]`.
- `broadcast.send_to.limited[P]` = 127.0.0.1:(40100 + P),
  `directed[P]` = 127.0.0.1:(40200 + P) for 1024 and 7251: the image receives
  the datagram as a broadcast on port P. A datagram from the image to the
  limited or directed broadcast address goes to the bench port it names.
- `hosts`: two in-subnet addresses (the 20th and 21st host of the image's
  subnet) and 10.99.0.20 outside it; port ranges 52000–52099, 52100–52199,
  52200–52299 on 127.0.0.1.
- Every datagram in both directions is written to `network.pcapng`
  (Ethernet/IPv4/UDP, wall-clock time stamps).

## Fixtures (format section 4)

Offered now: `hw.observe`, `hw.trace`, `hw.input`, `power.cycle`, `reset.wdt`,
`ctrl.set`, `time.scale` (fixed 1.0), `net.broadcast`, `net.hosts`. Not yet:
`link.set`, `time.advance`, `power.cut`, `dhcp.*`, `sensor.set` (needs the HIS
scales). A test that needs
one of these is skipped with `fixture:<name>`.

- `hw.observe`: snapshot with the keys of the format, from the HIS 85
  safe-state items: pin levels, the last frame latched into SPI1 chain A
  (latch PA11) and chain B (latch PH12), ATU latch writes (PD13, PG2, PG5), the
  PA DAC words (`null` if never written) and PB14.
- `hw.trace`: `start {pins}` returns `{"file"}`; `stop` returns
  `{"events", "dropped", "file"}`. One live JSON Lines file per start
  (`trace-<n>.jsonl`, flushed per event). Events: `gpio` (pin, electrical
  level; pins sampled every 50 µs of emulated time, latch edges exact), `spi`
  (SPI1 transfers, `cs` null, data as hex), `reset` (`power` after a
  `power.cycle`).
- `hw.input {pin, level}`: PH6, PH3, PH4 only (his/85 PTT and key inputs); the
  emulator drives the port input, edges reach EXTI; other pins
  `"pin not offered: <pin>"`.
- `power.cycle`: the emulator restarts the image from reset; the bridge,
  control socket and an active trace continue.
- `reset.wdt`: the MCU is reset as by an IWDG1 timeout (`docs/IWDG-model.md`):
  RCC_RSR then shows IWDG1RSTF and PINRSTF (plus any flags not yet cleared
  with RMVF), the core boots from 0x08000000; RAM keeps its contents, as on the
  hardware. An active trace gets a `reset` event with cause `wdt` (a software
  reset of the image gives cause `software`).
- `ctrl.set {absent|silent}`: in both states the Main's interlink receives no
  bytes (no Control unit is emulated in product runs).

The agent inside the emulator is `models/csharp/G7250BenchAgent.cs` (SPI1
observer with latch inputs, pin reads, input driving, emulated time).

## Self-check

`tests/fakebench/` is a stand-in bench with the same command line and output
(RE-side scaffolding), run against the own program
`firmware-tests/bench/net_test.c` (sets the safe state, brings up the CH394,
echoes UDP on 1024): UDP echo, the outside host, limited broadcast,
hw.observe, hw.trace with hw.input edges, a refused pin, power.cycle with the
reset event, plus a host-only test and a missing fixture that skip.
`m1_smoke.sh` runs it as step [7].

A pipeline check with the real test bench (tb/v1.1 6db1746, unpacked into
`.private/tb/`) against the same program ran end to end; its tests fail or
error there because that program does not speak the product protocol.

## Known gaps

- **SPI6 through the BDMA.** The CH394 is on SPI6 in the D3 domain, whose DMA
  is the BDMA (via DMAMUX2). Renode 1.17's BDMA and SPI DMA requests do not
  deliver: an image that moves SPI6 data by DMA reads zeros. Polled SPI works
  (the self-check). Planned as step 1g.
- `link.set`, `sensor.set` and the later fixtures, above.
