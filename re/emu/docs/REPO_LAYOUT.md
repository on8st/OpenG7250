# Layout

The emulator is the `re/emu/` component of the OpenG7250 umbrella (historically
the stand-alone `g7250-emu` repository; see "Historical names" below).
Its internal structure follows spec §13 (`G7250-emulator-test-spec-v0.5.md`).
Generated artifacts go in `re/emu/.private/` (git-ignored); locally installed
tooling in `re/emu/tools/` (git-ignored); decrypted images never enter any
repo (see `SECURITY.md`).

## Place in the umbrella

The former separate repositories map onto the umbrella
as follows (see the umbrella `README.md` for the full layout):

| Former repo | Umbrella path | Side | Role |
| --- | --- | --- | --- |
| `g7250-base` | `re/analysis/` | RE | The umbrella's own RE findings, consumed here as data (`deps/re-analysis.lock`). `g7250-base` itself (fork of `fventuri/g7250-base`) was the data pin until 2026-11-19 and is now an external comparison source, not a part of the umbrella. |
| `g7250-emu` | `re/emu/` (**this component**) | RE | Emulator, harness, tests, hash manifest |
| `g7250-his` | `his/` | Gate | Clean-room hardware-interface spec, the only hardware source for the firmware. Migrated into the umbrella on the stacked PR #4 (branch `his/integrate`), content cleared at the gate (review 5). |
| `g7250-fw` | `product/firmware/` | Product | Clean-room firmware (headless Main first). Never checked out in an RE work tree. |
| — | `testbench/` (planned) | Shared | Black-box tests against the specs |

## Historical names

Older documents, source comments and the frozen test spec v0.5 still use the
names from before the umbrella. They are kept where they describe history and
mean the following:

| Historical name | What it was | Current place |
| --- | --- | --- |
| `g7250-emu` | Stand-alone emulator repository (tag `emu-v1`) | `re/emu/` (imported with its history) |
| `g7250-base` | Data pin until 2026-11-19: fork of `fventuri/g7250-base`, branch `re/round5` (commit in `deps/re-analysis.lock`) | `re/analysis/` for the umbrella's own findings. A citation "g7250-base `<path>`" in a model, platform or doc refers to that pinned snapshot, which is now an external comparison source. |
| `g7250-his` | Separate hardware-interface-spec repository | `his/` in the umbrella (PR #4, branch `his/integrate`) |
| `g7250-fw` | Firmware repository | `product/firmware/` (product side, never in an RE work tree) |
| Local checkouts of the above | Working copies on the RE machine | Worktrees of the umbrella; local paths are not recorded in tracked files |

## Component structure (§13.1)

```
re/emu/
├── README.md CLEANROOM.md SECURITY.md LICENSE AGENTS.md
├── deps/re-analysis.lock        pinned re/analysis commit (consumed as data)
├── images/                      manifest.yaml (provenance hashes) + verify.py
├── platforms/                   .resc / .repl (Main, Control, overlays, SLIP)
├── models/                      csharp/ (Renode peripherals), devices/, system/
├── harness/g7250emu/            Python: bootcore, runners, observe, imagemap
├── firmware-tests/              own small clean-room test firmwares (incl. M0)
├── tests/baseline/              m1_smoke, boot_milestone, fpga_boot, acceptance_ae
├── docs/                        design docs, this spec, status reports, architecture
├── local.config (git-ignored)   image-map path, image hashes, HIS location
├── tools/       (git-ignored)   renode-1.17.0/  pinned Renode portable (arm64)
└── .private/    (git-ignored)   runs, logs, recordings, heatmaps, build output
```

Directories from §13 not yet populated (`matrix/`, `ci/`, further `tests/`
suites) are created when the work that needs them begins (see spec §14).

## Key artifacts

- `harness/g7250emu/`: `bootcore`, `m1_run` (`--fpga`), `superloop_run`
  (test A), `fpga_nco_probe` (HIS-02), `control_run`, `slip_run`, `r0_confirm`, `observe` (strict / hang / heatmap), `imagemap`,
  `platform_extras`.
- `models/csharp/`: `G7250Slaves` (EEPROM, CH394), `G7250Gpioe` (fused
  GPIOE+FPGA), `G7250Mdma` (MDMA), `G7250PaPlant` (PA bias plant and metering
  feed), `G7250Dma` (DMA1/DMA2), `G7250Usart` (USART2 with DMA requests).
- `models/devices/`, `models/system/`: host-tested device, plant and power
  logic.
- `platforms/*.repl`: `g7250_main`, `g7250_control`, the I2C/CH394/FPGA
  overlays, SLIP.

Milestone history (M0 spike, M1 scaffolding, M2, D2, M8c) is in the status
reports under `docs/`.
