# Test list and result format (proposal, v1)

- Status: **proposal** from openg7250-tb, 2026-11-20. Agree it with the emulator side
  through the coordinator (openg7250) before anyone builds on it.
- Purpose: the file that `product_run.py <elf> <tests> -o <result>` reads (`<tests>`), and the file
  it writes (`<result>`). The same two formats are used by the test bench runner against the
  reference model and against the radio, so that results from all three can be compared per
  test id.
- Clean-room note: both files carry spec ids, test ids and observations of the network and of
  the emulated hardware pins only. They carry nothing about how the stock firmware behaves.
  The test code stays in `testbench/`. The emulator side runs it as a subprocess and does not
  import it.

## 1. Division of work

| Side | Does |
|---|---|
| `product_run.py` (emulator) | boots `<elf>` on the emulated main unit; puts the radio's network on a link the test host can reach; offers **fixtures** (section 4) on a local control socket; for each test in `<tests>`, starts the test bench as a subprocess with an endpoint file (section 3); collects the per-test JSON it prints; writes `<result>` |
| test bench (`testbench/`) | implements every test id; talks to the radio only over the network (UDP 1024–1035, 7250, 7251) and, where a test declares it, through fixtures; prints one result object per test |

Command line of one test run, as the runner calls it:

```
python3 -m testbench run --endpoint <endpoint.json> --test T-B3-01 [--test …] --json
```

The test bench writes one JSON object per test (JSON Lines) to stdout, in the result-entry form
of section 5. Exit code 0 means it ran (whatever the verdicts); anything else means a test bench
error, which the runner records as `error` for the tests not reported.

## 2. Test list file (`<tests>`)

UTF-8 JSON. The test bench generates it from its catalogue (`testbench/catalogue/`), filtered by
target. The runner can also take a hand-written subset.

```json
{
  "format": "g7250-testlist",
  "version": 1,
  "spec":  { "branch": "spec/draft", "commit": "4551925", "release": "rc4" },
  "his":   { "commit": "9d10e72" },
  "catalogue": { "commit": "<tb/main commit>", "sha256": "<of catalogue.json>" },
  "defaults": { "timeout_s": 30 },
  "tests": [
    {
      "id": "T-B3-01",
      "title": "Broadcast discovery yields exactly one reply",
      "reqs": ["B3.2"],
      "target": "emulator",
      "scope": "v1.0",
      "needs": ["net.broadcast"],
      "timeout_s": 5,
      "params": {}
    },
    {
      "id": "T-B10-01L",
      "title": "TX locked: every TX request rejected, safe state held",
      "reqs": ["B10", "B10.2 item 3", "A3.6", "HIS 85"],
      "target": "emulator",
      "scope": "v1.0",
      "needs": ["hw.observe"],
      "timeout_s": 20,
      "params": { "requests": ["arm", "ptt", "cwx", "atu_tune"] }
    }
  ]
}
```

| Field | Required | Meaning |
|---|---|---|
| `id` | yes | spec test id (Annex X3, part C C9) or a test bench id of the form `T-<group>-<nn><suffix>` for a test the bench adds (suffix letter, e.g. `L` for the TX-locked variants); stable |
| `title` | yes | one line, for logs |
| `reqs` | yes | spec requirement ids the test proves (section numbers such as `B3.2`, `A1.4`; decision ids such as `0032`; HIS sections such as `HIS 85`) |
| `target` | yes | the minimum environment: `host`, `emulator` or `needs-radio` (section 6) |
| `scope` | yes | `v1.0`; `not-v1.0` (for example 0030); `after-G6` (level 2); `from-M2` (wideband). The runner runs only `v1.0` unless told otherwise |
| `needs` | no | fixtures the test uses (section 4). If the runner lacks one, the result is `skip` with reason `fixture:<name>` |
| `timeout_s` | no | wall-clock limit at time scale 1; the runner scales it by the time-scale fixture |
| `params` | no | test parameters; opaque to the runner |

## 3. Endpoint file (runner → test bench)

```json
{
  "format": "g7250-endpoint", "version": 1,
  "kind": "emulator",
  "radio": { "ip": "192.168.73.10", "netmask": "255.255.255.0", "mac": "02:00:00:00:72:50" },
  "hosts": [
    { "ip": "192.168.73.20", "in_subnet": true },
    { "ip": "192.168.73.21", "in_subnet": true },
    { "ip": "10.99.0.20",   "in_subnet": false }
  ],
  "broadcast": { "limited": true, "directed": "192.168.73.255" },
  "control": { "transport": "tcp", "address": "127.0.0.1:7290" },
  "fixtures": ["net.broadcast", "time.scale", "power.cycle", "hw.observe"],
  "time_scale": 1.0
}
```

- `hosts`: source addresses the test bench may bind. Tests with two hosts need two (different
  ports on one address count as different hosts, A2.2, but some tests need a second IP); tests
  about B11.15 and A1.10 S4 need one outside the subnet. Missing ones lead to `skip`.
- `kind`: `refmodel`, `emulator` or `radio`. The test bench uses it only for reporting and for
  tolerances that the spec states per environment (for example B11.6a: "the reference model
  applies the rule exactly").

## 4. Fixtures (control socket)

The control socket carries JSON Lines, one request and one reply per line:
`{"op": "<name>", "args": {…}}` → `{"ok": true, "value": …}` or `{"ok": false, "error": "…"}`.
A runner offers any subset and lists it in `fixtures`.

| Fixture | Ops | Used for |
|---|---|---|
| `net.broadcast` | none (capability flag) | B3.2 limited and directed broadcast |
| `net.hosts` | none (flag: `hosts` has ≥ 2 in-subnet addresses and 1 outside) | takeover, A1.10 S4, B11.15 |
| `link.set` | `link.set {speed: 0\|10\|100}` | B5.8, B5.9, B5.13 (0 = link down) |
| `power.cycle` | `power.cycle {}`; `power.cut {at: "<event>"}` | A1.8 U-5, T-A1-08, T-B3-09, token after power cycle |
| `reset.wdt` | `reset.wdt {}` (processor watchdog reset) | A3.5 `WDT_RESET_AT_START`, T-A3-38 |
| `time.scale` | `time.advance {s: n}` or a fixed scale | 10 min update abandonment, 300 s window, 5 min TX time |
| `sensor.set` | `sensor.set {name, value}`; names: `fwd_power_mw`, `rev_power_mw`, `pa_temp_mc`, `supply_mv`, `current_ma`, `fwd_decay {tau_ms}` | Table A3-2 rows, RF-gone waits, A3.4c |
| `hw.observe` | `hw.observe {}` → snapshot of the HIS 85 safe-state items (below); `hw.trace {start\|stop}` → time-stamped changes | the TX-locked tests: safe state held under every TX request |
| `ctrl.set` | `ctrl.set {state: "absent"\|"silent"}` | 0023 item 4, T-A1-57 |
| `dhcp.server` | `dhcp.offer {…}`, `dhcp.nak {}`, `dhcp.expire {}` | A1.11a, T-A1-35 |

`hw.observe` snapshot. Keys and values are taken from the HIS 85 safe-state table only:

```json
{ "t_us": 123456,
  "tr_lines": { "PF4": 1, "PF6": 0, "PC7": 0, "PH11": 0 },
  "spi1_a": { "oe_PA10": 1, "last_frame": null },
  "spi1_b": { "oe_PH10": 1, "last_frame": null },
  "atu_latch_writes": { "PD13": 0, "PG2": 0, "PG5": 0 },
  "pa_dac": { "reset_PB14": 0, "ch": [null, null, null, null, null, null, null, null] },
  "codec_reset_PB10": 0, "PD12": 0 }
```

`last_frame` is the last byte latched into chain A (or chain B's 3 bytes), `null` if never latched.
`ch` holds the last 10-bit word per DAC channel, `null` if never written. `atu_latch_writes`
counts writes since boot. The test bench checks: T/R lines at RX levels, chain A bit 7 clear in
every latched frame, bias channels 2, 7 and 8 at 0 or never written, ATU latches never written
(0032), codec in reset (0031).

## 5. Result file (`<result>`)

```json
{
  "format": "g7250-testresult", "version": 1,
  "testlist": { "sha256": "…" },
  "subject":  { "kind": "emulator", "elf_sha256": "…", "fw_version": "0.1.0+12" },
  "runner":   { "name": "product_run.py", "version": "…", "emulator_commit": "…" },
  "testbench": { "commit": "…" },
  "started": "2026-11-20T21:00:00Z", "finished": "2026-11-20T21:04:10Z",
  "results": [
    { "id": "T-B3-01", "status": "pass", "duration_s": 0.21,
      "reqs": ["B3.2"], "checks": [ {"name": "one reply", "ok": true, "detail": "1 reply in 1.0 s"} ] },
    { "id": "T-A3-11", "status": "skip", "reason": "fixture:sensor.set" },
    { "id": "T-A1-29", "status": "blocked", "reason": "TBD:HIS-07,HIS-12 (P_rf_off, t_rf_off)" }
  ],
  "summary": { "pass": 0, "fail": 0, "skip": 0, "blocked": 0, "error": 0, "not_run": 0 }
}
```

| `status` | Meaning |
|---|---|
| `pass` / `fail` | verdict; a `fail` lists at least one check with `ok: false` |
| `skip` | not run: fixture or host address missing, or scope not selected (`reason` says which) |
| `blocked` | cannot be decided yet: a value the test needs is TBD in the spec (`reason`: `TBD:<HIS ids>`) |
| `error` | the test or the runner broke (exception, timeout); `reason` holds the message |
| `not_run` | listed but never started (runner aborted) |

Runner exit code: 0 if there is no `fail` and no `error`, 1 if there is a `fail`, 2 if there is an `error`.

## 6. Targets

| `target` | Runs where | Example |
|---|---|---|
| `host` | on the host alone, against the reference model, or checks host software (g7250ctl, an AetherSDR backend) and spec artefacts; no firmware | HMAC and masking vectors, conditions the spec marks "reference model" that an emulator cannot produce |
| `emulator` | network black box against the firmware in the emulator, and also against the reference model; fixtures as declared | discovery, register map, write rejections, the TX-locked state |
| `needs-radio` | the real radio, under the safety rules of HIS 90 (dummy load on both ports) | RF measurements, real link renegotiation, ICMP behaviour of the CH394Q, relay timing |

A test with target `emulator` is also run against the reference model. A test with target
`host` is not sent to `product_run.py`.

## 7. Open points for the emulator side

1. Can the emulator give the test host two in-subnet source IPs and one outside the subnet
   (a tap with aliases and a route)? Without them about 40 tests skip.
2. Which fixtures of section 4 are realistic for T1. The minimum for the TX-locked tests is
   `hw.observe`; for the safety tests later, `sensor.set` and `time.scale`.
3. Are the pin names of `hw.observe` (HIS 85) the right granularity, or would the emulator
   rather report peripheral writes (SPI1/SPI2 frames, GPIO set and reset) for the test bench to
   interpret?
