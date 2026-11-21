# Flash model (Main)

Renode 1.17's `STM32H7_FlashController` (flash interface at 0x52002000) plus
`models/csharp/G7250FlashTiming.cs`, a layer through bus hooks on that
controller, loaded by `product_run.py` (per-boot repl with the erase
duration). Sources: ST RM0468 (public), flash chapter; the durations shall come
from the STM32H725/735 datasheet (DS13311, "Flash memory characteristics"),
which is **not yet available here**: the durations are parameters.

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
  single-bank H72x; timers, the watchdog and DMA keep running. Each such fetch
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

## Not modelled

PGSERR on a write without PG (Renode lets that write reach the array), the
flash-word programming duration, ECC (no ECC errors on reading half-erased or
half-programmed words: store check ST-06 cannot show an ECC fault), option
bytes programming, bank 2 (absent on the H72x 1 MiB parts).

## Store checks

`harness/g7250emu/store_checks.py <TEST image>` runs ST-01..ST-04 of the
firmware's run sheet with a TEST image that compacts its store at every start,
and reports the image's own console lines, the forced condition and the ST-05
evidence. Default erase duration 4 s, a labelled STAND-IN (longer than the
2 s watchdog) until the datasheet maximum is set.
