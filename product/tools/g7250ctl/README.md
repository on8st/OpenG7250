# g7250ctl

Command-line tool for managing the firmware of a G7250 running the new firmware (G7250-P2),
from the computer, without the controller and without AetherSDR.

- Status: **milestones M1 to M6**: `discover` (management broadcast and P2), `info`, `stats`;
  `logs`, `config get`, `calibration get`; the authenticated commands (secrets, `config set`,
  `restart`, `factory-reset`, `switch-level`, `clear-log`, `calibration set`/`clear`); the
  firmware update (`install`, `status`, `abort`, `rollback`, `package`); the return to stock
  (`return-to-stock`, `vendor-recovery`, `install-stock`); the P2 capability record (`caps`).
  Unit tests against a local socket; M1 to M6 were run against the reference model (M5 with
  the F1 gate closed).
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

## Secrets and authenticated commands (A1.10)

- The secret (32 bytes, 64 hex digits) comes from `--secret-file <file>` or the environment
  variable `G7250_SECRET`, never from the command line. The tool never prints or logs it.
- `set-secret --generate <file>` and `change-secret --generate <file>` write a new random
  secret to a new file (an existing file is never overwritten; mode 0600 on Unix) before
  sending it. On Windows the tool does not restrict the file: it takes the access rights of
  its folder, and the tool says so when it writes one. Keep secret files in your own profile
  folder, not in a shared one.
- `SET_FIRST_SECRET` carries the secret in the clear (spec A1.10, known risk): set it on a
  network you trust. `CHANGE_SECRET` sends it masked; afterwards the tool sends
  `END_WRITE_SESSION` signed with the new secret, which checks the change.
- Keep the old secret file until the radio has restarted once since the change. If the
  store falls back to the older generation at a start (spec A1.10 "The old secret in
  flash"; 0069 addendum), the radio is on the older secret again: the new one gets status
  33, and `logs` shows the fall-back line and `store: stale secret erased`.
- Every authenticated request gets its own `GET_CHALLENGE`, from the same socket (the radio
  keeps one challenge per address and port). A retransmission is the identical signed
  datagram. Status 33 is reported, never retried with a new challenge, because the request
  may have been executed.
- The write session belongs to this run's address and port, so every write ends it with
  `END_WRITE_SESSION`; `restart` and `factory-reset` end it by the reset. `end-session` is
  for a session left by another tool on the same address and port; otherwise the radio
  ends it after 60 s idle, and `restart` is accepted from any host in the meantime.
- Replies are not authenticated, so writes are read back: `config set` with `GET_CONFIG`,
  `calibration set`/`clear` with `GET_CALIBRATION`, `set-secret` with `IDENTIFY` (secret set).
- `config set` with a network change: the radio applies it after the reply (A1.6), so the
  read-back goes to the new static address, else to the old one (a fallback, C-4, is
  reported); after a change to DHCP use `discover`. A change of address or netmask ends the
  write session and discards the challenges (A1.10; 0074): then no `END_WRITE_SESSION` is
  sent, and the next write takes a new challenge, as every write does. Without a certain
  change (same address and netmask read back) the session is ended as usual; with none held
  that is answered 0 and changes nothing.
- `config get` shows the SWR limit S2 and the PA temperature limit T3 as "not known" while
  the radio leaves tags 9 and 10 out (no S2 or T3 in the calibration set; Table M-3, 0074);
  `--json` gives null, never 0.
- `factory-reset` asks for confirmation unless `--yes`. `restart --wait` and
  `factory-reset --wait` wait until the radio answers with a lower uptime (30 s; 47 s after a
  factory reset, 0069).

## Firmware update (A1.8; Annex M "Package file")

- `install <addr> <package>…` checks every package file first (length, magic, header
  version, signature length, image SHA-256; 0071) and sends nothing if one fails. It reads
  the running slot (`IDENTIFY` tag 23) and sends the package linked for the other slot (or
  one for either slot); a release ships one per slot (0043).
- Block *n* is image bytes 512 *n* … 512 *n* + 511, each block its own authenticated
  request. Status 64 (store full, nothing taken): the same block is sent again as a new
  request; a second 64 in a row for one block ends the install. Status 53: sent again, at
  most twice.
- A failed `UPDATE_VERIFY` (55): every block is sent again, in order, with new request ids,
  and verified again (`--verify-retries`, default 1; 0067).
- Status 55 to a block is a write failure (A1.8 "Write failure"; 0043): the radio has
  discarded the transfer, so nothing is aborted; the tool ends the write session and reports
  that RX stays off until a restart (`restart`) or a new install.
- Another status that ends the transfer: the tool reads `UPDATE_STATUS`, and sends
  `UPDATE_ABORT` only if a transfer is in progress; the radio then stays in the update state
  until a restart (`abort --restart`, or `restart`). With no transfer, `IDENTIFY` decides: an
  uptime (tag 16) below the one read at the start shows that the radio restarted during the
  transfer (it answers the retransmitted block with 33, because its challenge did not survive
  the reset; RX runs again). Nothing is aborted, and the install is started again. If the radio stops
  answering, the transfer is left as it is: `install --resume` within 10 min continues from
  the blocks received (`UPDATE_STATUS`).
- `--resume` can compare only the block count, because `UPDATE_STATUS` gives no hash or
  version. A transfer of another package with the same number of blocks is therefore
  continued too, and then fails at the verify (55): every block is sent again, the verify
  fails again, and the transfer is aborted. Resume only the install that broke off.
- A request that ends in a reset (`RESTART`, `FACTORY_RESET`, `UPDATE_ACTIVATE`, `ROLLBACK`)
  loses its stored reply in the reset (Table A1-4). The host rule of A1.4 (0074): after a
  lost reply, retransmitting stops at the first 33, and the 33 or the silence is resolved with
  `IDENTIFY`, never with another request. The request counts as done only when the radio
  started after it (uptime, tag 16, below the time since the request or below the one read
  just before it) AND the last reset cause (tag 17) is software; for `UPDATE_ACTIVATE` and
  `ROLLBACK` the running slot (tag 23) must have changed as well; the version is never
  evidence. The log entry just before the start entry (`restart: restart`,
  `restart: factory-reset`, `restart: update-activated ...`, `rollback commanded: ...`) is
  shown as well. Otherwise the 33 (or the silence) is reported with what `IDENTIFY` showed
  (uptime, cause, slot; "cause unknown" when tag 17 is 0), the request is not sent again, and
  the operator decides. After an answered request the request ran: any restart ends the wait,
  and a cause other than software is named. Once
  verified, an install never sends `UPDATE_ABORT`.
- After `UPDATE_ACTIVATE` the tool waits for the reset and reads the result back: the new
  version runs in the target slot, as a trial, until it has confirmed itself (0044), or the
  rollback is reported. `--no-wait` skips this.
- `--downgrade` sets the downgrade flag for an older image (U-8). `rollback` starts the
  image in the other slot (0064) and waits the same way. `package <file>` checks a file
  and shows its header without sending anything.

## Runs against the reference model

`scripts/lossy-relay.py LISTEN TARGET SECONDS` sits between the tool and the model and drops
the first reply to every request once, so that retransmission is exercised (the model's
fixtures delay replies but lose none):

```sh
python3 scripts/lossy-relay.py 22251 21251 120 &
g7250ctl install 127.0.0.1:22251 a.g7pk b.g7pk --secret-file radio.secret
```

## Exit codes

| Code | Meaning |
|---|---|
| 0 | done, status 0 |
| 1 | the radio answered with another status (printed with its Table A1-3 meaning) |
| 2 | bad command line |
| 3 | no reply, or no radio found |
| 4 | a reply that does not decode, or a local error |
| 5 | not confirmed: the request was sent and may have taken effect, but the result could not be read back (`install-stock`: the activation reply was lost and the radio went silent, which is also what a pulled cable or a hung radio looks like) |

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
| `caps <addr> [--raw]` | the P2 capability record: `CAP_REQUEST` to the register port 7250, read-only, starts no session (done, M6) | B4, B8.1 |
| `set-secret <addr> [--generate file]` | set the first management secret, in the window after the first start or a factory reset (done, M3) | A1.10 |
| `change-secret <addr> --new-secret-file f \| --generate f` | change the secret, masked (done, M3) | A1.10 |
| `end-session <addr>` | release the write session (done, M3) | A1.10 |
| `config set <addr> [items]` | change configuration items of Table M-3, read back (done, M3) | A1.6 |
| `calibration set <addr> --source s [--date d] --entry ID=HEX…`, `calibration clear <addr>` | replace or remove the calibration set, read back (done, M3) | A1.12 |
| `restart <addr> [--wait]`, `factory-reset <addr> [--yes] [--wait]` | commanded reset; factory reset of Table A1-4 (done, M3) | A1.9, A1.6a |
| `switch-level <addr> <1\|2>`, `clear-log <addr>` | (done, M3) | A1.6 C-2, A1.7 |
| `install <addr> <package>… [--downgrade] [--resume] [--no-wait]` | read the running slot (`IDENTIFY` tag 23), send the package linked for the other slot, verify, activate; watch the trial boot until it confirms itself (done, M4) | A1.8 (0043, 0067, 0071) |
| `status <addr>` | progress of an update or installation (done, M4) | A1.8 |
| `abort <addr> [--restart]` | abandon an update (not possible once a stock installation has started); with no update in progress the radio answers "restart needed 0" and nothing changes (0074) (done, M4) | A1.8, A1.8a item 3 |
| `rollback <addr> [--no-wait]` | return to the image in the other slot (done, M4) | A1.8 (0064) |
| `package <file>` | check a package file and show its header; sends nothing (done, M4) | Annex M "Package file" |
| `install-stock <addr> <image> --sha256 <hash> [--yes] [--resume] [--save-package file]` | return to the stock firmware: one-way streaming installation of an owner-supplied stock image (done, M5) | A1.8a items 3, 5 |
| `return-to-stock <addr>`, `vendor-recovery <addr>` | `RETURN_TO_STOCK`, `ENTER_VENDOR_RECOVERY`: reserved, always status 56 in this version; the tool says why and what to use instead (done, M5) | A1.8a items 2, 4 |
| `logs <addr> [--from seq] [--follow] [--poll s]` | read the log in pages to its end, the page with flag bit 1 ("more blocks follow") clear (0074); `--follow` keeps reading, across a restart of the radio (log kept: the sequence continues; log lost: from 0 again, 0068); a gap left by the ring is reported (done, M2) | A1.7 |
| `config get <addr>` | every configuration item of Table M-3 (done, M2) | A1.6 |
| `calibration get <addr>` | the calibration set: id, date, source, Table M-5 entries (values in hex until Table P-1 fixes their encodings) (done, M2) | A1.12 |

## Capability record (B4)

- `caps <addr>` writes 1 to `CAP_REQUEST` (0x0100) on the radio's register port (7250, or
  `host:port`) from a local port that is never 7250, because the radio drops packets from
  that port (B8.5). The request does not start or take over a session (B4.5).
- Only the answer counts: a packet from the radio's address and register port with the echo of
  the request and every register up to `cap_length`, each once (B4.4 item 1). A packet without
  the echo (a register push, or a part of the unasked record of a session) is skipped. An answer
  that lacks registers or repeats one is reported as incomplete and is never shown as a record.
- No answer after 1 s: the request is sent again, at most once per second; after 5 the tool
  reports it (B4.4 item 4, exit 3).
- Every field of Table B4-1 (record version 1.1) is shown with its raw register. A field
  beyond `cap_length` is "absent" (B4.10 rule 2); bits and values the spec leaves open are
  shown raw as "not defined"; the registers of a later minor version are listed raw. An unknown
  major version is not read: the tool says the firmware is newer than itself (rule 4). `--raw`
  lists every register; `--json` gives every field's raw value and the registers.

## Return to stock (A1.8a)

- `install-stock <addr> <image> --sha256 <hash>` takes the stock image as the vendor supplies it
  and the SHA-256 the owner has for it. Before anything is sent it checks that the file has that
  hash and fits the application area (sectors 1 to 7, 917504 bytes), and asks for confirmation
  (`--yes` skips it). `--resume` asks too while the installer has not erased anything yet
  (`UPDATE_STATUS` state 3), because the point of no return is still ahead; once it is writing
  (state 4) it does not. It builds the package header itself (Table M-4: component 1, hardware
  0x00C8, version 0, slot 0) and sends `UPDATE_BEGIN` with the "foreign image" flag; the reply
  must say one-way. `--save-package <file>` also writes header and image as a package file.
- Status 56 to `UPDATE_BEGIN`: the gate is closed (until the drill F1 has shown the vendor path,
  A1.8a item 3); nothing changed.
- The blocks, the verify (55: every block again, `--verify-retries`) and `UPDATE_ACTIVATE`
  with the flag repeated; the reply says "permanent" (item 5). Nothing is read back after it,
  because the stock firmware does not speak this channel; a lost activation reply followed
  by 10 s of silence is reported as "the stock firmware has most likely started", with exit
  code 5 (not confirmed), because a pulled cable or a hung radio looks the same.
- A failure before the first erase (`UPDATE_STATUS` state 3): `UPDATE_ABORT`, and the radio
  restarts into this firmware by itself with its secret and configuration (restart needed 0;
  0074). Only an abort answered 0 is reported so; otherwise the tool says the abort was not
  confirmed and what `UPDATE_STATUS` last showed. After the first erase (state 4)
  there is no way back: the tool never aborts (that would get 60), and the installer waits for
  blocks with no time limit; `install-stock --resume` with the same image continues. Keep the
  power on: after a power loss only the vendor's update path on the controller helps. `abort`
  explains a 60 the same way.
- The installer erases the management secret with the first sector; the radio returns to this
  firmware only through the vendor's update path (item 9), with no secret set.

## Not covered

- The first install on a radio that still runs the stock firmware: that goes once through the
  vendor's update path on the controller (decision 0028, spec A1.8a item 9).

## Licence

GPL-3.0-or-later, © 2026 ON8ST (decisions 0018, 0026).
