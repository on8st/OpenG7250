# g7250ctl

Command-line tool for managing the firmware of a G7250 running the new firmware (G7250-P2),
from the computer, without the controller and without AetherSDR.

- Status: **milestones M1 and M2**: `discover` (management broadcast and P2), `info`, `stats`;
  `logs`, `config get`, `calibration get`. Unit tests against a local socket; the run against
  the reference model is next.
- Protocol: the management protocol of `product/spec/spec/network-interface.md` part A
  (A1.1a, A1.3–A1.10, A1.8, A1.8a), UDP 7251. This tool is its reference implementation; any gap
  found while building it is fixed in the spec first (decision 0026).
- Tests: the host-side checks of T-A1-93 (decision 0070): `g7250ctl` drives every command of
  Table A1-2 against the reference model in `product/refmodel/` (decision 0019), and its own
  rules (request ids, retransmission, authentication, the update sequence, package checks) are
  tested there. The T-A1 tests themselves are tests of the radio; `g7250ctl` may be the host
  that runs them.

## Build and test

```sh
cargo build --release     # target/release/g7250ctl
cargo test                # unit tests and the local-socket tests in tests/socket.rs
```

Its own cargo workspace; the protocol crate is `../../firmware/crates/proto` (`g7250-proto`).
Static builds: `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`, Windows
`x86_64-pc-windows-msvc` with `-C target-feature=+crt-static`; on macOS the binary is
self-contained but links the system libraries (libSystem), as every macOS binary must.

## Transport (A1.4, 0069)

- A new request id for every request, increasing by one from a random start.
- No reply within 500 ms: the identical datagram is sent again (same id, payload and HMAC
  trailer), so the radio answers from its stored reply and does not execute it twice.
- An ordinary request is retransmitted 5 times. A request that may do flash work
  (`SET_CONFIG`, `SET_FIRST_SECRET`, `CHANGE_SECRET`, `FACTORY_RESET`, `SET_CALIBRATION`,
  `CLEAR_CALIBRATION`, `UPDATE_BEGIN`, `UPDATE_BLOCK`, `UPDATE_ACTIVATE`, `ROLLBACK`) keeps
  being retransmitted every 500 ms for at least 16 s (0069 item 4).
- Only a reply from the address the request went to, with the same request id and command,
  is taken; of several copies the first is used.
- `--interval` and `--retries` change the timing for tests; the 16 s flash-work window stays.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | done, status 0 |
| 1 | the radio answered with another status (printed with its Table A1-3 meaning) |
| 2 | bad command line |
| 3 | no reply, or no radio found |
| 4 | a reply that does not decode, or a local error |

`--json` prints JSON on stdout for every command; errors go to stderr.
- AetherSDR's firmware page follows on the same protocol (`product/spec/spec/annex-aethersdr.md`,
  Q-O4).

## Language (decision 0037)

Rust, sharing the protocol crate with the firmware (not with the reference model); one static binary
for Linux, macOS and Windows.

## Planned commands

| Command | What it does | Spec |
|---|---|---|
| `discover [--wait s] [--broadcast addr]… [--p2]` | find G7250s on the local network: `IDENTIFY` to 255.255.255.255:7251 and any `--broadcast` address; `--p2` uses P2 discovery on port 1024 and lists any P2 radio (done, M1) | A1.5, B3.2 |
| `info <addr>` | model, MAC, firmware versions (running slot, other slot, boot stage), update state, owner, every tag of Table M-2 (done, M1) | A1.5 |
| `stats <addr>` | the `GET_STATS` counters (done, M1) | A1.7, B11 |
| `p2 caps <addr>` | the capability record (planned, M6) | B4 |
| `set-secret <addr>` | set the first management secret, in the window after a factory reset | A1.10 |
| `install <addr> <package-A> <package-B>` | read the running slot (`IDENTIFY` tag 23), send the package linked for the other slot, verify, activate; trial boot follows | A1.8 (0043) |
| `status <addr>` | progress of an update or installation | A1.8 |
| `abort <addr>` | abandon an update (not possible once a stock installation has started) | A1.8, A1.8a item 3 |
| `rollback <addr>` | return to the image in the other slot | A1.8 |
| `install-stock <addr> <image> --sha256 <hash>` | return to the stock firmware: one-way streaming installation of an owner-supplied stock image | A1.8a item 3 |
| `logs <addr> [--from seq] [--follow] [--poll s]` | read the log in pages to its end; `--follow` keeps reading, across a restart of the radio (log kept: the sequence continues; log lost: from 0 again, 0068); a gap left by the ring is reported (done, M2) | A1.7 |
| `config get <addr>` | every configuration item of Table M-3 (done, M2) | A1.6 |
| `calibration get <addr>` | the calibration set: id, date, source, Table M-5 entries (values in hex until Table P-1 fixes their encodings) (done, M2) | A1.12 |

`install-stock` asks for an explicit confirmation, because it cannot be undone once the first
sector is erased: a broken transfer then needs the vendor's update path through the controller.
The tool resumes a broken transfer as long as the installer still runs.

## Not covered

- The first install on a radio that still runs the stock firmware: that goes once through the
  vendor's update path on the controller (decision 0028, spec A1.8a item 9).

## Licence

GPL-3.0-or-later, © 2026 ON8ST (decisions 0018, 0026).
