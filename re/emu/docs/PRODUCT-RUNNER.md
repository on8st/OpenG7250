# Product runner (step 3)

`harness/g7250emu/product_run.py`, step 3 of `docs/PRODUCT-TESTBENCH-PLAN.md`
and decision 0007. It runs a product firmware image (ELF) against a test list
of the test bench and writes the result file. Format: `testbench/docs/testlist-format.md`
(g7250-testlist / g7250-endpoint / g7250-testresult, v1.3 at fork tb/v1.1
6db1746), product-side material that the emulator side may read.

```
python3 harness/g7250emu/product_run.py <image.elf> <tests.json> [-o result.json] [--run-id ID] [--scope v1.0] [--boot standin|none]
```

The test bench directory comes from `testbench_dir = <dir>` in `local.config`
(git-ignored) or the env `G7250_TESTBENCH`: the directory where
`python3 -m testbench run ...` works. The runner starts it as a subprocess and
never imports it. The result's `testbench.commit` is the bench's own commit: a
`.tb-commit` file in that directory (for a tree unpacked without git, e.g. by
`git archive`), else the commit of the git work tree it belongs to; "unknown"
if neither applies.

## Boot (reset path)

On the radio, 0x08000000–0x0801FFFF holds the vendor bootloader (never written,
his/15) and the product image starts with its own boot stage at 0x08020000
(product decision 0028). Whether the vendor bootloader starts an image it did
not write is HIS-19, open until the F1 drill on the radio. The emulator has no
vendor bootloader, so a product run states its reset path:

| `--boot` | At power-on and after every reset (watchdog, software) | `environment.boot` in the result |
| --- | --- | --- |
| `standin` (default) | **stand-in for the vendor bootloader:** the core takes VTOR, SP and PC from the vector table at 0x08020000, nothing else (no checks, no delays, no vendor behaviour). Assumption: **F1 passes** (HIS-19 open) | `standin-0x08020000` |
| `none` | F1 fails: BOOT_ADD0 = 0x08000000 and an empty first sector; the core finds no vector table and stays halted. Every test reports `error` "no image started" | `none-empty-0x08000000` |

The stand-in and the halt live in the watchdog model's boot path
(`docs/IWDG-model.md`), so power-on, `reset.wdt`, a real watchdog timeout and
a software reset all take the same path. The own self-test programs
(`--selftest`) boot from their own vector table at 0x08000000. The runner
self-check program `net_test` is linked at 0x08020000.

## What one run does

Per test (or once per list with `"boot": "per-list"`):

1. Skip without booting: `target` `host` or `needs-radio`, a `scope` other than
   the one selected, or a `needs` fixture the runner does not offer
   (`fixture:<name>`).
2. Boot the image on the Main platform with the generic models only (DMA,
   DMAMUX, USART2, ADC, MDMA, PA plant, GPIOE + FPGA at the his/80 rate, SPI6 +
   BDMA + DMAMUX2, CH394, IWDG + reset flags, PWR, DBGMCU, EEPROM and I2C
   devices), plus the network bridge and the bench agent. No
   stock image map, no stock-only shim.
3. Wait until the image has programmed an address into the Ethernet
   controller; else `error` "the image configured no IP address ...".
4. Write `endpoint.json` (radio address as the image set it, three hosts: two
   in its subnet and one outside, `radio.connect`, `broadcast.send_to`, the
   control socket, the offered fixtures) and run the test bench for that test.
5. Stop at `timeout_s` of emulated time (`error` "timeout: ... emulated"), or at
   a wall-clock cap.
6. Take the bench's JSON line, add `duration_s` (emulated), `wall_s` and
   `artifacts` (console, console_timed, network.pcapng, trace files). The
   console is exactly what the image wrote; `console-timed.txt` has the same
   lines, each prefixed with its emulated time in seconds (start of the line). If the test failed or
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

- Port plan per boot: a base B chosen from 20000, 30000, 10000, the first whose
  whole span (B + 1024 … B + 8299) can be bound, and clear of the ports the
  format doc uses as examples (41024–47451, 52000–52299). The bridge binds its
  UDP sockets without SO_REUSEADDR, so a port another process holds fails the
  check. If no base is free, the test is `error` "runner: no free local port
  base ...", before anything boots. B is recorded in `endpoint.json` as
  `runner.port_base`.
- `radio.connect[P]` = 127.0.0.1:(B + P) for the spec ports 1024–1035,
  7250, 7251. A datagram the bench sends there from local port p reaches the
  image from `hosts[i].ip`:p (host i owns p by its `ports` range).
- A datagram from the image to `hosts[i].ip`:p is sent to `hosts[i].bind`:p
  from `radio.connect[<radio source port>]`.
- `broadcast.send_to.limited[P]` = 127.0.0.1:(B + 100 + P),
  `directed[P]` = 127.0.0.1:(B + 200 + P) for 1024 and 7251: the image receives
  the datagram as a broadcast on port P. A datagram from the image to the
  limited or directed broadcast address goes to the bench port it names.
- `hosts`: two in-subnet addresses (the 20th and 21st host of the image's
  subnet) and 10.99.0.20 outside it, on 127.0.0.1. hosts[0] owns 1024 … 7250,
  so the fixed source ports 1024 and 7250 some spec tests need are bound at the
  same numbers (both checked free before booting); hosts[1] and hosts[2] have
  B + 8100 … B + 8199 and B + 8200 … B + 8299.
- Every datagram in both directions is written to `network.pcapng`
  (Ethernet/IPv4/UDP, wall-clock time stamps).

## Boot event and exact pin changes (spec 0045)

- `{"kind": "boot", "at": "0x08020000", "pc": "<reset handler>", "t_us": T}`:
  the core executes its first instruction after a reset (power-on, power cycle,
  power cut, watchdog or software reset). `at` is the vector table the core boots
  from (0x08020000 under the stand-in), `pc` the reset handler from it. The runner
  arms it after the image is loaded (`sysbus.spi1.agent ArmBootHook`); later resets
  re-arm it.
- Watched pins: the 11 HIS 85 safe pins, PH9 and the latch/strobe pins PA11,
  PH12, PD13, PG2, PG5. A trace start emits the state of each of them (and of
  the requested pins); hw.observe `pins` lists them.
- In the boot phase (10 ms of emulated time after a boot event) pin changes of the
  watched pins come from write hooks on the GPIO ports (MODER, OTYPER, PUPDR, ODR,
  BSRR) at the instant of the write. Afterwards the 50 us sampler reports changes,
  as before: the handler at every GPIO write slows emulation about tenfold.
- `hw.trace start` returns `{"file", "t_us"}`: `t_us` is the emulated time of the
  start; replayed boot events lie before it.
- `reset.wdt` returns once the reset has happened.
- Within 10 ms (emulated) of a boot event a gpio event also carries
  `since_boot_us`: executed instructions since the boot event / core MHz. In that
  phase each instruction is its own translation block, so the count is exact; it
  assumes one instruction per core cycle (flash wait states, bus latencies and
  dual issue are not modelled) and excludes time in WFI/WFE. It is a lower bound
  for the hardware, not a worst case.
- A trace started after a boot begins with the last boot event and the boot-phase
  gpio events, with their original `t_us` (before the start time).
- Self-test: `tests/fakebench/boot.json` (T-EMU-16, trace starts with the boot event).

## Format v1.6: net.log, key.log, link events

- `net.log {action: start|read|stop, from?}`: every UDP datagram at the radio in
  emulated time, kept by the CH394 model: `to_radio` when the datagram is placed
  in the socket's receive buffer (the image can first see it), `from_radio` when
  the image's SEND executes. Entry: `t_us`, `dir`, `socket`, `radio_port`,
  `peer` (ip:port as the radio sees it, the endpoint's host addresses), `len`,
  `head` (first 16 payload bytes, hex). Pages of at most 5000 entries
  (`from`, `next`, `count`); a cap of 100000 entries, beyond it `dropped`.
- `key.log {action: start|read|stop, from_key?, from_hp?}`: `key` = chain A's
  latched frame has bit 7 set and its output enable PA10 is low (the bench's
  choice from HIS 85); every change with `tr` (the four T/R levels) and
  `bias_nonzero` (PA DAC channel 2, 7 or 8 written non-zero). While the log runs
  the port A write hook stays active, so PA10 changes are exact. `hp`: every
  datagram received on radio port 1027 with `seq` (payload bytes 0-3, big
  endian), `b4`, `b5`. Pages and caps as net.log (`count_key`, `count_hp`,
  `next_key`, `next_hp`, `dropped`).
- Trace event `{"kind": "link", "up", "speed", "t_us"}`: the CH394's link at
  power-on (at the boot event) and every change (link.set, a reset of the chip by
  RST#). A trace start replays the last one after the boot phase. The model does
  not model PHY renegotiation time: after a chip reset the link is up at once.
- Self-test T-EMU-17 (runner-selfcheck.json).

## Format v1.7: net.inject, time.wait

- `net.inject {host, from_port, to_port, at_us: [...], payloads: [hex]}` ->
  `{"t_us": [...]}`: each datagram is placed in the CH394 at emulated now +
  its offset, exactly, at any emulation speed. "now" is taken on the emulation
  thread; the answer gives every placing time. The datagram comes from
  `hosts[host].ip:from_port` (a port the bench has bound, so the radio's answers
  reach it as usual) and goes through the same receive path as a bridged one
  (socket by port, buffer size, the "buffer full" drop rule, the 8-byte header,
  RECV); in net.log it is a `to_radio` (or `dropped`) entry with
  `"injected": true`.
- `time.wait {us}` -> `{"t_us": now}`: returns when emulated time has advanced
  by `us` (the bench's waits in radio time).
- Self-test T-EMU-18 (runner-selfcheck.json).

## Format v1.8 (part 1): gpio.force, dac.force, time.advance

- `gpio.force {pin, target: idr|odr|moder, level|mode, from_us?, duration_us?}`
  -> `{pin, target, level|mode, t_us}`: `idr` holds the pin's input-register
  bit at `level` (a board fault; persists across reset.wdt, released with
  `level: null` or by power.cycle); `odr` writes the pin's output bit once and
  `moder` its 2-bit mode once (`mode`: input, output, af, analog), as a
  corrupted register the image may re-apply. `from_us` and `duration_us` are
  exact in emulated time: the start is scheduled in the emulator, the release
  (or, for odr/moder, the write-back of the previous value) exactly
  `duration_us` after the start. Trace events `force` and `release`.
- `dac.force {channel: 2|7|8, value, from_us?}`: the PA DAC channel's output word
  is written as a simulated firmware fault; `value: null` restores the word
  the image last wrote. Trace event `dac_force`.
- `time.advance {us}` -> `{t_us}`: Renode's advance-immediately mode (idle time
  skipped to the next timer event) until emulated now + us. The gain depends on
  how much the image idles: fw-d01f507 with no traffic, 2.7 times real time.
  The bench sends nothing during it but net.inject. The trace pin sampler pauses
  during an advance (one sample at its end), and at its end the time source's
  host stopwatches move forward by the time gained, so the emulation goes on at
  real time instead of stalling until host time catches up. The fixture server
  polls the emulated time to the target, so other calls are answered meanwhile.
- Self-test T-EMU-20.

## Fixture calls over 10 s
The test bench waits 30 s for the answer of a fixture call (tb 5fcdcf5); the stand-in bench does the
same (it waited 10 s before). A `power.cycle` answers when the new emulator has started, which took
4 to 10 s on a loaded host and can take longer. So that such a wait does not become a silent pass,
every call over 10 s is in the result: `emulator.slow_fixture_calls` of its test (wait, the parts of a
power cycle, agent requests over 1 s inside the call, the 1-minute load), a line in its `reason` also
when it passes, and a count `slow_fixture_calls` for the run. A call not answered at the end of the
test is listed as such. Checked without an emulator by tests/baseline/slow_calls_check.py.

## Format v1.9 fixtures

`flash.dump`, `flash.fill` (docs/FLASH-model.md), and:
- `reset.flags {flags: [...]}`: the next MCU reset (not a power cycle, which starts a new
  emulator with the power-on flags) shows exactly these RCC_RSR flags: software, watchdog, pin,
  power-on, brown-out, window-watchdog, low-power; `[]` for none. RAM is kept as over any reset;
  the trace names the true cause.
- `log.corrupt {seq}`: offered only where the runner was started with `G7250_LOG_RING=<address
  of the kept ring>`; the address is not in the endpoint. Layout as the product's (docs of the
  firmware side): little-endian 64-bit words, 48 bytes of header copies, then 64 entries of 96
  bytes, entry at base + 48 + (seq mod 64) x 96; word 0 is seq (low) and uptime (high). One bit of
  the first text word is flipped at the next MCU reset (after the old run's last write, before the
  new run's first read), once; an entry the image overwrote before it is not changed, and a power
  cycle drops it. An entry whose number is not there at the call is refused. A change of the
  firmware's layout makes the fixture wrong. (8a2a100 changed the RAM at once, while the image
  ran: the log.corrupt checks of T-A1-91 in the runs on that build do not count.)
- Every `flash.ecc`, `flash.fill`, `log.corrupt`, `cpu.fault` and `power.cut` call with its answer
  is in the test folder's fixture-log.jsonl, with `boot`, the boot of the call. A `power.cut` entry
  also gets its outcome: `fired` true with `fired_boot` (the boot it fired in) and `fired_entry`
  (the flash.log entry it ended: op, address, t_us, fraction_done or the torn word; `at_us` for a
  time cut), or `fired` false with `outcome` (dropped at the power cycle that ended its boot,
  replaced by a later `power.cut`, not fired by the end of the test, or, for `at_power_on`, not
  armed: no power-on after it). An `at_power_on` entry has `armed_boot`, the boot it was armed
  for. A test's entry gives the `flash.ecc` calls of that test
  (`emulator.flash_ecc`), and its reason names each one that was not in force when answered.
- The runner refuses a run folder that is already used (it would append to the old logs).

## cpu.fault: a processor fault at a chosen emulated time (0075, way (a))

The name and the shape are this emulator's proposal; the bench format lists the fixture as pending
and does not call it yet.
- `cpu.fault {at_us | in_us, mode?: "any"|"thread"|"handler", exception?, within_us?: 1000000}` →
  `{at_us, mode, exception, within_us, stub}`: from `at_us` (emulated us since the boot, as `hw.trace`)
  or `in_us` from now, the core's mode is looked at in a time event, between translated blocks, and
  then every 10 us of emulated time until `at_us + within_us`. When it is the mode asked for (thread:
  IPSR 0; handler: IPSR not 0, or the one `exception` number given, 2 to 178; any), the PC is set to
  an undefined instruction (UDF) in the emulator's own stub memory at 0x0FFF0000 (a range the chip
  leaves reserved; a fetch there on the chip would itself fault). The core takes the fault as for an
  undefined instruction: HardFault with HFSR FORCED and CFSR UNDEFINSTR, or UsageFault when the image
  has set USGFAULTENA. The registers and the stack are the image's; the stacked PC names the stub,
  not the image's code (way (b), a patched instruction, is not built: the stacked PC is not judged).
- `cpu.fault {read: true}` → `{state, mode, exception, at_us, until_us, samples, injected: {t_us, pc,
  ipsr, stub}, entry: {t_us, exception, handler, cfsr, hfsr, stacked_pc, stacked_ipsr}}`; `state` is
  `armed`, `injected`, `entered` (the first entry of the HardFault, MemManage, BusFault or UsageFault
  vector after it), `missed` (the mode did not come within the window; nothing done) or `dropped`
  (an MCU reset before the injection, or between the injection and the entry). A power cycle starts a
  new emulator, without the fixture. A new call replaces an armed one. In `hw.trace` the events
  `{"kind": "fault", "result": "injected"|"missed", ...}` and `{"kind": "fault_entry", exception}`.
- The sampling step is 10 us: a handler shorter than that can be missed for a long time, and the
  first instruction of a handler is not hit on purpose (a hook at the handler's address would do
  that; not built). The entry latency is not cycle-true. Not modelled: bus faults (also imprecise
  ones), MemManage without a real MPU violation, stacking and unstacking faults, lazy FP stacking,
  LOCKUP (a fault inside HardFault or with FAULTMASK set; what Renode does there is not known, so no
  test may rest on it). A fault while the core is stalled by a fetch during a masked erase lands at the
  polling loop's fetch from RAM.
- Checked by T-EMU-29 (tests/fakebench/cpu-fault.json, m1_smoke step [12]) against the bench image
  net_test.c: thread mode, the handler of exception 44, UsageFault, a missed window, a reset before the
  time, and the image's own handler seeing the same frame (IPSR, CFSR, HFSR, stacked PC and IPSR,
  EXC_RETURN). Every call is in fixture-log.jsonl.

## The emulator process

The runner records an emulator process that ends unasked (any exit code) and the macOS crash
reports of the emulator written in a test's or the run's time window (names only; result
`emulator_crash_reports`, also in result_report.py, 0 included; `null`: the folder is not
readable). A test with either is an error ("the emulator process died"), never a pass, fail or
hang of the image.

## Format v1.8 (part 3): the flash controller fixtures

`flash.log`, `flash.timing`, `flash.fault`, `flash.ecc`, `power.cut
{after_flash_words | during_erase | at_address}` (also `{during_erase | after_flash_words,
at_power_on: true}`, for the next power-on), `slot.corrupt`, `ch394.silent`;
the endpoint carries `flash_timing {erase_us, program_us, t_erase_max_us, source}`,
and `net.endpoint` answers the same whole endpoint (with `minor`, `usart1_timed` and
`time_scale`; before, these were lost at the bench's first `net.endpoint`).
What each does in the model: docs/FLASH-model.md "Format v1.8".
- `flash.log` runs from power-on; `start` moves the read origin, entries of every boot
  carry `boot`; the server pulls the layer's log at reads and before a power cycle.
- What a new emulator must keep (power cycle): the flash dump, the ECC words (and the
  torn word of a cut), the `flash.timing` erase duration and the rest of `ch394.silent`
  go into its start-up script.
- `ch394.silent {boots}`: for the next `boots` MCU boots (a reset of any kind) the CH394
  gives no answer: MISO reads 0xFF (HYPOTHESIS: the level of an absent chip is not in the
  HIS), writes and arriving datagrams are ignored.
- A new `power.cut` (any form) replaces one still armed.
- Self-tests T-EMU-22, T-EMU-23.

## RAM and reset causes

- **AXI SRAM** (320 KiB at 0x24000000) holds a pseudo-random pattern with a fixed seed
  (0x7250) at every power-on (a power cycle starts a new emulator), never zeros and never
  the old content; results carry `environment.axi_sram_at_power_on`. `G7250_RAM_FILL=0`
  switches it off (diagnostics). The other RAMs (TCM, SRAM1/2/4, backup SRAM) are zero at
  power-on. A software or watchdog reset keeps the AXI SRAM (T-EMU-23 checks 0x2401E700
  and 0x2401FFFC). Not modelled: an ECC error on reading RAM not written since power-up.
- **RCC_RSR** (0x580244D0): power-on 0x00E80000; a watchdog reset IWDG1RSTF | PINRSTF, any
  other reset SFT1RSTF | PINRSTF, added to what is set; RMVF clears all (docs/IWDG-model.md).
  The alias at RCC + 0x130 is not served.

## Fixtures (format section 4)

Offered now: `hw.observe`, `hw.trace`, `hw.input`, `power.cycle`, `power.cut`,
`reset.wdt`, `link.set`, `ctrl.set`, `time.scale` (fixed 1.0), `net.broadcast`,
`net.hosts`. Not yet: `time.advance`, `dhcp.*`, `sensor.set` (needs the HIS
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
  One change of a pin's state (level, mode, pull) gives one `gpio` event,
  whichever path sees it first: `hw.input`, the 50 µs sampler, a latch edge or
  the boot-phase port hooks. A `hw.input` that does not change the state gives no
  event. (Until 2026-11-24 a `hw.input` could give an edge a second time.)
- **Pin state (proposal for format v1.4, agreed):** every `gpio` trace event
  and every pin in the snapshot's `pins` block carries `mode` (`output`,
  `input`, `af`, `analog`, from MODER), `pull` (`up`, `down`, `none`) and
  `level`: 0 or 1 when the pin is driven (push-pull output, open-drain output
  low, an input pulled up or down, or an input driven by `hw.input`), `"z"` when
  nothing drives it (analog mode, an input without pull, a released open-drain
  output without pull), `null` for an alternate-function pin the emulator does
  not model at pin level. A change of mode or pull alone is also an event. The
  snapshot's existing keys keep their place and use the same level values.
- `hw.input {pin, level}`: PH6, PH3, PH4 only (his/85 PTT and key inputs); the
  emulator drives the port input, edges reach EXTI; other pins
  `"pin not offered: <pin>"`.
- `power.cycle`: the core stops, power goes off and on: flash keeps its
  contents (both banks are dumped from the running emulator and loaded into
  the new one), RAM does not; the image boots through the run's reset path.
  The bridge, the control socket and an active trace continue; the trace gets
  a `reset` event with cause `power`.
- `power.cut {at_us}`: at emulated time `at_us` (microseconds since boot, the
  trace time base; a time in the past cuts at once) the core stops at once,
  then power returns as with `power.cycle`. The reply gives the cut time.
  Whatever the image wrote to flash before the cut stays; the flash
  programming interface is Renode's `STM32H7_FlashController`, not yet checked
  against RM0468 (a self-test of erase and program comes before the first
  update-path test).
- `link.set {speed: 0|10|100}`: the Ethernet controller's PHY status (link,
  speed); while the link is down, datagrams in both directions are lost.
- `reset.wdt`: the MCU is reset as by an IWDG1 timeout (`docs/IWDG-model.md`):
  RCC_RSR then shows IWDG1RSTF and PINRSTF (plus any flags not yet cleared
  with RMVF), the core boots from 0x08000000; RAM keeps its contents, as on the
  hardware. An active trace gets a `reset` event with cause `wdt` (a software
  reset of the image gives cause `software`).
- Runner options for RE-side checks: `--flash-erase-us N` (sector erase
  duration, `docs/FLASH-model.md`), `--flash-preload ADDR=FILE` (load a file
  into flash before the first boot, e.g. a pre-filled store sector),
  `--pwr-cr3 HEX` (supply configuration locked at power-on), `--no-address`
  (do not wait for a network address), env `G7250_NO_FIXED_PORTS=1` (no fixed
  source ports for hosts[0], for runs without spec tests that need them).
- `ctrl.set {absent|silent}`: in both states the Main's interlink receives no
  bytes (no Control unit is emulated in product runs).

The agent inside the emulator is `models/csharp/G7250BenchAgent.cs` (SPI1
observer with latch inputs, pin reads, input driving, emulated time).

## Self-check

`tests/fakebench/` is a stand-in bench with the same command line and output
(RE-side scaffolding), run against the own program
`firmware-tests/bench/net_test.c` (sets the safe state, brings up the CH394,
echoes UDP on 1024): UDP echo, the outside host, limited broadcast,
hw.observe, hw.trace with hw.input edges, one event per change while the image
changes the same pin (T-EMU-32), an ECC mark in force at once after an MCU
reset (T-EMU-33), a refused pin, power.cycle with the
reset event, plus a host-only test and a missing fixture that skip.
`m1_smoke.sh` runs it as step [7].

A pipeline check with the real test bench (tb/v1.1 6db1746, unpacked into
`.private/tb/`) against the same program ran end to end; its tests fail or
error there because that program does not speak the product protocol.

## Known gaps

- SPI6 through the BDMA: done in step 1g (`docs/SPI-model.md`,
  `docs/BDMA-model.md`); SPI1 and SPI2 still use Renode's model (polled).
- `link.set`, `sensor.set` and the later fixtures, above.
