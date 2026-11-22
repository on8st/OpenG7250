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
  emulator's start-up script. **Not modelled: the bus error a double error gives on the
  read**: the data reads as stored and only the flags show it. Renode's controller
  watches the bank's writes itself while PG is set, and a watchpoint of ours at the same
  addresses left it blind (no EOP, programming unseen): the ECC watchpoints are lifted
  while PG is set (no ECC detection during a program) and put back 1 us after PG is
  cleared (a read of a marked word within that microsecond shows no error; an ECC word that
  `flash.ecc` adds is in force at the same point). They are installed only from a timer event:
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
  replaces one still armed (any form).
- **Evidence rule**: `harness/g7250emu/flash_dump_check.py <run dir>` compares the flash
  dump of each boot with flash.log; a cut or ECC result is reported only with it (a build
  before d605292 missed programmed words and placed cuts on the wrong word).
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
- **slot.corrupt**: one byte of slot 1 (0x08040000) or 2 (0x08080000) XORed in the array
  (no ECC error); `restore` XORs it back, also after power cycles.

Exercised by the self-test T-EMU-23 (net_test "F" commands): faults on erase and program,
`after`, the refusals, a never-ending erase ended by the watchdog, cut replacement,
`erase_us` null. `flash.ecc` on reads is exercised by product tests only (T-A1-87).

## Not modelled

PGSERR on a write without PG (Renode lets that write reach the array), the
256-bit write-buffer rules beyond Renode's (all-ones writes, FW, INCERR as
Renode does them), the flash-word programming duration, the bus error of a
double ECC error, option bytes programming (refused and logged), bank 2
(absent on the H72x 1 MiB parts).

## Store checks

`harness/g7250emu/store_checks.py <TEST image>` runs ST-01..ST-04 of the
firmware's run sheet with a TEST image that compacts its store at every start,
and reports the image's own console lines, the forced condition and the ST-05
evidence. Default erase duration 4 s, a labelled STAND-IN (longer than the
2 s watchdog) until the datasheet maximum is set.
