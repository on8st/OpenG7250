# g7250ctl

Command-line tool for managing the firmware of a G7250 running the new firmware (G7250-P2),
from the computer, without the controller and without AetherSDR.

- Status: **planned. No code yet.** The project is in the spec phase; code starts in the build
  phase (decision 0028, Decision 4).
- Protocol: the management protocol of `product/spec/spec/network-interface.md` part A
  (A1.1a, A1.3–A1.10, A1.8, A1.8a), UDP 7251. This tool is its reference implementation; any gap
  found while building it is fixed in the spec first (decision 0026).
- Tests: every A1 conformance test of Annex X3, first against the reference model in
  `product/refmodel/` (decision 0019), then against the radio.
- AetherSDR's firmware page follows on the same protocol (`product/spec/spec/annex-aethersdr.md`,
  Q-O4).

## Language (decision 0037, proposed)

Rust, sharing the protocol crate with the firmware (not with the reference model); one static binary
for Linux, macOS and Windows.

## Planned commands

| Command | What it does | Spec |
|---|---|---|
| `discover` | find G7250s on the local network | A1.5 |
| `info <addr>` | model, MAC, firmware versions (running slot, other slot, boot stage), update state | A1.5 |
| `set-secret <addr>` | set the first management secret, in the window after a factory reset | A1.10 |
| `install <addr> <package-A> <package-B>` | read the running slot (`IDENTIFY` tag 23), send the package linked for the other slot, verify, activate; trial boot follows | A1.8 (0043) |
| `status <addr>` | progress of an update or installation | A1.8 |
| `abort <addr>` | abandon an update (not possible once a stock installation has started) | A1.8, A1.8a item 3 |
| `rollback <addr>` | return to the image in the other slot | A1.8 |
| `install-stock <addr> <image> --sha256 <hash>` | return to the stock firmware: one-way streaming installation of an owner-supplied stock image | A1.8a item 3 |
| `logs <addr>` | read the log | A1.7 |

`install-stock` asks for an explicit confirmation, because it cannot be undone once the first
sector is erased: a broken transfer then needs the vendor's update path through the controller.
The tool resumes a broken transfer as long as the installer still runs.

## Not covered

- The first install on a radio that still runs the stock firmware: that goes once through the
  vendor's update path on the controller (decision 0028, spec A1.8a item 9).

## Licence

GPL-3.0-or-later, © 2026 ON8ST (decisions 0018, 0026).
