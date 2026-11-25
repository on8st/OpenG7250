# Flash model (Main)

Renode 1.17's `STM32H7_FlashController` (flash interface at 0x52002000) plus
`models/csharp/G7250FlashTiming.cs`, a layer through bus hooks on that
controller, loaded by `product_run.py` (per-boot repl with the erase
duration). Sources: ST RM0468 (public), flash chapter; the durations shall come
from the STM32H725/735 datasheet, ST DS13311 Rev 5, Table 48 "Flash memory
characteristics" (format v1.8 build).

## Self-test against RM0468 (`firmware-tests/bench/flash_test.c`)

| Check | Renode's controller alone | With the layer |
| --- | --- | --- |
| CR1 locked after reset | pass | pass |
| CR1 writes ignored while locked | **fail** (accepted) | pass |
| unlock with KEYR1 | pass | pass |
| sector erase: EOP, content 0xFF | pass (instant) | pass, busy for the set duration |
| flash-word program (8 words): EOP, content | pass (instant) | pass |
| FW (force write) of a partial word | pass | pass |
| write without PG: no change, PGSERR | **fail** | **fail** (not modelled yet) |
| INCERR (second word before the first is complete) | pass | pass |
| relock | pass | pass |

## What the layer adds

- FLASH_CR1 writes are ignored while LOCK is set; the unlock sequence on KEYR1
  clears it (RM0468).
- A sector erase (SER + START) or bank erase (BER + START) keeps SR1.BSY and
  SR1.QW set for `eraseSectorUs` / `eraseBankUs` of emulated time (runner
  `--flash-erase-us`). 0 = instant. **The datasheet values are not set**; runs
  with a duration state it as a stand-in.
- An instruction fetch from the flash bank (0x08000000–0x080FFFFF) while the
  erase is busy stalls the core until the erase ends, as the bus does on the
  single-bank H72x (found by a PC sample every 10 us, see below); timers, the
  watchdog and DMA keep running. Each such fetch
  is counted (`FetchesDuringErase`, `FirstFetchPc`).
- At each erase start PRIMASK and the watchdog's refresh count are recorded;
  at the end the refreshes during the erase (store check ST-05).
- Power loss during an erase: the flash kept over the power cycle has the
  first fraction (elapsed / duration) of the sector erased and the rest as it
  was at START (HYPOTHESIS: linear progress; the real content of an
  interrupted erase is undefined).
- Programmed flash words are counted from the controller's end-of-programming
  flag (a new EOP seen in SR1 while PG is set, re-armed when CCR1 clears EOP);
  the core can be stopped after the k-th word (store check ST-04). This needs
  the image to clear EOP after each word.

## Format v1.8 (bench fixtures; docs/PRODUCT-RUNNER.md)

- **Erase durations by PSIZE** (CR1 bits 5:4 at START), DS13311 Rev 5 Table 48: sector
  erase typical 2.0 / 1.8 / 1.1 s for x8 / x16 / x32, maximum 4 / 3.6 / 2.2 s; bank erase
  typical 13 / 8 / 6 / 5 s, maximum 26 / 16 / 12 / 10 s. x64 has no sector figure: typical
  as x32 (1.1 s), maximum the longest listed (4 s, x8), named so in the source text.
  `flash.timing {erase_us}` sets a fixed duration (T-B5-22 uses `t_erase_max_us`, 4 s).
  `--flash-erase-us` still overrides the sector erase. Word programming time: not modelled (0).
- **The erase is the layer's**: START is not passed to Renode's controller; at the end the
  range is written 0xFF (the erased state) and SR1.EOP set until CCR1 clears it. A new board's
  flash outside the image is 0xFF too (the runner loads a 0xFF bank before the ELF).
- **flash.log**: `erase` / `bank_erase` (t_us, t_end_us, result ok | cut with fraction_done |
  error:FLAG | busy (never ends) | reset), `program` (one entry per run of back-to-back
  32-byte words in one sector, found by comparing the bank with its last state at each EOP;
  a cut gives result cut, words_written and `torn`; an entry ends at a reset, at a pause of
  more than 1 ms of emulated time between two words, at any other flash operation and at a read
  of the log; `t_us` and `t_last_us` are its first and last word, `fpga_bus` is taken at the
  first. Before this an entry went on while the words were adjacent, over resets and pauses: runs
  of fw-01a108d and earlier show such merged entries), `option_write` (always refused: an
  OPTSTART is not carried out), `ecc`, `corrupt`. Every erase and program carries `fpga_bus`:
  `open` when STB (PE9, his/80) is an output driven low at its start, else `idle`.
- **flash.fault {op, after, flag, never_end}**: the next matching operation (after `after`
  more) sets the RM0468 SR1 flag (WRPERR PGSERR STRBERR INCERR RDPERR RDSERR SNECCERR
  DBECCERR CRCRDERR; CCR1 clears it). An erase with a flag leaves the sector as it was; a
  program with a flag leaves the word as it was; `never_end` (op erase only, no flag
  needed; refused for op program) keeps BSY and QW set until a reset (fetches from the
  bank stall meanwhile); its entry is `running` with t_end_us null, then `reset`. Other
  running erases are listed with result `running` and the predicted t_end_us. A reset
  during an erase leaves the sector as it was (a power cut erases the fraction done).
- **flash.ecc {address, type}** and torn words: reads of that 32-byte word (sysbus read
  watchpoints) set SNECCERR or DBECCERR and FLASH_ECC_FA1R (the flash-word index), one
  flash.log entry while the flag is set. The word cut by `power.cut {after_flash_words}` is
  kept as a double ECC error (the bench's "a torn word then reads with a double error").
  ECC words, the erase duration and `ch394.silent` carry over power cycles through the new
  emulator's start-up script. An MCU reset (software, the watchdog's) keeps the marks; Renode takes
  the core's watchpoints away at it, and the layer installs them again in its reset, before the core
  runs (the first code of the start sees the error: T-EMU-33). Until 2026-11-24 they came back only at
  the next PG cycle: a read of a marked or torn word after a reset and before the next program showed no
  error (the T-A1-83 and T-A1-92 caveat of the faac704, e524b8c and adb8f45 runs). **Not modelled: the bus error a double error gives on the
  read**: the data reads as stored and only the flags show it. Renode's controller
  watches the bank's writes itself while PG is set, and a watchpoint of ours at the same
  addresses left it blind (no EOP, programming unseen): the ECC watchpoints are lifted
  while PG is set (no ECC detection during a program) and put back 1 us after PG is
  cleared (a read of a marked word within that microsecond shows no error; `flash.ecc` answers when its mark
  is in force, at the next clock step, with `in_force` false if it is not after 3 s, for
  instance while PG stays set). They are installed only from a timer event:
  inside a store of the core, a read of the word in the same translated block crashed the
  emulator (found by T-EMU-25; no crash in the fw-01a108d run). **Not covered: reads of a
  marked word while PG is set, or in the microsecond after it or after a `flash.ecc`** (a driver
  that programs and verifies at once reads there): they show no error and are not counted. The
  runner counts the spans and their instructions (`emulator.ecc_spans_not_covered`,
  result_report.py); the marks of a power cycle or a cut are in from the first instruction
  (T-EMU-26). An erase clears the ECC marks of its range (a cut erase
  those of the fraction done), also in what a power cycle carries over; a program alone keeps a
  mark. Before this a mark stayed for good: T-A1-92's last check and the store write failure at
  the end of T-A1-86 in the fw-01a108d run are that fault.
- **power.cut {after_flash_words | during_erase | at_address}**: the layer stops the core
  at the k-th programmed word, at that fraction of the next erase, or at the programming of
  that word; the runner then power-cycles as for `power.cut {at_us}`. A new `power.cut`
  replaces one still armed (any form). A cut armed for a boot and not fired is dropped at the
  power cycle that ends that boot.
- **power.cut {during_erase | after_flash_words, at_power_on: true}**: the same cut, armed for
  the next power-on (a `power.cycle`, or the start after a cut): the first erase, or the k-th
  programmed word counted from that power-on. It is carried to the next emulator's start-up
  script (`NotifyCut`, `CutDuringErase` or `CutAfterWords`, before the core runs) and used once.
  It does not replace a cut armed for the running boot, and a plain `power.cut` after it does
  not drop it; a second `at_power_on` replaces the first. Refused with `at_us` or `at_address`,
  with neither or both of `during_erase` and `after_flash_words`, f outside 0..1, k below 1, or
  `at_power_on` other than true. Reply `{"ok": true, "value": {"during_erase": f, "at_power_on":
  true}}` (or `after_flash_words`). In the boot it is armed for it is a cut of that boot: a plain
  `power.cut` there replaces it. Self-test T-EMU-31 (`tests/fakebench/cut-at-power-on.json`,
  m1_smoke step [14]).
- **Evidence rule**: `harness/g7250emu/flash_dump_check.py <run dir>` compares the flash
  dump of each boot with flash.log; a cut or ECC result is reported only with it (a build
  before d605292 missed programmed words and placed cuts on the wrong word). flash.fill calls
  (fixture-log.jsonl) are replayed in their order: a filled word that nothing touched after it
  must hold the fill's data. A run without fixture-log.jsonl and with fills (builds before it)
  stays DIFFERENT.
- **CPU speed during an erase**: as at all other times (one instruction per core
  clock cycle, docs/CLOCKS-model.md). During an erase fw-e518331 runs its TX path
  check from RAM: per SR1 poll some 270 instructions and 27 peripheral accesses (MODER,
  OTYPER, ODR, IDR of six GPIO ports, the watchdog, DWT, SR1), each a C# model call, and
  the emulation runs at about 0.22 of real time (a 1.1 s erase in 4.9 s wall). After it
  Renode catches up with the host: 1.1-1.4 of real time for some 20 s wall after one such
  erase, so waits in wall time there see more radio time than they count, and duration_s
  over a whole test can equal wall_s although erases were in it. There is
  no cheap path for these accesses in Renode. The bench's waits across an erase must be
  in emulated time or allow for this.
  `G7250_ERASE_MIPS_DIV=N` (opt-in, off by default) divides the core's MIPS by N during
  an erase for speed; each test with an erase is then marked "not modelled: CPU speed
  during flash work (MIPS/N)" (result reason and `emulator.not_modelled`) and every erase
  entry carries `core_mips_divisor`: no result timed inside an erase counts as evidence.
- **Fetch stalls**: an instruction fetch from the bank during an erase stalls the core
  until the erase ends. The PC is sampled every 10 us of emulated time (a hook at every
  translated block cost about 35 us of host time per poll loop): code fetched from the
  bank during an erase runs up to 10 us before it stalls. Erase entries carry `wall_s`,
  `fetch_stalls`, `sr1_reads` and `instructions`.
- **flash.dump {address, len}** (v1.9): the bank as it is, hex, at most 32768 bytes; refused
  below 0x08020000 and when the runner was started with `--flash-preload`; no ECC event.
- **flash.fill {address, len, data?}** (v1.9): the store sectors (0x080C0000 to 0x080FFFFF)
  only, 32-byte aligned; data repeated over len, 0xFF without data; ECC marks in the range go,
  no flash.log entry, kept over power cycles; refused while the image erases or programs. The
  layer's shadow follows, so the next program entry is the image's own.
- **slot.corrupt**: one byte of slot 1 (0x08040000) or 2 (0x08080000) XORed in the array
  (no ECC error); `restore` XORs it back, also after power cycles.

Exercised by the self-test T-EMU-23 (net_test "F" commands): faults on erase and program,
`after`, the refusals, a never-ending erase ended by the watchdog, cut replacement,
`erase_us` null. `flash.ecc` on reads is exercised by product tests only (T-A1-87).

## Diagnostic flash read log (off by default)

For a diagnosis only, never in a counted run unless the report says so: with
`G7250_FLASH_READLOG=<lo>-<hi>` (hex, inside the bank) in the runner's environment, every
emulator start writes `flash-reads-N.jsonl` into the test's folder (N: the power-on of the
test). Each line is JSON with `t_us`, the emulated time since that power-on, and `op`:

- `start` (the range) and `cr1` 1 us after the power-on and after each MCU reset (`reset`):
  FLASH_CR1 as the bus reads it, `pg`;
- `read`: a read by the core of the range (byte, halfword, word or doubleword, aligned), its
  `address`, `width` in bytes, `value` as the bus holds it (little-endian), `pc`, and
  `ecc_mark: true` when the word carries a `flash.ecc` mark (its data reads as stored);
- `sr1`: every 32-bit FLASH_SR1 read with the value the core gets (also BSY and QW of an erase),
  `dbeccerr`, `pc`; a run of equal values is one line and a `sr1_repeat` line with its count
  (a byte or halfword read of SR1 is not logged and does not get the layer's flags);
- `ccr1` (with `pc`) and `cr1_write` (with `locked`): every write of FLASH_CCR1 and FLASH_CR1;
- `unwatched` / `watched`: the read watchpoints are lifted while CR1.PG is set and put back
  1 us after PG is cleared, as the ECC ones (Renode's controller does not see the program
  through them otherwise); reads between the two lines are not logged. At an MCU reset an
  `unwatched` and a `watched` line with `why` "MCU reset" follow the `reset` line: the watchpoints
  are installed again before the core runs (until 2026-11-24 not until the next PG cycle: no read of
  the range after a reset was logged before the next program).

Nothing of it reaches `flash.log`, the fixtures, the endpoint or the bench. It costs one
watchpoint per aligned address and width (7680 for 4 KiB): keep the range small. Each program
lifts and puts back all of them, about 12 s host per program for 4 KiB, so a boot that fills a
store (about 2000 programs) takes hours. `G7250_FLASH_READLOG_BOOTS=<lo>-<hi>` (or `<n>`)
limits the log to those power-ons of the test (1 is the first emulator start, as N above,
taken from the console file name); the other power-ons run without it and write no file.
Unset, every power-on is logged. With neither variable set, the start-up script is as before.
The self-test T-EMU-30 (`tests/fakebench/flash-readlog.json`, m1_smoke step [13], window 1-2)
runs it on net_test's sector 7 and checks that power-on 3 writes no file.

## Not modelled

PGSERR on a write without PG (Renode lets that write reach the array), the
256-bit write-buffer rules beyond Renode's (all-ones writes, FW, INCERR as
Renode does them), the flash-word programming duration, the bus error of a
double ECC error (an experiment with Renode 1.17's precise bus fault call from a read watchpoint
set CFSR and BFAR but took no exception, also with BUSFAULTENA set; nothing of it is used), ECC
errors of the retained RAM, option bytes programming (refused and logged), bank 2
(absent on the H72x 1 MiB parts).

## Store checks

`harness/g7250emu/store_checks.py <TEST image>` runs ST-01..ST-04 of the
firmware's run sheet with a TEST image that compacts its store at every start,
and reports the image's own console lines, the forced condition and the ST-05
evidence. Default erase duration 4 s, a labelled STAND-IN (longer than the
2 s watchdog) until the datasheet maximum is set.

## A cut takes the state at its instant
Until 8a2a100 a cut halted the core and told the runner, and the emulator ran on until the runner had read
the logs and dumped the flash: an erase went on (and could end: its entry changed from `cut` to `ok`), the
dump took the erased part from the time of the dump, a watchdog reset could start the core again (seen: the
image ran a second erase after the cut; by reading only: during the erase, the entry would become `reset` and
the sector be dumped with its old content), and a core stalled by a fetch ran again when the
erase ended. Now, for power.cut in every form and for power.cycle (its halt, with the machine paused for it):
- `Freeze` copies the 2 MiB of flash in the event that halts the core, with the erased part of a running
  erase up to that instant (whole 32-byte words) at 0xFF, closes its entry as `cut` with `fraction_done`, and
  keeps the ECC marks. Dumps, flash.log and the marks for the next start come from that copy.
- After it: no flash operation starts or ends, a watchdog does not reset, a reset does not start the core, an
  erase's end does not let a stalled core run; the CH394 sends nothing and the agent sends no trace event; the
  runner drops net, socket and console log entries with a later time.
- The copy costs host time only (`cut_copy_host_us` in the result); emulated time does not move in it.
- Limits: an access of the core already in the same translated block may still run after the halt (T-EMU-28
  shows no second flash word is programmed, with the one sequence it tries); a DMA transfer or a SEND already
  started at the cut is not looked at; the emulator itself is not paused at a cut, only at power.cycle.
flash_dump_check compares each cut erase with its recorded fraction where old content is known. A cut erase erased only
the words below its fraction: a word programmed earlier in the same boot and kept above it must still be
logged and changed in the dump (until 2026-11-24 the check left such words out of the logged set and called
them "not logged": the fw-adb8f45 T-A1-92 boot 3 and 4 mismatch). The words at the fraction, within its
tolerance, are left out of both counts. Check case: tests/baseline/dump_check_case.py (m1_smoke step [2i]).
