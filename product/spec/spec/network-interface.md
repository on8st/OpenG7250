# G7250 network interface specification

- Version: rc4, 2026-11-19; changes in `spec/CHANGELOG.md`
- Status: release candidate rc4, for review by ON8ST. Brings the text in line with decisions
  0003, 0004, 0006, 0007 and 0008, all accepted by ON8ST on 2026-11-19, and applies the
  detail choices of 0020 (proposed, for ON8ST review at rc1), the points of 0021 that concern
  parts A and B, and the review fixes of 0022 (proposed, for ON8ST review at rc1), including
  §22–§32 from a second review of the transmit side, §33–§51 from the final review of rc1,
  §52–§61 from the review of the rc2 draft, §62–§73 from the review of rc2, §74–§77 from the
  review of the rc3 draft, §78–§83 from the review of rc3, §84–§86 from the review of
  the rc4 draft and §87 and §88 from the reviews of rc4, and, after ON8ST's review of rc4
  (2026-11-20), the scope statement for the controller (0023) and the return to stock
  firmware (0024), and, after rc5, one installation path on the main unit without the
  controller (0025, proposed). Focus: milestone M1 (discovery, capability block, RX IQ on G7250-P2); sections for M2–M5 are complete except for
  values that depend on the hardware.
- © 2026 ON8ST, CC BY 4.0 (see `LICENSE.md`).
- Structure: decision 0001, "(2) Network-interface spec", as revised by 0016, split into parts
  A, B and C by 0011.

Sources are cited by key (see `sources.md`) or by decision number (see `decisions/`).
Citations of [P2] give the packet name and byte number of [P2] V4.4. A value that depends on
a value that only the real radio can settle is marked "pending hardware" with its parameter id
from Annex P and its HIS id, for example "pending hardware (P-RF-2; HIS-07)", and a value that
waits for the level-2 feasibility gate is marked "pending G6" (0035; see `open-questions-his.md`). Every other value is fixed by a decision; detail choices are cited
as "0020 §n" and "0022 §n". Register names in `CODE` style have their addresses in B8 (Table B8-2).

The key words of B1.2 apply to the whole document, parts A, B and C.

---

## Part A — Common to both levels

Part A holds what applies to every level (0011, Consequences): the management channel, level
selection and ownership, and the safety behaviour that a host can see on the network.

**Scope of the firmware** (0023, scope accepted by ON8ST 2026-11-20). This document specifies
the firmware of the G7250 main unit only. The controller (head) keeps its stock firmware; the
G7250 MUST NOT send anything over CTRL that is meant to change, erase or update the
controller's firmware, and MUST NOT send anything over CTRL that could damage the controller
or leave it unable to start with the stock main-unit firmware (0023 items 2, 3). Levels 1 and 2
MUST work with the controller disconnected or not responding (0023 item 4). The main unit leaves
CTRL idle towards the stock controller ([HIS] HIS 70); how the controller's panel behaves with a
silent main unit is pending hardware (P-SYS-4; HIS-17); a controller in an unknown or error state is accepted (0023 item 5). The G7250
leaves CTRL idle and does not use the controller as a front panel (no display, buttons, audio
or PTT over CTRL), on either level, unless a later decision changes it (0023, note of
2026-11-20). Only the first install, and a re-install after a return to stock, needs the
controller (A1.8a item 9; 0028). The FPGA configures itself from its own flash, and this
firmware neither loads nor replaces it ([HIS] HIS 15; 0028). The owner can always return to
the stock firmware (A1.8a; 0024, 0028). The product is headless: no management command
switches the radio off, and it is switched off by removing power (0029). While no session runs
on either level, the radio is in a quiet idle state: RF section in its safe state ([HIS]
HIS 85), no streams, and the management channel, discovery and TCI listening as usual (0029).
A standby with less power draw is a later option (P-SYS-6). How the main unit powers up
without the controller is pending hardware (P-SYS-3; HIS-17).
In v1.0 the local PTT and KEY inputs of the main unit are not used (0030), and the main unit
carries no audio (0017, 0031).

### A1 Management channel

This section fixes the requirements of the management channel: port, framing, commands,
status codes and timeouts (0008, Consequences; 0020 §2, §24, §27). Payload layouts of the
individual commands are fixed with the reference model (0019) before M5 and do not change the
framing (0020 §27).

A1.1 **Purpose.** The management channel carries identification, configuration, logs and
firmware update, on a connection separate from G7250-P2 and TCI (0001 spec section 10; 0008).

A1.1a **Firmware management lives on the host side** (0026). The radio has no user interface
for firmware management: host software drives it over this channel, with the protocol of
A1.3–A1.10 as the only protocol. Its reference implementation is a standalone command-line
tool, `g7250ctl` (planned at `product/tools/g7250ctl/`, GPL-3.0-or-later): identify, show version
and state, install an image (new firmware or an owner-supplied stock image, A1.8a), verify,
activate, abort, rollback, return to stock (the streaming installation of A1.8a item 3), read
logs, set the first secret. It MUST pass every A1 conformance test of Annex X3 against the reference model (0019).
AetherSDR's firmware page uses the same protocol and follows the CLI (annex Q-O4). The very first
install, on a radio still running stock firmware, cannot use this protocol; it goes once through
the vendor path on the controller (A1.8a item 9; 0028).

A1.2 **Transport.** The management channel MUST use its own UDP port, served by the eighth
socket of the CH394Q (0008, Decision, Transport 1; 0017). That socket is reserved at both
levels, MUST be open whenever the network is up, and MUST NOT be closed to make room for
another level (0008, Transport 1; Socket budget). The one exception is the DHCP rule of
A1.11: while an exchange borrows the socket, at most 2 s at a time, management requests are
lost and the host's retransmission (A1.4) covers the gap (0008, Socket budget; 0022 §73
item 17; 0041).

A1.3 **Port number.** The management channel uses local UDP port **7251** (0020 §2). This
lies outside 1024–1029, outside the wideband default ports 1027–1034 and outside the DDC
range 1035–1114 ([P2] General packet bytes 17–22; 0008, Transport 2), and differs from the
TCI port 50001 (0021 §2). If a General packet asks to move a P2 port onto the management
port, the G7250 MUST NOT give the management socket away; it MUST keep the default for that
P2 port and log the conflict (0008, Transport 2; B2.3; Annex X1, D21, D22). The port is
announced in the capability record field `mgmt` (B4, 0003).

A1.4 **Framing.** The channel is request/response. Every request MUST carry a request id;
every reply MUST echo it and carry a status code. Large transfers MUST be split into numbered
blocks, each acknowledged; the host retransmits on a timeout (0008, Transport 3). The channel
has its own protocol version, reported in identification, independent of the P2 version and
of the capability-record version (0008, Transport 4). Every message is one UDP datagram,
big-endian, with the header of Table A1-1 (0020 §27).

Table A1-1 — Management message header (0020 §27)

| Bytes | Field |
|-------|-------|
| 0–3 | magic 0x47374D47 ("G7MG") |
| 4, 5 | protocol version major, minor; 1, 0 in this version |
| 6 | type: 1 request, 2 reply |
| 7 | flags: bit 0 authenticated (HMAC trailer present); bit 1 more blocks follow |
| 8–11 | request id, chosen by the host, echoed in the reply |
| 12–13 | command (Table A1-2) |
| 14–15 | status (Table A1-3); 0 in requests |
| 16–17 | payload length *N* |
| 18 … 17 + *N* | payload |
| next 32 bytes | HMAC-SHA256 [RFC2104], [FIPS180-4] over the 16-byte challenge followed by bytes 0 … 17 + *N* (Annex M); present only if flag bit 0 is set |

- A request MUST NOT exceed 1000 bytes, so that it fits a 1 KB socket receive buffer with the
  8-byte CH394Q receive header ([CH394] section 12.2.4); a reply MUST NOT exceed 1400 bytes;
  an update block carries 512 data bytes (0020 §27). A request whose header declares more
  than 1000 bytes (18 + *N*, plus 32 with flag bit 0 set) MUST be answered with status 19 and
  not executed, decided from the header alone and before the length check below, because a
  longer datagram may arrive truncated by the receive buffer; a datagram longer than 1000
  bytes whose *N* declares 1000 bytes or less fails the length check (status 17) (0022 §77
  item 6).
- A repeated request from the same host (IP address and port) MUST be answered with the
  stored reply and MUST NOT be executed again (0020 §27). A request is a repeat only if its
  request id, its command (bytes 12–13) and the SHA-256 hash [FIPS180-4] of its payload
  (bytes 18 … 17 + *N*) all equal those of the stored reply's request; a request with the
  same id but another command or payload is a new request, is executed and replaces the
  stored reply (0022 §83 item 3). A repeat answered from the stored reply does not refresh
  the write session (A1.10; 0022 §86 item 7). For a commanded reset the reply is stored when
  it is sent at step 3 of A1.9, and the status 1 and 35 answers of "One commanded reset at a
  time" (A1.9) are not stored (0022 §87 item 3). The G7250 keeps the last reply per
  host for 5 s, for up to 4 hosts (IP address and port), evicting the least recently used;
  after expiry a repeated authenticated request fails with status 33, because its challenge
  is spent (0022 §11).
- A host SHOULD retransmit after 500 ms without a reply, at most 5 times (0020 §24).
- A datagram with another magic MUST be dropped without reply. A datagram with the right
  magic whose type (byte 6) is not 1 (request) MUST be dropped without reply and counted, so
  that two radios never answer each other's replies (0022 §73 item 20). A datagram with the right
  magic but shorter than 18 bytes, or whose length does not match *N*, MUST be answered with
  status 17 if bytes 8–11 are present (12 bytes or more), else dropped (0022 §20). A request with an unknown major version
  MUST be answered with status 18, the G7250's own version in bytes 4–5 (0020 §27; B11.13).

Table A1-2 — Management commands (0020 §27)

| Code | Command | Auth | Function |
|------|---------|------|----------|
| 0x0001 | `IDENTIFY` | no | A1.5 |
| 0x0002 | `GET_STATS` | no | packet, error and gap counters (B11) |
| 0x0010 | `GET_CHALLENGE` | no | fresh challenge for one authenticated request (A1.10) |
| 0x0011 | `SET_FIRST_SECRET` | no (window only) | A1.10; refused with status 50 while armed or keyed (B2.3, "Flash work"; 0055) |
| 0x0012 | `CHANGE_SECRET` | yes | A1.10; refused with status 50 while armed or keyed (B2.3, "Flash work"; 0055) |
| 0x0013 | `END_WRITE_SESSION` | yes | releases the write session (A1.10) |
| 0x0020, 0x0021 | `GET_CONFIG`, `SET_CONFIG` | no, yes | A1.6 |
| 0x0022 | `FACTORY_RESET` | yes | A1.6 C-5; scope Table A1-4 (0022 §60) |
| 0x0023 | `SWITCH_LEVEL` | yes | A1.6 C-2; procedure A2.5 |
| 0x0024 | `RESTART` | yes | unkey and disarm first, wait until RF is gone, reply, then reset; status 59 if RF stays (A1.9, commanded reset; 0022 §40, §59, §61); also clears class 2 faults (A3.5) |
| 0x0030, 0x0031 | `READ_LOG`, `CLEAR_LOG` | no, yes | A1.7 |
| 0x0040–0x0046 | `UPDATE_BEGIN`, `UPDATE_BLOCK`, `UPDATE_VERIFY`, `UPDATE_ACTIVATE`, `UPDATE_ABORT`, `UPDATE_STATUS`, `ROLLBACK` | yes (`UPDATE_STATUS` no) | A1.8; refused with status 50 while armed or keyed, `ROLLBACK` included (0022 §32); `UPDATE_STATUS` is a read and is answered at any time (0022 §51) |
| 0x0050 | `RETURN_TO_STOCK` | yes | Reserved (A1.8a item 2): this version retains no stock image; always status 56, after the checks for status 50 and 58 (0024, 0028) |
| 0x0051 | `ENTER_VENDOR_RECOVERY` | yes | Reserved (A1.8a item 4): no software entry into the vendor path is known; always status 56, after the checks for status 50 and 58 (0024, 0028) |
| 0x0060 | `GET_CALIBRATION` | no | reads the calibration set (A1.12; 0039) |
| 0x0061, 0x0062 | `SET_CALIBRATION`, `CLEAR_CALIBRATION` | yes | write or clear the calibration set; refused with status 57 while a P2 session runs, 50 while armed or keyed on any level, 58 during an update (A1.12; 0039) |

The payload of every command is in Annex M (0039). Every status this table names for a
command applies subject to the precedence of Table A1-3: where an earlier code also applies (for
example 36 for `SET_FIRST_SECRET` with a secret already set, or 57 for `SET_CALIBRATION` during a
P2 session), the reply carries that one. Every such request is refused either way, so the order
changes only the code, never whether flash work starts (B2.3, "Flash work"; 0055).

Table A1-3 — Management status codes (0020 §27)

| Code | Meaning | Code | Meaning |
|------|---------|------|---------|
| 0 | OK | 48 | invalid configuration |
| 1 | accepted, in progress | 49 | not allowed (would loosen a limit beyond its built-in value, or touch TX; 0022 §31) |
| 16 | unknown command | 50 | refused while armed or transmitting |
| 17 | malformed | 51 | wrong hardware |
| 18 | unsupported protocol version | 52 | older image without downgrade flag |
| 19 | too large | 53 | bad block CRC |
| 32 | authentication required | 54 | block out of order |
| 33 | authentication failed | 55 | verification failed |
| 34 | rate-limited | 56 | not supported in this firmware |
| 35 | busy (another writer) | 57 | refused while a P2 session runs |
| 36 | first-secret window closed or secret already set | 58 | refused while a firmware update is in progress (0022 §51) |
| | | 59 | reset refused: RF present (A1.9; 0022 §59) |
| | | 60 | not possible: one-way installation in progress (A1.8a item 3; 0028) |
| | | 61 | image too large for the application area (A1.8a item 3; 0028) |
| | | 62 | refused: the value needs the calibration set (A1.12; 0039) |
| | | 63 | image linked for the running slot (A1.8; 0043) |

**Malformed payload** (0056). Status 17 also covers a payload that does not have the size or
structure Annex M gives for its command (a wrong length, a TLV that runs past the end, a missing
fixed field): such a request cannot be read, so it is answered with 17 in the framing group,
before authentication and every state check (36, 35, 58, 57, 50). A payload that decodes but
carries a value that is not allowed is a command-specific fault (48, 49, 51–56, 59–63).

Where several codes apply, the reply carries the first of: framing and version (19 for a
declared length above 1000 bytes, then 17, 18, 16, and 19 for any other datagram size limit), authentication and rate (32, 33, 34, 36), 35, 58, 57, 50, then the command-specific codes
(48, 49, 51–56, 59, 60, 61, 62, 63) (0022 §51, §59; 0028, 0039, 0043). A region change while a P2 session runs and TX is armed therefore
gets 57: ending the session also ends the arm on level 1 (A2.3, Table A3-1).

A1.5 **Identification** (read; no authentication) (0008, Functions):

- model "G7250", MAC address (HIS-15), serial number if the hardware has one (HIS-15) (I-1);
- versions of the main-unit firmware (running image and other slot), this firmware's boot
  stage (A1.8), hardware (I-2; [HIS] HIS 15; 0028); the vendor bootloader's version is not
  read;
- that no stock image is retained and that `ENTER_VENDOR_RECOVERY` is not supported, as two
  flags, so that a later version can set them (I-2a; A1.8a; 0024, 0028);
- whether TX is calibrated (record `tx` bit 4) and the id and date of the calibration set
  (I-7; A1.12; 0039);
- integrity result of both images and update state: normal, trial boot, rolled back, or
  installer (a streaming installation of A1.8a item 3 in progress, with the number of sectors
  written) (I-3; 0028);
- whether a secret is set, and whether the first-secret window is open and for how long
  (I-3a);
- active level, owner of the RF chain (host IP address and UDP port, or none) and TX arm
  state; read-only (I-4; 0011 point 2; 0015);
- uptime, last reset cause (HIS-13), and the management, P2 and capability-record protocol
  versions (I-5);
- the controller state: unknown, not detected or detected; unknown until HIS-17 tells how
  the main unit detects it (I-6; 0023 item 5).

Identification MUST also be answered to a broadcast request (0008, I-6).

A1.6 **Configuration** (write needs authentication, A1.10) (0008, Functions):

- network: static or DHCP; address, netmask, gateway (C-1; [CH394] sections 7.1.2–7.1.5); a
  change of the radio's address or netmask ends a running session (A2.3), is drop event 9
  while armed (`SESSION_CHANGED`, Table A3-1), and is carried out in the order unkey and
  disarm, reply, RF gone (A3.9a item 1, bounded as A3.9a item 1a), apply the address (0022
  §44; §73 item 1); an address change by DHCP is handled the same way (A1.11; 0022 §54);
- level selection: default level at power-on, and a request to switch level now; level 2 is
  selectable only once it exists, after gate G6 (C-2; 0011 point 4);
- safety settings that MAY only be made stricter than the built-in limits: maximum TX time,
  power limit per band, SWR and temperature limits; built-in limits and the TX band edges per
  region MUST NOT be writable, over any channel (C-3; 0011 point 3; 0007 sub-decision 4
  rule 1; 0022 §31). These are the persistent tier of A3.4, the only tier management writes.
  A persistent value MAY be set anywhere between the strictest value and the built-in value,
  so a value tightened by mistake MAY be loosened back toward the built-in value; such a
  loosening is accepted with status 0 and MUST be logged with the old value, the new value and
  the source (A1.7); a value looser than the built-in value MUST be refused with status 49
  (0022 §31). Any change while armed is carried out in the order unkey and disarm
  (`MAINTENANCE`, Table A3-1 event 11), apply the limit, reply (0022 §6; §73 item 1).
  `FACTORY_RESET` does not change the limits (0022 §31; Table A1-4);
- configuration MUST be validated as a whole before it is applied, stored in non-volatile
  memory with a version and a check value, and survive a restart; a network change is applied
  after the reply is sent; if the new network configuration cannot be brought up, or an
  address conflict is detected ([CH394] section 7.1.7, GINT IP_CONFLI), the G7250 MUST fall
  back to the previous configuration and log it (C-4);
- a factory reset of network configuration and secrets MUST be possible over this channel
  (authenticated) and MUST also be possible locally without the network; this headless product
  has no local input for it, so the local path is a re-install through the vendor path
  (A1.8a item 9), after which the first start uses the factory values of Table A1-4, whatever
  the configuration area holds (C-5; 0035); both restore exactly the items of Table A1-4 (0022 §60);
- the region whose transmit ranges apply (C-6): [G7250-MAN] gives one list of transmit ranges
  and says that the ranges of a unit "will be set according to local regulatory
  requirements"; this version therefore has one region, 0, with the ranges of Table A3-0,
  which is also the factory default; a region other than 0 MUST be refused with status 48
  (0039). An owner whose licence allows less narrows it with the C-3 power limit per band
  (0 W closes a band). A change MUST be refused with status 57
  while a P2 session runs, so that the record stays constant within a session (0020 §23;
  B4.7), and with status 50 while TX is armed or keyed on any level; on level 2, unarmed, it
  applies at once (0022 §9);
- an optional TCI client allow-list (C-7) of 0 to 8 entries, each an IPv4 address with a
  prefix length 0–32; empty (the factory default) means any address in the radio's own
  subnet. It applies to TCI connections only; when a new list is applied, open TCI
  connections it no longer allows MUST be closed and logged (0021 §10; 0020 §31). TCI
  connections are accepted only from the radio's own subnet in any case (0021 §10).

A1.6a **Scope of a factory reset** (0022 §60). `FACTORY_RESET` and the local factory reset
(C-5) MUST restore or keep each configuration item as Table A1-4 states, and nothing else.
The factory reset is logged with its source before the reset; it is then carried out as a
commanded reset (A1.9). **Atomicity** (0022 §69): the items of Table A1-4 MUST be written only
as part of a reset that actually starts, after RF is gone, between steps 4 and 5 of the
commanded reset; the new values take effect with that reset. A `FACTORY_RESET` refused with
status 59 (RF not gone) MUST change nothing of Table A1-4: the secret stays set, no
first-secret window opens, and network, level, region, allow-list and TCI port keep their
values; only the log entry of the attempt and the refusal, and the effects of step 1 (unkey,
disarm), remain. A power loss before the reset starts leaves the old values.

Table A1-4 — What a factory reset restores (0022 §60)

| Item | After a factory reset | Source |
|------|-----------------------|--------|
| C-1 network (address mode, address, netmask, gateway) | factory default: DHCP, with an IPv4 link-local address [RFC3927] while no lease is held (A1.11; 0035) | 0008 C-5 |
| C-2 default level at power-on | level 1 | 0011 point 4 |
| C-3 persistent limits | kept; an authenticated C-3 write may return them toward the built-in values (A1.6) | 0022 §31 |
| C-5 management secret | cleared; a new first-secret window opens (A1.10) | 0008 C-5, S3 |
| C-6 region | region 0, the transmit ranges of Table A3-0 | 0020 §23; 0039 |
| C-7 TCI allow-list | empty (any address in the radio's own subnet) | 0020 §31 |
| TCI port (part C C2.4) | 50001 | 0021 §2 |
| Log (A1.7) | kept | 0008 L-2 |
| Fault entry counters, reset counters, `TRIP_COUNT` (A3.5) | kept; cleared only at power-on (survival across resets: P-SYS-9) | 0022 §27; 0050 |
| Firmware images and update state (A1.8) | unchanged | 0008 U-5 |
| Calibration set (A1.12) | kept | 0039 |
| Write session, challenges, stored replies (A1.4, A1.10) | discarded, as at every reset | 0022 §11 |

A1.7 **Logs** (read without authentication; clearing needs authentication) (0008, Functions):
a ring buffer of events with an uptime timestamp in ms, wall-clock time only if the main unit
knows it (HIS-17) (L-1). The log MUST record at least: resets and their cause, safety-core
trips and faults, TX arm and disarm, host takeover, level changes, configuration changes with
the source address (including P2 Set IP, B3.9), every update step, authentication failures and
port conflicts (L-2; 0007, Consequences). It is read in pages by sequence number (L-3). Size
and persistence: the log is kept in RAM, its size is a firmware choice, and it is lost at
power loss; whether it survives a warm reset is pending hardware (P-SYS-2; HIS-13, HIS-18)
(0035). Packet, error and sequence-gap counters are read
with `GET_STATS` (0020 §19, §28).

A1.8 **Firmware update** (authenticated) (0008, Functions, U-1 to U-10):

- An update MUST be refused while TX is armed or the radio is transmitting; this holds for
  every update command, `ROLLBACK` included, which is refused with status 50 (0022 §32);
  `UPDATE_STATUS` is a read and is answered at any time (0022 §51). Once an update
  starts, the G7250 MUST leave RUN, refuse any new P2 RUN or arm, and stay unarmed and unkeyed,
  in the TX-side state of A3.6 (0015) and with all streams stopped, until it restarts (U-1;
  0007 drop event 11, `MAINTENANCE`; wording 0022 §73 item 23). In detail
  (0022 §10): from `UPDATE_BEGIN` on, the P2 session ends and all RX streams stop; a High
  Priority packet with run set is answered, at most once per 100 ms, with `STREAM_STATUS`
  reason 10; register writes are rejected with `BUSY`, which takes precedence over
  `NOT_OWNER`; `CAP_REQUEST` is still answered. The state lasts until a restart, also after
  `UPDATE_ABORT` (which discards the staging area and says in its reply that a `RESTART` is
  needed) and after an abandoned transfer. A transfer is abandoned when no update command
  other than `UPDATE_STATUS` has arrived for 10 min; the G7250 then aborts it as
  `UPDATE_ABORT` does, logs it and restarts into the running image as a commanded reset (A1.9),
  so that RX is available again without a visit to the radio (0022 §51); if RF is present,
  the restart is not made (A1.9, "RF not gone"; 0022 §59). The same 10-minute rule MUST also
  run after `UPDATE_ABORT` and after `UPDATE_VERIFY` that is not followed by
  `UPDATE_ACTIVATE`: 10 min after the last update command other than `UPDATE_STATUS`, the
  G7250 restarts into the running image in the same way, the staging area discarded
  (0022 §77 item 1).
- **Arm and update serialise** (0022 §61). The acceptance of an arm (B10.2 items 3–4; part C
  C5.5.4) and the acceptance of `UPDATE_BEGIN` MUST be one atomic decision of the safety
  core: whichever is processed first wins. If the arm wins, `UPDATE_BEGIN` gets status 50; if
  `UPDATE_BEGIN` wins, the arm is rejected with `MAINTENANCE`. There is no moment at which an
  arm is confirmed while an update is in progress.
- One package per update, with a header: magic, target hardware id and revision, component
  list, versions, length, SHA-256 [FIPS180-4] per component, and a signature field that is
  reserved and empty in this version. An image for other hardware MUST be refused before
  anything is written (U-2; 0008 option I1).
- **Slot-linked images** (0043). An image of this firmware is linked for one slot; the
  package header names that slot (Table M-4 byte 52: 1 slot A, 2 slot B; 0 for an image that
  runs in either slot). Because an update is staged in the other slot, `UPDATE_BEGIN` with an
  image for the running slot MUST be refused with status 63 before anything is written. A host
  reads the running slot from identification (Annex M, Table M-2 tag 23) and sends the
  package for the other one; a release therefore ships one package per slot. A package carries
  one image; a package with both is not defined in this version.
- **No update during a trial boot** (0043). From the start of a trial boot until the image has
  confirmed itself or has been rolled back, the update is still in progress: `UPDATE_BEGIN`
  MUST be refused with status 58, because staging would erase the image a rollback returns to.
- **Write failure** (0043). If writing the staging slot fails, the G7250 MUST discard the
  transfer, answer the request that hit the failure with status 55, log the failure (A1.7) and
  leave the running image and the boot record unchanged; the host starts again with
  `UPDATE_BEGIN`.
- Transfer in numbered blocks with a CRC-32 each, into the other image slot, which is the
  staging area; resumable; abandonable without effect on the running image (U-3). The foreign
  image of A1.8a item 3 is the exception: it has no staging area.
- **Boot stage** (0028 Decision 3). The vendor bootloader starts the application at 0x08020000
  ([HIS] HIS 10). This firmware places a boot stage of its own there, which picks one of two
  image slots in the application area and carries out the atomic switch (U-5) and the trial
  boot and rollback (U-6). The boot stage is part of the first install (A1.8a item 9) and is
  not updated through this channel (U-7). The boot record (active slot, trial state) MUST be
  kept outside the boot stage's sector, so that an update never erases the boot stage (the
  erase unit is a 128 KiB sector, [HIS] HIS 15). The boot stage MUST NOT write the vendor
  bootloader region or the option bytes ([HIS] HIS 15 rules 1, 2).
- The staged image MUST be hashed and compared with the header before activation. No
  signature is checked in this version (U-4; 0008, Resolved 5).
- Activation MUST be atomic; power loss before the switch leaves the old image running (U-5).
- The first boot of a new image is a trial; without self-confirmation within 60 s of start,
  the boot stage MUST return to the previous image and log the rollback (U-6; 0020 §24; 0028).
  **Self-confirmation** (0044): the image confirms itself, by writing the boot record, once it
  has run for at least 10 s with the network up (CH394 initialised and the management socket
  open), its main loop running and the watchdog serviced, and with no class 2 fault (A3.5). It
  needs no host. If it has not confirmed itself 60 s after start, it MUST carry out a commanded
  reset (A1.9) after which the boot stage starts the previous image. If the trial image resets
  for any other reason before it has confirmed itself (a crash, the watchdog, a power loss),
  the boot stage MUST also start the previous image at the next start, because the boot record
  still marks the trial as unconfirmed. Either rollback is logged.
  Until the image has confirmed itself, RX works but an arm MUST be rejected with
  `MAINTENANCE` (0022 §10).
  Two slots and an atomic boot choice are feasible with the boot stage above (U-10; 0028).
  Whether the vendor bootloader itself uses a sector of the application area as a staging
  area (sector 6 is suspected) is an open check of [HIS] HIS 15 rule 5, settled by the drill
  F1 (P-SYS-5); the slot layout is a firmware design choice that MUST take its answer into
  account.
- An image older than the running one MUST be refused unless the request carries an explicit
  "downgrade" flag; a downgrade is logged; configuration the older image does not understand
  is kept but not used, and the reply says so (U-8; 0008, Resolved 3).
- A recovery path that does not depend on this firmware MUST exist: the vendor's documented
  update path through the controller (USB drive and update menu), which this firmware never
  disables (U-9; [HIS] HIS 15; 0028). The return to the stock firmware is A1.8a (0024, 0028).
- The G7250 MUST NOT program itself through P2 Erase and Program packets (B3.9).

A1.8a **Return to stock firmware** (authenticated) (0024, 0028). Installing this firmware
MUST be reversible: the owner can always return the main unit to the stock firmware (0024
R1). The FPGA is not changed by this firmware ([HIS] HIS 15), so only the main unit is
concerned.

**Installation paths** (0028, amends 0024 and 0025). There are two paths, and only the
first install needs the controller ([HIS] HIS 15; 0028 Decisions 1, 2):

- **Stock to this firmware** goes once through the vendor's documented path on the controller
  (item 9).
- **This firmware to stock** and **this firmware to a new image of this firmware** go through
  the main unit, over this channel, with no step that needs the controller (0025 R2). Both use
  the same commands (`UPDATE_BEGIN`, `UPDATE_BLOCK`, `UPDATE_VERIFY`, `UPDATE_ACTIVATE`,
  `UPDATE_STATUS`), the same block transfer with a CRC-32 per block, the same hardware-id and
  SHA-256 checks and the same commanded-reset order before the switch (A1.9) (0025 R3). A new
  image of this firmware is staged in the other slot (A1.8); a stock image cannot be staged and
  is written by the streaming installer of item 3.

This document specifies only this side: the vendor image format and the vendor bootloader's
behaviour are not specified here (0025, "Clean room").

1. **Vendor path left intact** (0024 R2; [HIS] HIS 15 rules 1, 2). The G7250 MUST NOT write
   the vendor bootloader region (sector 0, 0x08000000–0x0801FFFF) or the option bytes, and
   MUST NOT disable the vendor's update path through the controller, which is the recovery path
   (A1.8, U-9). An update (A1.8) writes only this firmware's boot record and the other image
   slot, never the boot stage (U-7), except the streaming installation of item 3, which
   overwrites the whole application area, boot stage included.
2. **No retained stock image** (0028 Decision 2; replaces 0024 Decision 3). The application area
   has no room for a stock image beside this firmware ([HIS] HIS 15). `RETURN_TO_STOCK` is
   reserved and gets status 56; identification reports that no stock image is retained (A1.5
   I-2a).
3. **Streaming installation of a stock image** (0024 Decision 1 as amended by 0028 Decision 2).
   The update of A1.8 MUST accept an owner-supplied stock Main image when `UPDATE_BEGIN` carries
   the "foreign image" flag. The package header names the target hardware id, the length and the
   SHA-256 supplied by the owner; the G7250 checks the hardware id (status 51), that the image
   fits sectors 1–7 (status 61), the block CRCs (status 53) and, at `UPDATE_VERIFY`, the hash
   (status 55), and nothing of the vendor format. The downgrade check (U-8) does not apply.
   - **Gate.** Until the drill F1 has shown that the vendor path through the controller can
     install the stock firmware into an empty application area (P-SYS-5; [HIS] HIS 15,
     installer item 4), a foreign `UPDATE_BEGIN` MUST be answered with status 56 and changes
     nothing: without that proof a broken transfer could leave the radio without a way back.
   - **Hand-over.** An accepted foreign `UPDATE_BEGIN` is a switch in the sense of item 6: the
     P2 session ends, the streams stop, and steps 2–4 of the commanded reset run (A1.9). Then the
     G7250 hands over to an installer that runs from RAM, with the session's authentication
     state in RAM. From here on this firmware no longer runs; its flash is still intact until
     the first sector erase.
   - **Installer state.** The installer MUST keep the TX-side state of A3.6 that steps 2–4 left
     (bias channels at 0, chain A output disabled or bit 7 clear, T/R lines at their receive
     levels, codec in reset, SEND and TUNER trigger released, [HIS] HIS 85), MUST NOT change any
     RF control pin, keys nothing,
     ignores the local PTT and KEY inputs (0030), and MUST service the independent watchdog if it
     runs (its start mode is pending hardware, P-SYS-7). It answers only `IDENTIFY` (with the
     installer state and the sectors written), `GET_CHALLENGE`, `UPDATE_BLOCK`, `UPDATE_STATUS`,
     `UPDATE_VERIFY`, `UPDATE_ACTIVATE` and `UPDATE_ABORT`, and a repeat of the accepted
     `UPDATE_BEGIN`, which it answers from the stored reply (A1.4); every other management
     command gets status 58. Discovery, the P2 ports and TCI are closed.
   - **Before the first erase.** `UPDATE_ABORT` before the first sector erase restarts the main
     unit into this firmware, with its secret and configuration unchanged; it is logged. The
     10-minute rule of A1.8 applies until the first erase: 10 min without an update command
     other than `UPDATE_STATUS` restart the main unit into this firmware in the same way.
   - **Writing.** The installer receives the blocks over this channel and writes them to the
     application area from 0x08020000, at most one 128 KiB sector at a time, each sector only
     after all its blocks have arrived with a good CRC ([HIS] HIS 15, installer items 1–3). It
     answers `UPDATE_STATUS` at any time with the sectors written. It writes only sectors 1–7.
   - **Check and switch.** At `UPDATE_VERIFY` the installer hashes the written range and compares
     it with the header (status 55 on a mismatch; the host may send the blocks again).
     `UPDATE_ACTIVATE` MUST repeat the "foreign image" flag (status 17 without it) and is
     accepted only after a good `UPDATE_VERIFY`; it then resets the main unit.
   - **No way back.** The installer MUST erase the management secret together with the first
     sector erase, before any sector is written (item 7). Once the first sector is erased there
     is no return to this firmware, and
     `UPDATE_ABORT` is answered with status 60 (the reply says the transfer must be completed or
     the recovery path used). If the transfer breaks off after the first erase, the installer
     keeps waiting for blocks and never restarts by itself (the 10-minute rule of A1.8 no
     longer applies); if power is lost, the
     main unit has no valid application and the recovery path is the vendor path through the
     controller (U-9; 0028 Decision 2), which the gate above has shown to work.
   - Before the first erase, the reply to `UPDATE_BEGIN` MUST state that the installation is
     one-way and that a broken transfer needs the vendor path through the controller.
4. **No vendor recovery mode** (0028 Decision 2; replaces 0024 Decision 4). No software entry
   into the vendor path is known ([HIS] HIS 15). `ENTER_VENDOR_RECOVERY` is reserved and gets
   status 56.
5. **Permanent, not a trial** (0024, Details). A stock image is activated without trial boot
   and without rollback to this firmware (U-6 does not apply), because it cannot confirm
   itself. The reply says so.
6. **Safety and order** (0024 R4). A foreign `UPDATE_BEGIN`, `RETURN_TO_STOCK` and
   `ENTER_VENDOR_RECOVERY` MUST be refused with status 50 while armed or
   keyed and with status 58 while another firmware update is in progress; the last two then
   get status 56. When a foreign `UPDATE_BEGIN` is accepted, the P2 session ends and all
   streams stop as from `UPDATE_BEGIN` (A1.8; 0022 §10), level-2 TCI connections are closed,
   and steps 2–4 of the commanded reset run (A1.9) before the hand-over of item 3; a refusal
   with status 59 changes nothing.
7. **Secrets** (0024 R5). The management secret MUST be erased from storage with the first
   sector erase of item 3, so that the stock firmware cannot read it and an abort before that
   erase leaves it in place (A1.10). The installer keeps what it
   needs to authenticate the rest of the transfer in RAM only. The G7250 MUST leave the factory
   EEPROM (I²C1 0x50) unwritten and MUST NOT use the Backup SRAM or the RTC backup registers
   ([HIS] HIS 15 rules 3, 4).
8. **Log** (0024, Details). The hand-over MUST be logged with its source and its target
   (foreign image, length, SHA-256) before the installer starts (A1.7).
9. **Stock to this firmware** (0028 Decision 1; replaces 0025 R4 and 0024 R6). The first
   install, and every re-install after a return to stock, goes once through the vendor's
   documented update path on the controller (USB drive and update menu) ([HIS] HIS 15), with an
   image of this firmware (boot stage and first slot) in the form that path accepts. This is
   the only installation step that needs the controller. That the vendor bootloader starts such
   an image is to be shown by the rollback drill F1 on the radio before the first flash
   (pending hardware, P-SYS-5; HIS-19); if F1
   finds a bootloader entry without the controller, a later decision may use it. After the
   first install the radio has no management secret (A1.10).
10. **No vendor images in this project** (0024 R7; 0018). The owner supplies any stock image;
    this project does not redistribute one, and the reference model (0019) tests with a
    synthetic opaque image.

A1.9 **Management never keys the radio or loosens a limit.** No request on this channel MAY
key the radio, set PTT, arm TX, set the drive level, switch an RF-path relay or loosen a
safety limit beyond its built-in value (0008 S1; 0011 point 3; 0015; 0007). The one relay
movement a management command causes is the return to the relay set of A3.6 after the unkey
and RF gone, as part of a commanded reset (step 4 below), a level switch or a session end it
causes (A3.9a item 5); it never moves a relay toward a TX state (0022 §73 item 16). Returning a
persistent value toward the built-in value is the C-3 correction of A1.6 (0022 §31). Management MAY unkey and disarm, and then
always in the order of A3.2 (0022 §8):

- `RESTART` and `FACTORY_RESET` are accepted while armed or keyed; they unkey and disarm
  first, then reply, then reset, as a commanded reset below. Both drop the arm with
  `MAINTENANCE` (Table A3-1 event 11), also when the reset is then refused with status 59;
  after the reset `TX_ARM_STATE` reads `RESET` (Table A3-1 event 5) (0022 §61; §73 item 2);
- `SWITCH_LEVEL` unkeys and disarms first (`LEVEL_CHANGE`), then replies (A2.5a; 0022 §61);
- `SET_CONFIG` with C-3 limits, while armed, unkeys and drops the arm (`MAINTENANCE`), then
  applies the limit, then replies; a region change is refused while armed or keyed (A1.6 C-6)
  (0022 §73 item 1);
- `SET_CONFIG` with a C-1 change of the radio's address or netmask, while armed, unkeys and
  drops the arm (`SESSION_CHANGED`, Table A3-1 event 9), then replies, and applies the new
  address only after RF is gone (A1.6 C-1; 0022 §44; §73 item 1);
- `SET_CONFIG` with a C-7 allow-list on level 2 that closes the TCI connection of the arm
  holder or of a keying client: the closure unkeys and drops the arm as for a lost connection
  (part C C5.5.5 row 3, `HOLDER_LOST`), before the reply and before the connection is closed
  (A1.6 C-7; 0022 §73 item 18);
- a firmware update, and every update command including `ROLLBACK`, is refused with status 50
  while armed or transmitting (A1.8; 0022 §32); `UPDATE_STATUS`, a read, is answered (0022
  §51); `RETURN_TO_STOCK`, `ENTER_VENDOR_RECOVERY` and a foreign `UPDATE_BEGIN` are refused with
  status 50 as well (A1.8a item 6; 0024, 0028);
- every other command leaves the TX state unchanged; this includes a C-7 allow-list that
  closes no connection of the arm holder or of a keying client.

**Commanded reset** (0022 §40, §59, §61). Every reset that a command causes (`RESTART`,
`FACTORY_RESET`, `UPDATE_ACTIVATE`, `ROLLBACK`, and the restart after an abandoned update,
A1.8), and steps 2–4 before the hand-over of a foreign `UPDATE_BEGIN` (A1.8a item 3; 0028),
MUST be carried out in this order:

1. if keyed, unkey; if armed, disarm, new token and `TX_ARM_STATE` push (A3.2);
2. wait until RF is gone (A3.9a item 1: forward power below `P_rf_off`, or `t_rf_off` after
   the unkey; pending hardware P-RF-3; HIS-07, HIS-12), at most `t_reset_wait` = 2 s from the unkey, or from the
   receipt of the command if the radio was not keyed; `t_rf_off` MUST be shorter (HIS-12);
   this is the bound of A3.9a item 1a, which applies to every wait for RF gone (0022 §68);
3. send the reply (status 0) and store it (A1.4); a repeat of the accepted request (A1.4) that
   arrives during steps 1–2 is answered with status 1, and one that arrives after this step
   with the stored reply until the reset (0022 §87 item 3). After the reset the stored replies
   and challenges are gone, so a retransmission gets status 33; a host confirms the reset from
   `IDENTIFY` (uptime, last reset cause; Annex M) (0039);
4. relays to the state of A3.6 (A3.9a item 5);
5. the reset.

The reset MUST NOT start before RF is gone, because the state of the PA, SEND and relay lines
during a reset is not known (HIS-13). The unkey comes before the reply, so the host never sees
a reply while the radio still transmits; the reply waits for RF gone, normally `t_rf_off`,
well below the 500 ms retransmission time of A1.4 (0022 §61). `UPDATE_ACTIVATE`,
`ROLLBACK` and a foreign `UPDATE_BEGIN` are refused while armed or keyed (A1.8, A1.8a), so for
them step 1 does not apply.

**One commanded reset at a time** (0022 §86 item 5). From the acceptance of a commanded reset
until the reset, or until its refusal with status 59, the G7250 MUST run no second one. A
further commanded reset request in that time, from any host, is not executed: a `RESTART`
(a repeat of the accepted request included) is answered with status 1, because the reset it
asks for is under way; a `FACTORY_RESET`, `UPDATE_ACTIVATE`, `ROLLBACK` or a foreign `UPDATE_BEGIN` is answered
with
status 35 (busy), so that the host knows that its own command did not run and nothing of
Table A1-4 is reset by it. Status 35 takes the place it has in the precedence of Table A1-3.
A status 1 or 35 answer of this rule is not stored (A1.4): a `RESTART` of a second host
answered with status 1 leaves that host's stored reply as it was, and its retry is a new
request. After a refusal with status 59 (below) every request that this rule answered with
status 1 is answered with status 59 when it is retried: the repeat of the accepted request
from the stored reply, which is then the status 59 reply, and a retried `RESTART` of another
host as a new commanded reset that "RF not gone" refuses while RF is present (0022 §87
item 3).

**RF not gone** (0022 §59). If RF is not gone within `t_reset_wait`, which can happen only
where the hardware measures forward power (`P_rf_off`; HIS-11), the G7250 MUST NOT reset. It
logs the event, enters the fault state with `RF_NOT_KEYED` (class 2, Table A3-2 "RF while not
keyed"; normally that check has already entered it) or stays in it, keeps RX running and
replies with status 59 ("reset refused: RF present"). Every later commanded reset follows the
same rule; a power cycle at the radio remains. The restart after an abandoned update (A1.8)
follows the same rule without a reply: it is logged, and the radio stays in the update state
and in fault.

0022 §8 amends 0008 U-1 ("the management channel cannot disarm … for them"): management MAY
unkey and disarm through the commands above, always in the order of A3.2; an update itself
never disarms, the owner disarms first (0022 §8, §51).

A1.10 **Security on a LAN** (0008, Security):

- Writes (configuration, level switch, log clear, update, rollback, return to stock, A1.8a)
  MUST require a per-radio
  secret, proven by challenge and response: a fresh random challenge from the radio, answered
  with HMAC-SHA256 [RFC2104], [FIPS180-4] over the challenge and the request. Reads need no
  authentication (S2). A challenge is valid for one request and 10 s (0020 §24). Source of
  randomness: the MCU's hardware true random-number generator ([HIS] HIS 10; HIS-20).
- **Challenge per host** (0022 §11). The G7250 MUST keep one outstanding challenge per host,
  keyed by source IP address and UDP port, for up to 4 hosts; a fifth host evicts the least
  recently used. A new `GET_CHALLENGE` replaces that host's challenge. The challenge is used
  by the next authenticated request from the same IP address and port, and only once.
- **Secrets on the wire** (0022 §11). The secret is 32 bytes. `SET_FIRST_SECRET` carries it
  in the clear (known risk; S3, S6). `CHANGE_SECRET` MUST be authenticated with the old
  secret and carries the new secret masked as new secret XOR HMAC-SHA256(old secret, "G7NS"
  ‖ challenge) [RFC2104], so it does not cross the network in the clear. Apart from
  `SET_FIRST_SECRET`, the secret never crosses the network.
- **Replies are not authenticated** (known limit, 0022 §11): a host on the LAN can forge a
  reply. A tool SHOULD verify a write by reading the value back.
- **First secret.** In factory state, during a one-time window after the first power-on or
  after a factory reset, the first host that sends a "set first secret" request sets it,
  without confirmation on the controller. The window lasts 300 s of uptime from the moment
  the network is up; it is used up when it has run to its end or a secret is set, and a
  restart before that opens a fresh 300 s window (0020 §24; 0008 S3, "a few minutes"). The
  window MUST open only while no secret is set, MUST NOT reopen once used up without a
  factory reset, and the claim MUST be logged with its source address (S3; 0008, Resolved 1).
  Risk, stated by 0008: anyone on the LAN during the window can claim the radio.
- Requests MUST be answered only from the radio's own subnet and never from a broadcast
  source, except identification; authentication failures MUST be rate-limited and logged
  (S4). Replies MUST NOT exceed 20 per second per source IP address and port; requests above
  that are dropped and counted (`GET_STATS`) (0022 §16).
- **Total reply caps** (0022 §71). An identification reply can be up to 1400 bytes for an
  18-byte request (A1.4), so a spoofed source gains up to about 78 times the traffic. In
  addition to the per-source limit, the G7250 MUST send at most 100 management replies per
  second in total, of these at most 50 identification replies per second, and of those at
  most 10 per second to sources outside the radio's own subnet. Requests above a cap are
  dropped without reply and counted (`GET_STATS`). A flood can therefore delay management
  for other hosts; it cannot touch TX (A1.9, A3.10).
- **Authentication-failure limit and status 34** (0022 §73 item 21). After 5 authentication
  failures (status 33) from one source IP address within 10 s, every authenticated request
  and every `GET_CHALLENGE` from that address is answered with status 34 and not executed,
  until 10 s have passed without a failure from it; each such answer is logged with a count
  (at most one log entry per second per address). Status 34 is sent only for this limit and
  counts against the reply limits above; a request above a reply limit or cap gets no reply
  at all.
- One writer at a time: a write session is held by one host (address and port), opened by its
  first authenticated write and ended by `END_WRITE_SESSION` or after 60 s idle (0020 §24);
  others get status 35 ("busy") for writes and normal answers for reads (S5). Only an
  authenticated request from the holder whose HMAC verifies refreshes the 60 s idle timer;
  reads (`GET_STATS`, `IDENTIFY` and the other unauthenticated commands), `GET_CHALLENGE`,
  any unauthenticated request and any request that fails authentication MUST NOT refresh it,
  from the holder or from any other host (0022 §75). A repeated authenticated request that is
  answered from the stored reply (A1.4) is not executed and MUST NOT refresh it either: only
  the first execution of the request counts (0022 §86 item 7).
- **`RESTART` is exempt from the write session** (0022 §83 item 5). An authenticated
  `RESTART` whose HMAC verifies MUST be accepted from any host, also while another host
  holds the write session; it neither needs, opens nor refreshes a write session. The reset
  ends any write session. So a radio whose write session is held by a crashed tool can be
  recovered without waiting for the 60 s idle timer. `FACTORY_RESET` and every other write
  keep the lock (status 35).
- No confidentiality in this version (S6).
- P2 Set IP on port 1024 is the one unauthenticated write, with the source rule of B3.9
  (B3.9, B11.15; S7; 0022 §70).

A1.11 **DHCP** (0008, Socket budget; 0035). The CH394 has no DHCP client (0008; [CH394]); network
behaviour is defined by this spec ([HIS] HIS 40; HIS-14). The DHCP exchange at power-on runs before the
P2 sockets are opened. Later exchanges (renewal and rebinding, and the retries while no lease
is held) borrow the management socket, which then listens on UDP port 68 (0041). While no lease
is held (no server, or the lease ended), the G7250 MUST use an IPv4 link-local address chosen
as in [RFC3927] and keep trying DHCP, with retries spaced from 4 s doubling up to at most every
64 s (RFC 2131 section 4.1 retransmission back-off); a lease, when it comes, is an address
change under A1.11a (0035).

- **Bound on start-up** (0048, 0057). Counted from link up (the CH394 reports the link,
  B5.8), the management socket and the P2 sockets MUST be open within 5 s when a DHCP server
  answers the first DISCOVER, and within 17 s when none answers: at most 6 s of DHCP attempts at
  power-on, then the RFC 3927 probe and both announcements on the link-local address, at most
  9 s by the constants of [RFC3927] section 9 (PROBE_WAIT 1 s, two probe intervals of at most
  2 s, ANNOUNCE_WAIT 2 s, ANNOUNCE_INTERVAL 2 s), then the sockets, with 2 s margin. The time from power-on to link up (boot and
  auto-negotiation) is not part of this bound.
- **Link-local address** (0049). The G7250 chooses, probes and announces its link-local
  address as [RFC3927] section 2. Only socket 0 can work in MACRAW ([CH394]), and no socket is
  free (0008, 0017), so:
  - the probe and announce run on socket 0 in MACRAW, at start-up before the P2 and
    management sockets open, and again whenever a link-local address is chosen while running
    (after the end of a lease, a DHCPNAK or a conflict, each an address change of A1.11a that
    ends the session). During such a probe, at most 10 s, discovery (port 1024, socket 0) and
    the other sockets are closed;
  - the DHCP attempts and the probe run one after the other, as the start-up bound assumes;
  - **if socket 0 cannot be opened in MACRAW** (a chip or driver fault) after three tries 10 ms
    apart, the G7250 uses the chosen link-local address without a complete probe, so that the
    management channel stays reachable, and keeps watching IP_CONFLI as above (0057). It logs
    it with a log entry containing `link-local <address> used without a complete RFC 3927
    probe`, counts it in `GET_STATS` as `ll_unprobed`, reports it in `IDENTIFY` (Annex M,
    Table M-2 tag 24) and once on the debug console (USART1 as A3.5a) with the line
    `G7250 FAULT LINKLOCAL <address in dotted decimal> UNPROBED` followed by CR LF. It
    is not a fault of A3.5: it does not touch transmit;
  - defence is [RFC3927] section 2.5 option (a): on a conflict ([CH394] GINT IP_CONFLI) the
    G7250 gives the address up at once, without a defending ARP, chooses a new one and probes
    it; after 10 conflicts it waits 60 s before the next probe ([RFC3927] section 9,
    MAX_CONFLICTS, RATE_LIMIT_INTERVAL). Every conflict is logged.
- **DHCP messages while link-local** (0052). The CH394's source IP address is chip-wide
  ([CH394]), and the P2 sockets keep using the link-local address during a borrow, so the
  G7250 sends its DHCPDISCOVER and DHCPREQUEST with the link-local address as IP source while
  it holds one. [RFC3927] section 2.11 requires the DHCP client to keep trying while a
  link-local address is configured, and section 1.9 that the host not alter its use of DHCP
  because of that address; [RFC2131] section 4.1 asks for source 0.0.0.0 "prior to that client
  obtaining its IP address". Sending 0.0.0.0 would mean clearing the chip-wide address and
  closing every socket for each retry, so this is a deliberate reading forced by the chip, kept
  as close to [RFC2131] as it allows: these messages MUST carry `ciaddr` = 0.0.0.0 and the
  BROADCAST flag set, so that the server or relay answers by broadcast and identifies the
  client by `chaddr` and option 61, never by the source address. Before any address is held (the first exchange at
  power-on), the source is 0.0.0.0 as [RFC2131] section 4.1 says.
- **Conflict on a DHCP or static address** (0049). A conflict on a leased address: the G7250
  sends DHCPDECLINE ([RFC2131] section 3.1 item 5), uses a link-local address and starts DHCP
  again. A conflict on a static address: fall back to the previous configuration and log it
  (A1.6 C-4); if there is none, or it conflicts too, use a link-local address. Each is an
  address change of A1.11a.
- **Bound on a borrow** (0041). One borrow MUST last at most 2 s, so that a host's
  retransmission of A1.4 (every 500 ms, up to 5 times) still reaches the management channel
  after it; an exchange not finished in 2 s is abandoned and retried at the next interval, so
  between two borrows the channel is always free for at least the retry interval (4 s or
  more). A request that arrives during a borrow gets no answer and is not
  counted as an error; the host's retransmission covers it.

A1.11a **Address change by DHCP** (0022 §54). Each of these events during a level-1 session or
while level 2 is active is a change of the radio's own address, handled as a C-1 change (A1.6):

- a renewal or rebinding that gives another address or netmask than the one in use (RFC 2131
  section 4.4.5, "Reacquisition and expiration");
- the end of the lease without a new one (RFC 2131 section 4.4.5), or a DHCPNAK in reply to a
  renewal (RFC 2131 section 4.4, client state diagram), so that the radio has to stop using
  its address;
- an address conflict detected while the address is in use ([CH394] section 7.1.7, GINT
  IP_CONFLI).

It ends a running session (A2.3) and, while armed, is drop event 9 (`SESSION_CHANGED`, Table
A3-1): unkey, drop the arm, and stop using the old address, or take the new one, only after RF
is gone (A3.9a item 1), which delays the change by at most `t_reset_wait` (A1.9). On level 2
it is part C C5.5.5 row 14. A renewal that keeps the address and netmask changes nothing. What
the radio does next with its address (a new DHCP exchange, or the fall-back of C-4) is HIS-14.


A1.12 **Calibration set** (0039; Annex P). The values of Table P-1 that are measured on a unit
form its calibration set. It is written over this channel only.

1. `SET_CALIBRATION` (authenticated) replaces the stored set as a whole; `CLEAR_CALIBRATION`
   removes it; `GET_CALIBRATION` reads it (payloads: Annex M, Table M-5). Both writes MUST be
   refused with status 57 while a P2 session runs, so that record `tx` bit 4 stays constant
   within a session (B4.7), with status 50 while TX is armed or keyed on any level, and with
   status 58 during a firmware update. A set is validated as a whole: an unknown parameter id,
   a value that does not decode, or a value beyond the hardware rating gets status 48 and
   changes nothing.
2. The set is stored in this firmware's own flash with a check value (A1.6 C-4); it is kept by
   `FACTORY_RESET` (Table A1-4) and by updates of this firmware, and lost by a return to stock
   (A1.8a item 3), so a host SHOULD read and keep it before one. Every write is logged with its
   id, date and source (A1.7).
3. Record `tx` bit 4 is 1 exactly when the stored set holds every parameter that the freeze
   rule of Annex P names, and the firmware accepts them; then the arm is no longer refused with
   `NOT_CALIBRATED`. A host whose write would set bit 4 sees it from the next session on.
4. Values that need the set are refused until the set holds them: a management write with
   status 62 (Table M-3), a P2 register write with write reject reason `NOT_CALIBRATED` (B8.4).
5. Who measures the values, and how, is outside this spec (Annex P, HIS 90); the encoding of
   each value is fixed in Table P-1 when it is set.
### A2 Ownership and level selection

A2.1 **Single owner.** Only one level controls the RF chain at a time, and only that level's
owner MAY transmit (0011 point 2).

A2.2 **Owner identity.** On level 1 the owner is the host of the current session, identified
by its source IP address and UDP port together (0007, Terms; Resolved 4). A host MUST send all
its C&C packets of one session from one UDP source port (0007, Consequences, "keep one UDP
socket and port per session"). A host that reopens its socket on a new port is a different
host; its first C&C packet is a takeover (A2.4; B5.7).

A2.3 **Session.** A session starts when a High Priority packet with run = 1 (byte 4 bit [0])
arrives while the radio is not in a session, or when a host takes over a running session
(A2.4). "Run set from 0 to 1" refers to the radio's RUN state, not to the host's packets: no
0→1 edge in the host's own run bit is needed. A host whose session ended without its own stop
(a watchdog standby, a link loss; B5.6, B5.8) and that goes on sending High Priority packets
with run = 1 therefore restarts its session with its next such packet, with its RX-side
settings kept (restart, B5.4; [P2] "Detailed operation": the session "can be re-started …
by sending a High Priority Packet with the RUN bit set, if the same network session is still
active") (0022 §78). It ends when RUN is cleared, the P2 watchdog sends the radio to standby, another host
takes over, the link goes down or below 100 Mbit/s (B5.8), a firmware update starts (A1.8),
the level changes (A2.5), the radio's own address or netmask changes (Set IP, B3.9; C-1,
A1.6; DHCP, A1.11a), or the radio resets (0007, Terms; 0022 §3, §10, §16, §44, §54). Ownership starts only in
these two ways, and only for a host inside the radio's own subnet (B11.15; 0022 §49); no
memory-mapped packet makes a host the owner (B4.5). General, DDC-specific and DUC-specific
packets that a host sends before its run bit count for its session (set-up phase, B5.1a;
0022 §34).

A2.4 **Takeover.** Takeover by another host MUST be allowed at any time (0007, Decision 3;
Resolved 1). A takeover is the first C&C packet (B5.6) from a host that is not the owner
while a level-1 session runs. The G7250 MUST handle it in this order (0022 §3):

1. End the old session with the actions of A3.2 in their order: unkey, drop the arm
   (`TAKEOVER`), issue a new token, send a last `TX_ARM_STATE` to the old owner. The last
   `TX_ARM_STATE` MUST be sent three times, about 10 ms apart, with identical register content
   (the same address and data pairs); each copy carries the next sequence number of the old
   session's port 7250 counter towards the old owner (B2.7, "Port 7250 counters per
   destination"; 0051) (0022 §29; §73 item 22). The copies are sent asynchronously: steps 2–5
   MUST NOT wait for the second and third copy (0022 §77 item 2). This step applies in full
   also while the old owner is not armed: `TX_ARM_STATE` then reads disarmed with drop reason
   4 `TAKEOVER` ([23:16]), a new token is issued, and the three copies and the repeated notice
   below are sent as while armed, so that an unarmed host learns that it lost the radio
   (exception to the rule for drop events while not armed, A3.2; 0022 §79), to every old
   owner, as every other register push ("Notice to every old owner" below; 0022 §87 item 2). Then the TX-side state of A3.6 applies: PA off,
   SEND and TUNER released, ATU tuning aborted, DUC FIFO emptied (0022 §32), session limits
   back to the persistent values (A3.4), `TX_MODE_CLASS` = 0, `KEYED_TIMEOUT` = 500 ms,
   hardware-timer value "no General packet yet" (B5.6), and the TX settings of A3.6a back to
   their session-start values: PA enable cleared, every DUC-specific setting as if no
   DUC-specific packet had been received (0022 §64); relays move only as A3.9a allows. Apart from the three copies and the
   repeated notice below, no stream packet and no register push is sent to the old owner;
   answers to its own requests (discovery replies, `CAP_REQUEST` and `NOT_OWNER` answers)
   follow their usual rules and rate limits (B11.14; 0022 §51).
2. Make the new host the owner and start its session at once. RUN stays set, so the new
   owner's watchdog runs from this packet.
3. Send every stream of the session (B2.4) to the new owner's source IP address and port from
   the next packet on; all outgoing sequence counters restart at 0 (B2.7).
4. Keep the RX-side settings (DDC0 frequency, rate and enable; front end) until the new owner
   writes them. TX settings are not kept (step 1; A3.6a). The capability record is sent once, unasked, at the new owner's first General
   packet (B4.4).
5. Process the takeover packet itself.

The new owner starts unarmed. Two hosts that both send C&C take the radio from each other at
every packet ("ping-pong"); each takeover unkeys and disarms, so neither can transmit. The
G7250 counts takeovers (`GET_STATS`) and logs them, at most one log entry per second per pair
of hosts, with a count (0022 §3). Hosts SHOULD warn the user before taking over a radio that
reports 0x03 in discovery byte 4 ([P2] Discovery reply byte 4 NOTE; 0007, Resolved 5). There
is no radio-side protection against an auto-connecting host on level 1 (0007, Resolved 5).

**Repeated notice** (0022 §48). For 5 s after a takeover, every packet from the old owner that
does not make it the owner again (DUC I&Q, memory-mapped packets) MUST be answered with one
copy of the final `TX_ARM_STATE` (`TAKEOVER`), at most once per 100 ms. A C&C packet from the
old owner is a new takeover by it ([P2] Discovery reply byte 4 NOTE; 0007, Decision 3), with
its own three notices to the host it displaces.

**Notice destination** (0022 §58). The three copies of step 1 and the repeated notice are
register packets sent from local port 7250 to the old owner's IP address, at the register
destination its session had at the takeover: General bytes 31–32 of that session if they were
non-zero, else its source port (B8.1; [P2] General packet bytes 31–32). They are not `NOT_OWNER`
answers: they do not count against the `NOT_OWNER` limit of B8.5, and `NOT_OWNER` answers do
not count against them. A memory-mapped write from the old owner within the 5 s therefore gets
its `NOT_OWNER` answer at its source port (B8.5) and, within its own limit, the notice.

**Notice to every old owner** (0022 §87 item 2, replacing §86 item 1). The three copies of
step 1 and the repeated notice MUST be sent to every old owner, armed or not, whether or not it
used the register map. They are register packets from local port 7250, like the other
register pushes that every owner already receives at its register destination: the unasked
capability record (B4.4), `TX_ARM_TOKEN` at the session start (B10.2 item 1) and the telemetry
snapshot (B9). A session carried over at a restart by the run bit (B5.4 item 1) counts as the
old owner's session for the notice destination: General bytes 31–32 carried over are its
register destination. The rest of step 1 applies as stated: `TX_ARM_STATE` reads drop
reason 4 `TAKEOVER` and a new token is issued.

Host guidance (0022 §29, §48, §87 item 2):

- a standard [P2] host that does not use the register map SHOULD ignore packets from source
  port 7250 by source-port demultiplexing: [P2] sends each kind of packet from its own port
  (Table B2-1, B2.3), so a packet from port 7250 is never a C&C, status or stream packet;

- a host that receives a `TX_ARM_STATE` with drop reason `TAKEOVER` SHOULD stop sending C&C at
  once and tell the user;
- a host that receives neither DDC0 data nor High Priority status packets for more than
  200 ms while it believes it owns the radio SHOULD treat ownership as lost, stop sending C&C
  and tell the user; the status packet comes at least every 50 ms in RUN (B9.4), so 200 ms is
  four missed packets;
- a host that sees a stream of its session (DDC0, High Priority status) restart at sequence
  number 0 without having cleared and set the run bit itself SHOULD treat this as a sign that
  the radio changed hands, because the counters restart at every takeover (B2.7); it SHOULD
  stop sending C&C and tell the user;
- a host SHOULD start a new session only on user action.

Known limit (0022 §48): C&C that the old owner sent before it saw the notice still takes the
radio back, as [P2] makes every C&C packet from a non-owner a takeover. With the rules above
such an exchange ends after at most one bounce, possibly with both hosts stopped; neither
transmits meanwhile, because every takeover unkeys and disarms.

A2.5 **Active level.** The active level MUST be visible in management identification (A1.5,
I-4) and, on level 1, in the capability record field `level` (B4; 0003) (0011 point 2). While
level 2 is active port 7250 has no socket, so the record cannot be read; a P2 host learns that
the radio is in use from discovery byte 4 = 0x03 (0003, Resolved) and the active level from
management identification (0022 §18). The default level at power-on and a level switch are
set over the management channel (A1.6, C-2). A change of active level MUST drop the arm
(`LEVEL_CHANGE`, 0007 drop event 10), and an arm is accepted only while level 1 is active
(0007 rule 1). While level 2 is active a P2 host MUST NOT be able to take the RF chain: the
P2 C&C ports have no socket on level 2, a P2 C&C packet starts no session and no takeover,
a General packet on port 1024 (open for discovery) is ignored and counted (B11.1) and starts
no set-up phase (B5.1a; 0022 §51), and the level changes only through the management channel (0021 §16; 0020 §25; 0008, Socket
budget; Annex X1, D23).

A2.5a **Level switch procedure.** `SWITCH_LEVEL` MUST be carried out in this order (0022 §9):

1. Validate, in the precedence of Table A1-3: firmware update in progress, status 58; unknown
   target, status 48; level 2 not in this firmware (before G6), status 56; target already
   active, status 0 and no change (0022 §51; 0039).
2. If TX is armed or keyed: unkey, disarm (`LEVEL_CHANGE`), new token, push `TX_ARM_STATE`
   to the owner (level 1: from port 7250, before it closes; level 2: part C).
3. Reply with status 1 (accepted, in progress); the unkey comes first, as for every command
   that ends a transmission (A1.9; 0022 §61).
4. Stop all streams of the old level and end its session with the state of A3.6.
5. Log the change with the source address (A1.7).
6. Close the old level's sockets: on 1→2 the P2 sockets 1025, 1026, 1027, 1029, 1035 and 7250;
   on 2→1 the TCI listener and its connections. Ports 1024 and 7251 stay open (B3.1; A1.2).
7. Open the new level's sockets. The new level is active with no session; identification
   (I-4) shows it, and on level 1 the record field `level`.

A2.6 **Read without ownership.** Any host MAY read the capability record without taking over
or starting a session (B4.4; 0003 §2) and MAY read management identification, logs and status
(A1.5, A1.7).

### A3 Safety behaviour visible on the network

The safety core sits below both levels; a level MAY make limits stricter, never looser (0011
point 3). This section states the level-independent behaviour; B5 and B10 give the P2 side and
B8 the registers.

A3.1 **One TX authority.** The PA MUST key only when the owner of the active level has armed
TX and a TX request arrives (0015; 0007 sub-decisions 1 and 6). On level 1 the arm handshake
is B10.2. The arm is a protection against accidents, not security: P2 has no authentication,
and anyone on the LAN who sees the token can arm ([P2]; 0007 option 1C, "Against").

A3.1a **Key rule** (level-independent; 0022 §24). The radio MUST key only on a 0→1 edge of a
TX request at a moment when every key condition holds:

- armed, `TX_ARM_STATE` state 1 (not inhibited), and the arm confirmed before the edge
  (B10.2 item 4; C5.5.4);
- not in fault (A3.5);
- the TX frequency inside the transmit ranges in force (Table A3-2, `LIMIT_TX_FREQ`);
- the RF-path state (TX low-pass filter, band relays, ATU in/out and network, antenna) the one
  required for the effective TX frequency in force: DUC0 on level 1, the TCI TX frequency on
  level 2 (part C C4, `TX_FREQUENCY`); A3.1b (0022 §33);
- on level 1, PA enabled (General byte 58 bit [0], B10.10); no frequency-encoding mismatch
  standing (General byte 37 bit [3], B3.8; 0022 §73 item 12); for PTT with DUC I&Q, no
  DUC-specific refusal or EER standing (B10.6);
- a TX mode class defined (B10.3; on TCI, a modulation other than `WFM`, C5.6);
- the requester allowed (0022 §65). On level 1 the allowed requesters are the owner's PTT and
  CWX bits (High Priority bytes 4 and 5), the owner's `ATU_TUNE`, and one local source: the
  KEY input, and only while the owner has selected hardware CW with DUC-specific byte 5 bit
  [1] in this session, under the owner's arm (B10.9). In v1.0 the KEY input is not used and
  is never a requester (0030). No controller request is a requester on
  level 1 (A3.7). On TCI the TX policy decides (C5.5.8);
- not keyed by another source, and no ATU tune cycle running (A3.9b). On level 1 with
  break-in set (DUC-specific byte 5 bit [7]), the first source keeps the transmission: a PTT
  edge while a CW transmission of KEY or CWX is keyed (its hang delay included) and a KEY or
  CWX element while a PTT transmission is keyed are refused, not latched. With break-in clear,
  KEY and CWX are not sources of their own: their elements form RF only inside the owner's
  PTT transmission, and there they are not another source (B10.5b; 0022 §85). KEY and CWX
  drive the same keyer and are one source, "the keyer", so with break-in set a KEY element
  in a CWX transmission's hang delay, or the reverse, is not another source (B10.5b item 1;
  0022 §87 item 5). A request waiting under A3.9a item 3 counts as keyed by its source for
  this condition (0022 §87 item 4);
- relays settled (A3.9a item 2): every RF-path relay of A3.9 except the T/R relay and the RX
  attenuator during TX, which move as part of the key sequence of A3.9a item 4 and settle
  before RF (0022 §51).

A TX request refused for any reason MUST NOT be kept pending or latched: it does not key when
the condition clears, and keying needs a new 0→1 edge after it. Examples: a PTT bit held at 1
while PA enable is 0, while a DUC-specific refusal or EER stands, outside the ranges, while
unarmed or inhibited; an `ATU_TUNE` while unarmed (B10.9). CW text (`CW_MACROS`, `CW_MSG`, part
C) received while it cannot key MUST be discarded, not queued. The one exception is the bounded
wait of A3.9a item 3, while relays move or settle, whatever the cause (0022 §23, §24, §33,
§37).

**Keyer elements** (0022 §74). Every keyer-generated element (CWX paddles, B10.5a; the
hardware keyer from KEY, B10.9; on TCI `KEYER` and CW text, part C) is a key subject to every
condition above. If a condition fails at an element, that element does not key, the keyer
stops, and keying needs a new request edge (a new paddle or KEY closure, a new `KEYER`
key-down or new CW text); the stopped request is not latched. For RF-path purposes (A3.9,
A3.9a, A3.9c and the start of an ATU tune cycle, A3.9b) "keyed" means the continuous
transmission of A3.4d, the CW hang delay included. On level 1 with break-in clear an element
is not a key of its own: it forms RF only while the owner's PTT transmission is keyed, needs
the conditions of A3.1a that the PTT transmission met at its key, and is under the limits
acting on that transmission; the transmission, and "keyed" for RF-path purposes, is the PTT
transmission (B10.5b; 0022 §85). Keyer RF inside it starts only from the PTT key, the moment
RF is enabled at the end of the key sequence of A3.9a item 4 (after the T/R settle and the
RF delay of DUC-specific byte 13); a closure or paddle edge before that moment counts as
held at the PTT key and needs a new closure or edge (B10.5b item 2.2; 0022 §87 item 1). For the check "RF while not keyed" (Table A3-2) "keyed" has
a narrower meaning: RF commanded, an element on with its ramps, so that check runs in every
gap between elements (A3.9a item 6; 0022 §84); inside a PTT transmission with DUC I&Q a run
of all-zero DUC samples longer than `t_blank` is such a gap, also when the host forms the CW
itself (A3.9a item 6; 0022 §88 item 1).

A3.1b **RF-path state for the TX frequency** (level-independent; 0022 §33).

- **While unkeyed** the RF-path state (filters, band relays, ATU in/out and stored network,
  and the antenna where it depends on the band, HIS-06, HIS-08) follows the receive
  frequency: DDC0 (High Priority bytes 9–12 [P2]) on level 1, the receiver centre frequency
  (`DDS`, part C C4) on level 2. The radio receives while unkeyed, and receiving through the
  filter of another band degrades RX. Which state a receive frequency needs is the filter
  plan, pending hardware (P-RF-1; HIS-07). If HIS-07 shows that the TX low-pass filter is not in the receive path,
  the radio MAY keep that filter set for the TX frequency while unkeyed; the check at the key
  still applies. That filter, and only that filter, then follows DUC0 (the TCI TX frequency)
  while unkeyed (0022 §73 item 11).
- **At every key** the radio MUST compare the state in use with the state required for the
  effective TX frequency in force. If they differ, it MUST switch while unkeyed, wait until
  the relays have settled (A3.9a item 2) and only then key, if the request is still asserted
  and every condition of A3.1a holds; this is the bounded wait of A3.9a item 3. If they are
  equal, it keys without moving a relay. A DUC0 change while unkeyed moves no relay by itself,
  except the TX low-pass filter where the first item lets it follow the TX frequency (0022
  §73 item 11).
- **While keyed** the state stays the TX state: a change that would need another TX state
  unkeys first (A3.9, TX frequency); a receive-frequency change that would need another state
  is deferred until the unkey (A3.9, RF-path relays). With CW break-in, keyed includes the
  hang delay (A3.4d); a change that arrives in a gap between elements ends the hang delay
  as A3.9a item 4 states (0022 §74).
- **At the unkey** the radio returns to the state for the receive frequency once RF is gone
  (A3.9a items 1, 4), with CW break-in after the hang delay ([P2] DUC-specific bytes 11–12).
- **Split and cross-band.** With DDC0 (on TCI channel A) at 7.1 MHz and DUC0 (on TCI channel B
  with split) at 14.2 MHz, the radio receives through the 7 MHz state, switches to the 14 MHz
  state before each key and back after each unkey; each such key waits about `t_settle`
  (pending hardware, P-RF-2; HIS-07). A key edge that arrives before the return to the receive state has settled
  does not wait for it: the radio goes straight to the TX state, so the wait stays within the
  bound of A3.9a item 3 (A3.9a item 3a; 0022 §83 item 2). Until the filter plan is known, each `tx_band` entry (B4.8) counts as its own TX
  state (A3.9).

A3.2 **Events that drop the arm.** Each drop MUST unkey first, then disarm, then issue a new
token and push `TX_ARM_STATE` with the reason (0007 sub-decision 1 rule 5). The complete list
is Table A3-1.

Table A3-1 — Arm drop events (0007 sub-decision 1, rule 5)

| # | Event | Reason code (symbolic) |
|---|-------|------------------------|
| 1 | Owner writes 0 to `TX_ARM` | `HOST_DISARM` |
| 2 | RUN cleared (High Priority byte 4 bit [0] = 0) | `RUN_CLEARED` |
| 3 | P2 watchdog: no C&C from the owner for 1 s (B5.6) | `WATCHDOG` |
| 4 | Takeover, including the owner's IP address on another UDP port (A2.4) | `TAKEOVER` |
| 5 | Any reset: power-on, brown-out, processor watchdog, software reset (HIS-13) | `RESET`: after the reset `TX_ARM_STATE` reads disarmed with drop reason 5 `RESET` ([23:16]) until the next drop; logged (A1.7) (0022 §61) |
| 6 | Entry into the fault state (A3.5), including a limit trip with action `FAULT`; the cause is in `FAULT_CODE` (0022 §41) | `FAULT` |
| 7 | Limit trip with action `DISARM` (A3.4); no limit of Table A3-2 has this action in this version (0022 §41) | `LIMIT_<name>` |
| 8 | Ethernet link down, or renegotiated below 100 Mbit/s, if the CH394Q reports it (HIS-14; B5.8; 0022 §16) | `LINK_DOWN` |
| 9 | Loss of the socket or port set-up the session uses (for example a new address by Set IP, B3.9, by a management network change, A1.6 C-1, or by DHCP: another address or netmask at renewal, end of lease, DHCPNAK, address conflict, A1.11a; the change applies only after RF is gone; General packet port moves are not honoured, B2.3) (0022 §44, §54) | `SESSION_CHANGED` |
| 10 | Change of active level (A2.5) | `LEVEL_CHANGE` |
| 11 | Start of a firmware update (A1.8), a configuration change of limits over the management channel (A1.6 C-3; 0022 §77 item 3), or a commanded reset by `RESTART` or `FACTORY_RESET` while armed (A1.9; 0022 §73 item 2) | `MAINTENANCE` |
| 12 | New Discovery from the owner while armed | `REDISCOVERY` |
| 13 | A TX path that must be blocked is found not blocked (A3.7) | `FAULT` |

The numeric reason codes are in B8.7, Table B8-3 (0020 §8).

**Drop events while not armed** (0022 §73 item 3; §79). An event of Table A3-1 that occurs
while TX is not armed drops nothing: it MUST NOT change `TX_ARM_STATE` (the last drop reason
in [23:16] included), MUST NOT issue a new token and pushes nothing, apart from the other
effects of the event itself (a session end, a fault entry, a stop). Three exceptions:

1. after any reset [23:16] reads 5 `RESET` (event 5);
2. a takeover (event 4) sets [23:16] to 4 `TAKEOVER` and sends the three copies and the
   repeated notice of A2.4 to the old owner, armed or not (A2.4; 0022 §79, §87 item 2);
3. the token: a new token is issued at every session end, armed or not (A3.6), and pushed at
   the start of the next session (B10.2 item 1). A drop event while not armed that does not
   end the session (`TX_ARM` = 0, a C-3 change, a fault entry) issues none (0022 §79). So a processor watchdog reset reads drop reason 5 `RESET`; the fault it causes at start-up
(`WDT_RESET_AT_START`, A3.5) is entered while unarmed and shows in `SAFETY_STATE` and
`FAULT_CODE`, not as drop reason 6 `FAULT`.

Not drop events (they end the transmission but keep the arm): the keyed timeout and TX IQ
underrun (B10.5), the CWX key-down cap (B10.5a), maximum continuous TX time and other
`UNKEY`-level limit trips, a retune while keyed that needs an RF-path change, an ATU memory
recall included (A3.9), a TX mode class change while keyed (A3.9c), the end of an ATU tune
cycle (A3.9b), and General byte 58 bit [0] cleared while keyed (B10.10) (0022 §22, §25, §26,
§28); changes of frequency, antenna, filters or drive while not keyed (0007 rule 5;
0022 §1, §14).

A3.3 **Limit actions**, in increasing order (0007, Decision 4):

- `WARN`: flag in `TX_STATE`, no effect on TX;
- `REDUCE`: the radio lowers drive below the host's drive level; TX continues; the host's
  value is not changed (0007 rule 4). For forward power and supply voltage, drive is lowered
  until the value is back inside the limit; for SWR and PA temperature, forward power is
  capped at a fixed value (A3.4a) (0022 §7);
- `UNKEY`: ends the current transmission; a new 0→1 TX request is needed; arm kept;
- `DISARM`: unkey and drop the arm;
- `FAULT`: unkey, drop the arm, enter the fault state (A3.5).

A3.4 **Limits.** Each limit has a built-in value, an action and a recovery rule (Table A3-2).
Rules (0007, Decision 4, rules 1–5):

1. **Tiers.** Each limit has a built-in value (firmware, HIS-12; never writable), a
   persistent value (management configuration C-3, A1.6; equal to the built-in value unless
   tightened) and a session value (P2 `LIMIT_<name>` register, B8; or its TCI equivalent,
   part C). Each tier is at least as strict as the one above it. The **limit in force** is
   the strictest of the three; it is the only value that acts (0022 §6; 0007 rules 1–2; 0008
   C-3).
2. A limit that a host may change has a `LIMIT_<name>` register (B8, Table B8-2). A session
   write MUST be accepted only if it is equal to or stricter than the persistent value; a
   looser write MUST be rejected (not clamped) with `LOOSER_LIMIT`. Writing 0 and the end of
   the session return the session value to the persistent value. A persistent change (C-3)
   while armed drops the arm (`MAINTENANCE`) and then applies at once; a session value looser
   than the new persistent value is replaced by it (0022 §6). Built-in values are never
   writable, over any channel; management writes only the persistent tier, between the
   strictest value and the built-in value (A1.6 C-3; 0022 §31). The built-in value is fixed
   in the firmware, never beyond the hardware rating (HIS-12).
3. The host MUST declare a TX mode class before arming (B10.3).
4. The mapping of P2 drive level 0–255 ([P2] High Priority byte 345) to power is pending hardware (P-PWR-1; HIS-12).
5. All limits apply on both levels (0011). ATU tuning uses its own SWR and power profile,
   values pending hardware (P-ATU-1; HIS-08, HIS-12). The profile's power MAY only be stricter than the limit in
   force, never looser: the **tune power** is the lower of the profile's tune power and the
   forward-power bound in force for the TX mode class in force (Table A3-3), so a built-in,
   persistent or session power limit below the profile's tune power lowers the tune carrier
   (0022 §53).
6. Per-session limit registers, their units and codes are in B8 (0020 §8, §10).

Table A3-0 — Transmit ranges of region 0 ([G7250-MAN], "Parameters": Transmit; 0039)

| Range | From (MHz) | To (MHz) |
|---|---|---|
| 1 | 1.8 | 2.0 |
| 2 | 3.5 | 3.9 |
| 3 | 7.0 | 7.2 |
| 4 | 10.1 | 10.15 |
| 5 | 14.0 | 14.35 |
| 6 | 18.068 | 18.168 |
| 7 | 21.0 | 21.45 |
| 8 | 24.89 | 24.99 |
| 9 | 28.0 | 29.7 |
| 10 | 50.0 | 54.0 |

Each range is one `tx_band` entry of the record, in this order (B4.8). Both edges belong to the
range. The maximum power per range is 100 W for SSB, CW and NFM and 25 W for AM [G7250-MAN];
range 10 is pending hardware (P-PWR-2).

Table A3-2 — TX limits (0007, Decision 4; open thresholds carry their HIS id; timing values from 0020 §29; reaction times 0022 §80)

| Limit | Measured by | Threshold(s) | Action | Maximum time from threshold to action | Recovery |
|-------|-------------|--------------|--------|---------------------------------------|----------|
| TX frequency | DUC0 frequency ([P2] High Priority bytes 329–332); carrier only (A3.4b) | outside the transmit ranges of the configured region, Table A3-0 (A1.6 C-6; 0020 §23; 0039) | not keyed: TX request ignored (no key); keyed: `UNKEY` before the new frequency applies (A3.9; 0022 §1) | checked at every key before RF (no RF outside a range); keyed: the unkey comes before the new frequency applies, so 0 | inside a range again; next TX request |
| SWR | forward and reflected power (HIS-11) | above S1: `REDUCE` (cap `P_swr`, A3.4a); above S2: `UNKEY`; both evaluated only above the minimum forward power `P_min`; S1, S2, `P_min` pending hardware (P-PWR-2; HIS-11, HIS-12) | as listed | `REDUCE` ≤ 10 ms; `UNKEY` ≤ 10 ms (design value; end-to-end with the detector response, pending hardware P-SEN-2; HIS-11, HIS-12) | cap held to the end of the continuous transmission (A3.4a, A3.4d); `UNKEY`: next TX request; 3 `UNKEY` trips within 10 s → `FAULT` `SWR_REPEATED`, class 1 (0020 §29; 0022 §5) |
| PA temperature | PA sensor (HIS-11), filtered (A3.4a) | T1 `WARN`, T2 `REDUCE` (cap `P_temp`, A3.4a), T3 `UNKEY` with inhibit, T4 `FAULT` `PA_OVERTEMP`; all pending hardware (P-PWR-2; HIS-11, HIS-12) | as listed | ≤ 100 ms after the filtered value crosses the threshold (filter time constant pending hardware P-SEN-4; HIS-11, HIS-12) | cap lifts below T2 minus hysteresis; inhibit lifts below T3 minus hysteresis; hysteresis pending hardware (P-PWR-2; HIS-11, HIS-12) |
| Forward power | forward power (HIS-11): PEP and 100 ms mean (A3.4c), vs band and TX mode class; without a PEP detector, the PEP estimate from the digital crest factor of the DUC samples (A3.4c) | 100 W PEP SSB/CW/NFM; AM 25 W carrier (carrier estimate, A3.4c) and 100 W PEP [G7250-MAN]; every tier (built-in, persistent, session; P2 and TCI) is enforced through the bounds of Table A3-3: with a PEP detector on the measured PEP, without one on the PEP estimate for every class, the class-based mean bounds acting in addition, never instead (0022 §43, §52, §62); 50 MHz pending hardware (P-PWR-2; 90 W in [G7250-PUDX], [G7250-RIGPIX]; HIS-12); margin pending hardware (P-PWR-3; HIS-12) | `REDUCE`; if still over 200 ms after `REDUCE` started, `UNKEY` (0020 §29) | `REDUCE` ≤ 10 ms after the bound of Table A3-3 is exceeded in the 100 ms window; `UNKEY` 200–210 ms after `REDUCE` started | next TX request |
| Supply voltage | supply sensor (HIS-11) | starting values 13.8 V ±15 % (11.73–15.87 V) [G7250-MAN]; final HIS-11, HIS-12 | under-voltage while keyed: `REDUCE`, below a lower value (HIS-12) `UNKEY` with inhibit; over-voltage: `UNKEY` with inhibit | ≤ 10 ms (design value; sensor update period pending hardware P-SEN-2; HIS-11) | `REDUCE` lifts inside the range; each inhibit lifts when the supply has been inside the range for 2 s (0020 §29; 0022 §42) |
| Supply / PA current | current sensor, if any (HIS-11) | above 23 A [G7250-MAN]; final HIS-12 | `UNKEY` | ≤ 10 ms (design value); whether the PA needs faster protection in hardware: pending hardware (P-PWR-4; HIS-12) | next TX request |
| Maximum continuous TX time | firmware timer, any TX source; "continuous" as A3.4d (0022 §51) | 5 min built-in (0007, Resolved 3); may be set stricter, never looser; to be checked against the thermal rating (pending hardware, P-PWR-4; HIS-12) | `UNKEY` (arm kept) | not before the limit, unkeyed by the limit + 10 ms (0020 §16) | next TX request |
| ATU tune time | firmware timer | initial tuning ≤ 10 s [G7250-MAN] plus margin pending hardware (P-ATU-1; HIS-08) | `UNKEY`, tune reported failed; the tune cycle ends unkeyed (A3.9b) | not before the limit, unkeyed by the limit + 10 ms (0020 §16) | next tune command |
| Sensor plausibility | all of the above | sensor missing, stuck or out of range (HIS-11) | `FAULT` | ≤ 10 ms after the firmware detects the condition; detection time pending hardware (P-SEN-2; HIS-11) | A3.5 |
| RF while not keyed | forward power (HIS-11) | above a threshold pending hardware (P-SEN-3; HIS-11), not above `P_rf_off` (A3.9a item 1a; 0022 §68), while no RF is commanded and after the blanking time `t_blank` since the end of the last RF-commanded interval: for this check "keyed" means RF commanded, with CW an element on, its ramps included, and `t_blank` runs from the end of each element, so the check runs in every gap between elements, in the hang delay and inside a PTT transmission with break-in clear; inside a PTT transmission with DUC I&Q, RF is commanded only up to the last non-zero sample fed to the DUC, so a run of all-zero samples longer than `t_blank` (host CW sent as I&Q, silence, an underflow) is also a gap where the check runs (A3.9a item 6; pending hardware P-SEN-3; HIS-11, HIS-12; 0022 §23, §84, §88 item 1) | `FAULT` | ≤ 10 ms (design value; detector response pending hardware P-SEN-2; HIS-11) | A3.5 |

The reaction time (0022 §80) runs from the first measurement beyond the threshold that the
safety core has (a detector or sensor sample, the filtered temperature, the 100 ms window
value) to the action taking effect: drive lowered (`REDUCE`), or RF stopped (`UNKEY` and
stronger; relays then follow A3.9a). The detector and sensor response before that sample is
hardware, pending hardware (P-SEN-2; HIS-11); whether the sum protects the PA is HIS-12. The times hold under any
load (A3.10).

The limit ids and their symbolic names (`LIMIT_TX_FREQ` … `LIMIT_RF_NOT_KEYED`) are in Table
B8-3. Only maximum continuous TX time, forward power, SWR and PA temperature are host-tunable
(Table B8-2); the others have no `LIMIT_<name>` register (0022 §19).

A3.4a **Fold-back caps** (0022 §7). For SWR above S1 the safety core MUST cap forward power at
a fixed value `P_swr`, pending hardware (P-PWR-2; HIS-12), and hold the cap to the end of the continuous
transmission of A3.4d, that is until an unkey after which the T/R path returns to receive
(A3.9a item 4), even if SWR falls below S1 again; with CW break-in the cap therefore holds
across element gaps shorter than the hang delay (0022 §67). The next continuous transmission
starts uncapped. `P_swr` MUST be above `P_min`, so that SWR
stays measurable while capped (HIS-11, HIS-12). For PA temperature at or above T2 the safety
core MUST cap forward power at a fixed value `P_temp`, pending hardware (P-PWR-2; HIS-12), and lift the cap below T2
minus the hysteresis; the temperature is filtered with a time constant of seconds,
pending hardware (P-PWR-2; HIS-11, HIS-12), and the cap may change during a transmission.

A3.4b **TX range check is on the carrier** (known limit, 0022 §20). The range check uses the
DUC0 frequency only. The occupied bandwidth of the signal can extend beyond a range edge;
keeping the signal inside the band is the host's and the operator's duty.

A3.4c **Forward power measurement** (0022 §17, §43). The safety core measures forward power at
the PA coupler (HIS-11) over a sliding 100 ms window: the mean power, the peak envelope power
(PEP) and, where the detector delivers envelope samples fast enough to follow the modulation
(HIS-11), the **carrier estimate**, the square of the mean envelope amplitude, in the power
scale of the coupler. For AM, whose modulation averages to zero, the mean envelope amplitude
is the carrier amplitude, so the carrier estimate is the carrier power at any modulation
depth; the mean power is not: a 25 W carrier with 100 % sine modulation has a mean of 37.5 W
and a PEP of 100 W.

**PEP estimate from the digital crest factor** (0022 §62). The firmware generates or forwards
every sample the DUC transmits, so it knows the envelope of the signal before the PA. Over the
same sliding 100 ms window as the coupler measurement, aligned to it by the delay from the DUC
input to the coupler (pending hardware P-TIM-1; HIS-18), the safety core MUST compute the **digital crest factor**
*CF* = max(I² + Q²) / mean(I² + Q²) of the samples fed to the DUC, whatever the TX source: the
host's DUC I&Q after the drive scaling, the keyer's CW waveform, the tune carrier. The **PEP
estimate** is the measured mean forward power × *CF*; with no RF in the window (mean of the
samples 0) it is 0. Where the detector delivers no PEP (HIS-11), the safety core MUST enforce
PEP estimate ≤ *L* for every TX mode class, so a declared class never stands in for the
signal actually sent. The estimate assumes that the analogue chain does not raise the
crest factor; a PA in compression lowers it, so the estimate then errs high; filter overshoot
and peaks between samples are covered by the margin, pending hardware (P-PWR-3; HIS-11, HIS-12). A TX path whose
RF does not come from samples the firmware feeds to the DUC has no estimate and MUST NOT key
on a radio without a PEP detector; whether such a path exists: HIS-18.

**One mapping for every limit in force** (0022 §52). The bounds use *L*, the forward-power
limit in force for the band (A3.4 rule 1): the strictest of the built-in value (100 W PEP
[G7250-MAN]; 50 MHz HIS-12), the persistent value (C-3, A1.6) and the session value
(`LIMIT_FWD_POWER`, Table B8-2; on TCI `G7250_LIMIT`, part C C5.10). *L* is always a PEP
value. The safety core MUST enforce *L* through Table A3-3, whichever tier sets it; a tier is
never compared with the mean directly. The AM carrier limit of 25 W [G7250-MAN] is built in
and is not host-tunable.

Table A3-3 — Forward-power bounds per detector and TX mode class (0022 §43, §52)

| Detector (HIS-11) | SSB | CW, FM | AM |
|-------------------|-----|--------|----|
| PEP and envelope samples | PEP ≤ *L* | PEP ≤ *L* | PEP ≤ *L* and carrier estimate ≤ 25 W |
| PEP, no envelope samples | PEP ≤ *L* | PEP ≤ *L* | PEP ≤ *L* and mean ≤ 25 W |
| mean only | PEP estimate ≤ *L* and mean ≤ *L* × `P_ssb_mean` / 100 W | PEP estimate ≤ *L* and mean ≤ *L* | PEP estimate ≤ *L* and mean ≤ min(*L* / 4, 25 W) |

- Mean only: the PEP estimate is the bound that holds for any signal; the class-based mean
  bound is an additional bound and can only lower the power further, never raise it (0022
  §62). Example: SSB voice with a 12 dB crest factor under *L* = 100 W is held to a mean of
  about 6.3 W by the estimate, below the class bound of 25 W; CW declared but SSB I&Q sent,
  with 6 dB crest factor, is held to a mean of 25 W by the estimate, below the CW bound of
  100 W; AM modulated beyond 100 % is held to PEP ≤ *L* by the estimate, where *L* / 4 alone
  would not hold it.
- `P_ssb_mean` is the mean of an SSB signal of 100 W PEP; starting value 25 W (a PEP-to-mean
  ratio of 6 dB), final value pending hardware (P-PWR-2; HIS-11, HIS-12). With *L* = 10 W the SSB mean bound is
  2.5 W, so a signal with 6 dB PEP-to-mean ratio stays at 10 W PEP.
- CW and FM have a constant envelope, so mean and PEP are equal when the signal sent is CW or
  FM; the estimate covers a signal that is not.
- AM with mean only: mean ≤ *L* / 4 keeps PEP ≤ *L* at any modulation depth up to 100 %,
  because PEP is then at most 4 × the carrier and the mean at least the carrier (at 100 %
  sine modulation the mean is 1.5 × the carrier, so PEP ≤ 2/3 *L*); mean ≤ 25 W keeps the
  carrier ≤ 25 W. With *L* = 100 W both give 25 W.
- AM without envelope samples: mean ≤ 25 W holds the carrier to 25 W at any modulation (at
  100 % sine modulation the carrier is then 16.7 W).

The class-based mean bounds alone are not conservative: they hold PEP ≤ *L* only for a signal
within the assumption of its class (a crest factor of at most 6 dB for SSB, a constant
envelope for CW and FM, a modulation depth of at most 100 % for AM). A crest factor above
6 dB, a class declared for another signal, or overmodulated AM can exceed *L* under them;
that is why the PEP estimate is required without a PEP detector, and the class-based bounds
only add to it (0022 §43, §52, §62). The AM carrier bound with mean only (mean ≤ 25 W) holds
at any modulation, because the mean of AM is never below its carrier.

A3.4d **Continuous TX** (0022 §51). For the maximum continuous TX time a transmission is
continuous from the key (A3.1a) until an unkey after which the T/R path returns to receive
(A3.9a item 4). With CW break-in, gaps between elements shorter than the hang delay ([P2]
DUC-specific bytes 11–12) do not end it, for every CW source (KEY, CWX, TCI `KEYER`, CW text);
with a hang delay of 0 every element is a transmission of its own. On level 1 with break-in
clear (DUC-specific byte 5 bit [7] = 0) the transmission is the owner's PTT transmission, from
the key of A3.1a to the PTT unkey, whatever the elements of KEY or CWX inside it; the hang
delay is not used (B10.5b; 0022 §85). The maximum continuous TX time counts from that key,
not from "the PTT key" of B10.5b item 2.1, which comes later, after the T/R settle and the RF
delay (0022 §88 item 2). The CWX and `KEYER` key-down
cap (B10.5a; part C C8.2) measures instead how long one key bit or key-down is held.

The same continuous transmission, the hang delay included, is what "keyed" means for RF-path
purposes: changes while keyed (A3.9), the relay sequence (A3.9a), the TX mode class (A3.9c)
and the start of an ATU tune cycle (A3.9b) (0022 §74). Each element within it is still a key
under every condition of A3.1a; with break-in clear it needs the conditions of the PTT
transmission instead (A3.1a, keyer elements; 0022 §85). For the check "RF while not keyed"
alone, "keyed" means RF commanded, an element on with its ramps, or inside a PTT transmission
with DUC I&Q a non-zero DUC sample within `t_blank`; that check runs in every gap between
elements, whether the radio forms them or the host sends them as I&Q (A3.9a item 6; 0022
§84, §88 item 1).

**When a change arrives** (0022 §86 item 8). A change requested through a P2 state field that
the host resends (High Priority, DUC-specific and General packet fields) arrives with the
first packet whose value of the field differs from the value in force; later packets with
the same value are the same change. A deferred change still pending from before the gap
(for example a field changed while an element was on, A3.9) does not end the hang delay by
itself; it is applied when the hang delay ends, or with a change that arrives in the gap. A
command (a register write) arrives when it is received. With break-in set, in a gap between
elements:

- a change that needs another TX RF-path state (a DUC0 frequency, A3.9), an RF-path relay
  change requested by the owner (A3.9) or a TX mode class change (A3.9c) ends the hang delay
  (`TX_STATE` unkey reason 6, or 4 with limit id `LIMIT_TX_FREQ` when the DUC0 frequency
  leaves the transmit ranges, or 10 for the mode class; 0022 §86 item 2); the radio waits for RF gone (A3.9a
  items 1, 1a), releases SEND, returns the T/R relay and the RX attenuator during TX to their
  RX position, and only then moves relays or applies the change;
- PA enable cleared (General byte 58 bit [0], B10.10) ends the hang delay in the same way
  (unkey reason 7);
- a change of the receive frequency alone (DDC0; on TCI the receiver centre frequency) stays
  deferred until the hang delay ends (A3.1b);
- the keyed timeout (B10.5) runs during the hang delay as while the PA is keyed: no High
  Priority packet from the owner within `KEYED_TIMEOUT` ends the hang delay (`TX_STATE`
  unkey reason 2), with RF gone, SEND released and the T/R return as above
  (0022 §83 item 1).

With break-in clear, a gap between elements inside the PTT transmission is keyed time like
any other: changes follow A3.9 as while keyed, and nothing ends the transmission but the
events that end a PTT transmission (0022 §85).

The hang delay in force is at most 2000 ms; a larger value in DUC-specific bytes 11–12 is
clamped to 2000 ms (Table B10-1; 0022 §83 item 1).

A3.5 **Fault state** (0007, Decision 5):

- **Classes.** Class 1, host-clearable: limit faults with a measurable condition (temperature
  T4, repeated SWR trips) and a processor watchdog reset seen at start-up. Class 2, latched
  until reset: sensor failure, RF while not keyed, a TX path not blocked, a failed start-up
  self-test, internal consistency errors. The class of every fault code is in Table B8-3
  (0022 §5).
- **Entry.** Any `FAULT` action of Table A3-2, 3 SWR `UNKEY` trips within 10 s (the only
  repeated-trip rule; 0020 §29), drop event 13, a processor watchdog reset seen at start-up
  (HIS-13), a failed start-up self-test, or an internal error (0022 §5).
- **Effect.** Unkey, drop the arm (`FAULT`), release SEND and the TUNER trigger, refuse arm
  requests. RX MUST continue, so that the host can still see the radio and the fault.
- **Reporting.** `SAFETY_STATE`, `FAULT_CODE` (first cause), `FAULT_FLAGS` (latched bitmask
  since the last clear), `FAULT_CLASS` (the highest class among the codes set in
  `FAULT_FLAGS`, 0022 §5) and `TRIP_COUNT`, pushed on change (B8; codes 0020 §8) and logged
  (A1.7). The P2 High Priority status packet stays standard (0005; B9.2).
- **Clearing.** The owner writes the current fault code to `FAULT_CLEAR` (on TCI,
  `G7250_FAULT_CLEAR` with the code name, part C C5.10; 0022 §32). The clear MUST
  succeed only if every code set in `FAULT_FLAGS` is class 1, its condition is gone, and at
  least 10 s have passed since the latest fault entry (hold-off, 0022 §27); it then clears
  all flags. Otherwise it MUST be rejected with `WRITE_STATUS` reason
  `NOT_CLEARABLE` and nothing is cleared (0022 §5). "Condition gone": for `SWR_REPEATED` the
  radio is unkeyed; for `PA_OVERTEMP` the temperature is below T4 minus the hysteresis,
  pending hardware (P-PWR-2; HIS-11, HIS-12); for `WDT_RESET_AT_START` always (the clear acknowledges a past event).
  Class 2: only by a reset (power cycle, or `RESTART` over the management channel). The G7250
  MUST NOT reset itself to clear a fault. A clear MUST NOT re-arm. A class 2 fault is not
  stored across a reset; the start-up self-check detects a persistent cause again and keeps
  the radio in fault, RX only, and the fault is logged (0007, Resolved 9; 0022 §5).
- **Escalation** (0022 §27). The third entry of the same class-1 code since power-on MUST be
  handled as class 2: latched until reset, `FAULT_CLASS` 2, `FAULT_CLEAR` rejected with
  `NOT_CLEARABLE`.
- **Counters** (0022 §27). The fault entry counter per code, a reset counter per reset cause
  and `TRIP_COUNT` count since power-on. They MUST survive `RESTART` and every other reset
  without loss of power and are cleared only at power-on. They are kept in RAM; whether that RAM
  content survives a reset without power loss, including the vendor bootloader that runs
  before the boot stage, is pending hardware (P-SYS-9; HIS-13). P-SYS-9 is part of the freeze
  rule of Annex P: until it is shown, TX stays locked, so a lost count can never let a
  `RESTART` lift an escalation (0050). They are read with `GET_STATS` and logged (A1.7). An
  escalated code therefore stays class 2 after a `RESTART`; only a power cycle at the radio
  clears the count.

A3.5a **Faults before the network** (0040). Some faults are found by the boot stage before the
network is brought up, so no host can be told over the network. In this version this is the
core supply check: the supply configuration of the MCU can be written only once per power-on,
and one that does not match the board wiring can harm the part ([HIS] HIS 10, `6e2208a`).

1. If the boot stage finds a supply configuration that does not match the one this firmware
   selects ([HIS] HIS 10), or cannot confirm it within its time-out, it MUST hold the safe
   state of A3.6 ([HIS] HIS 85), MUST NOT bring up the CH394 (RST# held low, [HIS] HIS 40) or
   any other peripheral beyond the debug console, and MUST NOT write flash.
2. It reports the fault only on the debug console, USART1 TX on PB6 ([HIS] HIS 90), 115200
   bit/s, 8 data bits, no parity, 1 stop bit, once per second, as one ASCII line:
   `G7250 FAULT SUPPLY <PWR_CR3 as 8 hex digits> <MISMATCH|TIMEOUT>` followed by CR LF, the
   hex digits in upper case and zero-padded, for example `G7250 FAULT SUPPLY 0100002E MISMATCH`.
3. The radio stays in that state until power is removed. A host sees no discovery reply and no
   management answer; `g7250ctl discover` finds nothing. The owner's way out is the vendor path
   (A1.8a item 9), which does not depend on this firmware.
4. The fault is not stored and not reported in identification: writing flash on a supply the
   firmware believes wrong is not safe, and a network bring-up on it is not either (0040).

A3.6 **State after power-on, any reset, and every session end** (0007, Decision 5): PA and
its bias off (bias channels at 0; the transmit bias per unit is P-PWR-5), TX path disabled, SEND and TUNER trigger released (HIS-09, HIS-10, HIS-13; [HIS] HIS 85); the firmware sets this state first in its start-up, because pin states before its GPIO init are pending hardware (P-SYS-1). **From when** (0045): the RF control pins of [HIS] HIS 85 MUST be at their safe-state levels no later than 20 µs after the boot stage's first instruction at 0x08020000, and before any other initialisation (clocks beyond the reset clock, other peripherals, network); from then on the safe state holds as this section says. Before that point the pin levels are set by the hardware (board pulls) and by the vendor bootloader in sector 0, which runs first, and are outside this specification; whether the board pulls hold safe levels in that window is pending hardware (P-SYS-1); DUC
FIFO emptied (0022 §32); not armed, new token, session limits back to the persistent values (A3.4; after a reset the
persistent values are read from the stored configuration), `TX_MODE_CLASS` = 0,
`KEYED_TIMEOUT` = 500 ms, watchdog counted as enabled, the TX settings of A3.6a at their
session-start values (0022 §64); ATU not tuning, its latches unwritten and its state
unknown (`ATU_STATE` 5) until P-ATU-1 is set, then as stored per band (0032; HIS-08); antenna latch unwritten and
`ANT_STATE` 0 until P-ANT-1 is set (HIS-06); the strobes of these latches (PD13, PG2, PG5) are
held at their inactive level (high) and never pulsed ([HIS] HIS 85, `d199d33`, MEDIUM); filters for the default RX frequency, preamplifier
off and attenuation as pending hardware (P-RF-5; HIS-07).
At a takeover only the TX-side part applies; RX-side settings are kept for the new owner
(A2.4; 0022 §3). At a session end by a stop, a watchdog standby or a link loss (Table A3-1
events 2, 3, 8, and the same ends while unarmed) also only the TX-side part applies: the
RX-side settings stay in force for a restart by the same host (B5.4; 0022 §78); the RX-side
part applies when another host starts the next session (B5.2). At a session end, takeover, standby or fault entry while keyed, the radio
unkeys first and switches the relays of this state only after RF is gone (A3.9a items 1 and
5; 0022 §23). Before a commanded reset the order of A1.9 applies, so RF is gone before the
reset starts (0022 §40).
What the hardware does between reset and the moment the firmware drives these pins is
HIS-13; if the PA can key from floating lines, M4 waits for a mitigation (0007).

A3.6a **TX settings at every session end and takeover** (0022 §64). The transmit settings that a
level-1 host writes in General and DUC-specific packets belong to its session. At every session
end and at every takeover, after the unkey and before the new owner's first packet is
processed, the G7250 MUST return them to these values, the same as at power-on:

Table A3-4 — TX settings after a session end or takeover (0022 §64; fields [P2] General packet
byte 58, DUC specific packet)

| Setting | Field ([P2]) | Value after session end or takeover |
|---------|--------------|-------------------------------------|
| PA enable | General byte 58 bit [0] | 0: PA disabled; no TX request keys (B10.10) until the new owner sets it |
| CW select (hardware CW from KEY) | DUC-specific byte 5 bit [1] | 0: KEY keys nothing (A3.1a) |
| Keyer options (reverse keys, iambic, sidetone, mode B, strict spacing) | DUC-specific byte 5 bits [2]–[6] | 0 |
| Break-in | DUC-specific byte 5 bit [7] | 0: KEY and CWX form RF only inside a PTT transmission of the owner (B10.5b; 0022 §85) |
| EER | DUC-specific byte 5 bit [0] | 0: no EER refusal stands |
| Sidetone level and frequency | DUC-specific bytes 6–8 | 0 (HIS-17) |
| Keyer speed and weight | DUC-specific bytes 9–10 | 0: CWX forms no element and does not key until a speed is set (Table B10-1) |
| Hang delay | DUC-specific bytes 11–12 | 0 ms |
| RF delay | DUC-specific byte 13 | 0 ms |
| DUC0 sampling rate and bits | DUC-specific bytes 14–16 | 0, read as 192 ksps and 24 bits: no DUC-rate refusal stands |
| CW ramp period | DUC-specific byte 17 | 0, read as the G7250 default 8 ms |
| Line in, mic boost, line-in gain | DUC-specific bytes 50–51 | 0 (HIS-17) |
| RX step attenuator during TX | DUC-specific bytes 57–59 | the attenuation of A3.6, pending hardware (P-RF-4; HIS-07) |

The values are those of a DUC-specific packet of zeros read through Table B10-1, plus PA
enable cleared. The new owner sets its own values with the takeover packet itself, its set-up
phase (B5.1a) or later packets; nothing of the old owner's TX set-up carries over. RX-side
settings follow A2.4 step 4 at a takeover and B5.4 at a restart by the same host (0022
§78). Reason: a takeover hands the RF chain to a host that may not know
what the old owner set; reset is the safe default (0022 §64).

A3.7 **Other TX paths.** "While level 1 runs" means: level 1 is the active level, with or
without a P2 session (0022 §15). Local inputs (controller microphone PTT, KEY) MUST be
reported and MUST NOT key the radio by themselves on level 1; KEY keys only as an allowed
requester of A3.1a, with hardware CW selected by the owner and under its arm (B10.9; 0022
§65), and the controller microphone PTT never keys on level 1; the controller has no
standalone operation while level 1 runs (0007, Decision 6; Resolved 6). A controller request
that reaches the main unit (HIS-17) is not a TX request for the arm, does not start a session
and does not block an arm (0022 §15). SEND, the TUNER trigger and internal ATU tuning are
under the arm (B10.9). Controller TX functions (ATU key, SWR scan) are refused while level 1
runs unless they reach the main unit as reportable requests (HIS-17). If a path cannot be
blocked, M4 is blocked for that path until ON8ST decides (HIS-10, HIS-17; 0007, Decision 6).
In v1.0 the main unit's local PTT and KEY inputs are not used: they key nothing and are
reported as 0 (0030; [HIS] HIS 85). The controller is not used as a front panel on either level (part A, "Scope of the firmware";
0023, note of 2026-11-20); the rules above are safety rules for controller inputs that reach
the main unit anyway.

A3.8 **Management** never keys the radio, arms it or loosens a limit; it may unkey and
disarm in the order of A3.2 (A1.9; 0022 §8).

A3.9 **Changes while keyed** (level-independent; 0022 §1, §2).

- **TX frequency.** While keyed (with CW break-in, the hang delay included, A3.4d; 0022 §74),
  the G7250 MUST evaluate every new DUC0 frequency before
  applying it. If it lies outside the transmit ranges in force, or needs another TX filter,
  band or relay state than the one in use, the G7250 MUST unkey first (`UNKEY`, arm kept;
  `TX_STATE` unkey reason 6, or reason 4 with limit id `LIMIT_TX_FREQ` when the cause is the
  range), then apply the frequency and switch relays while unkeyed; keying again needs a new
  0→1 TX request. A retune inside the same transmit range that needs no change of filter,
  band or relay state MUST be followed while keyed, without unkey. Which frequencies share
  one RF-path state is the filter plan: pending hardware (P-RF-1; HIS-07); until it is known, each `tx_band` entry
  of the record (B4.8) counts as its own state. At a key the state is checked against the TX
  frequency by A3.1b (0022 §33).
- **RF-path relays** (antenna, ATU in/out and its L/C network, TX and RX filters,
  preamplifier, attenuator, T/R relay, RX attenuator during TX) MUST NOT change while keyed,
  whatever the source; with CW break-in, keyed includes the hang delay (A3.4d). An owner's
  change that arrives in a gap between elements ("arrives" as defined in A3.4d; 0022 §86
  item 8) ends the hang delay instead of waiting for it, and is applied after the sequence of A3.9a item 4 (`TX_STATE` unkey reason 6; 0022
  §74); a register write that does so is answered once the change is applied; a controller
  button press stays ignored, as while keyed. A change requested through a P2 state field
  that the host resends (High Priority, DUC-specific and General packet fields, as A3.4d) is deferred: the latest
  value is applied right after the unkey, before any new key. A change requested by a
  command is refused: a register write is answered with the value in force and
  `WRITE_STATUS` reason `KEYED`; a TCI command as in part C (0021 §8); a controller ANT or ATU
  button press that reaches the main unit ([G7250-MAN]; HIS-17) is ignored and logged.
  Management switches no relay, apart from the return to the relay set of A3.6 after RF gone
  (A1.9; 0022 §73 item 16). The only exception is the ATU relays inside an ATU
  tune cycle at the tune power (A3.9b; 0007 Decision 4 rule 5; 0022 §22). A controller that
  switches relays by itself is a finding for HIS-10 and HIS-17.
- **ATU memory recall** (0022 §28). Every change of the ATU L/C network or in/out state is an
  RF-path state change. The ATU MUST NOT recall a stored state while keyed. A retune while
  keyed that would recall another ATU state MUST unkey first (`TX_STATE` unkey reason 6), as
  a retune that needs another filter; the recall is applied after RF gone and settle
  (A3.9a). Whether and per which frequencies the ATU stores states: HIS-08.

A3.9a **Relay switching sequence** (level-independent; 0022 §23). Applies to every RF-path
relay of A3.9.

1. **RF gone.** After an unkey, no relay MUST move until RF is gone: forward power below a
   threshold `P_rf_off`, or a fixed guard time `t_rf_off` after the unkey, whichever the
   hardware design uses; values pending hardware (P-RF-3; HIS-07, HIS-12). With CW break-in the unkey is the end of
   the hang delay (A3.4d): RF stopping between elements lets no relay move (0022 §74); with
   break-in clear it is the end of the PTT transmission (B10.5b; 0022 §85).

   **1a. Bound on every wait for RF gone** (0022 §68). Every wait for RF gone, whatever caused
   the unkey (a normal unkey, a session end, a takeover, a fault entry, an address or level
   change, a commanded reset), is bounded by `t_reset_wait` = 2 s from the unkey (A1.9). If
   forward power is still at or above `P_rf_off` at the bound, which can happen only where
   the hardware measures forward power (HIS-11), the G7250 MUST enter the fault state with
   `RF_NOT_KEYED` (class 2) or stay in it, and log the event. Relays still do not move while
   RF is present; they move as soon as RF is gone. The actions that waited and move no relay
   then proceed: the session end, the address change (A1.11a), the level switch (A2.5a); a
   commanded reset is refused with status 59 (A1.9). In addition the threshold of "RF while
   not keyed" (Table A3-2) MUST NOT be above `P_rf_off`, and `t_blank` MUST be shorter than
   `t_reset_wait`, so that RF that stops a relay from moving is always a fault (HIS-11,
   HIS-12).
2. **Settle.** The radio MUST key only after the settle time `t_settle` has elapsed since the
   last relay change; pending hardware (P-RF-2; HIS-07).
3. **Bounded wait** (0022 §23, §33, §37). A TX request whose 0→1 edge arrives while an
   RF-path change is pending, in progress or settling waits until the relays have settled
   (item 2), and then keys if the request is still asserted and every condition of A3.1a holds
   at that moment. This covers: changes that arrive with the request (a High Priority packet
   that carries a new DUC0 frequency, Alex word or step attenuator and a PTT or CWX 0→1 edge;
   a TCI frame with a frequency command and `TRX:0,true`), which are applied first; changes
   from an earlier packet or frame that are still moving or settling when the edge arrives;
   the switch to the TX state at the key (A3.1b); and an edge right after an unkey, while RF
   is going (item 1) or the deferred changes and the return to receive are being applied
   (item 4), in which case the deferred changes that concern the TX RF-path state are applied
   and then the TX state of A3.1b, while deferred changes of the receive frequency alone stay
   deferred to the next unkey (item 3a; 0022 §86 item 3).
   The wait is bounded by `t_rf_off` + `t_settle` from the edge, pending hardware (P-RF-2, P-RF-3; HIS-07, HIS-12). A
   further change of the TX RF-path state requested during the wait (a DUC0 frequency that
   needs another state, an Alex word, a step attenuator, an antenna or ATU change), or
   reaching the bound, refuses the request, which is not latched (A3.1a). A change of the
   receive frequency alone (DDC0; on TCI the receiver centre frequency) during the wait does
   not refuse it: it is deferred to the next unkey, as while keyed (A3.1b) (0022 §73 item
   10). This is the only deferred TX request. While it waits, the request holds the
   transmission for its source: a TX request edge of another source (on level 1 PTT, the
   keyer of KEY and CWX, `ATU_TUNE`; on TCI another client or source, part C) that arrives
   during the wait is refused as "keyed by another source" (A3.1a) and is not latched, also
   when the waiting request is refused later (0022 §87 item 4).

   **3a. No return to receive while an edge waits** (0022 §83 item 2). If a TX request edge
   is waiting under item 3 when the return to the receive state of A3.1b would start, or
   arrives while that return is moving or settling, the radio MUST NOT complete the return:
   it applies the deferred changes that concern the TX RF-path state, switches straight to
   the TX state of A3.1b from wherever the relays are, and keys after one `t_settle` counted
   from the last relay command (item 2). Deferred changes of the receive frequency alone stay
   deferred to the next unkey. So with split or cross-band (A3.1b) the wait never contains
   two settle times and stays within the bound of item 3. Whether a relay may be commanded
   again before it has settled: HIS-07. Where a relay cannot be commanded again before it
   has settled, the radio MUST wait until that relay has settled, then command it to the TX
   state, and key one `t_settle` after the last relay command; if the bound of item 3 is
   reached first, the request is refused and not latched (A3.1a) (0022 §86 item 4).
4. **T/R relay and RX attenuator during TX.** At a key: T/R relay and the RX attenuator
   during TX (DUC-specific byte 59 [P2], B10.6) to their TX position, settle (item 2), SEND,
   RF delay (DUC-specific byte 13), RF. With break-in clear the moment RF is enabled at the
   end of this sequence is "the PTT key" of B10.5b: keyer RF inside the PTT transmission
   starts only from it, and a KEY closure or paddle edge before it counts as held at the PTT
   key (0022 §87 item 1). At an unkey: RF off, RF gone (item 1), SEND released,
   then both back to their RX position and deferred RF-path changes applied. With CW
   break-in, between elements RF stops but SEND, the T/R relay and the RX attenuator during
   TX stay in their TX position for the hang delay (DUC-specific bytes 11–12); only when the
   hang delay has run out without a new element are SEND released (RF being gone) and both
   returned, so that an external amplifier does not switch at every element (0022 §73 item
   9). A drop event, standby, fault entry or session end during the hang delay ends it at
   once. So does, in a gap between elements, a change that needs another TX RF-path state, an
   RF-path relay change of the owner or a TX mode class change (`TX_STATE` unkey reason 6, or
   4 with `LIMIT_TX_FREQ` for a range exit, or 10; 0022 §86 item 2), and PA enable cleared
   (reason 7): RF gone (item 1), SEND released, both back to their
   RX position, and only then the relays move or the change is applied. A receive-frequency
   change alone stays deferred until the hang delay ends (A3.4d; 0022 §74). With break-in
   clear (B10.5b) the T/R relay, the RX attenuator during TX and SEND follow the owner's PTT
   transmission: to their TX position at the PTT 0→1 edge accepted under A3.1a, as the
   first steps of this key sequence, and back at the PTT unkey, as above (0022 §88 item 2);
   between the elements of KEY or CWX inside it only RF stops, and the hang delay is not used
   (0022 §85). Whether the T/R switch is a relay: HIS-07.
5. **Session end.** The relay set of A3.6 is applied only after the unkey and RF gone.
6. **Blanking** (0022 §23, §84, §88). The check "RF while not keyed" (Table A3-2) MUST run
   whenever no RF is commanded, after a blanking time `t_blank` from the end of the last
   RF-commanded interval; `t_blank` is at least the time of item 1; pending hardware (P-SEN-3; HIS-11, HIS-12). For
   this check alone, "keyed" means RF commanded:
   - **PTT with DUC I&Q.** RF is commanded by the samples fed to the DUC (A3.4c: the host's
     DUC I&Q after the drive scaling, and the zero samples fed during an underflow, B10.5).
     Inside the PTT transmission RF is commanded from the PTT key (B10.5b) up to the last
     sample with I or Q non-zero, and ends there; the check runs from `t_blank` after that
     sample (or after the PTT key if no non-zero sample has been fed yet) until the next
     non-zero sample. `t_blank` is counted once: there is no second blanking time after a
     zero run has reached `t_blank` (0022 §88 item 1).
     The firmware feeds every DUC sample (A3.4c), so the radio needs nothing from the host to
     find the gaps. This covers CW that a host forms itself and sends as DUC I&Q with PTT,
     zero between elements (on level 1 TCI CW through AetherSDR, part C C8.2.5, and any P2
     host that sends CW as I&Q), and silence or an underflow in any other PTT transmission.
     For this case `t_blank` also covers the delay from the DUC input to the coupler
     (pending hardware P-TIM-1; HIS-18, A3.4c), and the threshold MUST lie above the forward power that the TX chain
     emits with all-zero DUC samples (carrier leakage; HIS-11).
   - **CW whose elements the radio forms** (on level 1 KEY and CWX; on level 2 TCI `KEYER`
     and CW text): each element from the start of its leading ramp to the end of its
     trailing ramp (DUC-specific byte 17).
   - **ATU tune cycle:** the tune carrier.

   `t_blank` runs from the end of each element or of the last non-zero sample, so with CW the
   check runs in every gap between elements longer than `t_blank`: during the hang delay with
   break-in, inside a PTT transmission with break-in clear (B10.5b), and inside a PTT
   transmission whose CW the host sends as I&Q, although the transmission stays keyed for
   RF-path purposes (A3.4d). RF in a gap after `t_blank` is therefore a fault even while SEND
   and the T/R relay stay in their TX position. Known limit (0022 §88 item 1): a host that
   sends non-zero samples in its gaps (noise, dither, a DC offset, a ramp tail longer than
   the gap) commands RF there, so the check does not run in those gaps; RF there that the
   samples do not command is then seen only at the next all-zero run longer than `t_blank`
   or after the unkey, at most the maximum continuous TX time later, while the limits of
   Table A3-2 act throughout. A host that sends CW or pauses as DUC I&Q SHOULD send exact
   zeros (I = Q = 0) in its gaps.

A3.9b **ATU tune cycle** (level-independent; 0022 §22). The firmware controls the internal
ATU itself; there is no ATU controller ([HIS] HIS 85; 0032). Until the ATU parameters of the
pending-hardware list are set, the ATU latches stay unwritten, their strobes held at the
inactive level (high) and never pulsed ([HIS] HIS 85, `d199d33`, MEDIUM), so its state is not
known:
`ATU_STATE` reads 5 (unknown), and a start is rejected with write reject reason `NOT_CALIBRATED` (on TCI as part C
C5.7) and is not latched (0032). Once they are set, `ATU_TUNE` (B10.9) and
`G7250_ATU_TUNE` (part C C5.7) start a tune cycle:

1. It MUST start only from unkeyed. A start while keyed by any other source, including a
   transmission of the owner or of the requesting client itself and the CW hang delay between
   elements (A3.4d; 0022 §74), MUST be rejected with `KEYED` and is not latched (A3.1a). A repeated start while the cycle itself runs, or within 2 s
   after the end of the previous cycle, is a no-op: it is answered with the value in force and
   `ATU_STATE`, and no cycle starts, so a retried command never gives a second tune carrier
   (B8.4; 0022 §39). A start while armed and unkeyed but with another condition of A3.1a
   failing (fault, inhibit, outside the TX ranges, PA disabled, no mode class, on TCI the TX
   policy) is rejected with `NOT_KEYABLE` and is not latched (0022 §51). Where several apply,
   the answer carries the first of: `BUSY`, `NOT_OWNER`, `BAD_VALUE`, `NOT_CALIBRATED`,
   `NOT_ARMED`, the no-op (cycle running or within 2 s of the end of the previous one),
   `KEYED`, `NOT_KEYABLE` (0022 §73 item 6; 0032). `ATU_TUNE` = 0 from the owner MUST always be accepted, also while the
   cycle keys and whatever the arm state: it aborts a running cycle (item 6) and is a no-op
   when no cycle runs (0022 §77 item 5).
2. All conditions of A3.1a hold at the start. Before RF, the cycle MUST apply pending RF-path
   changes and switch the RF path to the TX state of A3.1b for the effective TX frequency in
   force, and wait until the relays have settled (A3.9a items 2, 3), so that the tune carrier
   goes through the TX filter of the TX frequency, also with split or cross-band (0022 §66).
3. The radio MUST set the drive to the tune power, the lower of the ATU profile's tune power
   (pending hardware P-ATU-1; HIS-08, HIS-12) and the forward-power bound in force (A3.4 rule 5, Table A3-3),
   before RF starts and before the ATU moves any relay (0022 §53).
4. The cycle owns the key for its whole duration (TX source 4, `ATU_STATE` 2); other TX
   requests neither key nor end it.
5. ATU relays move under RF only inside the cycle and only at the tune power; this is the
   exception of A3.9.
6. The cycle MUST end unkeyed in every case: tuned, failed, aborted, ATU tune time
   (`LIMIT_ATU_TIME`), keyed timeout, any limit unkey or drop. The tune carrier is never
   handed over to another TX source; keying again needs a new 0→1 edge. `TX_STATE` unkey
   reason (0047): a cycle that ends by itself, `ATU_STATE` 3 (tuned) or 4 (failed), gives
   11 (ATU tune cycle ended); an abort by `ATU_TUNE` = 0 or a stop request (item 1; part C
   C5.7) gives 1 (TX request released), with `ATU_STATE` 4; the ATU tune time gives 4 with
   limit id `LIMIT_ATU_TIME`; a keyed timeout, a limit or a drop give their own reasons (2, 4,
   5).
7. **After the cycle** (0022 §38, §66). At every end of the cycle, once RF is gone (A3.9a
   item 1), the RF path MUST return to the unkeyed state of A3.1b for the receive frequency,
   as after every unkey; the next key applies A3.1b again for the TX frequency. A cycle that
   ended tuned leaves its result stored for the TX frequency where the ATU stores states
   (HIS-08), and the next key recalls it. A cycle that did not end tuned (failed, aborted,
   time limit, keyed timeout, a limit unkey or drop, a takeover or another session end)
   stores nothing: the ATU is at the stored state for the receive frequency if one exists
   (HIS-08), else bypassed, and the next key recalls the stored state for the TX frequency,
   or bypass. `ATU_STATE` = 4 (failed) then means "the last tune did not end tuned; the ATU is
   at a stored state or bypassed", and stays until the next tune cycle or an ATU in/out
   change. `ATU_STATE` = 4 does not block keying; the SWR limits of Table A3-2 protect the PA.

A3.9c **TX mode class never changes under a transmission** (level-independent; 0022 §26). On
level 1 a `TX_MODE_CLASS` write with another value while keyed, the CW hang delay included
(A3.4d; 0022 §74), MUST unkey first (`UNKEY`, arm kept, `TX_STATE` unkey reason 10), then apply; a write of the value in force is a no-op
(B10.3). On TCI a `MODULATION` change while keyed is refused with `KEYED` (part C C7.6.3).
Either way no RF runs under a class other than the one in force.

A3.10 **Safety timing is independent of load** (0022 §16). The watchdog, the keyed timeout,
limit actions and arm drops MUST meet their timing under any received packet load the link
can carry. Load MAY cause the owner's packets to be lost and so an unkey or a watchdog stop
(the safe direction); it MUST NOT delay an unkey.

---

## Part B — Level 1: G7250-P2

### B1 Scope and conformance language

B1.1 This part specifies the network interface of level 1 of the G7250 main-unit firmware,
called **G7250-P2**: openHPSDR Protocol 2 with a G7250 board type (0016, 0002).

B1.2 The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD
NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY" and "OPTIONAL" in this document are to be
interpreted as described in BCP 14 (RFC 2119, RFC 8174) when, and only when, they appear in all
capitals, as shown here. "The G7250" means the G7250 main unit running firmware that
conforms to this part. "The host" means a P2 host application.

B1.3 **Normative reference.** [P2] openHPSDR Ethernet Protocol V4.4 (see `sources.md`). Where
this document says "[P2]" without a version, V4.4 is meant.

B1.4 **Default rule.** The G7250 MUST behave as [P2] specifies, for every packet, field and
procedure, unless this document states a deviation (0009 as revised by 0016).

B1.5 **Deviations.** Every deviation from [P2] MUST be stated in the section concerned and
listed in Annex X1 (0009, Consequences; 0005, Consequences). A behaviour that is not in
Annex X1 is not a deviation.

B1.6 **Clarifications.** Where [P2] is ambiguous or contradicts itself, this document states
the reading the G7250 follows and lists it in Annex X2. A clarification is not a deviation:
it picks one of the readings [P2] allows.

B1.7 **Extensions.** G7250 functions that no standard P2 field describes with identical
meaning are carried in the G7250 register map on the P2 memory-mapped register packets
(0005; B8). Reserved bytes in the standard P2 packets MUST NOT be given a G7250 meaning (0005,
rule 3). This rule also covers the discovery reply, bytes 14–19 and 24–59 included (0003 §1).

B1.8 **Scope of this draft.** B1–B6 cover M1. B7–B11 are complete; the only open values
depend on the hardware and carry their HIS id (0020, Context). Wideband data is M2 (0004
point 4); transmit (B10) is M4 and only after M3 (design note §6; 0007, header).

B1.9 **Main unit only.** Level 1 runs in the main unit; the controller keeps its stock
firmware and is out of scope (part A, "Scope of the firmware"; 0023). Controller inputs on
level 1: A3.7.

### B2 Transport

B2.1 **UDP over IPv4.** All G7250-P2 packets are UDP datagrams; IP and UDP headers are as per
the UDP/IP standards ([P2], "openHPSDR Host to Hardware Protocol" and "openHPSDR Hardware to
Host Protocol"). Diagrams in this document show the UDP payload only, starting at byte 0
([P2], same sections).

B2.2 **Link.** The Ethernet interface is 10/100 Mbit/s; Gigabit is not available. Source:
WCH CH394Q, 10/100 MAC and PHY [CH394] (ON8ST hardware inspection, 2026-11-19; to be
confirmed, HIS-14). A host MUST NOT assume Gigabit throughput. The throughput towards the
host is further limited by the SPI link between processor and CH394Q: at most 62 MHz by the
datasheet [CH394], and on this board about 17 MHz seen in use (MEDIUM), the highest working
clock being pending hardware (P-NET-1; [HIS] HIS 40, `d199d33`; `analysis/ch394-network-budget.md`
§2; 0004, "Throughput arithmetic"; HIS-14). On a link
below 100 Mbit/s the G7250 refuses the session (B5.8; 0004 point 9).

B2.3 **Local ports.** The G7250 MUST open exactly the local UDP ports in Table B2-1, and no
other port for level 1. Sources: [P2] General packet bytes 5–32 (defaults); socket
analysis (`analysis/ch394-network-budget.md`, addendum); 0005; 0008; 0017.

Table B2-1 — Local UDP ports of the G7250 (level 1)

| # | Local port | In (host → G7250) | Out (G7250 → host) | Status in G7250-P2 | Source |
|---|-----------|-------------------|--------------------|--------------------|--------|
| 1 | 1024 | Discovery, General packet, Set IP (Erase/Program ignored, B3.9) | Discovery reply | MUST be open | [P2] "Detailed operation"; General packet; Discovery reply; 0008 points 5–6 |
| 2 | 1025 | DDC-specific packet | High Priority status (B9.2) | MUST be open | [P2] General bytes 5–6, 11–12 |
| 3 | 1026 | DUC-specific packet | Microphone data: not sent in this version (B6.11; 0031) | MUST be open | [P2] General bytes 7–8, 19–20; 0004 point 3 |
| 4 | 1027 | High Priority packet | Wideband ADC0 data, from M2 (B6.10) | MUST be open | [P2] General bytes 9–10, 21–22; 0004 point 4 |
| – | 1028 | DDC audio | – | MUST NOT be opened | 0017 |
| 5 | 1029 | DUC0 I&Q | – | MUST be open (used from M4) | [P2] General bytes 15–16; 0012 |
| 6 | 1035 | – | DDC0 I&Q | MUST be open | [P2] General bytes 17–18 |
| 7 | 7250 | G7250 register map (memory-mapped registers from PC) | G7250 register map (memory-mapped registers from hardware) | MUST be open | [P2] General bytes 29–32; 0005; 0020 §1 |
| 8 | 7251 | Management channel | Management channel | MUST be open, at both levels | 0008, Transport 1; 0020 §2 |

- Ports 1036 and up (DDC1 and further, [P2] General bytes 17–18) are not opened while the
  G7250 implements one DDC (0004 point 1). Number of DDCs: 1 in this version (0004). The FPGA
  has two independent receive tuning words ([HIS] HIS 80, `9d10e72`, MEDIUM); whether they
  are two physical receive paths is pending hardware (P-RX-1; HIS-02), and a second DDC would
  be a later decision (socket budget, 0004). The third tuning word most likely tunes the
  transmit path (P-TX-1; HIS-02, HYPOTHESIS).
- **Memory-mapped register port.** [P2] gives the default of General bytes 29–30 and 31–32 as
  "xxxx", so it is undefined; this document defines it (0005; 0003, "left to spec part B").
  The port number is 7250 (0020 §1). The G7250 MUST use one and the same local port for both
  directions (0005, Consequences; 0017). Using a defined default is a deviation (Annex X1, D2).
- **Port 1028.** The G7250 opens no socket on port 1028 (0017). A host SHOULD NOT send DDC
  audio packets. The G7250 accepts General packet bytes 13–14 and ignores them (0017). How
  the CH394Q reacts to a datagram for a closed port is not stated in [CH394]; the firmware
  rule and the host rule are in B11.9 (0020 §28). This is a deviation (Annex X1, D1).
- **Non-default ports.** [P2] lets the host set every port in General bytes 5–22 and 29–32,
  zero meaning the default. Bytes 5–12, 15–22 and 29–30 name G7250 local ports ([P2] General
  packet). For these, zero and the default value MUST be honoured; any other value MUST NOT be
  honoured: the G7250 keeps the default port, logs the request and sets `LINK_STATUS` bit 17
  (0020 §13; Annex X1, D22). Reason: the default set shares sockets between pairs of fields,
  so a single relocation would need a ninth socket (0020 §13;
  `analysis/ch394-network-budget.md`). In particular no P2 port is ever moved onto the
  management port (A1.3; 0008, Transport 2). Bytes 31–32 name the host's destination port and
  are honoured when non-zero; zero means the owner's source port (B8.1; 0020 §13). A host
  SHOULD send zero in bytes 5–30 (0020 §13).
- **Socket budget.** Table B2-1 uses all 8 CH394Q sockets: 7 for G7250-P2, one for the
  management channel (0008, Socket budget; 0017; [CH394]). The division of the 16 + 16 KB
  socket buffers MUST be that of Table B2-2, set at bring-up (0053; [CH394] sections
  7.2.12–7.2.13: 0–16 KB per socket in 1 KB steps). Every buffer size MUST be 0 or a power of
  two (1, 2, 4, 8 or 16 KB): the socket pointers are 16-bit and wrap at 65536, and how a
  buffer of another size handles that wrap is not documented ([CH394]). Reason: in UDP mode the CH394 has no flow
  control ([CH394] section 12.2.4), so a socket drops a datagram that does not fit; the default
  2 KB holds one 1444-byte packet, and a stop followed at once by a run, or any burst of High
  Priority packets, must not lose the stop.

Table B2-2 — Socket buffers (0053)

| Local port | Receive (KB) | Holds | Transmit (KB) | Holds |
|---|---|---|---|---|
| 1024 | 1 | 16 discovery, General or Set IP packets (60 bytes) | 1 | discovery replies |
| 1025 | 2 | one DDC-specific packet (1444 bytes); a host that sends a second at once may have to resend it | 1 | High Priority status (60 bytes) |
| 1026 | 1 | DUC-specific packets (60 bytes) | 0 | no microphone stream in this version (B6.11; 0031) |
| 1027 | 4 | two High Priority packets (1444 bytes each, 1452 with the receive header) | 2 | one wideband packet (1028 bytes); the processor keeps the frame and hands packets on as space frees (B6.10) |
| 1029 | 4 | two DUC0 I&Q packets; at 192 ksps a packet arrives every 1.24 ms, so the firmware reads the socket at least every 2.5 ms (its own FIFO, B10.7, B11.7) | 0 | — |
| 1035 | 0 | — | 8 | five DDC0 I&Q packets |
| 7250 | 2 | one register-map packet of up to 1444 bytes | 2 | one register push or answer of up to 1444 bytes |
| 7251 | 2 | management requests (at most 1000 bytes each, A1.4) | 2 | one reply (at most 1400 bytes) |
| total | 16 | | 16 | |

Nothing is sent from ports 1026 and 1029, and nothing is received on 1035.

**High Priority bursts** (0054). Reading one 1444-byte datagram from the CH394 takes about
0.7 ms at the SPI clock known to work (about 17 MHz, P-NET-1), against about 0.12 ms on a
100 Mbit/s wire, so no firmware can take an unlimited burst at wire speed. On port 1027 the
G7250 MUST process without loss:

1. any two High Priority packets that arrive back to back at wire speed, if no third one
   arrives within 2 ms of the first; and
2. a sequence of any length in which the packets arrive at least 1 ms apart.

These figures hold at the SPI clock known to work and do not depend on P-NET-1; a faster clock
(up to 62 MHz, [CH394]) only adds margin. `GET_STATS` MUST count every High Priority packet read
from port 1027, from any source and before the owner check, under the name `hp_rx` (0058), so
that a lost packet shows even when it repeats the state in force. The [CH394] datasheet gives no
per-socket receive-overflow count for UDP sockets, so none is defined; a drop shows as a
shortfall in `hp_rx`.

**Flash work** (0055). Writing or erasing the MCU's flash can stall the processor for up to a
few seconds (a sector erase). While a management write that writes flash (`SET_CONFIG`,
`SET_FIRST_SECRET`, `CHANGE_SECRET`, `FACTORY_RESET`, `SET_CALIBRATION`, `CLEAR_CALIBRATION`,
and a compaction of the store that such a write needs), or a firmware update (A1.8) is in
progress, the rules above do not apply: High Priority packets beyond the two that port 1027
holds may be lost, and a P2 session may end by its watchdog (B5.6) and be restarted by the host
(B5.4). To keep this away from transmit:

- flash work MUST NOT start while TX is armed or keyed on any level: a `SET_CONFIG` while armed
  first unkeys and disarms (A1.6), the other writes are refused with status 50 while armed or
  keyed, and updates are refused as A1.8 says;
- the G7250 MUST NOT start flash work on its own while a P2 session runs, except the trial-boot
  confirmation (A1.8), which happens while no arm is possible (`MAINTENANCE`); a store compaction
  runs only as part of the management write that needs it;
- the host learns of it from the management reply, which is sent after the flash work has
  ended, and from the update state (`UPDATE_STATUS`, `IDENTIFY`). A host SHOULD NOT send High Priority packets closer
than 1 ms apart, except a stop or run directly after another packet, which item 1 covers.

B2.4 **Remote endpoint.** The G7250 MUST reply to the discovery from local port 1024 to the
IP address and source port of the host's discovery packet ([P2] "Detailed operation";
Discovery reply, "Source Port", "Destination Port"). The G7250 MUST send every stream of a
session (DDC0 I&Q, High Priority status, microphone, wideband and register pushes) to the
owner's source IP address and UDP port (A2.2), fixed when the session starts (at a takeover:
the new owner's, A2.4). This is the one destination rule; register packets go to General
bytes 31–32 at the owner's IP address when those are non-zero (B8.1; 0020 §13; 0022 §12).
[P2] sends the status, microphone and wideband streams to "the Source Port of the Host that
initiated the Discovery Packet" and DDC I&Q to "the Source Port of the Host that initiated
the Discovery Command", and has the host send its General packet from that same port ([P2]
High Priority Status, Microphone, Wideband and DDC packets, "Destination Port"; General
packet, "Source Port"), so for a P2 host both are one port; where they differ, the owner's
port is used (0020 §14; Annex X2, C12). Answers to a non-owner's register request go to that
request's source address and port (B8.5).

B2.4a **Local source ports.** So that a host can demultiplex by sender port, the G7250 MUST
send: discovery replies from 1024, High Priority status from 1025, microphone data from 1026,
wideband data from 1027, DDC0 I&Q from 1035, register-map packets from 7250 and management
replies from 7251 (Table B2-1; 0020 §14).

B2.5 **Byte order and integer interpretation.** Multi-byte fields MUST be sent and read MSB
first (network byte order, big-endian), and all values are unsigned integers unless stated
otherwise ([P2] "openHPSDR Host to Hardware Protocol"; General packet byte 39: zero selects
Big-Endian). The same applies from the G7250 to the host; [P2] "openHPSDR Hardware to Host
Protocol" writes "MSB first (little Endian)", which the G7250 reads as MSB first (Annex X2,
C1). The G7250 offers no other endian mode (B3.7; 0004 point 1).

B2.6 **Packet length.** No G7250-P2 UDP payload exceeds 1444 bytes (byte 0 to byte 1443),
the largest payload in [P2] (High Priority packet, DDC-specific packet, memory-mapped
register packets, DDC I&Q packet, DUC I&Q packet, all ending at byte 1443).

B2.7 **Sequence numbers.** Every packet starts with a 4-byte sequence number, bytes 0–3,
MSB first ([P2] "Detailed operation"). For each local port and direction the G7250 sends on,
it MUST keep its own counter that starts at 0x00000000, increments by one per packet and
rolls over after 0xFFFFFFFF ([P2] DDC packet, Memory mapped registers from hardware,
"Sequence Number"). The counter MUST be reset to zero at power-on and every other reset,
whenever a stop is received (B5.4), and at a takeover (A2.4) ([P2] DDC packet, "Sequence
Number"; 0022 §3), and at a session start by a host other than the last owner, which for
that host is as a takeover. At every other session end (watchdog standby, link loss, an
address or level change) the counters keep their value (0022 §86 item 10),
and a restart by the same host (B5.4) continues them, so that the host does not read a
restart at 0 as a takeover (A2.4, host guidance) (0022 §83 item 4).

**Port 7250 counters per destination** (0051). On local port 7250 the "direction" is the
destination: the G7250 MUST keep one counter for the packets to the owner's register
destination (the session's counter, reset and continued as above), and one separate counter
for all answers to hosts that are not the owner (`NOT_OWNER` and `CAP_REQUEST` answers, B8.4,
B8.5), which no session event resets, only a reset of the radio. The packets to an old owner
after a takeover (the three copies of the last `TX_ARM_STATE` and the repeated `TAKEOVER`
notice, A2.4) continue the old session's counter towards that host. So a host that checks its
own register stream for loss sees no gap caused by answers to other hosts.
Exception, as [P2] specifies: the wideband counter restarts at 0 for every frame, so the 8
packets of a frame carry 0–7 ([P2] Wideband Data packet, pp. 52–53: "set to zero … whenever a
block of raw samples has been sent"; B6.10; Annex X2, C16; 0022 §21). The discovery reply
always carries sequence number 0 ([P2] Discovery reply bytes 0–3). On the receive side the
G7250 SHOULD NOT reject or reorder a packet because of its sequence number ([P2] Discovery,
Erase, Set IP, General, DDC-specific, DUC-specific and High Priority packets give the host's
sequence number as 0x00000000, Annex X2, C2; 0020 §19; 0022 §21). The one exception is a
High Priority packet from a host that numbers its packets (B11.6a; 0022 §35). A DDC0 rate change, a
resume after a refusal, or a resume after DDC-specific byte 7 bit [0] was cleared MUST NOT
restart an outgoing counter (0020 §19). Gaps are handled in B11.6.

### B3 Discovery

B3.1 **Listening.** The G7250 MUST listen on UDP port 1024 for discovery packets at all
times while level 1 is reachable, including while running ([P2] "Detailed operation";
Discovery reply byte 4, value 0x03), and while level 2 is active, answering with byte 4 = 0x03
(0003, Resolved; A2.5). Discovery is also answered on a link below 100 Mbit/s (0004 point 9).

B3.2 **Discovery request.** A discovery request is a UDP payload to port 1024 with byte 4 =
0x02 ([P2] Discovery packet). Bytes 0–3 are the sequence number, set to 0x00000000 by the
host; bytes 5–59 are zero ([P2] Discovery packet). The G7250:

- MUST answer a request sent to its own unicast address, to the limited broadcast
  255.255.255.255, and to the directed broadcast address of its subnet ([P2] Discovery packet,
  "Destination Address");
- MUST answer regardless of the endian mode selected by an earlier General packet ([P2]
  Discovery packet, NOTE; Discovery reply byte 22, NOTE);
- SHOULD NOT require bytes 0–3 or 5–59 to be zero, nor the packet to be longer than 5
  bytes, to answer (B11.3; 0020 §28);
- MUST answer every request with exactly one reply, within the rate limit of B11.14: at
  most 10 replies per second per source IP address and port and 50 per second in total;
  requests above it are dropped and counted (0022 §16);
- MUST NOT answer, and MUST drop and count, a datagram to port 1024 whose source port is 1024.
  A G7250 discovery reply has byte 4 = 0x02 and comes from port 1024 (B3.3), so without this
  rule two radios could answer each other's replies without end. A host SHOULD NOT send
  discovery from local port 1024 (0022 §73 item 20).

A discovery from the owner (same IP address and UDP port, A2.2) while TX is armed drops the
arm (`REDISCOVERY`, Table A3-1 event 12). A discovery from any other host has no effect on the
session. This includes a discovery from the owner's IP address on another UDP port, for
example a host's separate discovery socket: it is answered with 0x03 while the session runs
(B3.4) and MUST NOT drop the arm (0020 §14; 0007, Resolved 4; [AETHER-SRC]
`AnanDiscovery.cpp:240–263`).

B3.3 **Discovery reply.** The G7250 MUST reply with the [P2] short reply (format 1), a 60-byte
payload (bytes 0–59), from local port 1024 to the host's IP address and source port ([P2]
Discovery reply, "Source Port", "Destination Port"; 0003 §1). The reply always uses the
format below, irrespective of the selected endian mode ([P2] Discovery reply byte 22, NOTE).
The G7250 MUST NOT report board type 254 or 255, MUST NOT use byte 4 = 0xFE or 0xFF, and MUST
NOT send the Appendix C reply (0003 §1).

Table B3-1 — G7250 discovery reply ([P2] Discovery reply, format 1; 0003 §1)

| Byte | [P2] field | G7250 value | Source |
|------|-----------|-------------|--------|
| 0–3 | Seq # | 0x00000000 | [P2] Discovery reply bytes 0–3 |
| 4 | Response to discovery | 0x02 or 0x03, see B3.4 | [P2] byte 4; 0003 §1 |
| 5–10 | Board MAC, MSB first | the G7250's MAC address, read from the CH394's own registers ([HIS] HIS 40; HIS-15) | [P2] bytes 5–10 |
| 11 | Board type | 200 (0xC8) | 0002; 0003 §1; [P2] byte 11 |
| 12 | openHPSDR protocol version supported | 44 (V4.4), see B3.5 | [P2] byte 12; B1.3 |
| 13 | Firmware code version | see B3.5 | [P2] byte 13; 0003 §1 |
| 14–19 | Mercury 0–3, Penny, Metis code versions | 0x00 | [P2] bytes 14–19 (Atlas only); 0003 §1 |
| 20 | Number of DDCs implemented | equal to record field `num_ddc`; 1 in this version (0004; HIS-02, [HIS] HIS 80) | [P2] byte 20; 0003 §1; 0004 point 8 |
| 21 | Frequency or phase word | 1 = phase word, always, with the fixed scale of B3.8 (0022 §81) | [P2] byte 21; General packet byte 37 NOTE |
| 22 | Available endian modes | 0x00 (Big-Endian, 3-byte I&Q only), see B3.7 | [P2] byte 22; 0004 point 1 |
| 23 | Beta version | 0x00 for a release; non-zero for a beta build | [P2] byte 23; 0003 §1 |
| 24–59 | Reserved (AD0ES) / unassigned | 0x00, no G7250 meaning | [P2] byte 24; 0003 §1 |

B3.4 **Status byte (byte 4).**

- The G7250 MUST send 0x02 when it is not running, or when it is running and the discovery
  comes from the owner ([P2] Discovery reply byte 4; A2.2).
- The G7250 MUST send 0x03 when it is running and the discovery comes from any host that is
  not the owner, including the owner's IP address with another UDP source port ([P2] Discovery
  reply byte 4; 0007, Resolved 4; Annex X2, C7). "Running" means the run bit is set (B5.3).
- The G7250 MUST send 0x03 while level 2 is active (0003, Resolved; Annex X2, C8).
- The G7250 MUST NOT send 0xFE or 0xFF (0003 §1; Annex X2, C3).

B3.5 **Version encoding.** [P2] bytes 12 and 13 are single bytes that the host reads as a
decimal number with one implied decimal place: 104 is version 10.4 ([P2] Discovery reply
"Byte 12", "Byte 13").

- Byte 12 MUST be 44, since the normative reference is V4.4 (B1.3); it MUST equal record
  field `p2_version` (0003 §3).
- Byte 13 MUST carry the G7250 firmware version in P2 form, major × 10 + minor ("15 = 1.5"),
  taken from record field `fw_version` (0003 §1 and §3). The full version, with build number,
  is in `fw_version` (B4). G7250 releases are numbered major.minor with minor 0–9 (after x.9
  comes (x+1).0) plus a 16-bit build number; byte 13 = major × 10 + minor, capped at 255
  (0020 §15). Byte 23 is non-zero for a beta build ([P2] byte 23).

B3.6 **MAC address.** Bytes 5–10 MUST hold the MAC address the G7250 uses on the link, MSB
first ([P2] Discovery reply, "MAC"). The MAC address MUST be unique per unit, because the
host uses it to tell radios apart and to address Set IP packets ([P2] Set IP Address packet).
The record carries no serial number; the MAC suffices (0003, Resolved). Where it is stored
and whether it is unique per unit: the CH394 provides it in its own registers ([HIS] HIS 40);
uniqueness is pending hardware (P-NET-2; HIS-15).

B3.7 **Endian modes and DDC data format (byte 22).** The G7250 offers only Big-Endian with
3-byte I&Q: no little-endian, float or double format (0004 point 1, option 2A). Byte 22 MUST
therefore be 0x00, which [P2] defines as "Big-Endian and I&Q data is in 3 Byte format" ([P2]
Discovery reply byte 22). A General packet byte 39 that asks for another format is refused
(B6.3; B11.2).

B3.8 **Frequency or phase word (byte 21)** (0022 §81). The G7250 MUST report 1 (phase word)
in byte 21 and MUST accept phase words in the High Priority frequency fields ([P2] High
Priority packet bytes 9–12, 329–332), with General packet byte 37 bit [3] = 1. The phase word
is a firmware convention with a fixed scale, the **virtual DSP clock** of 122 880 000 Hz:
`phase_word = 2^32 × f(Hz) / 122 880 000` ([P2] General packet byte 37, Discovery reply byte
21), whatever clock the hardware uses. The firmware converts the phase word to its own
tuning word; the frequency it tunes is `phase_word × 122 880 000 / 2^32` Hz, to within the
resolution of its own clock. Record field `dsp_clock_hz` reports 122 880 000 (B4.8; 0003 §3;
Annex X2, C4). The real clock of the DDC and DUC is HIS-04; it does not enter the wire
format. Reason: [P2] states that all openHPSDR FPGA code requires phase words ([P2] General
packet byte 37 NOTE), and every board of [P2] Appendix A that gives a DSP clock gives
122.88 MHz ([P2] Appendix A, Boards 1–4: "DSP clock frequency (Hz) … 122.88MHz" or
"122,880,000", and
"DDC Frequency or Phase Word … Phase"), so a host that does not read the record computes the
right word (B4.11).
A General packet whose byte 37 bit [3] differs from byte 21 asks for an encoding the G7250 does
not use, so every frequency of that host would be misread. The G7250 MUST keep reading the
High Priority frequency fields in the encoding of byte 21, MUST NOT run DDC0 and MUST NOT key
while the mismatch stands (`STREAM_STATUS` reason 13 for DDC0 and DUC0; A3.1a), and MUST log
it; a General packet with the matching bit ends it (0022 §73 item 12).

B3.9 **Other port-1024 commands.**

- **Erase (0x04) and Program (0x05)** ([P2] Erase and Program packets) MUST be ignored
  without a reply; a P2 host then hits its own timeout (0008, Decision point 5). They MUST NOT
  change the firmware or any stored state. The G7250 does not claim the V3.8 firmware checksum
  feature ([P2] revision history V3.8; Command Reply bytes 13–14; 0008 point 5). Firmware is
  updated only through the management channel (A1.8). Deviation: Annex X1, D12.
- **Set IP Address (0x03)** ([P2] Set IP Address packet) MUST be accepted exactly as [P2]
  specifies (0008 point 6; Resolved 2): a packet to 255.255.255.255:1024 whose bytes 5–10 hold
  the G7250's MAC address sets the IP address in bytes 11–14; 0.0.0.0 selects DHCP; the G7250
  stores it and responds with a Discovery reply ([P2] Set IP Address packet; Command reply
  packet, "Set IP"). The G7250 MUST NOT add preconditions: no refusal while armed or while a
  P2 session runs, and no option to disable it (0008 point 6). The change MUST be logged with
  its source address (A1.7; 0008 L-2), which is not visible on the wire. The stored value is
  the network configuration of A1.6 (C-1), applied after the reply, with the fallback on an
  address conflict (0008 point 6; C-4). Applying a new address ends the network set-up of a
  running session; while armed this is drop event 9 (`SESSION_CHANGED`, Table A3-1). A Set IP
  packet whose bytes 5–10 hold another MAC address is not for this unit and MUST be ignored
  ([P2] Set IP Address packet, "MAC Address"). A Set IP sent to the unit's unicast address or
  to the subnet's directed broadcast address MUST be accepted under the same rules, but only
  from a source address inside the radio's own subnet; from any other source it MUST be
  ignored and counted (0020 §21; 0022 §70; Annex X2, C13). A Set IP to the limited broadcast
  255.255.255.255 is accepted from any source address, because a router does not forward it,
  so it comes from the radio's own link; a host with a wrong address set-up can therefore still
  correct the radio ([P2] Set IP Address packet, "Broadcast to Port 1024"; RFC 1812 section
  5.3.5.1: a router does not forward a limited broadcast). If the Ethernet controller does not
  report the destination address of a received datagram (whether the CH394 does is not known,
  pending hardware P-NET-3; [HIS] HIS 40, HIS 90), the G7250 MUST treat every Set IP as one to the unicast or
  directed broadcast address: accepted only from a source inside the radio's own subnet, and
  from any source while the radio uses an IPv4 link-local address [RFC3927], since a
  link-local radio is reachable only on its own link (0039). A host with a wrong address
  set-up then gives itself an address in the radio's subnet, which the discovery reply shows,
  before it sends Set IP.
- **Netmask and gateway after Set IP** (0039). [P2] Set IP carries no netmask. The G7250
  MUST keep the netmask in use if the new address is a host address of that subnet, and
  otherwise use 255.255.255.0; it MUST keep the gateway if it lies inside the resulting subnet,
  and otherwise clear it. 0.0.0.0 selects DHCP (A1.11), which brings its own netmask and
  gateway. The values applied are logged (A1.7). Risk stated in 0008 S7, as
  narrowed by 0022 §70: any host on the same link (limited broadcast) or in the same subnet
  that knows the MAC can change the address and so cut a running session; a host behind a
  router cannot; none can key the radio.

### B4 Capability block

B4.1 **Form.** The capability block is a **capability record**: a read-only range of the
G7250 register map (B8), sent by the G7250 in standard [P2] "Memory mapped registers from
hardware" packets (0003, Decision option A). It is not in the discovery reply (0003 §1).

B4.2 **Place.** The record occupies `CAP_BASE` to `CAP_BASE + CAP_LENGTH − 1`, at most 239
registers, so that one memory-mapped packet (240 pairs [P2]) carries the request echo plus the
whole record and a host always sees one consistent copy (0003 §2). `CAP_BASE` = 0x0000
(0003 §2; 0020 §3): the version 1.1 record occupies 0x0000–0x00CE, and the block
0x0000–0x00FF is reserved for it (B8.3).

B4.3 **Encoding.** One field per 32-bit register, big-endian ([P2] memory-mapped packets, Data
[31:0]; 0003 §2). Text is ASCII, 4 characters per register, first character in bits [31:24],
padded with 0x00. Frequencies in Hz. Power in units of 0.1 W. Bit numbers: 0 = LSB (0003 §2,
§3).

B4.4 **When the record is sent** (0003 §2):

1. **On request.** The host writes 0x00000001 to the register `CAP_REQUEST` (0x0100, B8;
   0020 §3, §5). The G7250 MUST answer with one memory-mapped packet that holds the echo of
   the `CAP_REQUEST` pair and the whole record. Any other value MUST be rejected with
   `WRITE_STATUS` reason `BAD_VALUE` and no record (B8.4; 0020 §5).
2. **Unasked, once per session** (0003 §2 places it at the host's first General packet;
   0022 §49): at the session start if the owner's set-up phase holds a General packet
   (B5.1a), otherwise at the owner's first General packet of the session; at a takeover by a
   General packet, in answer to that packet (A2.4 step 4). It goes to the owner's register
   destination (item 3). It has its own limit, separate from the `CAP_REQUEST` answers: it
   is left out only if an unasked record went to the same destination IP address and port
   less than 1 s before, so that a takeover ping-pong cannot multiply it (B11.14). A
   `CAP_REQUEST` answer never suppresses it and it uses up no `CAP_REQUEST` allowance, so a
   host that reads the record before its run bit (B5.1 step 2) still gets the unasked record at
   its session start (0022 §49; §73 item 8). A General packet while no session runs (set-up
   phase) gets no record. At a restart by the same host (B5.4 item 1) the General packet
   fields carried over from its ended session count as a General packet of the set-up phase:
   the unasked record is sent at the session start, to the register destination carried
   over (General bytes 31–32, item 3), within the 1 s limit above, so that a host whose
   keepalive is the High Priority packet alone gets the record again (0022 §86 item 6).
   **In parts** (0059). Unlike the answer of item 1, the unasked record MAY be sent in up to
   three memory-mapped packets, so that no single send blocks the reading of port 1027 (B2.3,
   "High Priority bursts"). The parts go to the same register destination, in ascending
   register address order, with consecutive numbers of the session's port 7250 counter (B2.7),
   all within 50 ms of the first; together they hold every register of the record exactly once.
   The record is constant within a session (B4.7), so the parts never disagree; a host MUST
   NOT assume the unasked record arrives in one packet, and between parts it may see other
   register pushes. A host that needs the record in one packet uses `CAP_REQUEST` (item 1).
3. **Destination.** To the source IP address and UDP port of the request when the requesting
   host is not the owner (has no session); otherwise to the owner's memory-mapped port set in
   General packet bytes 31–32, or to the owner's source port (A2.2) if those bytes are zero
   ([P2] General packet bytes 31–32; 0003 §2; 0020 §13). Answers to a host that is not the
   owner are rate-limited: at most once per 1 s per source IP address and port and 10 per
   second in total; requests above that are dropped and counted (B11.14; 0022 §16).
4. **Retry** (0022 §83 item 6). A host that has no record 1 s after its `CAP_REQUEST` SHOULD
   send it again, at most once per second, the rate the G7250 answers a non-owner (item 3),
   and SHOULD tell the user after 5 tries without an answer. A request repeated sooner may be
   dropped by the rate limit and does not bring the answer sooner.

B4.5 **Request-only packet is not C&C.** A memory-mapped packet that contains only a write to
`CAP_REQUEST` MUST NOT take over a session, MUST NOT start one, and MUST NOT feed the watchdog
(0003 §2; Resolved). More generally, no memory-mapped packet is C&C on the G7250 (B5.6; 0007
sub-decision 2.1). This holds also when a host sends the request from its future session
socket before its first General packet: the answer goes to that socket, and the port does not
become the owner; only the run bit (while no session runs) or a takeover by a C&C packet makes
a host the owner (A2.3, A2.4; 0020 §14; 0022 §3). Listed in Annex X1, D11.

B4.6 **Read-only.** Writes to any address in the record range MUST be ignored, and the G7250
MUST answer with the current values of the addresses written, so that the host sees that the
write had no effect (0003 §2), plus `WRITE_STATUS` reason `READ_ONLY` (B8.4; 0020 §11). The
pair (0x0000, 0x00000000) is padding and is not answered (B8.1; 0020 §4).

B4.7 **Stability.** Record values MUST be constant for the duration of a session. They MAY
change only between sessions: after a firmware update, a region change of the TX bands, a
change of the active level or a new per-unit calibration (0003 §2). The record holds only the
minimum link speed (`link`), which does not change; the current link speed is in
`LINK_STATUS` (0022 §73 item 19).
Dynamic state is not in the record: TX armed, fault state and limits in force, telemetry
values, current link speed and session refusal reason are separate status registers (B8;
0003 §2).

B4.8 **Fields, record version 1.1.** Offsets are relative to `CAP_BASE`. "HIS-nn" means the
value is not known yet; the value shown is the current assumption (0003 §3). Offsets
0x00–0xB8 are version 1.0 (0003 §3); offsets 0xB9–0xCE are added by version 1.1 (B4.9; 0020
§26). The G7250 sends version 1.1.

Table B4-1 — Capability record, version 1.1 (0003 §3; 0020 §26)

| Offset | Name | Content | Value / source |
|--------|------|---------|----------------|
| 0x00 | `cap_magic` | ASCII "G725" = 0x47373235; never changes | 0003 §3, §4 rule 5 |
| 0x01 | `cap_version` | [31:16] major, [15:0] minor of the record format | 0x00010001 (1.1) (0020 §26) |
| 0x02 | `cap_length` | number of registers in the record, header included | 0xCF (207) for version 1.1; 0xB9 (185) was version 1.0 (0003 §3, notes; 0020 §26) |
| 0x03 | `regmap_version` | [31:16] major, [15:0] minor of the G7250 register map | 0x00010000 (1.0) (0020 §6) |
| 0x04–0x07 | `product_name` | "G7250-P2", padded with 0x00: 0x47373235, 0x302D5032, 0, 0 | 0002, 0016 |
| 0x08 | `fw_version` | [31:24] major, [23:16] minor, [15:0] build | as built; B3.5 |
| 0x09 | `p2_version` | as discovery byte 12 | 44 ([P2]; B3.5) |
| 0x0A | `hw_revision` | main-unit hardware revision, 0 = not identified | HIS-18 |
| 0x0B | `level` | bit 0 level 1 present (always 1); bit 1 level 2 present in this firmware; bit 2 level 2 active; bits [7:4] active level (0 none, 1, 2) | 0011; 0003, Resolved; level 2 only after G6 |
| 0x0C | `streams` | bit 0 RX IQ; bit 1 TX IQ (DUC); bit 2 DDC audio (always 0); bit 3 microphone data; bit 4 wideband data; bit 5 IQ time stamping | bit 0 = 1 once the FPGA receive parameters are set (P-RX-2), 0 until then (DDC0 not offered, B6.1; 0042); bit 1 per `tx` bit 1; bit 2 = 0 (0017); bit 3 = 0 in this version (no audio, 0031; HIS-17); bit 4 = 0 before M2, 1 from M2 (0004 point 4; HIS-01, HIS-18); bit 5 = 0 (0004 point 1) |
| 0x0D | `num_adc` | number of RF ADCs | HIS-01, HIS-03 (assumed 1) |
| 0x0E | `num_ddc` | number of DDCs; equals discovery byte 20; 1 in this version | 0004; HIS-02 |
| 0x0F | `num_duc` | number of DUCs | assumed 1 [G7250-MAN]; HIS-18 |
| 0x10 | `ddc_rates` | bit mask: bit 0 48, bit 1 96, bit 2 192, bit 3 384, bit 4 768, bit 5 1536 ksps | 0x0F if 384 ksps is offered, else 0x07 (B6.7; 0004 point 6); bits 4, 5 always 0 (0004 point 7) |
| 0x11 | `duc_format` | [31:16] TX IQ rate in ksps, [7:0] bits per sample | 0x00C00018 (192 ksps, 24 bits; 0004 point 2); HIS-18 |
| 0x12 | `dsp_clock_hz` | scale of the phase word in Hz (virtual DSP clock, B3.8), as [P2] Appendix C bytes 27–30; not necessarily the hardware clock | 122 880 000, always (0022 §81); the hardware clock is HIS-04 |
| 0x13 | `rx_range_count` | number of valid receive ranges (max 4) | 2 [G7250-MAN] |
| 0x14–0x1B | `rx_range[0..3]` | pairs of start Hz, stop Hz | 500,000–30,000,000; 50,000,000–54,000,000 [G7250-MAN]; unused pairs 0 |
| 0x1C | `antennas` | [7:0] number of antenna connectors; bit 8 host can select; bit 9 separate RX-only input | 2 [G7250-MAN]; bit 8 HIS-06; bit 9 HIS-06, HIS-07 (assumed 0) |
| 0x1D | `atu` | bit 0 internal ATU present; bit 1 host can control it; bit 2 per-band memory; bit 3 external tuner interface present; bit 4 host can control the external tuner interface | bits 0, 2, 3 = 1 [G7250-MAN]; bit 1 = 1 (this firmware drives the ATU, [HIS] HIS 85; 0032); bit 4 = 0 in this version (SEND and TUNER interface pending hardware, P-SYS-8; HIS-09) |
| 0x1E | `atu_range` | [31:16] minimum, [15:0] maximum load in Ω | 20, 150 [G7250-MAN] |
| 0x1F | `telemetry` | bit 0 forward power; bit 1 reverse power; bit 2 PA temperature; bit 3 supply voltage; bit 4 supply current; bit 5 PA current; bit 6 ADC overload | bits 0, 1, 2, 3, 5 = 1 for the ADC1 inputs of [HIS] HIS 85 (forward, reflected, PA temperature, supply voltage, PA supply current Idd); bit 4 = 0 (no separate supply-current input); a value stays invalid in `SENSOR_VALID` until its calibration is set (P-SEN-1, P-SEN-2); bit 6 = 1 (ADC peak, [HIS] HIS 80); meaning in B9 (0006; HIS-11) |
| 0x20 | `tx` | bit 0 TX hardware present; bit 1 TX implemented in this firmware; bit 2 TX arm required (always 1); bit 3 declared TX mode class required for arm (always 1); bit 4 TX calibrated: the calibration set holds every parameter of the Annex P freeze rule (A1.12) | bit 0 = 1 [G7250-MAN]; bit 1 = 1 in every r1.0 firmware (TX is part of r1.0, 0036), 0 only in a build before M4 that has no key path; bit 2 = 1 (0015); bit 3 = 1 (0007); bit 4 from the calibration set, 0 until it is complete, and always 0 while bit 1 is 0 (0039) |
| 0x21 | `tx_band_count` | number of valid entries below (max 16) | 10 [G7250-MAN] |
| 0x22–0x51 | `tx_band[0..15]` | 3 registers per band: start Hz; stop Hz; [31:16] maximum power SSB/CW/FM, [15:0] maximum power AM, both in 0.1 W | ranges per region [G7250-MAN]; 1000 (100 W) and 250 (25 W) [G7250-MAN]; 50 MHz pending hardware (P-PWR-2; HIS-12; 90 W in [G7250-PUDX], [G7250-RIGPIX]) |
| 0x52 | `mgmt` | bit 0 management channel present; [31:16] its port | 0x1C530001: bit 0 = 1 (0008), port 7251 (A1.3; 0020 §2) |
| 0x53 | `regmap_groups` | bit mask of register groups present in the G7250 map | bit *n* = group *n* of Table B8-1: bits 0, 1 = 1; bit 2 from M3; bits 3, 4, 5 = 1 (sensors, ATU and antenna groups present, their values pending hardware: P-SEN-1, P-ATU-1, P-ANT-1); bits 6, 7 = 0 in this version (front end and external interfaces, P-RF-4, P-SYS-8; HIS-07, HIS-09) (B8.3; 0020 §3; 0035) |
| 0x54 | `link` | [15:0] minimum link speed in Mbit/s for a session | 100 (0004 point 9; [CH394]) |
| 0x55 | `wb_format` | wideband: [31:16] samples per packet, [15:8] bits per sample, [7:0] update rate in ms; 0 when `streams` bit 4 = 0 | from M2: 0x02001032 (512, 16, 50 ms; 0004 point 4) |
| 0x56 | `wb_frame` | wideband: [7:0] packets per frame; 0 when `streams` bit 4 = 0 | from M2: 8 (0004 point 4) |
| 0x57 | `cal_info` | bit 0 power constants per band (set *i* applies to `tx_band[i]`; else set 0 applies to all bands); bits 1–4 forward power, reverse power, exciter power, supply volts filled; [15:8] number of power constant sets (1, or `tx_band_count`); [31:16] ADC full-scale divisor *N* | 0006; per unit and per band HIS-11; *N* = 4095 in [P2] Appendix A; until the calibration set holds P-SEN-1, bits 0–4 = 0 (no constants) while the raw status words are still filled (B9.3), and the engineering-unit registers stay invalid in `SENSOR_VALID` (0039) |
| 0x58 | `cal_supply_c` | supply constant *c* in units of 10⁻⁶, for `V = x / N × c` | this unit's calibration (0006; HIS-11) |
| 0x59–0xB8 | `cal_power[0..15]` | 6 registers per set, units of 10⁻⁶: forward *c1*, *c2*; reverse *c1*, *c2*; exciter *c1*, *c2*, for `W = (x / N × c1)² / c2` | this unit's calibration (0006; HIS-11); unused sets 0 |
| 0xB9 | `status_fill` | High Priority status fields filled: bit 0 byte 4 [0]; bit 1 byte 4 [1]–[2]; bit 2 byte 4 [4]; bit 3 byte 5; bit 4 bytes 6–7; bit 5 bytes 14–15; bit 6 bytes 22–23; bit 7 byte 30; bit 8 bytes 31–36; bit 9 bytes 39–40; bit 10 bytes 49–50 | per Table B9-1 (0006; 0020 §26); HIS-11, HIS-17 |
| 0xBA | `clock_ref` | bit 0 reference PLL present; 0 means "no reference PLL" and status byte 4 [4] is always 0 | HIS-05 (0020 §26) |
| 0xBB | `coupler` | [3:0] coupler position: 0 not reported, 1 between PA and ATU, 2 antenna side of the ATU | HIS-08, HIS-11 (0006; 0020 §26) |
| 0xBC | `tlm_rates` | [31:16] snapshot period, ms; [15:8] fast period during TX, ms; [7:0] minimum spacing of event pushes, ms | 1000, 10, 1 (B9.4; 0006; 0020 §26) |
| 0xBD | `status_rates` | [31:16] High Priority status period when not transmitting, ms; [15:0] while transmitting, ms | 50, 1 (B9.4; 0020 §26) |
| 0xBE | `tlm_count` | number of valid telemetry descriptors (max 8) | HIS-11 (0020 §26) |
| 0xBF–0xCE | `tlm_desc[0..7]` | 2 registers each: [31:16] telemetry register address, [15:0] sensor update period in ms; [31:16] resolution, [15:0] hysteresis, both in the register's milli-units; a register is present when it has a descriptor | HIS-11 (0006; 0020 §26); unused entries 0 |

Notes (0003 §3):

- Transmit-power values are the nominal maxima in the configured region, not the limits in
  force; limits in force are status (B8; A3.4).
- Fields with an Appendix C counterpart (`num_adc` byte 36, `num_ddc` byte 39, `dsp_clock_hz`
  bytes 27–30, DDC audio byte 59, microphone byte 72, DUC rate and format bytes 79–80) have the
  same meaning as there ([P2] Appendix C).
- `streams` bit 2 and `num_ddc` MUST agree with what the radio does: no socket on port 1028
  (0017), and only as many DDC I&Q ports as `num_ddc`.
- The record carries no serial number (0003, Resolved).

B4.9 **Fields added in version 1.1.** 0006 ("What the host learns from the capability block")
asks the record to also state: for each standard status field whether it is filled
(`status_fill`); the coupler position relative to the ATU (`coupler`); for each telemetry
register presence, resolution, sensor update rate and hysteresis (`tlm_count`, `tlm_desc`);
the telemetry rates in use (`tlm_rates`, `status_rates`); and "no reference PLL"
(`clock_ref`; B9.2, byte 4 bit [4]). Version 1.0 (0003 §3) covered only part of this
(`telemetry`, `cal_info`). Version 1.1 adds these fields at the end of the record (0003 §4
rule 1; 0020 §26); a version 1.0 host ignores them (B4.10 rule 1).

B4.10 **Versioning rules** (0003 §4):

1. A `cap_version` minor change only adds fields at the end of the record and increases
   `cap_length`. A host MUST read `cap_length` and MUST ignore registers beyond the fields it
   knows.
2. A field the host knows that lies beyond `cap_length` counts as absent: "no" for flags,
   "not reported" for numbers.
3. Unused bits in a bit mask MUST be 0; they MAY get a meaning in a later minor version, always
   so that 0 means the old behaviour. Hosts MUST ignore bits they do not know.
4. A field MUST NOT be moved, removed or given another meaning within one major version. A
   host that does not know the major version MUST NOT rely on the record; it MAY still receive
   with the standard P2 set-up (0015) and SHOULD tell the user that the firmware is newer than
   the host.
5. `cap_magic` never changes; a different magic means "not a G7250 capability record".
6. `regmap_version` follows the same rules, independently. A host decides which register
   groups to use from `regmap_groups`, not by comparing version numbers.
7. The record MUST NOT exceed 239 registers. If more is needed, a second range with its own
   request register is added and announced by a bit in `regmap_groups`.

B4.11 **Host that does not know board type 200.** It sees a standard short reply with an
unknown board type, can set up receive from bytes 20–22 and the standard DDC packets (byte 21
= 1: phase words; a host that assumes the 122.88 MHz clock of the [P2] Appendix A boards
computes them right, B3.8; 0022 §81), never
reads the record, never writes the register map, and so can never arm or transmit (0003 §5;
0015). Rates outside `ddc_rates` are refused (B6.7).

### B5 Session, ownership and watchdog

B5.1 **Order of a session start.** The host establishes a session as follows ([P2]
"Detailed operation"; General packet introduction; DDC specific packet introduction; High
Priority data packet introduction):

1. discovery request and reply on port 1024 (B3);
2. a G7250-aware host reads the capability record (B4.4; 0003, Consequences);
3. General packet to port 1024; the first General packet after a discovery reply MUST be
   Big-Endian, and its byte 39 selects the data format for all later packets ([P2] General
   packet byte 39); the G7250 keeps it for the host's session (set-up phase, B5.1a) and sends
   the capability record once, unasked, when the session starts (B4.4 item 2; 0022 §49);
4. DDC-specific packet to port 1025 (B6.5), and DUC-specific packet to port 1026 (B10.6);
5. High Priority packet to port 1027 with byte 4 bit [0] (run) set. This starts the session
   (A2.3; 0007, Terms); the packets of steps 3 and 4 count for it (B5.1a).

B5.1a **Set-up phase** (0022 §34; Annex X2, C18). [P2] has the host send the General,
DDC-specific and DUC-specific packets before the High Priority packet that sets run ([P2]
"Detailed operation"; General, DDC specific and DUC specific packet introductions), and a host
may send the General packet at set-up only (`annex-aethersdr.md` §2.2: AetherSDR's keepalive
is the High Priority packet). While no session runs:

1. The G7250 MUST keep the latest General, DDC-specific and DUC-specific packet received from
   each source IP address and UDP port since the last session end or power-on, for up to 4
   sources, evicting the least recently heard. Kept packets start no session, feed no watchdog
   and take nothing over.
2. When a host starts a session with the run bit (A2.3), its kept packets count for that
   session as if received in it, in the order received and before the run packet itself:
   every field the G7250 acts on in the kept General packet (Table B7-1), among them bytes
   23–28 (B6.10), 31–32 (B8.1), 37 (B3.8, B6.6), 38 bit [0] (B5.6), 39 (B6.3) and 58 bit [0]
   (B10.10), and every field of the kept DDC-specific and DUC-specific packets (Tables B6-2,
   B10-1) (0022 §73 item 14).
3. Kept packets of every other source MUST be discarded when the session starts; set-up from
   another source never counts for the owner. The kept set holds only packets received since
   the last session end. A host that restarts with the run bit only (B5.4) keeps the RX-side
   settings of its ended session, but not its TX settings (Table A3-4) and not its General
   byte 38: before a General packet of the new session or of its set-up phase it cannot arm
   (`WATCHDOG_DISABLED`, B10.2 item 3) (0022 §76, §78).
4. While a session runs there is no set-up phase: a C&C packet from another host is a
   takeover (A2.4), and the takeover packet itself counts for the new session (A2.4 step 5).
5. Nothing is kept while level 2 is active (A2.5) or from a source outside the radio's subnet
   (B11.15).

B5.2 The G7250 MUST NOT send DDC I&Q data before it has received a High Priority packet with
the run bit set ([P2] "Detailed operation": run "in order to start data from the Hardware").
If the run bit arrives with no General or DDC-specific packet from that host, neither kept
from its set-up phase (B5.1a), nor carried over from its own ended session (restart, B5.4),
nor received in the session, the G7250 MUST use the [P2]
defaults and the G7250 safe RX state (0015, Consequences; A3.6; RX default state pending hardware,
P-RF-5, P-ANT-1; HIS-06, HIS-07). Without such a DDC-specific packet DDC0 counts as not enabled (DDC-specific byte 7 bit
[0] read as 0): no DDC0 I&Q is sent, `STREAM_STATUS` gives DDC0 reason 1 "not enabled by the
host", and the High Priority status packets run; DDC0 starts with the first DDC-specific packet
that enables it (0022 §73 item 13). A discovery is not a precondition: a run bit from a host that sent no discovery
starts the session normally, and the streams go to that host's source address and port
(B2.4; [P2] "Detailed operation", restart path; 0020 §14).

B5.3 **Run.** High Priority byte 4 bit [0] set enables the G7250, clear disables it ([P2] High
Priority packet, "Byte 4"). While run is set, DDC0 is enabled (B6.5) and no refusal applies
(B6.7, B5.8), the G7250 MUST send DDC0 I&Q (B6).

B5.4 **Stop.** A High Priority packet from the owner with byte 4 bit [0] clear stops the
session: the G7250 MUST stop all outgoing streams and reset its outgoing sequence counters
(B2.7) ([P2] High Priority packet byte 4; DDC packet, "Sequence Number"), and MUST unkey and
drop the arm (`RUN_CLEARED`, Table A3-1 event 2; 0007). The G7250 MUST keep answering
discovery on port 1024 (B3.1).

**Restart** (0022 §78, reversing part of §76; Annex X1, D32; Annex X2, C21). [P2] "Detailed
operation" gives two restart paths: a High Priority packet with the run bit set, "if the same
network session is still active", or a Discovery exchange followed by re-initiation of all
C&C settings and then a High Priority packet with the run bit set. The G7250 supports both:

1. **Same host.** After a session end by a stop (B5.4), a watchdog standby (B5.6) or a link
   loss or slow link (B5.8), the RX-side settings of the ended session stay in force for its
   host (the same source IP address and UDP port): every General packet field of Table B7-1
   except byte 38 bit [0] and byte 58 bit [0], the DDC-specific packet (Table B6-2), and the
   receive state set by the High Priority packet (DDC0 frequency, front end), as at a takeover
   (A2.4 step 4). A High Priority packet with run = 1 from that host restarts the session
   with them (A2.3; no 0→1 edge of the host's run bit is needed), and DDC0 streams as before
   if it was enabled. Packets of that host kept in the set-up phase since the session end
   (B5.1a) apply on top, in the order received. The carried-over General fields count as a
   General packet of the set-up phase for the unasked record (B4.4 item 2; 0022 §86 item 6).
2. **Not carried over.** The TX settings of Table A3-4 are reset at every session end (A3.6a);
   `TX_MODE_CLASS`, `KEYED_TIMEOUT` and the session limits return as in A3.6; General byte 38
   bit [0] counts as "no General packet yet" (B5.6), so an arm is rejected with
   `WATCHDOG_DISABLED` until a General packet of the new session or of its set-up phase with
   bit [0] set (B10.2 item 3). The new session starts unarmed with a new token (A3.2).
3. **Where the network session is not the same.** After a reset, a firmware update, a level
   change (A2.5a) or a change of the radio's address or netmask (B3.9; A1.6 C-1; A1.11a), and
   for any host other than the last owner, nothing is carried over: a run bit alone gives the
   [P2] defaults and the safe RX state of B5.2 (DDC0 not enabled, `STREAM_STATUS` reason 1),
   and the host re-initiates all C&C settings, before the run bit (set-up phase, B5.1a) or
   in the session ([P2] "Detailed operation", second path).

Sequence counters at a restart follow B2.7: they continue after a watchdog standby or a link
loss and restart at 0 after a stop (0022 §83 item 4).

B5.5 **Owner.** The owner is the host of the current session, identified by source IP address
and UDP port (A2.2; 0007, Resolved 4). The host MUST send all C&C packets of a session from
one source port (A2.2).

B5.6 **Watchdog** (0007, sub-decision 2):

- **C&C packets.** C&C packets are the four packets headed "Control Elements - PC to
  Hardware" in [P2]: General, DDC-specific, DUC-specific and High Priority (0007 sub-decision
  2.1; Annex X2, C6; 0022 §21).
  Memory-mapped register packets are not C&C: they MUST NOT feed the watchdog (0007 sub-decision
  2.1 and Resolved 8; 0003, Resolved) and MUST NOT cause a takeover (B5.7). Packets from hosts
  other than the owner MUST NOT feed the watchdog (0007 sub-decision 2.1).
- **Timeout.** In RUN, if no C&C packet from the owner arrives for 1 s, the G7250 MUST go to
  standby: unkey, drop the arm (`WATCHDOG`), release SEND and the TUNER trigger, and stop the
  RX streams by leaving RUN ([P2] "Detailed operation": "switch out of the RUN state into
  standby"; High Priority byte 4 bit [0]; 0007 sub-decision 2.1; 0022 §21). The RX-side
  settings stay in force for a restart by the same host (B5.4; 0022 §78). The host
  SHOULD send a C&C packet every 100 ms ([P2] "Detailed operation"). Tolerance: the G7250
  MUST NOT go to standby while the last C&C packet from the owner is less than 1000 ms old,
  and MUST be in standby no later than 1100 ms after it (0020 §16).
- **Hardware-timer bit** (General packet byte 38 bit [0]) (0007 Decision 2.3, option 2F):
  - the G7250 MUST store the value last written; the effective timer is
    `written OR armed`;
  - while not armed, the bit MUST be honoured as [P2] specifies ([P2] General packet byte 38);
  - while armed, a clear MUST NOT disable the timer; the clear takes effect again after
    disarm (0015; Annex X1, D4);
  - an arm request while the written bit is clear MUST be rejected (`WATCHDOG_DISABLED`,
    B10.2);
  - before any General packet the timer counts as enabled, but an arm is rejected with
    `WATCHDOG_DISABLED` until the owner has sent, in this session or in its set-up phase
    (B5.1a), a General packet that the G7250 acted on (not discarded as short, B11.3) with
    bit [0] set; a General packet with refused fields (for example byte 39) still counts for
    byte 38 (0022 §13, §34);
  - `WATCHDOG_STATE` reports the value written and the effective state (B8; 0020 §8).
- **Keyed timeout** while transmitting: B10.5.

Note: a session with the timer disabled (allowed only while not armed) holds the radio "in
use" after its host has gone and keeps sending RX IQ to it; the next host takes over (0007,
option 2F "Against").

B5.7 **Takeover.** Takeover is always allowed and follows the ordered procedure of A2.4
(0007, Decision 3; 0022 §3): the old session ends with the A3.2 actions and a last
`TX_ARM_STATE` to the old owner, sent three times about 10 ms apart (0022 §29); the new host becomes owner at once, RUN stays set, the
streams go to the new owner with sequence counters restarted at 0, and the new owner starts
unarmed with `TX_MODE_CLASS` = 0, session limits at the persistent values and the TX settings
of A3.6a reset (0022 §64). A host that
reopens its socket on a new UDP port takes over from itself (0007, Resolved 4). Only the arm
part deviates from [P2] ([P2] Discovery reply byte 4 NOTE; Annex X1, D5, D6).

B5.8 **Link below 100 Mbit/s.** On a link slower than 100 Mbit/s the G7250 MUST refuse the
session: it answers discovery but starts no stream (no DDC0, DUC0, microphone or wideband data),
and reports the reason ("link speed below 100 Mbit/s") in a status register (`LINK_STATUS`,
B8) (0004 point 9; Resolved). The record field `link` gives the minimum speed (B4.8; 0003 §3).
On such a link a run bit makes no owner and starts no stream, and no High Priority status
packet is sent. The G7250 MUST answer each High Priority packet with the run bit set, and each
`CAP_REQUEST`, with a register packet holding `LINK_STATUS` (refusal reason 1) and
`STREAM_STATUS` (reason 8), to the sender, at most once per 100 ms per source IP address and
port and at most 10 per second in total (0020 §20; 0022 §73 item 7). A `CAP_REQUEST` is in
addition answered with the record as on a fast link (B4.4, limits of B11.14), so that a host
can read `link` and tell the user why no session starts (0022 §73 item 7). The speed is
read from the CH394Q PHY configuration register PHY_CFG, bits SPDS (speed) and LINKS (link
up) ([CH394] section 7.1.15); this firmware owns the CH394, so the register is
usable ([CH394] section 7.1.15; 0035; HIS-14). Annex X1, D15. If the link goes down or renegotiates below 100 Mbit/s
during a session, the session MUST end: unkey, disarm (`LINK_DOWN`, Table A3-1 event 8),
streams stop, and `LINK_STATUS` gives the speed and refusal reason 1. A later return to
100 Mbit/s allows a new session; nothing restarts by itself: the next High Priority packet
with run = 1 from the last owner restarts its session with its RX-side settings (B5.4; 0022
§16, §78).

B5.9 **Transmit bit.** High Priority byte 4 bit [1] has no effect unless TX is armed (0015;
B10). Bits [2]–[7] never key the G7250 (0007 sub-decision 1 rule 4; Annex X2, C5).

### B6 Streams from the radio: RX IQ, wideband, microphone

B6.1 **Port and endpoint.** DDC0 I&Q MUST be sent from local port 1035 ([P2] General packet
bytes 17–18; DDC packet, "Source Port") to the owner's source IP address and UDP port (B2.4;
0022 §12). One DDC is
implemented, and DDC0 takes its data from ADC0 only (0004 point 1; HIS-02, HIS-01, HIS-03).
DDC0 needs a complete FPGA configuration frame (command 0x01), whose status byte and bytes
9–18 have no known meaning or safe values yet, and the I/Q order and the tuning word of DDC0
are not known ([HIS] HIS 80, `5292ba8`). Until these are set (pending hardware, P-RX-2) DDC0
is not offered: record `streams` bit 0 = 0, a DDC0 enable gets `STREAM_STATUS` reason 2 (not
offered by this firmware), which takes precedence over the rate and sample-size reasons 3 and
4 but not over the General-packet refusals (Table B8-3, stream reason order), and no DDC0
packet is sent; discovery, the register map, the
management channel and sessions work as usual. The firmware MUST NOT write a configuration
frame unless the RF section is in the safe state of A3.6, because its status byte may hold a
transmit enable ([HIS] HIS 80, HIS 85) (0042).

B6.2 **Packet format.** Each DDC0 packet MUST have the layout of Table B6-1 ([P2] DDC packet,
"DDC I&Q Data (DDC0 default port 1035)"; 0004 point 1), 1444 bytes in total.

Table B6-1 — DDC0 I&Q packet ([P2] DDC packet; 0004 point 1)

| Byte | Field | G7250 value | Source |
|------|-------|-------------|--------|
| 0–3 | Seq # [31:0] | per-port counter, B2.7 | [P2] DDC packet bytes 0–3 |
| 4–11 | Time stamp [63:0] | 0 (B6.6) | [P2] bytes 4–11; 0004 option 2C |
| 12–13 | Bits per sample [15:0] | 24 (0x0018) | [P2] bytes 12–13; 0004 point 1 |
| 14–15 | I&Q samples per frame [15:0] | 238 (0x00EE) | [P2] bytes 14–15; B6.4 |
| 16–18 | I sample 0 [23:0] | signed two's complement | [P2] bytes 16–18 |
| 19–21 | Q sample 0 [23:0] | signed two's complement | [P2] bytes 19–21 |
| 22–1437 | I and Q samples 1–236 | same layout, 6 bytes per sample | [P2] DDC packet |
| 1438–1440 | I sample 237 | | [P2] (follows from layout) |
| 1441–1443 | Q sample 237 [23:0] | | [P2] bytes 1441–1443 |

B6.3 **Samples and format.** Each I and Q value MUST be a 24-bit signed two's-complement
integer, MSB first, I before Q, samples in time order ([P2] DDC packet, "DDC I/Q Samples";
NOTE on I and Q order). Only 24 bits per sample and the Big-Endian 3-byte format are offered;
no float, double or little-endian format (0004 point 1). A General packet byte 39 that asks for
any other format MUST NOT be followed, and DDC0 MUST NOT run while that request stands; the
reason is reported in `STREAM_STATUS` (0004 point 7; B8). Byte 39 values that select only
Big-Endian (bit [0]) and/or 3-byte (bit [2]), and 0x00, are read as the offered format
(provisional; Annex X2, C9).

B6.4 **Samples per packet.** Each packet MUST carry 238 I&Q samples: 16 + 238 × 6 = 1444 bytes
([P2] DDC packet table, last sample "Q Sample 237" at bytes 1441–1443; 0004 point 1). Bytes
14–15 MUST state the number of samples that follow ([P2] DDC packet, "I&Q Samples per frame").
A host MUST read bytes 14–15 rather than assume 238 ([P2], same field). Informative: at
384 ksps this is about 1613 packets per second and 18.6 Mbit/s payload (0004, "Per stream").

B6.5 **DDC-specific packet.** The G7250 MUST accept the DDC-specific packet on port 1025
([P2] DDC specific packet) and treat its fields as in Table B6-2. Fields not listed are
ignored.

Table B6-2 — DDC-specific packet fields ([P2] DDC specific packet)

| Byte | Field | G7250 behaviour | Source |
|------|-------|-----------------|--------|
| 4 | Number of ADCs | accepted; the G7250 uses one ADC for DDC0 ([HIS] HIS 80; HIS-01) | [P2] byte 4 |
| 5 | Dither ADC0–7 | ignored in this version (0035; HIS-01) | [P2] byte 5 |
| 6 | Random ADC0–7 | ignored in this version (0035; HIS-01) | [P2] byte 6 |
| 7 | DDC enable DDC0–7 | bit [0] honoured: DDC0 sends I&Q only while set and run is set; bits [1]–[7] ignored | [P2] byte 7; B5.3; 0004 point 7 ("Enables for DDC1–DDC79 are ignored") |
| 8–16 | DDC enable DDC8–79 | ignored | [P2] bytes 8–16; 0004 point 7 |
| 17 | ADC for DDC0 | 0 (ADC0) honoured; other values ignored (one ADC, 0035; HIS-01, HIS-03) | [P2] byte 17; 0004 point 1 |
| 18–19 | Sampling rate DDC0, in ksps (48, 96, 192, 384) | offered rates honoured (B6.7); any other value: refused, DDC0 does not run (B6.7) | [P2] bytes 18–19; 0004 points 1, 7; unit as written by [AETHER-SRC] `P2Protocol.cpp:123–125` and [PIHPSDR] `new_protocol.c:1403–1404` (0020 §30) |
| 20–21 | CIC1, CIC2 DDC0 | ignored ("For future use" in [P2]) | [P2] bytes 20–21 |
| 22 | Sample size DDC0 | 24 honoured; 0 read as the default 24; any other value: refused, DDC0 does not run | [P2] byte 22; 0004 point 7 |
| 23–496 | ADC, rate, CIC, size DDC1–79 | ignored | [P2] bytes 23–496; 0004 point 7 |
| 1363–1442 | Sync DDC0–79 | ignored; synchronous DDCs not supported unless HIS-03 shows a second ADC | [P2] bytes 1363–1442; HIS-03 |
| 1443 | Currently not used | ignored | [P2] byte 1443 |

A change of DDC0 sampling rate to another offered rate while running SHOULD take effect
without a stop, since [P2] sends the DDC-specific packet "when a parameter changes" ([P2] DDC
specific packet, introduction). The sequence counter does not restart on a rate change, nor
when DDC0 resumes after a refusal (B2.7; 0020 §19).

B6.6 **Time stamp.** Time stamps and VITA-49 are not offered: bytes 4–11 of each DDC packet
MUST be sent as zero (0004 option 2C, point 1). A General packet with byte 37 bit [0] (time
stamp) or bit [1] (VITA-49) set MUST NOT be followed, and DDC0 MUST NOT run while it stands;
the reason is reported in `STREAM_STATUS` (0004 point 7). Record `streams` bit 5 = 0 (B4.8).

B6.7 **Sample rates.**

- The offered DDC0 rates are 48, 96, 192 and 384 ksps ([P2] DDC-specific bytes 18–19; 0004
  point 1, option 1B). 384 ksps is offered only if the SPI clock is about 50 MHz or more;
  below that the maximum is 192 ksps (0004 point 6; Resolved). SPI clock: about 17 MHz is seen in use on the board, which is not a datasheet limit ([HIS] HIS 40, `d199d33`, MEDIUM); the highest working clock is pending
  hardware (P-NET-1; HIS-14), and until it is shown to be 50 MHz or more the offered maximum is
  192 ksps. The
  set also depends on what the ADC chain can produce (HIS-01). The offered set MUST be stated
  in `ddc_rates` (B4.8).
- 768 and 1536 ksps MUST NOT be offered, not even receive-only (0004 point 7; Resolved).
- A DDC-specific packet with a DDC0 rate outside the offered set MUST be refused: DDC0 does not
  run (it stops if running) until a valid DDC-specific packet arrives, and the reason is
  reported in `STREAM_STATUS` (0004 point 7, option 6D). The rate MUST NOT be clamped to an
  offered value, and the previous rate MUST NOT be kept (0004 point 7; options 6A, 6B, 6C
  rejected). Annex X1, D13.
- Design target: worst-case SPI load (RX at the maximum rate, TX, status at 1 ms, all offered
  streams) at or below 60 % at the actual SPI clock (0004 point 6). Informative.

B6.8 **Receive frequency.** DDC0 frequency is set by High Priority bytes 9–12, as a phase
word with the fixed scale of B3.8, as declared in discovery byte 21 ([P2] High Priority packet bytes 9–12; General
packet byte 37 bit [3]; B3.8). Range: 0.5–30 MHz and 50–54 MHz [G7250-MAN], also in
`rx_range` (B4.8). Outside the range the G7250 MUST tune as requested, without refusal or
clamping; receiver performance there is not specified (B11.8; 0020 §28).

B6.9 **Pacing.** The G7250 SHOULD send a DDC0 packet as soon as 238 samples are available, to
keep latency low (provisional, no decision; by analogy with [P2] "Detailed operation", which
gives this advice for DUC data; the CH394Q has no flow control in UDP mode, [CH394] section
12.2.4). Overrun behaviour: B11.7 (0020 §28).

B6.10 **Wideband data (milestone M2).** (0004 point 4; option 5C; Resolved.)

- Not offered before M2: a General packet byte 23 enable MUST NOT be followed and no wideband
  packets are sent (0004 point 7). Record `streams` bit 4 = 0, `wb_format` = `wb_frame` = 0.
- From M2, for ADC0 only, out from local port 1027 ([P2] General bytes 21–22): fixed
  parameters 512 samples per packet, 16-bit samples, 8 packets per frame, one frame every 50 ms
  (4096 samples per frame, 20 frames per second), in [P2] Wideband Data packets of 1028 bytes
  (bytes 0–3 sequence number, 512 samples of 16 bits) ([P2] Wideband Data packet; General
  bytes 24–28; 0004 point 4). Stated in `wb_format` and `wb_frame` (B4.8). The sequence number
  is 0 in the first packet of every frame and 7 in the last, as [P2] specifies ([P2] Wideband
  Data packet, pp. 52–53; B2.7; 0022 §21).
- From M2, an enable for ADC0 (General byte 23 bit [0]) MUST be followed only while bytes
  24–28 are 0 or equal to the fixed values (512, 16, 50, 8). Any other value: wideband does not
  run until the request is valid, reason in `STREAM_STATUS`. Enables for other ADCs are not
  followed (0004 point 7). [P2] gives the defaults 512 samples, 16 bits, 20 ms and 32 packets,
  and lets zero select the default only for byte 26 ("If set to zero the default of 16 bits
  will be used"); byte 27 has the range "0 to 255mS" ([P2] General packet bytes 24–28). The
  G7250 reads zero in bytes 24–25, 27 and 28 as its fixed values; zero in byte 26 is the [P2]
  default 16 (Annex X1, D14; 0022 §21).
- Informative: 160 packets/s, 1.32 Mbit/s payload, about 2.3 % SPI at 62 MHz, the datasheet maximum [CH394]; at the clock that works on this board the share is higher (P-NET-1) (0004 point 4).
  The frequency resolution is the ADC rate / 4096 (HIS-01). Depends on HIS-01 and HIS-18; if
  raw ADC blocks cannot be delivered, this is revisited (0004 point 4).

B6.11 **Microphone data.** Not offered in this version: the main unit carries no audio and its
codec stays in reset (0031; [HIS] HIS 50, HIS 85). The G7250 MUST NOT send a microphone stream,
nor a silent one for timing (0004 point 3; option 4C rejected). Record `streams` bit 3 = 0
(B4.8). A later version that offers it would follow the [P2] Microphone Data packet: 48 ksps,
16-bit, 64 samples per packet, 132 bytes, out from port 1026 (0004 point 3).

B6.12 **DDC audio.** Not offered (0017; B2.3).

### B7 P2 field table

Per standard P2 field of the host-to-radio packets: honoured, ignored, refused, or open on a
hardware question with its HIS id (0005, Consequences). "Honoured" means as [P2] specifies (B1.4). "Ignored" means accepted with no
effect. "Refused" means the request is not followed and the stream concerned does not run,
with the reason in `STREAM_STATUS` (0004 point 7). The DDC-specific packet is in Table B6-2,
the DUC-specific packet in Table B10-1, the High Priority status packet (radio to host) in
Table B9-1.

Table B7-1 — General packet ([P2] General packet to SDR)

| Byte | Field | G7250 behaviour | Source |
|------|-------|-----------------|--------|
| 4 | Command 0x00 | honoured | [P2] byte 4 |
| 5–12, 15–22 | Port numbers | zero or the default value honoured; any other value not honoured: default kept, logged, `LINK_STATUS` bit 17 (never onto the management port) | B2.3; 0008 Transport 2; 0020 §13; Annex X1, D22 |
| 13–14 | DDC audio port | ignored | 0017 |
| 23 | Wideband enable | before M2: not followed; from M2: bit [0] per B6.10; other bits not followed | 0004 point 7 |
| 24–28 | Wideband parameters | from M2: 0 or the fixed values; otherwise refused | 0004 point 7; B6.10 |
| 29–30 | Memory-mapped port of the G7250 | zero or 7250 honoured; any other value not honoured, as bytes 5–22 | 0005; B2.3; 0020 §1, §13 |
| 31–32 | Memory-mapped destination port of the host | non-zero honoured; zero = the owner's source port | B8.1; 0020 §13 |
| 33–36 | Envelope PWM | ignored ("Reserved for future use" in [P2]) | [P2] bytes 33–36; 0005 rule 3 |
| 37 [0], [1] | Time stamp, VITA-49 | set: refused (DDC0 does not run while it stands) | 0004 point 7 |
| 37 [2] | VNA mode | set: refused (VNA not offered; DDC0 does not run while it stands; `STREAM_STATUS` reason 11) | [P2] byte 37; 0004 point 7; 0020 §22 |
| 37 [3] | Frequency or phase word | must be 1, matching discovery byte 21 (phase word, scale 122 880 000 Hz, B3.8); 0 is a mismatch: DDC0 does not run and no TX request keys while it stands, `STREAM_STATUS` reason 13 | B3.8; 0022 §73 item 12, §81 |
| 38 [0] | Hardware timer enable | honoured while not armed; forced on while armed; must be set to arm | 0007 Decision 2.3; B5.6 |
| 39 | Endian and I&Q format | Big-Endian 3-byte honoured; anything else refused | 0004 points 1, 7; B6.3 |
| 40–55 | Reserved | ignored | [P2]; 0005 rule 3 |
| 56, 57 | Atlas bus configuration, 10 MHz reference source | ignored (byte 56 "Atlas bus configuration"; byte 57 "For Atlas based systems this selects the source of the 10MHz reference clock" in [P2]) | [P2] bytes 56–57; 0022 §21 |
| 58 [0] | PA enable | honoured as an extra TX condition: while 0, a TX request does not key; cleared while keyed: unkey, arm kept (B10.10); `TX_STATE` bit 4; no exciter-only output (Annex X1, D30); cleared at every session end and takeover (A3.6a) | [P2] byte 58; 0005 rule 1; 0020 §22; 0022 §14, §64 |
| 58 [1] | Apollo ATU enable | ignored; the internal ATU is controlled through the register map (`ATU_TUNE`) | 0005 option D; 0007 Decision 6 |
| 58 [2], [3] | Mercury common frequency, clock source | ignored (Atlas only in [P2]) | [P2] byte 58 |
| 59 | Alex enable | ignored: the G7250 front end is driven by its own shift registers, not by Alex boards, so the meaning is not identical ([HIS] HIS 85; HIS-06, HIS-07) | 0005 option D, rule 1 |

Table B7-2 — High Priority packet ([P2] High Priority data packet)

| Byte | Field | G7250 behaviour | Source |
|------|-------|-----------------|--------|
| 4 [0] | Run | honoured | [P2] byte 4; B5.3, B5.4 |
| 4 [1] | Transmit (PTT0) | TX request; keys only when armed, on a 0→1 edge after arm confirmation, within all limits | 0015; 0007 rules 2, 4; B10 |
| 4 [2]–[7] | PTT1–PTT3 / reserved | ignored; never key | 0007 rule 4; Annex X2, C5 |
| 5 [0] | CWX | selects CW keying from the host through bits [1] and [2]; not a TX request by itself | [P2] High Priority packet byte 5, p. 34; Annex X2, C19; 0022 §36 |
| 5 [1], [2] | dot, dash | with bit [0] set: paddle inputs of the radio's keyer, elements at the keyer speed and weight (DUC-specific bytes 9–10); a TX request under the arm (B10.1); key-down cap B10.5a; with bit [0] clear: ignored | [P2] byte 5, p. 34; 0007 Decision 6; 0022 §36 |
| 6–8 | CWX1–CWX3 | ignored (reserved) | [P2]; 0005 rule 3 |
| 9–12 | DDC0 frequency / phase word | honoured | [P2] bytes 9–12; B6.8 |
| 13–328 | DDC1–DDC79 frequency | ignored while one DDC (HIS-02) | [P2]; 0004 point 1 |
| 329–332 | DUC0 frequency / phase word | honoured; outside the TX ranges a TX request does not key; while keyed, a change that leaves the range or needs an RF-path change unkeys first (A3.9) | [P2] bytes 329–332; A3.4; 0022 §1 |
| 333–344 | DUC1–DUC3 frequency | ignored (reserved) | [P2]; 0005 rule 3 |
| 345 | DUC0 drive level 0–255 | honoured; mapping to power pending hardware (P-PWR-1; HIS-12); `REDUCE` acts radio-side | [P2] byte 345; 0007 rule 4 |
| 346–348 | DUC1–DUC3 drive | ignored (reserved) | [P2]; 0005 rule 3 |
| 1396–1397 | Client control word | bit [0] TUNE accepted as information; does not start the internal ATU; other bits ignored | 0007 Decision 6; 0005 rule 1 |
| 1398–1399 | CAT over TCP port | ignored; CAT stays on the host side | 0011 point 5 |
| 1400 | Transverter and audio enable | ignored (no transverter output, no audio on the main unit) | [P2] byte 1400; 0017; 0020 §22 |
| 1401 | Open-collector outputs | ignored; no such outputs [G7250-MAN] | 0007 Decision 6 |
| 1402, 1403 | Metis user outputs, Mercury attenuator | ignored (Atlas only in [P2]) | [P2] bytes 1402–1403 |
| 1404–1427 | Alex 2–7 | ignored (reserved) | [P2]; 0005 rule 3 |
| 1428–1435 | Alex 0, Alex 1 words | ignored: the G7250 front end is driven by its own shift registers, not by Alex boards, so the meaning is not identical ([HIS] HIS 85; HIS-06, HIS-07) | 0005 option D, rule 1 |
| 1436–1441 | Step attenuators 2–7 | ignored (reserved) | [P2]; 0005 rule 3 |
| 1442 | Step attenuator ADC1 | ignored unless a second ADC exists (HIS-03) | [P2] byte 1442 |
| 1443 | Step attenuator ADC0 | ignored: the G7250 attenuator is a bit of SPI1 chain B with steps pending hardware (P-RF-4), not identical in meaning ([HIS] HIS 85; HIS-07; 0005 rule 1) | [P2] byte 1443 |

Fields of Tables B7-2 and B10-1 that switch an RF-path relay (Alex words, step attenuators)
are deferred while keyed and applied after the unkey (A3.9; 0022 §2).

Other host-to-radio packets: DUC I&Q (B10.7); memory-mapped registers from PC (B8); Erase,
Program, Set IP (B3.9); DDC audio (not received, B2.3).

The P2 field table is complete. Rows that still depend on the hardware carry their HIS id
and are closed when the HIS answers (0005, Consequences; 0020, Context).

### B8 G7250 register map

B8.1 **Port and packets.** The G7250 register map is carried by [P2] "Memory mapped registers
from PC" and "Memory mapped registers from hardware" packets: pairs of a 16-bit address and a
32-bit data word, bytes 4–1443 ([P2]; 0005). The G7250 uses local UDP port **7250** for both
directions (B2.3; 0005, Consequences; 0020 §1). It sends register packets to the owner's
source port, or to General packet bytes 31–32 when these are non-zero, and answers a
non-owner's request to the request's source address and port (B2.4; 0020 §13, §14).

- A packet carries *n* pairs, 1 ≤ *n* ≤ 240, and is 4 + 6*n* bytes long. The G7250 MUST send
  only the pairs it has (0020 §4). [P2] shows 240 pairs and has no count field; the shorter
  packet is the G7250 reading of [P2] (Annex X2, C15; 0022 §21).
- The G7250 MUST accept any length of at least 10 bytes and MUST ignore a trailing incomplete
  pair. Pairs with address 0xFFFF, and the pair (0x0000, 0x00000000), are padding: they MUST
  be ignored and are not answered (0020 §4).
- Incoming sequence numbers are not checked (B2.7).

B8.2 **Sending.** The G7250 sends a memory-mapped packet whenever data changes and optionally
periodically ([P2] Memory mapped registers from hardware); B9.4 gives the telemetry rates.
Register packets and High Priority status packets are not synchronised; a host MUST NOT
assume they describe the same instant (0006, Consequences).

B8.3 **Groups and addresses.** Addresses are grouped in blocks of 256; group *n* is present
when bit *n* of the record field `regmap_groups` is set (B4.8; 0003 §4 rule 6; 0020 §3).
Table B8-1 lists the groups, Table B8-2 the registers. Registers for antenna selection,
filter and front-end control and external interfaces are reserved in their blocks until the
HIS answers (HIS-06, HIS-07, HIS-09; 0005, Consequences). Addresses not listed are reserved;
a write to one is rejected (`UNKNOWN_ADDRESS`, B8.4).

Table B8-1 — Register groups (0020 §3)

| Bit in `regmap_groups` | Block | Group | Present |
|---|---|---|---|
| 0 | 0x0000–0x00FF | capability record (`CAP_BASE` = 0x0000; 0x0000–0x00EE usable, 239 registers; 0x00EF–0x00FF reserved) | always (0003 §2) |
| 1 | 0x0100–0x01FF | session and stream status, requests | always |
| 2 | 0x0200–0x02FF | TX arm, TX state, limits, faults | from M3; arm accepted only when record `tx` bit 1 = 1 (B10) |
| 3 | 0x0300–0x03FF | telemetry | always: the sensors of [HIS] HIS 85 exist; values valid once calibrated (P-SEN-1; HIS-11) |
| 4 | 0x0400–0x04FF | internal ATU | always; `ATU_STATE` 5 and `ATU_TUNE` rejected with `NOT_CALIBRATED` until P-ATU-1 is set (0032; HIS-08) |
| 5 | 0x0500–0x05FF | antenna | always; `ANT_STATE` 0 until P-ANT-1 is set (HIS-06) |
| 6 | 0x0600–0x06FF | front end and filters (reserved) | 0 in this version (P-RF-4; HIS-07) |
| 7 | 0x0700–0x07FF | external interfaces, SEND and TUNER (reserved) | 0 in this version (P-SYS-8; HIS-09) |
| 8–31 | 0x0800–0x1FFF | reserved for later groups | 0 |
| — | 0x2000–0xFFFE | reserved | — |
| — | 0xFFFF | padding (no operation) | — |

B8.4 **Write confirmation, rejection and ordering** (0020 §11; 0005, Consequences).

- The G7250 MUST process the pairs of a host packet in order. For each written address it
  MUST answer with the pair (address, value now in force), in one or more packets sent within
  5 ms of receipt. For `TX_ARM` the answer is `TX_ARM_STATE`; keying needs a TX-request edge
  after it (B10.2; 0007 rule 2).
- A rejected write MUST leave the value unchanged. The answer MUST carry the value in force
  (for `UNKNOWN_ADDRESS`, which has none: the address with the value 0x00000000; 0039)
  and a `WRITE_STATUS` pair: [31:16] the address, [15:8] a rejection counter (wraps at 255),
  [7:0] the reason: 1 `UNKNOWN_ADDRESS` (including an address of an absent group),
  2 `READ_ONLY`, 3 `NOT_OWNER`, 4 `BAD_VALUE`, 5 `LOOSER_LIMIT`, 6 `BUSY` (update in
  progress), 7 `NOT_CLEARABLE` (fault not clearable, A3.5), 8 `KEYED` (RF-path change while
  keyed, A3.9; `ATU_TUNE` while keyed, A3.9b), 9 `NOT_ARMED` (`ATU_TUNE` without arm, B10.9),
  10 `NOT_KEYABLE` (`ATU_TUNE` while another key condition of A3.1a fails, A3.9b item 1)
  (0020 §11; 0022 §5, §2, §14, §22, §51; also in Table B8-3).
- `BUSY` takes precedence over every other reason; then `NOT_OWNER` (0022 §10).
- A rejected arm precondition is reported only in `TX_ARM_STATE`, not in `WRITE_STATUS`. A
  `TX_ARM` write rejected as such (`NOT_OWNER`, `BUSY`) is reported in `WRITE_STATUS` to the
  writer only and changes nothing in `TX_ARM_STATE`, so a non-owner cannot overwrite the
  owner's last reject reason (0022 §13).
- The value MUST take effect before the answer is sent. [P2] orders nothing between ports: a
  host that needs a write in force before a High Priority action MUST wait for the answer
  (0020 §11; 0007 rule 2). A host MAY repeat a write after 100 ms without an answer; repeated
  writes are harmless: a correct `TX_ARM` while armed with the same token is a no-op, a
  `FAULT_CLEAR` without a fault is rejected (with `FAULT_FLAGS` = 0 a write of 0 gets `NOT_CLEARABLE`, any other value `BAD_VALUE`; 0039), and an `ATU_TUNE` start while tuning or within
  2 s after the end of the previous cycle is a no-op (A3.9b item 1), so a retry never starts a
  second tune carrier (0020 §7, §11; 0022 §39).
- Writes to the capability record and to other read-only registers are answered with the
  current values and `READ_ONLY` (B4.6).

B8.4a **Reading registers** (0022 §51). [P2] has no read request for memory-mapped registers.
A host learns register values from the pushes of B8.2 and B9.4 and from the answers of B8.4: a
write to a read-only register is answered with its current value and `READ_ONLY`, so writing
any value reads it; a writable register is read from the answer to its last write or from its
pushes. A non-owner's write is answered with the value in force and `NOT_OWNER` (B8.5), so a
non-owner can read every register, `TX_ARM_TOKEN` included. This is accepted: the token
protects against accidents, not against a host on the LAN (A3.1; 0007 option 1C, "Against").

B8.5 **Writes from a host that is not the owner.** `CAP_REQUEST` MUST be accepted from any
host (B4.4). Every other write from a host that is not the owner, or while there is no
session, MUST be rejected with `NOT_OWNER` and answered to the writer's source address and
port; it MUST have no other effect: no takeover, no watchdog feed (0020 §12; 0007 rule 1 and
Decision 5). `NOT_OWNER` answers MUST NOT exceed one per 100 ms per source IP address and
port; further rejected writes are dropped without answer and counted (B11.14; 0022 §16). The
takeover notices to an old owner (A2.4, "Notice destination") are not `NOT_OWNER` answers and
have their own limit (0022 §58). A memory-mapped packet from a non-owner whose source port is
7250 MUST get no answer at all (no `NOT_OWNER`, no `CAP_REQUEST` answer) and is dropped and
counted: register packets of another G7250 come from that port (B2.4a), and answering them
would let two radios answer each other without end. A host SHOULD NOT use local port 7250 as
its source port (0022 §73 item 20).

B8.6 **Units.** Analogue registers are signed 32-bit integers in milli-units of an SI unit
(mV, mA, mW, m°C, ms; SWR as 1000 × SWR). Status registers are enumerations or bit fields;
bit 0 is the LSB, unused bits are 0, and codes not listed are reserved (0006 option 3A;
0003 §2; 0020 §8, §10).

Table B8-2 — G7250 registers ("RO" = read-only, radio → host; "W" = written by the host and
echoed with the value in force, B8.4)

| Name | Address | Access | Content, units | Sent | Source |
|------|---------|--------|----------------|------|--------|
| capability record (Table B4-1) | 0x0000–0x00CE | RO | Table B4-1 | on `CAP_REQUEST`; once per session | 0003 §2, §3; 0020 §26 |
| `CAP_REQUEST` | 0x0100 | W, any host | 0x00000001 requests the record; other values `BAD_VALUE` | answer | 0003 §2; 0020 §5 |
| `STREAM_STATUS` | 0x0101 | RO | one byte per stream: [7:0] DDC0, [15:8] DUC0, [23:16] microphone, [31:24] wideband; bit 7 running, [6:0] reason (Table B8-3) | on change | 0004 point 7; 0020 §8 |
| `LINK_STATUS` | 0x0102 | RO | [15:0] speed in Mbit/s (0 = link down); bit 16 full duplex; bit 17 non-default P2 port not honoured (latched until the next session); [31:24] session refusal reason: 0 none, 1 link below 100 Mbit/s | on change; B5.8 | 0003 §2; 0004 point 9; 0020 §8, §13, §20 |
| `WRITE_STATUS` | 0x0103 | RO | last rejected write, B8.4 | on every rejection, to the writer | 0020 §11 |
| `TX_ARM_TOKEN` | 0x0200 | RO | 32-bit token; never 0, 0xFFFFFFFF or `ARM_MAGIC`; differs from the previous token, also from the last one before a reset (boot counter, B10.2 item 5) | at session start and on every change | 0007 Decision 1; 0020 §7; 0022 §73 item 4 |
| `TX_ARM` | 0x0201 | W, owner | `token XOR ARM_MAGIC` arms; 0 disarms; any other value rejected (`BAD_TOKEN` in `TX_ARM_STATE`) | answer is `TX_ARM_STATE` | 0007 Decision 1; 0020 §7 |
| `TX_ARM_STATE` | 0x0202 | RO | [3:0] state 0 disarmed, 1 armed, 2 inhibited (armed, but a limit inhibit is active so TX requests do not key; entered from 1 when an inhibit starts, back to 1 when it lifts, no new generation); [15:8] last reject reason of the owner; [23:16] last drop reason, 5 `RESET` after any reset until the next drop (Table A3-1 event 5); [31:24] arm generation (+1 per arm, wraps) (Table B8-3) | on every change | 0007 Decision 1; 0006; 0020 §8; 0022 §13, §61 |
| `TX_STATE` | 0x0203 | RO | bit 0 keyed (PA actually keyed); bit 1 `WARN` active; bit 2 `REDUCE` active; bit 3 last TX request outside the TX ranges; bit 4 PA disabled by the host (General byte 58 bit [0]); bit 5 inhibit active; [11:8] TX source 0 none, 1 PTT, 2 CWX, 3 KEY, 4 ATU; [23:16] last unkey reason 0 none, 1 TX request released, 2 keyed timeout, 3 TX IQ underrun, 4 limit, 5 arm dropped, 6 retune needs RF-path change, ATU memory recall included (A3.9), 7 PA disabled by the host while keyed (B10.10), 8 keying client lost (TCI only, part C C8.2; never set by the radio over P2), 9 CW key-down longer than 1 s (B10.5a; TCI `KEYER`), 10 TX mode class changed while keyed (A3.9c), 11 ATU tune cycle ended by itself, tuned or failed (A3.9b item 6; 0047); [31:24] limit id when reason 4 | on every change (event) | 0007 Decisions 1, 2, 4; 0006; 0020 §8, §22; 0022 §1, §14, §25, §26, §28, §32 |
| `TX_MODE_CLASS` | 0x0204 | W, owner | 0 none declared (not writable); 1 SSB (including data modes sent as SSB), 2 CW, 3 AM, 4 FM; other values `BAD_VALUE`; power class 100 W PEP for 1, 2, 4 and 25 W carrier for 3 [G7250-MAN] (A3.4c); back to 0 at session end and at a takeover; a change to another value while keyed unkeys first (`TX_STATE` unkey reason 10, arm kept), then applies; the value in force is a no-op (A3.9c) | answer | 0007 Decision 4 rule 3; 0020 §9; 0022 §3, §17, §26 |
| `KEYED_TIMEOUT` | 0x0205 | W, owner | ms; default 500; 100–500 accepted, other values `BAD_VALUE`; back to 500 at session end | answer | 0007 Decision 2; Resolved 2; 0020 §17 |
| `WATCHDOG_STATE` | 0x0206 | RO | bit 0 General byte 38 bit [0] as last written, 0 while bit 2 is set; bit 1 effective state (1 while bit 2 is set); bit 2 no General packet yet (counts as enabled) (0039) | on change | 0007 Decision 2.3; 0020 §8 |
| `SAFETY_STATE` | 0x0210 | RO | 0 normal, 1 fault | event | 0007 Decision 5; 0020 §8 |
| `FAULT_CODE` | 0x0211 | RO | first cause, Table B8-3; 0 = none | event | 0007 Decision 5; 0006; 0020 §8 |
| `FAULT_FLAGS` | 0x0212 | RO | bit *c* set for every fault code *c* (1–31) since the last clear | event | 0007 Decision 5; 0020 §8 |
| `FAULT_CLASS` | 0x0213 | RO | 0 none, 1 host-clearable, 2 latched until reset; the highest class among the codes set in `FAULT_FLAGS` (classes per code in Table B8-3; a class-1 code escalated by A3.5 counts as class 2) | event | 0007 Decision 5; 0022 §5, §27 |
| `TRIP_COUNT` | 0x0214 | RO | limit trips with action `UNKEY` or stronger since power-on; saturates at 0xFFFFFFFF; not cleared over P2; survives `RESTART` and resets without power loss (A3.5) | event | 0007 Decision 5; 0020 §8; 0022 §27 |
| `FAULT_CLEAR` | 0x0215 | W, owner | write the current `FAULT_CODE`; clears all flags if every flagged code is class 1 (not escalated), its condition is gone and 10 s have passed since the latest fault entry (A3.5), else `NOT_CLEARABLE`; other values `BAD_VALUE` | answer | 0007 Decision 5; 0022 §5, §27 |
| `LIMIT_STATUS` | 0x0220 | RO | bit *n*: limit *n* limiting (`WARN` or `REDUCE`); bit 16 + *n*: limit *n* tripped (`UNKEY` or stronger) since the last TX request; limit ids in Table B8-3 | event | 0006; 0007; 0020 §8 |
| `LIMIT_INHIBIT` | 0x0221 | RO | bit *n* (1–15): limit id *n* holds an inhibit (Table A3-2: `LIMIT_PA_TEMP` above T3; `LIMIT_SUPPLY_VOLTAGE` after an over-voltage or under-voltage `UNKEY`, until the supply has been inside the range for 2 s); bit 0 and bits 16–31 are 0; `TX_STATE` bit 5 is the OR of all bits | event | 0007 Decision 4; 0022 §32, §42 |
| `LIMIT_TX_TIME` | 0x0230 | W, owner | maximum continuous TX time, ms; built-in 300 000; 0 = back to the persistent value, not "no TX time" (B8.6) | answer | 0007 Resolved 3; 0020 §10; 0022 §73 item 5 |
| `LIMIT_FWD_POWER` | 0x0231 | W, owner | upper bound *L* on forward power, PEP, mW, all bands and classes; enforced through Table A3-3 like every other tier: on the measured PEP, or without a PEP detector on the PEP estimate from the digital crest factor of the DUC samples for every class, with the class-based mean bounds (SSB *L* × `P_ssb_mean` / 100 W, AM min(*L* / 4, 25 W), CW and FM *L*) in addition (A3.4c); also bounds the tune power (A3.4 rule 5); 0 = back to the persistent value, not "no power" (B8.6) | answer | 0007 Decision 4; 0020 §10; 0022 §17, §52, §53, §62; §73 item 5 |
| `LIMIT_SWR` | 0x0232 | W, owner | 1000 × SWR; checked against the persistent value of S2 (accepted if equal to or stricter than it, else `LOOSER_LIMIT`); replaces S2 when stricter, S1 in force becomes the lower of S1 and this value; 0 = back to the persistent value (B8.6) | answer | 0007 Decision 4; 0020 §10; 0022 §73 item 6 |
| `LIMIT_PA_TEMP` | 0x0233 | W, owner | m°C; replaces T3 when stricter; 0 = back to the persistent value (B8.6) | answer | 0007 Decision 4; 0020 §10; 0022 §73 item 5 |
| `TLM_PA_TEMP` | 0x0300–0x0303 | RO | m°C, signed; sensors 0–3 | slow | 0006; HIS-11 |
| `TLM_PA_CURRENT` | 0x0304 | RO | mA | fast | 0006; HIS-11 |
| `TLM_SUPPLY_CURRENT` | 0x0305 | RO | mA (if separate from PA current) | fast | 0006; HIS-11 |
| `TLM_SWR` | 0x0306 | RO | 1000 × SWR as used by the safety core; 0 = not valid | fast | 0006; 0007 |
| `SENSOR_VALID` | 0x0307 | RO | bit *n* as record `telemetry` bit *n*; bit 7 PA temperature sensors 1–3 | event | 0006; HIS-11; 0020 §8 |
| `ATU_TUNE` | 0x0400 | W, owner | 1 starts an ATU tune cycle (A3.9b; a TX request under the arm), 0 aborts; 1 while not armed: `NOT_ARMED`; 1 while keyed by another source: `KEYED`; 1 while another key condition of A3.1a fails: `NOT_KEYABLE`; 1 before the ATU parameters are set: `NOT_CALIBRATED`; none is latched; 1 while the cycle runs or within 2 s after its end: no-op; other values `BAD_VALUE` | answer | 0007 Decision 6; HIS-08; 0020 §8; 0022 §14, §22, §39, §51; 0032 |
| `ATU_STATE` | 0x0401 | RO | 0 off or bypassed, 1 in line, 2 tuning, 3 tuned, 4 failed (the last tune did not end tuned; the ATU is back at the stored state or bypassed; stays until the next tune or an in/out change; does not block keying, A3.9b item 7), 5 unknown (latches not written because their safe values are pending hardware, P-ATU-1; 0032) | event | 0006; HIS-08; 0020 §8; 0022 §38 |
| `ANT_STATE` | 0x0500 | RO | [7:0] connector actually selected, from 1; 0 unknown | event | 0006; HIS-06; 0020 §8 |

A `LIMIT_<name>` register holds the session value. A write is checked against the
persistent value (management C-3); a looser value is rejected with `LOOSER_LIMIT`. Writing 0,
and the end of the session, return the session value to the persistent value.

**Writing 0 is not "no power"** (0022 §73 item 5). For every `LIMIT_<name>` register, 0 means
"no session value, back to the persistent value"; it is never the limit value 0 (no power, no
TX time, SWR 0). The answer to a write of 0 carries the limit in force, which is then the
stricter of persistent and built-in value, so the host sees that the limit went up. A host
that wants no RF unkeys or disarms (B10.2), or writes a small non-zero value. The answer
carries the limit in force, the strictest of built-in, persistent and session value (A3.4;
0007 rules 1–2; 0020 §10; 0022 §6).

Constant `ARM_MAGIC` = 0x47375458 (ASCII "G7TX") (0020 §7).

B8.7 **Codes.** Table B8-3 gives the enumerations used in Table B8-2 (0020 §8). It is the
single authoritative list of symbolic code names of the G7250, both levels included. Part C
uses these names verbatim, in upper case as written here; part C's own enumerations, which
have no name in Table B8-3 (states, TX sources, mode classes, stream reasons), are lower case
(0021 §14, §22; 0022 §19, §32). A
limit id name is also the name of that limit's `LIMIT_<name>` register where the limit is
host-tunable (Table B8-2); the other limit ids have no register.

Table B8-3 — Reason and fault codes (0020 §8; 0022 §5, §19)

| Set | Codes |
|-----|-------|
| Arm reject reason (`TX_ARM_STATE` [15:8]) | 0 none; 1 `NOT_OWNER` (reserved, not set since 0022 §13; a non-owner gets `WRITE_STATUS`); 2 `NOT_RUNNING` (not set over P2: without RUN there is no session and every write is `NOT_OWNER`, B8.5; set on TCI, part C C5.5.6 item 1); 3 `WATCHDOG_DISABLED`; 4 `IN_FAULT`; 5 `INHIBITED`; 6 `PTT_ACTIVE`; 7 `LEVEL_NOT_ACTIVE`; 8 `MAINTENANCE`; 9 `NO_MODE_CLASS`; 10 `BAD_TOKEN`; 11 `TX_NOT_IMPLEMENTED`; 12 `NOT_CALIBRATED` (transmit parameters of Annex P not set; 0035) |
| Drop reason (`TX_ARM_STATE` [23:16]; Table A3-1) | 0 none; 1 `HOST_DISARM`; 2 `RUN_CLEARED`; 3 `WATCHDOG`; 4 `TAKEOVER`; 5 `RESET`; 6 `FAULT` (events 6 and 13); 8 `LINK_DOWN`; 9 `SESSION_CHANGED`; 10 `LEVEL_CHANGE`; 11 `MAINTENANCE`; 12 `REDISCOVERY`; 13 `STOP` (TCI only: `STOP` command, part C C5.5.5 row 2); 14 `HOLDER_LOST` (TCI only: arm holder's connection closed, part C C5.5.5 row 3); 7 unused; 13 and 14 are never set over P2; 0x40 + limit id `LIMIT_<name>` (event 7: action `DISARM` only; a trip with action `FAULT` gives 6 `FAULT`, the cause in `FAULT_CODE`; 0022 §41) (0022 §30) |
| Limit id (`TX_STATE` [31:24]; `LIMIT_STATUS` bit *n*; drop reason 0x40 + id) | 1 `LIMIT_TX_FREQ` (TX frequency); 2 `LIMIT_SWR` (SWR; host-tunable); 3 `LIMIT_PA_TEMP` (PA temperature; host-tunable); 4 `LIMIT_FWD_POWER` (forward power; host-tunable); 5 `LIMIT_SUPPLY_VOLTAGE` (supply voltage); 6 `LIMIT_CURRENT` (supply or PA current); 7 `LIMIT_TX_TIME` (maximum continuous TX time; host-tunable); 8 `LIMIT_ATU_TIME` (ATU tune time); 9 `LIMIT_SENSOR` (sensor plausibility); 10 `LIMIT_RF_NOT_KEYED` (RF while not keyed) |
| Fault code (`FAULT_CODE`, `FAULT_FLAGS` bit) and class | 0 none; 2 `SWR_REPEATED` (3 SWR trips in 10 s), class 1; 3 `PA_OVERTEMP` (PA temperature above T4), class 1; 9 `SENSOR_IMPLAUSIBLE` (sensor missing, stuck or out of range), class 2; 10 `RF_NOT_KEYED` (RF while not keyed), class 2; 16 `TX_PATH_NOT_BLOCKED` (drop event 13), class 2; 17 `WDT_RESET_AT_START` (processor watchdog reset seen at start-up, HIS-13), class 1; 18 `SELFTEST_FAILED` (start-up self-test failed), class 2; 19 `INTERNAL_ERROR` (internal consistency error), class 2; codes 1 and 4–8 reserved (those limits have no `FAULT` action) |
| Stream reason (`STREAM_STATUS` [6:0] per byte) | 0 none or running; 1 not enabled by the host; 2 not offered by this firmware; 3 sample rate not offered; 4 sample size not offered; 5 data format not offered (General byte 39); 6 time stamp or VITA-49 requested; 7 wideband parameters not offered; 8 link below 100 Mbit/s; 9 run bit clear; 10 update in progress; 11 VNA mode requested; 12 EER requested; 13 frequency encoding mismatch (General byte 37 bit [3] differs from discovery byte 21, B3.8; 0022 §73 item 12); 14 TX not calibrated (record `tx` bit 4 = 0, A1.12; 0039). Where several apply to one stream, the byte carries the first in this order: 10, 9, 8 (radio-wide), 1 (not enabled by the host), 13, 6, 11, 12, 5 (General-packet refusals), 2, 14 (not offered or not calibrated in this firmware), 3, 4, 7 (stream parameters) (0042) |
| Write reject reason (`WRITE_STATUS` [7:0]) | 1 `UNKNOWN_ADDRESS`; 2 `READ_ONLY`; 3 `NOT_OWNER`; 4 `BAD_VALUE`; 5 `LOOSER_LIMIT`; 6 `BUSY`; 7 `NOT_CLEARABLE`; 8 `KEYED` (RF-path change or ATU tune while keyed); 9 `NOT_ARMED`; 10 `NOT_KEYABLE` (ATU tune while another key condition of A3.1a fails, A3.9b); 11 `NOT_CALIBRATED` (ATU tune before the ATU parameters are set, A3.9b; 0032) (B8.4; 0022 §22, §51) |

Names introduced in this part and not in the decisions 0003–0008 (`STREAM_STATUS`,
`LINK_STATUS`, `WRITE_STATUS`, `LIMIT_STATUS`, `TRIP_COUNT`, the four `LIMIT_<name>`
registers, `ATU_STATE`, `ANT_STATE`, `TLM_*`, `SENSOR_VALID`) label registers that 0003, 0004,
0006 and 0007 describe without a name, or that 0020 adds (0020 §3). The limit id, fault code
and the write reject reasons 7–9 names are added by 0022 §19; `LIMIT_INHIBIT` and the drop
reasons `STOP` and `HOLDER_LOST` by 0022 §30, §32; the write reject reason `NOT_KEYABLE` by
0022 §51. 0006 "TX state: PA
actually keyed" and "TX arm state, and reason of the last disarm" are carried by `TX_STATE`
and `TX_ARM_STATE` of 0007. Every telemetry register exists only if the HIS confirms the
sensor or state; absent sensors are flagged absent in the capability record, not reported as
0 (0006, Decision; B4.8 `tlm_desc`).

### B9 Telemetry

B9.1 **Split.** Telemetry that a standard High Priority status field describes with identical
meaning is sent there; everything else is in the G7250 register map (0006 option 1A; 0005).

B9.2 **High Priority status packet** (radio → host, always from local port 1025, the default
of General bytes 11–12, since non-default ports are not honoured, B2.3) ([P2] High Priority
Status packet; 0006, Decision; 0020 §13; 0022 §20).

Table B9-1 — High Priority status packet ([P2]; 0006, Decision)

| Bytes | G7250 use | Depends on |
|-------|-----------|------------|
| 0–3 | sequence number, per [P2] (B2.7) | — |
| 4 [0] | PTT input that reaches the main unit (for example the controller microphone PTT); reports the input, never keys TX by itself; 0 in v1.0 | HIS-10, HIS-17; 0015; A3.7; 0030 |
| 4 [1], [2] | dot, dash of the KEY input, if it reaches the main unit; else 0; 0 in v1.0 | HIS-17; 0030 |
| 4 [4] | 1 only if a reference PLL exists and is locked; otherwise 0 | HIS-05 |
| 4 [3], [5]–[7] | 0 | [P2] ([5], [6] "no longer supported") |
| 5 | bit [0] ADC0 overload; other bits 0 unless more ADCs exist | HIS-01, HIS-03 |
| 6–7 | exciter forward power, raw ADC word, only if a separate exciter detector exists; else 0 | HIS-11 |
| 8–13, 16–21, 24–29, 43–48 | 0 (reserved / unused) | 0005 rule 3 |
| 14–15 | PA forward power, raw ADC word, always filled (no Alex) | HIS-11; Annex X1, D19 |
| 22–23 | PA reverse power, raw ADC word, always filled (no Alex) | HIS-11; Annex X1, D19 |
| 30 | [0] DDC FIFO overflow and [2] DUC FIFO underflow from the firmware's own buffers; [1] 0 (no microphone data, 0031); [3] 0 (no speaker) | 0004, HIS-17, 0017, 0031 |
| 31–32, 35–36 | DDC and DUC buffer depth in samples, from the firmware's own buffers | 0004 |
| 33–34 | 0 (no microphone data in this version) | HIS-17; 0031 |
| 37–38 | 0 (speaker FIFO; no speaker) | 0017 |
| 39–40 | ADC0 max magnitude in the message period, 0–32767 (bit 15 = 0), if the DDC chain can provide it; else 0; [P2]'s note "Max ADC1 magnitude" is read as ADC0 (Annex X2, C14) | HIS-01, HIS-18; 0022 §21 |
| 41–42 | 0 unless a second ADC exists | HIS-03 |
| 49–50 | supply voltage at the power input, raw ADC word | HIS-11 |
| 51–58 | 0 (general purpose analogue inputs, "user specific" in [P2]) | 0006 option 1B rejected; 0022 §21 |
| 59 | 0 (general purpose digital inputs in [P2]; none on the G7250) | [P2] byte 59; 0022 §21 |

B9.3 **Encoding of the standard fields.** The power and supply fields MUST carry raw ADC
words, exactly as [P2] does; this is not a deviation (0006 option 2A, Decision). The host
converts them with the [P2] Appendix A formulas and this unit's constants from the capability
record (B4.8; 0003 §3):

- supply: `V = x / N × c`, with *N* from `cal_info` [31:16] and *c* = `cal_supply_c` × 10⁻⁶;
- power: `W = (x / N × c1)² / c2`, with *c1*, *c2* from `cal_power[set]` × 10⁻⁶, for
  forward, reverse and exciter power; set *i* applies to `tx_band[i]` if `cal_info` bit 0 is
  set, else set 0 applies to all bands.

There are no fixed constants for board type 200; the constants are those of this unit, per
band where the hardware needs it (0006, Resolved; HIS-11). Taking them from the record
instead of [P2] Appendix A is listed in Annex X1, D20. The safety core uses its own calibrated
values, not these words through the host formula; host display and firmware limits can differ
where the formula does not fit exactly (0006 option 2A, "Against"). Which coupler position the
forward and reverse values describe is stated in record field `coupler`; its value:
pending hardware (P-SEN-1; HIS-08, HIS-11; B4.9), 0 until set. A generic P2 host has no
constants for this unit and may show wrong power and supply values (0006, Consequences).

B9.4 **Rates.**

- "Immediately" in this section means within 5 ms of the change, the bound of B8.4 (0020
  §11; 0022 §20).
- High Priority status: every 50 ms, every 1 ms while transmitting, and immediately when an
  input bit (bytes 4, 5) changes; immediate sends are limited to one per 1 ms ([P2] High
  Priority Status packet; 0006, Decision). If 1 ms is too costly (HIS-14, HIS-18), this is
  revisited and listed as a deviation (0006, Consequences).
- Register map (placeholders until HIS-11, HIS-14, HIS-18; 0006, Decision; Resolved): event
  registers immediately on change, at most one packet per 1 ms; fast registers every 10 ms
  while transmitting, otherwise with the periodic snapshot; a periodic snapshot of all
  telemetry registers every 1 s in one memory-mapped packet; analogue registers also on change
  beyond the hysteresis stated per register in record field `tlm_desc` (B4.8, B4.9; 0020 §26).
  The rates in use are stated in `tlm_rates` and `status_rates`.

B9.5 **G7250 telemetry registers.** The registers of B8 marked slow, fast or event in Table
B8-2, in the units of B8.6. The firmware computes SWR itself; the `TLM_SWR` register is the
value the limits act on. Forward and reverse power are not duplicated in registers (0006,
Decision). Raw readings behind the engineering-unit registers and calibration writes go
through the management channel, not the register map (0006, Consequences; A1).

### B10 Transmit (milestone M4)

Transmit MUST NOT be enabled in a firmware before M3 is closed: record `tx` bit 1 = 0 (B4.8;
design note §6; 0007, header). Release r1.0 implements transmit: an r1.0 firmware MUST report
`tx` bit 1 = 1 (0036), so an uncalibrated r1.0 radio rejects an arm with `NOT_CALIBRATED`, not
`TX_NOT_IMPLEMENTED`; `TX_NOT_IMPLEMENTED` is reserved for a build before M4 without a key path.
TX stays locked until the calibration set is complete (`tx` bit 4, A1.12): every arm is
rejected with `NOT_CALIBRATED`, so no TX request (PTT, CWX, `ATU_TUNE`) keys, none is latched,
`TX_STATE` bit 0 stays 0, and the safe state of A3.6 holds ([HIS] HIS 85; Annex P; 0035).

B10.1 **Every TX request needs the arm.** TX requests are: High Priority byte 4 bit [1] (PTT),
byte 5 bits [1] and [2] with bit [0] set (CWX dot and dash, B10.5a; Annex X2, C19), a KEY
closure in hardware CW while the owner has selected it (DUC-specific byte 5 bit [1]), and
internal ATU tuning (`ATU_TUNE`); these are the allowed requesters of A3.1a. CWX and KEY key
by themselves only with break-in set (DUC-specific byte 5 bit [7]); with break-in clear they
form RF only inside the owner's PTT transmission (B10.5b; 0022 §85). None of them keys
the radio unless TX is armed (0015; 0007 sub-decision 1 rule 4). A controller request is not a
TX request on level 1 and never keys (A3.7; 0007, Resolved 6; 0022 §65). High Priority byte 4 bits [2]–[7] never key (0007 rule 4). Every TX request follows
the key rule of A3.1a: a 0→1 edge at a moment when every key condition holds; a refused
request is not latched (0022 §24).

B10.2 **Arm handshake** (0007, Decision 1, option 1C):

1. The G7250 pushes `TX_ARM_TOKEN` at session start and whenever it changes.
2. The owner writes `TX_ARM_TOKEN XOR ARM_MAGIC` to `TX_ARM`. 0 disarms; any other value
   MUST be rejected.
3. The arm write MUST be accepted only if all of these hold; otherwise it MUST be rejected with
   a reason in `TX_ARM_STATE`: it comes from the owner (else `WRITE_STATUS` `NOT_OWNER`,
   B8.4); RUN is set; the watchdog is effectively on and General byte 38 bit [0] was written
   as set in a General packet of this session, its set-up phase included (else
   `WATCHDOG_DISABLED`; B5.6, B5.1a; 0022 §13, §34);
   the radio is not in fault; no limit inhibit is active; High Priority byte 4 bit [1] is 0 at
   that moment; level 1 is the active level; no firmware update is in progress and the running
   image is not an unconfirmed trial boot (else `MAINTENANCE`; A1.8; 0022 §10); a TX mode class
   has been declared in `TX_MODE_CLASS` for this session (else `NO_MODE_CLASS`) (0007 rule 1);
   and TX is implemented in this firmware, record `tx` bit 1 (else `TX_NOT_IMPLEMENTED`)
   (B4.8; 0020 §8); and the transmit parameters of Annex P are set, record `tx` bit 4 (else
   `NOT_CALIBRATED`; Annex P, freeze rule; 0035, 0036). Reason codes: Table B8-3. Where several reasons apply, `TX_ARM_STATE` carries the first
   in the order `BAD_TOKEN` (the written value, B10.2 item 2), then the conditions above (`NOT_RUNNING`, `WATCHDOG_DISABLED`, `IN_FAULT`,
   `INHIBITED`, `PTT_ACTIVE`, `LEVEL_NOT_ACTIVE`, `MAINTENANCE`, `NO_MODE_CLASS`,
   `TX_NOT_IMPLEMENTED`, `NOT_CALIBRATED`) (0035). The arm acceptance and `UPDATE_BEGIN` serialise:
   one of them wins atomically (A1.8; 0022 §61).
4. The arm takes effect when the G7250 pushes `TX_ARM_STATE` = armed with a new generation
   number. The host MUST wait for it. After arming, the G7250 MUST key only on a 0→1 edge of a
   TX request that arrives after the confirmation; a PTT bit that was already 1 does not key
   (0007 rule 2; [P2] does not order memory-mapped and High Priority packets).
5. The token MUST differ from the previous one and MUST NOT be 0, 0xFFFFFFFF or `ARM_MAGIC`
   (with the token equal to `ARM_MAGIC` the arm value would be 0, which disarms); it need not
   be unpredictable (0007 rule 3; 0020 §7; generation: HIS-18, HIS-20). It MUST also differ
   from the last token before a reset, which the firmware does not remember: the G7250 keeps
   a boot counter in non-volatile memory, incremented once at every start, and takes part of
   every token from it, so that no token of one boot repeats the last token of the boot before;
   a random part from the source of A1.10 MAY be added where HIS-20 gives one (0022 §73 item
   4). The boot counter lives in the same non-volatile store as the configuration and the
   calibration set (A1.6 C-4, A1.12); a firmware without that store MUST keep record `tx`
   bit 4 = 0, so that no arm can be accepted (0039). A correct arm write
   while already armed with the same token is a no-op: it is answered and does not raise the
   generation (0020 §7).
6. The arm is dropped by the events of Table A3-1.

B10.3 **TX mode class.** Values: 1 SSB (including data modes sent as SSB), 2 CW, 3 AM, 4 FM
(B8, Table B8-2; 0020 §9). The host MUST declare the class in `TX_MODE_CLASS` before arming and
MUST update it whenever its TX mode changes. A change while keyed unkeys first, then applies
(A3.9c; 0022 §26). The AM limit (25 W carrier [G7250-MAN], measured
as in A3.4c) applies whenever the declared class is AM (0022 §17). The power measurement applies whatever the host declares. A missing
declaration is never replaced by a default class; the arm is refused (`NO_MODE_CLASS`) (0007
rule 3; Resolved 7). Annex X1, D9.

B10.4 **Limits and faults.** Table A3-2 and A3.5 apply; actions are reported in `TX_STATE`,
`LIMIT_STATUS`, `SAFETY_STATE` and the fault registers (B8). Maximum continuous TX time: 5 min
built-in, `UNKEY` with the arm kept; a host MAY set it stricter through its `LIMIT_<name>`
register, never looser (0007, Resolved 3). TX frequency outside the transmit ranges: the TX
request does not key (Table A3-2).

B10.5 **Keyed timeout and TX IQ underrun** (0007, Decision 2):

- While keyed, the owner MUST send a High Priority packet at least every 100 ms ([P2]
  "Detailed operation", recommended rate). If no High Priority packet from the owner arrives
  within `KEYED_TIMEOUT`, the G7250 MUST unkey, keep the arm, and report the reason in
  `TX_STATE`. `KEYED_TIMEOUT` defaults to 500 ms; the host MAY write a shorter value, floor
  100 ms; a value above 500 ms or below 100 ms MUST NOT be accepted (0007 Decision 2;
  Resolved 2). A value outside 100–500 ms is rejected with `WRITE_STATUS` reason `BAD_VALUE`
  and the value in force stays (B8.4); `KEYED_TIMEOUT` returns to 500 ms at session end
  (0020 §17). Tolerance: the G7250 MUST NOT unkey before `KEYED_TIMEOUT` and MUST have unkeyed
  no later than `KEYED_TIMEOUT` + 10 ms (0020 §16). With CW break-in the keyed timeout also
  runs during the hang delay between elements and, when it expires there, ends the hang
  delay: RF gone, SEND released, T/R relay returned (A3.4d, A3.9a item 4; 0022 §83 item 1).
  Annex X1, D7.
- The keyed timeout also applies during internal ATU tuning; it ends the tune, reported as
  failed (`ATU_STATE` = 4) (0022 §14).
- Whenever the TX source is PTT (DUC I&Q), whatever the TX mode class, a DUC FIFO underflow
  ([P2] High Priority status byte 30 bit [2]) lasting longer than the underrun time MUST
  unkey, arm kept, `TX_STATE` unkey reason 3 (0007 Decision 2; 0022 §25). The one exception
  is a PTT transmission whose RF the keyer forms from KEY or CWX, with CW selected
  (DUC-specific byte 5 bit [1]) and break-in clear: it sends no DUC I&Q, so the check is
  waived (B10.5b; 0022 §85). Exact rule (0022 §57):
  - **Start.** The underflow starts at the key (A3.1a) if the DUC FIFO is empty then, and
    otherwise at the moment the FIFO runs empty while keyed. Every sample that enters the FIFO
    ends the underflow; the next one starts when the FIFO runs empty again.
  - **Zeros.** While the FIFO is empty the radio MUST feed the DUC with zero samples, never
    with old or undefined samples.
  - **Bounds.** The underrun time MUST be at least the key latency plus one DUC packet period
    (240 samples at 192 ksps = 1.25 ms, B10.7), so that a host that starts its I&Q with the
    PTT edge is not cut; the key latency is the longest time from the arrival of the edge to
    the key, the bound `t_rf_off` + `t_settle` of A3.9a item 3 (pending hardware P-RF-2, P-RF-3; HIS-07, HIS-12). It MUST
    be at most the `KEYED_TIMEOUT` in force, so at most 500 ms, and at most 100 ms when
    `KEYED_TIMEOUT` is at its floor. Exact value pending hardware (P-TIM-2; HIS-18); HIS-07 and HIS-12 must give
    `t_rf_off` + `t_settle` ≤ 98.75 ms for both bounds to hold.
  - **Tolerance.** The G7250 MUST NOT unkey before the underrun time and MUST have unkeyed no
    later than the underrun time + 10 ms after the underflow starts, as for the keyed timeout
    (0020 §16; 0022 §51).

  A host that sends CW as I&Q keeps the I&Q stream running during key-up, with zero
  samples, so that the check "RF while not keyed" runs in the key-up (A3.9a item 6; 0022 §88
  item 1). CWX and KEY use no
  DUC I&Q and have no underrun check, also inside a PTT transmission with break-in clear
  (B10.5b). Annex X1, D8; Annex X2, C20.
- What the keyed timeout catches (0022 §25): a host whose High Priority packets stop. It does
  not catch a host whose I/O thread keeps sending a cached PTT or CWX bit while its TX logic
  has hung; for that case the backstops are the underrun check (PTT), the CWX cap (B10.5a)
  and the maximum continuous TX time (Table A3-2). A host that keeps sending cached I&Q as
  well is caught only by the maximum TX time (known limit). So is, with break-in clear and CW
  selected, a host that keeps sending a cached PTT bit: the underrun check is waived there
  (B10.5b item 2.3), so the PTT bit alone holds SEND, the T/R relay and an external amplifier
  in TX, without RF, until the maximum continuous TX time (known limit; 0022 §88 item 4).
- After an unkey the host needs a new 0→1 TX request (A3.1a, A3.3).

B10.5a **CWX and its key-down cap** (0022 §25, §36). With High Priority byte 5 bit [0] (CWX)
set, bit [1] (dot) and bit [2] (dash) are the paddle inputs of the radio's keyer, which forms
the elements at the keyer speed and weight of DUC-specific bytes 9–10 ([P2] High Priority
packet byte 5, p. 34; Annex X2, C19).

**Keyer mode for CWX** (0022 §82). Of the keyer bits of DUC-specific byte 5 ([P2] DUC specific
packet byte 5), CWX uses iambic (bit [3]), mode B (bit [5], mode A if clear) and strict
character spacing (bit [6]). Reverse CW keys (bit [2]) does not apply: bits [1] and [2] of
High Priority byte 5 are named dot and dash in [P2] and are taken as such; reversing is a
matter of the wiring of a physical paddle on KEY. With iambic set, the keyer forms dots
while bit [1] is held, dashes while bit [2] is held and alternating elements while both
are, as iambic mode A or B. With iambic clear the keyer works as a straight key: the radio
is key-down for as long as bit [1] or bit [2] is held, with no element timing and no weight,
under the 1 s cap below and the ramp of byte 17. A keyer speed of 0 (Table B10-1) keeps CWX
from keying in both modes. The TX request is asserted while bit [0] and at least one
of bits [1], [2] are 1; with break-in set its 0→1 edge is the key edge of A3.1a, and with
break-in clear the elements form RF only inside the owner's PTT transmission (B10.5b; 0022
§85). Every element the keyer forms
is a key under every condition of A3.1a; if a condition fails at an element, the keyer stops
and keying needs both bits at 0 and then a new 0→1 edge, not latched (A3.1a, keyer elements;
0022 §74). The hang delay between elements counts as keyed for RF-path purposes (A3.4d). With
bit [0] clear, bits [1] and [2] are ignored. Known limit (0022 §86 item 9): iambic (bit [3])
is one bit for the hardware keyer from KEY and for CWX, so a host cannot use KEY as an iambic
paddle and CWX as a straight key, or the reverse, at the same time; the straight-key reading
for CWX is a deviation, Annex X1 D33. Either paddle bit held at 1 continuously for more than 1 s MUST unkey
(`UNKEY`, arm kept, `TX_STATE` unkey reason 9), whichever bit it is and also while the keyer is
between elements; the keyer stops, and keying again needs both bits at 0 and then a new 0→1
edge (A3.1a). Tolerance as B10.5: not before 1000 ms, unkeyed by 1010 ms. The same cap applies
to a TCI `KEYER` key-down (part C C8.2). The KEY input (hardware CW, HIS-17) is not capped.
Annex X1, D26.

**Keyer timing** (0046). For CWX with iambic set, and for the hardware keyer once it is used
(0030), the keyer MUST form its elements as follows ([P2] DUC specific packet bytes 9, 10 and
byte 5 bits [3], [5], [6]; element ratios of ITU-R M.1677-1 [ITU-M1677], with words per minute counted on the 50-unit word
PARIS).

1. **Unit.** With keyer speed *S* wpm (DUC-specific byte 9), the unit is *T* = 1200 / *S* ms.
   *S* is honoured from 1 to 60 ([P2]: "0 to 60 WPM"); 0 keeps CWX from keying (Table B10-1);
   a value above 60 is used as 60, logged (A1.7), the log entry containing the text `CW speed <S> used as 60` (S in
   decimal) once per DUC-specific packet that carries it, and counted in `GET_STATS` under the
   name `cw_speed_clamped`.
2. **Elements and spaces.** A dot is 1 *T* of mark, a dash 3 *T*, the space between the
   elements of one character 1 *T*.
3. **Weight.** With weight *W* (byte 10; [P2]: "33 to 66, nominal is 50"), every mark is
   lengthened by Δ = *T* × (*W* − 50) / 50 and the space after it shortened by the same Δ, so
   that a dot plus its space stays 2 *T* and a dash plus its space 4 *T*. *W* = 50 is neutral.
   A value below 33 is used as 33, above 66 as 66; 0 is read as 50.
4. **Iambic mode A and B** (byte 5 bit [5]; the Curtis definitions). The keyer decides the
   next element at the end of the 1 *T* space after each element (the decision point), and at
   a paddle edge while it is idle. While one paddle is held it repeats that paddle's element;
   while both are held it alternates, starting with the element of the paddle pressed first.
   - **Mode A** (no memory): at the decision point only the paddles held at that moment count;
     with none held the keyer stops. A paddle pressed and released during an element has no
     effect.
   - **Mode B** (memory): in addition, the opposite paddle pressed at any time during the mark
     of the current element counts at the decision point even if it has been released; so
     after a squeeze released before the end of an element the keyer sends one more element,
     the opposite one, and then stops. Only the mark is the memory window, not the space after
     it (0046).
   - **Latency.** An element that starts from a paddle edge while the keyer is idle MUST start
     (key-on) within 5 ms of the arrival of the High Priority packet that carries the edge,
     measured on the reference model and the emulator; when the edge needs an RF-path change,
     the bounded wait of A3.9a item 3 comes first.
5. **Strict character spacing** (byte 5 bit [6]). Set: when no paddle is held at the end of the
   1 *T* space after an element, the character has ended, and the next element MUST NOT start
   before 3 *T* of space have passed since the end of the last mark; at that point it starts
   only if a paddle is held then (a paddle pressed and released inside the wait has no effect,
   in both modes). Clear: the next element starts as soon as a paddle is held after the 1 *T*
   space, within the latency of item 4.
6. **Tolerance.** Each mark and each space MUST be within ±1 ms or ±2 % of its nominal length,
   whichever is larger, and the length of a whole character within ±2 ms or ±2 %. On the radio
   it is measured between the 50 % points of the RF envelope (the ramp of byte 17 is
   symmetric); on the reference model and the emulator between the key-on and key-off events of
   the keyed state. Tests: T-B10-43 to T-B10-46.

Host note (0022 §61). The cap counts how long a paddle bit stays at 1, not how long one
element lasts. A host that holds a paddle bit at 1 for an iambic string or a run of dots
longer than 1 s has the string cut at 1 s. A host SHOULD release the paddle bits between
characters, as a paddle does, or send long key-down or long strings by a means without this
cap: DUC I&Q with PTT (B10.5) or, on TCI, CW text (part C C8.2).

B10.5b **Break-in and CW with PTT** (0022 §85; Annex X2, C22). [P2] lists DUC-specific byte 5
bit [7] "Break_in" among the CW options without saying what it changes, and says nothing on
CW together with PTT ([P2] DUC specific packet byte 5; High Priority packet bytes 4, 5). The
G7250 reads the bit as follows, for KEY (hardware CW, B10.9) and for CWX (B10.5a):

1. **Break-in set.** KEY and CWX key by themselves (B10.5a, B10.9): each element is a key
   under A3.1a, and the hang delay of bytes 11–12 holds SEND and the T/R relay between
   elements (A3.4d, A3.9a item 4). PTT and CW never share a transmission; the first source
   keeps it. A PTT 0→1 edge while a CW transmission is keyed, its hang delay included, is
   refused; a KEY closure or CWX paddle edge while a PTT transmission is keyed is refused for
   that element and the keyer stops. Neither is latched, and neither ends the transmission in
   progress (A3.1a). KEY and CWX drive the same keyer and are one source, "the keyer", for
   this rule (0022 §87 item 5): inside one CW transmission, its hang delay included, a KEY
   element may follow a CWX element and the reverse, each a key under A3.1a, and the
   transmission and its hang delay continue; while both are active the keyer's dot and dash
   inputs are the OR of the KEY contacts and of CWX bits [1] and [2]. The 1 s cap of B10.5a
   measures the CWX bits alone.
2. **Break-in clear.** KEY and CWX MUST form RF only inside the owner's PTT transmission
   (High Priority byte 4 bit [1]; A3.1a):
   1. The transmission is the PTT transmission. It keys at the PTT 0→1 edge under every
      condition of A3.1a and ends at the PTT release or any other unkey. "The PTT key" is
      the moment at which RF is enabled at the end of the key sequence of A3.9a item 4: after
      any wait of A3.9a item 3, the T/R relay and the RX attenuator during TX settled, SEND
      and the RF delay of DUC-specific byte 13 (0022 §87 item 1). The T/R relay, the RX
      attenuator during TX and SEND follow the PTT, not the elements (A3.9a item 4); the hang
      delay is not used; `TX_STATE` shows TX source 1 (PTT). The keyed timeout (B10.5), the
      maximum continuous TX time and every limit apply to it (A3.4d).
   2. Inside it, each element of KEY (with CW selected, byte 5 bit [1], B10.9) or of CWX
      (B10.5a) forms RF at the keyer speed, weight and ramp in force. An element is not a key
      of its own: it needs the conditions of A3.1a that the PTT transmission met at its key,
      and a condition that fails ends the PTT transmission by its own rule (A3.9, A3.9c,
      B10.10, Table A3-2). Keyer RF MUST NOT start before the PTT key. An element needs a
      KEY closure or paddle 0→1 edge after the PTT key; a closure or paddle bit already held
      at the PTT key, including one that closed or rose between the PTT 0→1 edge and the PTT
      key (during the settle or the RF delay), forms no RF until it is released and closed
      again (not latched, A3.1a; 0022 §87 item 1). For CWX the edge is the edge of the CWX
      request of B10.5a: after the PTT key the request MUST go to 0, both paddle bits at 0,
      and then rise again; a change from one held paddle bit to the other is not an edge
      (0022 §88 item 3). When the PTT transmission ends, the
      element in progress ends with its trailing ramp and the keyer stops.
   3. With CW selected (byte 5 bit [1] set) the keyer alone forms the RF of the PTT
      transmission. DUC I&Q is not transmitted: the DUC FIFO is emptied at the key, DUC I&Q
      received while keyed is dropped and counted (`GET_STATS`), and the underrun check of
      B10.5 is waived. The DUC gets the keyer's CW waveform, zero between elements, and the
      PEP estimate of A3.4c is computed on it. With CW not selected (bit [1] clear) KEY keys
      nothing (B10.9), the PTT transmission sends DUC I&Q as usual, and a CWX paddle edge in
      it is refused: with break-in clear CWX needs CW selected. It forms no RF and is not
      latched (A3.1a; 0022 §88 item 3). Host note (0022 §87 item 6): with CW selected and
      break-in clear, a PTT transmission without KEY or CWX elements is silent: the T/R
      relay and SEND are in TX, the DUC I&Q of the host is dropped, and no RF is sent. A host
      that sends voice or data with PTT SHOULD clear bit [1] in a DUC-specific packet first,
      or set break-in.
   4. The CWX key-down cap of B10.5a applies: a paddle bit held for more than 1 s ends the
      PTT transmission (`TX_STATE` unkey reason 9), arm kept; keying again needs a new PTT
      0→1 edge (A3.1a).
   5. Without the owner's PTT, a KEY closure or CWX paddle edge forms no RF and keys
      nothing; it is not latched, so a PTT edge later does not send it. KEY closures are
      reported as always (High Priority status byte 4 bits [1]–[2], B10.9).
   6. The refusals that A3.1a names for PTT with DUC I&Q (a DUC-specific refusal or EER
      standing, B10.6) apply to the PTT transmission also with CW selected.
3. **Change while keyed.** A change of bit [7] or bit [1] in a DUC-specific packet while
   keyed, the hang delay included, MUST be deferred (A3.9): the latest value is applied right after the
   unkey, before any new key, as for the other state fields of A3.9. So a transmission never
   changes between the two readings.

Reason (0022 §85): a host that keys the transmitter with its PTT (MOX) and has CW selected
expects the radio's keyer to form the RF while the PTT is held; break-in is the CW option that
lets the key alone switch the transmitter. Either reading keeps one transmission at a time.

B10.6 **DUC-specific packet.** Accepted on port 1026 ([P2] DUC specific packet).

Table B10-1 — DUC-specific packet ([P2] DUC specific packet)

| Byte | Field | G7250 behaviour | Source |
|------|-------|-----------------|--------|
| 4 | Number of DACs | ignored ("Presently unused" in [P2]) | [P2] byte 4 |
| 5 [1] | CW | honoured: selects hardware CW from KEY | [P2] byte 5; 0007 Decision 6 |
| 5 [2]–[6] | Reverse keys, iambic, sidetone, mode B, strict spacing | honoured for the hardware keyer from KEY; for CWX (B10.5a) iambic [3], mode B [5] and strict spacing [6] apply, reverse keys [2] does not; sidetone ignored (no audio, 0031; HIS-17); the hardware keyer from KEY is not used in v1.0 (0030) | [P2] byte 5; 0007 Decision 6; 0022 §82 |
| 5 [7] | Break-in | honoured: set, KEY and CWX key by themselves, with the hang delay of bytes 11–12, and PTT and CW never share a transmission; clear, KEY and CWX form RF only inside the owner's PTT transmission, which T/R relay and SEND follow, with no hang delay and, with CW selected, no DUC I&Q and no underrun check; a change while keyed is deferred to the unkey (B10.5b) | [P2] byte 5; 0007 Decision 6; 0022 §85 |
| 5 [0] | EER | set: refused (EER not offered): DUC I&Q not accepted and transmit refused until cleared; `STREAM_STATUS` reason 12 | [P2] byte 5, DUC I&Q NOTE; 0004 point 7; 0020 §22 |
| 6–8 | Sidetone level, frequency | ignored (no audio, 0031; HIS-17) | [P2] bytes 6–8 |
| 9, 10 | Keyer speed, weight | honoured as B10.5a "Keyer timing" (unit 1200 / *S* ms, *S* 1–60; weight 33–66, 50 neutral; 0046); speed 0 (also the value after a session end, A3.6a): the keyer forms no element and CWX does not key, the request is not latched (A3.1a) | [P2] bytes 9–10; 0007 Decision 6; 0022 §64 |
| 11–12 | Hang delay, ms | used with break-in set only (B10.5b); honoured up to 2000 ms; a larger value is clamped to 2000 ms, logged (A1.7), the log entry containing the text `CW hang delay <ms> clamped to 2000` and counted in `GET_STATS` under the name `cw_hang_clamped`; the keyed timeout ends a hang delay (B10.5) | [P2] bytes 11–12; 0007 Decision 6; 0022 §83 item 1 |
| 13 | RF delay, ms | honoured; SEND asserted before RF | [P2] byte 13; 0007 Decision 6 |
| 14–15 | DUC0 sampling rate | 192 honoured; any other value refused: DUC I&Q not accepted and transmit refused (PA off) until a valid packet; 0 read as 192 | [P2] bytes 14–15; 0004 points 2, 7; 0020 §18 |
| 16 | DUC0 bits | 24 honoured; 0 read as 24; any other value refused, as bytes 14–15 | [P2] byte 16; 0004 points 2, 7; 0020 §18 |
| 17 | CW ramp period | 5–10 ms honoured; 0 and any other value: G7250 default 8 ms; whether the hardware CW path can shape the ramp: HIS-12 | [P2] byte 17; 0020 §22 |
| 18–49 | Reserved | ignored | [P2]; 0005 rule 3 |
| 50, 51 | Line in, mic boost, Orion mic; line-in gain | ignored (no audio, 0031; HIS-17) | [P2] bytes 50–51 |
| 52–56 | Reserved | ignored | [P2]; 0005 rule 3 |
| 57–59 | RX step attenuator during TX | byte 59 (ADC0) ignored until P-RF-4 is set (HIS-07); bytes 57–58 ignored unless more ADCs (HIS-03) | [P2] bytes 57–59 |

**Refusal arriving while keyed** (0022 §87 item 8). EER set (byte 5 bit [0]), a DUC0 sampling
rate other than 192 (bytes 14–15) or DUC0 bits other than 24 (byte 16) that arrive while
keyed, the CW hang delay included, are deferred to the unkey as every DUC-specific state
field (A3.9): the transmission in progress continues with the DUC settings in force and DUC
I&Q is still accepted at them; the refusal stands from the unkey, before any new key, so the
next PTT 0→1 edge is refused (A3.1a) until a valid packet clears it. `STREAM_STATUS` shows
the refusal's reason (12 for EER) from the unkey. Reason: a refusal changes no RF-path state
and the transmission in progress was keyed under valid settings; deferring keeps one rule for
every DUC-specific field while keyed.

Host note (0022 §88 item 5). A host SHOULD NOT change the DUC0 sampling rate (bytes 14–15)
or bit width (byte 16) while keyed, the CW hang delay included: the change is deferred, so
I&Q that the host sends at the new rate is played at the old rate until the unkey. A host
that changes the rate unkeys first and keys again after the DUC-specific packet.

B10.7 **TX IQ (DUC0).** In on port 1029, 192 ksps, 24-bit I and Q, 240 samples per packet,
1444-byte packets as [P2] DUC I&Q Data packet ([P2] DUC I&Q Data packet; "Detailed operation"
NOTE2; 0004 point 2). Record `duc_format` (B4.8). TX IQ is accepted only while no refusal of
B10.6 stands; the PA keys only under B10.2. A DUC I&Q packet whose source IP address and UDP
port are not the owner's (A2.2), or that arrives while there is no session, MUST be discarded
whole, MUST NOT enter the DUC FIFO and MUST have no other effect; it is counted per source in
`GET_STATS` (B11.1). A host MUST therefore send DUC I&Q from its session socket (0022 §4;
Annex X1, D24). The G7250 MUST empty the DUC FIFO at every unkey, every session end and every
takeover, so that a later key never sends stale I&Q (0022 §32). While record `tx` bit 1 or
bit 4 is 0, DUC I&Q from the owner MUST also be discarded whole and counted in `GET_STATS`, and
`STREAM_STATUS` DUC0 reads not running with reason 2 (TX not implemented) or 14 (TX not
calibrated) (0039).

**DUC FIFO while unkeyed** (0022 §63). A host may stream DUC I&Q continuously, keyed or not.
While unkeyed, the G7250 MUST keep only the owner's most recent DUC I&Q, at most 4800 samples
(25 ms at 192 ksps, 20 packets), and only samples received within the last 25 ms: a sample
that would exceed the depth, or that has been in the FIFO for 25 ms, is dropped, oldest first,
and counted (`GET_STATS`). The first sample transmitted at a key is therefore at most 25 ms
old when the key comes, and a host that sends its I&Q with or shortly before its PTT edge has
its first samples sent. While keyed the FIFO holds what the host sends up to its capacity,
pending hardware (P-TIM-2; HIS-18); a full FIFO discards whole packets (B11.7). Dropping old samples while unkeyed is
not an underflow (B10.5).

B10.8 **Drive level.** High Priority byte 345 is honoured; mapping to power pending hardware (P-PWR-1; HIS-12);
`REDUCE` lowers drive on the radio side without changing the host's value (0007 rule 4).

B10.9 **Other TX paths** (0007, Decision 6; A3.7):

| Path | Rule |
|------|------|
| Controller mic PTT | reported in status byte 4 bit [0] if it reaches the main unit (HIS-17); never keys by itself |
| KEY (Morse) | reported as dot/dash, status byte 4 bits [1]–[2]; the one local requester of A3.1a: keys hardware CW only when armed, CW selected by the owner in this session (DUC-specific byte 5 bit [1]; cleared at every session end and takeover, A3.6a) and all limits pass; delays per DUC-specific bytes 5, 11–13 (HIS-17); a closure that cannot key is not latched (A3.1a; 0022 §65); every element of the hardware keyer is a key under every condition of A3.1a, and a failed condition stops the keyer until a new closure (0022 §74); the hang delay counts as keyed for RF-path purposes (A3.4d); with break-in set KEY keys by itself and a closure during a PTT transmission is refused, with break-in clear KEY forms RF only inside the owner's PTT transmission and keys nothing without it (B10.5b; 0022 §85) |
| Host CWX | High Priority byte 5: bit [0] selects host CW, bits [1] and [2] are the keyer's paddles; a TX request under the arm; a paddle bit held for more than 1 s unkeys (B10.5a; 0022 §36); with break-in clear, RF only inside the owner's PTT transmission and with CW selected (B10.5b; 0022 §85) |
| SEND output | asserted only while armed and keyed, before RF (RF delay), and with CW break-in during the hang delay between elements (no RF then, A3.9a item 4); released after RF gone at the end of the transmission; released, ending any hang delay, on every drop event, on standby and in fault (HIS-09; 0022 §73 item 9); released after RF gone, ending the hang delay, when a change that needs another TX RF-path state, an owner's RF-path relay change or a TX mode class change arrives in a gap, or PA enable is cleared, before any relay moves or the change is applied (A3.9a item 4; 0022 §74); with break-in clear, asserted and released with the owner's PTT transmission, not with the elements of KEY or CWX inside it (B10.5b; 0022 §85) |
| TUNER trigger | under the arm, as ATU tuning (HIS-09) |
| Internal ATU tuning | a tune cycle (A3.9b): needs arm, owner, frequency in a range, limits, own profile and time limit; started by `ATU_TUNE` from unkeyed only (HIS-08); tune power the lower of the profile's (pending hardware P-ATU-1; HIS-08, HIS-12) and the forward-power bound in force (A3.4 rule 5; 0022 §53), set before RF and before any ATU relay moves; `ATU_TUNE` while not armed is rejected with `NOT_ARMED`, while keyed by another source with `KEYED`, neither latched; a PTT or CWX edge during the tune neither keys nor ends it, and a new 0→1 TX request is needed after it; the cycle ends unkeyed; the keyed timeout applies; a start within 2 s after the previous cycle is a no-op; with another key condition failing, `NOT_KEYABLE`; a start during the CW hang delay gets `KEYED` (A3.4d); `ATU_TUNE` = 0 from the owner is always accepted and is a no-op when no cycle runs (0022 §74, §77 item 5); the cycle first switches the RF path to the TX state for the TX frequency and settles; after every cycle the RF path returns to the state for the receive frequency, a cycle that did not end tuned storing nothing (A3.9b items 1, 2, 7) (0022 §14, §22, §38, §39, §66) |
| P2 TUNE bit (High Priority 1396–1397 bit [0]) | information only; does not start the internal ATU |
| Controller ATU key, SWR scan, other controller TX functions | refused while level 1 runs (A3.7), unless they reach the main unit as reportable requests (HIS-17) |
| Controller ANT and ATU in/out buttons | while keyed: ignored and logged, if they reach the main unit (A3.9; HIS-17) |
| Open-collector outputs (High Priority byte 1401) | ignored |

On level 1, controller inputs never key the radio (0007, Resolved 6).

B10.10 **PA enable while keyed.** General byte 58 bit [0] cleared while keyed, the CW hang
delay included (A3.4d; 0022 §74), MUST unkey (`UNKEY`, arm kept, `TX_STATE` unkey reason 7); setting it again MUST NOT key without a new
0→1 TX request ([P2] General packet byte 58; 0020 §22; 0022 §14). The bit is cleared at every
session end and takeover (A3.6a; 0022 §64); while it is 0 there is no exciter-only output
either (Annex X1, D30).

### B11 Error handling and robustness

This section gives the complete error handling of G7250-P2 (0001 spec section 11; 0004
point 7; 0020 §28). Values and reason codes are in B8 (Tables B8-2, B8-3).

B11.1 **Principle.** A packet the G7250 does not act on MUST have no effect at all: it MUST NOT
feed the watchdog, cause a takeover, start a session or change any state (0020 §28). Every
discarded packet is counted per port and reason and can be read with `GET_STATS` over the
management channel (A1.4, Table A1-2; 0020 §19, §28). Where a host can be told, the reason is
reported in a status register.

B11.2 **Requests not offered.** The G7250 MUST NOT clamp a request to an offered value and
MUST NOT silently keep a previous setting. It refuses: the stream concerned does not run until
a valid request arrives, and the reason is reported in `STREAM_STATUS` (0004 point 7; options
6A–6C rejected). Cases: B6.3, B6.6, B6.7, B6.10, B10.6, and VNA mode and EER (Table B7-1,
Table B10-1; 0020 §22). Annex X1, D13.

B11.3 **Malformed and short packets.** A packet shorter than the minimum of Table B11-1 MUST be
discarded whole; bytes beyond the [P2] length of the packet MUST be ignored (0020 §28).

Table B11-1 — Minimum UDP payload length accepted (0020 §4, §28)

| Port | Packet | Minimum length (bytes) | Source |
|------|--------|------------------------|--------|
| 1024 | Discovery (byte 4 = 0x02) | 5 | [P2] Discovery packet; B3.2 |
| 1024 | Set IP Address (0x03) | 15 | [P2] Set IP Address packet bytes 0–14 |
| 1024 | General (0x00) | 60 | [P2] General packet bytes 0–59 |
| 1025 | DDC-specific | 1444 | [P2] DDC specific packet bytes 0–1443 |
| 1026 | DUC-specific | 60 | [P2] DUC specific packet bytes 0–59 |
| 1027 | High Priority | 1444 | [P2] High Priority packet bytes 0–1443 |
| 1029 | DUC I&Q | 1444 | [P2] DUC I&Q Data packet |
| 7250 | Memory-mapped from PC | 10 (one pair) | [P2] Memory mapped registers from PC; B8.1 |
| 7251 | Management | 18 to be executed; 12–17 bytes with the right magic are answered with status 17; below 12 bytes dropped | A1.4, Table A1-1; 0022 §20 |

A truncated High Priority packet is never acted on in part, because PTT and drive would then be
read from missing bytes (0020 §28).

B11.4 **Port 1024 commands.** Erase (0x04) and Program (0x05) are ignored without a reply
(B3.9). A packet whose byte 4 is not 0x00, 0x02, 0x03, 0x04 or 0x05 MUST be ignored without a
reply (0020 §28).

B11.5 **Register writes** (B8.4, B8.5; 0020 §11, §12):

- a write to an address not in Table B8-2, or in a group that is not present, is rejected with
  `UNKNOWN_ADDRESS`;
- a write to a read-only register, including the capability record, is ignored and answered
  with the current value and `READ_ONLY` (B4.6);
- a write from a host that is not the owner, other than `CAP_REQUEST`, is rejected with
  `NOT_OWNER` and has no other effect;
- a value outside the allowed set is rejected with `BAD_VALUE`; a looser limit with
  `LOOSER_LIMIT` (A3.4); any write during a firmware update with `BUSY` (A1.8);
- a `FAULT_CLEAR` that cannot clear with `NOT_CLEARABLE` (A3.5); an RF-path write or an
  `ATU_TUNE` start while keyed with `KEYED` (A3.9, A3.9b); `ATU_TUNE` while not armed with
  `NOT_ARMED` (B10.9) (0022 §19, §22);
- arm writes that fail a precondition are rejected with a reason in `TX_ARM_STATE` (B10.2);
- padding pairs and a trailing incomplete pair are ignored (B8.1).

B11.6 **Sequence numbers and gaps.** The G7250 MUST NOT reject or reorder an incoming packet
because of its sequence number (B2.7), except a High Priority packet as B11.6a states. It counts gaps in the sequence numbers received on
ports 1027 and 1029 per session; the counts are read with `GET_STATS` (0020 §19). Outgoing:
a packet the G7250 drops (B11.7) still uses its sequence number, so that the host sees the
loss as a gap (0020 §19).

B11.6a **Stale High Priority packets** (0022 §35; Annex X1, D28). A reordered or duplicated
High Priority packet can carry an old PTT or CWX value and so make a false 0→1 edge.

Every session starts, and every takeover and resynchronisation restarts, in unnumbered mode.
"Newer" is 32-bit serial-number arithmetic (RFC 1982, SERIAL_BITS = 32); the number 0 is never
newer. Only the owner's High Priority packets of the session that are acted on (not
discarded under B11.1, B11.3) take part (0022 §35, §56).

- **Unnumbered mode** (as [P2] gives it: every number 0; also before numbered mode engages).
  Order cannot be told. Mitigation, exactly (0022 §55): a 0→1 edge of PTT, or of the CWX
  request (B10.5a), is accepted only if `t_edge` − `t_ref` ≥ 10 ms, where `t_edge` is the
  arrival time of the packet that carries the bit at 1 after a packet with it at 0, and
  `t_ref` is the arrival time of the latest earlier packet of the session in which that bit
  read 1, whether or not that packet keyed; if there is none, the latest arm confirmation
  (B10.2 item 4). Arrival times are taken at receipt with a resolution of 1 ms or better; the
  reference model applies the rule exactly, the radio MUST ignore a difference of 9 ms or less
  and MUST accept one of 11 ms or more. An edge that fails the test is ignored and not latched
  (A3.1a); the bit then counts as 1 for the next edge. Known limit: a stale packet delayed by
  more than 10 ms behind a newer one can still key the radio until the next packet unkeys it;
  on a LAN this needs a reordering delay above 10 ms.
- **Engaging numbered mode** (0022 §56). Numbered mode engages with the third of three
  consecutive packets whose numbers are non-zero and each newer than the one before. Until
  then the unnumbered rule applies, also to the three packets.
- **Numbered mode.** The G7250 MUST discard, with no effect (B11.1), every High Priority
  packet whose number is not newer than the last one accepted, and count it (`GET_STATS`).
- **Resynchronisation** (0022 §56). The G7250 MUST leave numbered mode, return to unnumbered
  mode, log it (A1.7) and count it (`GET_STATS`) when a packet is the fifth in a row that
  is not newer than the last one accepted, or when a packet is older than the last one
  accepted by more than 2¹⁶.
  That packet is not discarded: it is acted on, except that a 0→1 edge in it is ignored and
  not latched; a 1→0 in it is followed. A 0→1 edge in a discarded packet is never accepted.
  While packets are discarded the keyed timeout (B10.5) runs on; an unkey by it is the safe
  direction.
- **Release in a discarded packet** (0022 §72). A discarded packet is otherwise not acted on,
  but if its PTT bit, or its CWX request (B10.5a), reads 0 while the radio is keyed by that
  request, the G7250 MUST unkey (`TX_STATE` unkey reason 1, arm kept) and count it: a 0 is
  always taken as a release. After such an unkey the bit counts as 1 until an accepted packet
  reads 0, so keying again needs an accepted packet with the bit at 0 and a later 0→1 edge;
  a stale release therefore ends the transmission and never re-keys it. This is the same
  outcome as a stale 0 in unnumbered mode, where the next 1 fails the 10 ms test.
- **Wrap-around** (0022 §72). Because 0 is never newer, a host that numbers its packets MUST
  skip 0 at the wrap: after 0xFFFFFFFF comes 1, which is newer (RFC 1982). A packet numbered
  0 in numbered mode is discarded (and counts for resynchronisation).
- A host SHOULD number its High Priority packets from 1 upwards and restart at 1 only with a
  new session.

Test vector (unnumbered, PTT; arm confirmed at *t* = 0 ms; arrival in ms, PTT bit, result):
(20, 1) keys, 20 − 0 ≥ 10; (25, 0) unkeys; (28, 1) ignored, 28 − 20 = 8; (30, 1) no edge;
(40, 0); (42, 1) keys, 42 − 30 = 12; (45, 0) unkeys; (50, 1) ignored, 50 − 42 = 8; (55, 0);
(60, 1) keys on the reference model, 60 − 50 = 10 (T-B11-13).

B11.7 **Overruns.**

- **RX.** When a DDC0 packet cannot be handed to the CH394Q in time (socket transmit buffer
  full), the G7250 MUST drop the whole packet, MUST NOT send a partial packet, and MUST set
  High Priority status byte 30 bit [0] ([P2] High Priority Status packet; 0020 §28). The same
  applies to wideband and microphone packets.
- **TX.** While keyed, a DUC I&Q packet that finds the DUC FIFO full MUST be discarded whole;
  the CH394Q has no flow control in UDP mode ([CH394] section 12.2.4; 0020 §28). While unkeyed
  the oldest samples are dropped instead, so that the FIFO holds the latest 25 ms (B10.7; 0022
  §63). Underflow: B10.5. DUC I&Q
  from a source other than the owner never reaches the FIFO (B10.7; 0022 §4). The FIFO is
  emptied at every unkey, session end and takeover (B10.7; 0022 §32).

B11.8 **Frequencies outside the ranges.** A DDC0 frequency outside `rx_range` is tuned as
requested, without refusal or clamping; receiver performance there is not specified (B6.8;
0020 §28). A DUC0 frequency outside the transmit ranges is accepted, but a TX request then
does not key (Table A3-2; `TX_STATE` bit 3).

B11.9 **Packets to closed ports, including 1028.** [CH394] describes only how the CH394Q
reports a received ICMP "port unreachable" (GINT bit UNREACH, registers UNIP and UNPORT,
sections 7.1.7, 7.1.13 and 7.1.14); it does not state whether the chip sends one for a
datagram to a port that has no open socket. Firmware rule: nothing for such a port reaches the
firmware, and the firmware MUST NOT send any reply. A host MUST NOT depend on either silence or
an ICMP message (0020 §28; 0017). Test T-B11-08 records what the chip does.

B11.10 **Stale sessions and hosts.**

- The 1 s watchdog ends a session whose host is gone (B5.6).
- With the hardware timer disabled (allowed only while not armed) the session stays until a
  takeover, as [P2] allows ([P2] General packet byte 38; 0007, option 2F "Against"). An ICMP
  "port unreachable" for the owner's port ([CH394] section 7.1.7) MUST be logged and counted
  and MUST NOT end the session (0020 §28).
- A management write session ends after 60 s without an authenticated, verified request from
  its holder (A1.10; 0020 §24; 0022 §75).

B11.11 **Session refusal** on a link below 100 Mbit/s, with the reason in `LINK_STATUS` (B5.8).

B11.12 **Non-default ports.** A non-default radio-side port in a General packet is not
honoured; the default is kept and `LINK_STATUS` bit 17 is set (B2.3; 0020 §13).

B11.13 **Management channel errors.** A1.4 and Table A1-3 (0020 §27).

B11.14 **Rate limits and flood resistance** (0022 §16, §49). To limit the amplification of
spoofed requests, answers to sources other than the owner MUST be limited per source IP
address and UDP port: `CAP_REQUEST` answers at most once per 1 s per source and 10 per second
in total (B4.4); `NOT_OWNER` answers at most once per 100 ms per source (B8.5); discovery
replies at most 10 per second per source and 50 per second in total (B3.2); management
replies at most 20 per second per source, 100 per second in total, identification 50 per
second in total and 10 per second to sources outside the subnet (A1.10; 0022 §71). Datagrams
that another radio's replies could be get no answer at all: a datagram to port 1024 from
source port 1024 (B3.2), a management datagram whose type is not 1 (A1.4), and a non-owner's
register packet from source port 7250 (B8.5) (0022 §73 item 20). Requests above a limit are
dropped and
counted (`GET_STATS`). The owner's own traffic is not limited, except the unasked capability
record (B4.4 item 2). Safety timing does not depend on the load (A3.10). Known limit (0022
§49): a spoofed High Priority packet with run set, from an address inside the radio's subnet
while no session runs, starts a session and so makes the radio send High Priority status to
the spoofed address, and, if a spoofed DDC-specific packet enabling DDC0 comes with it (B5.2),
stream DDC0 there, up to 18.6 Mbit/s at 384 ksps (B6.4); this is inherent in [P2], where the
run bit starts the data ([P2] "Detailed operation"), and B11.15 confines it to the radio's own
subnet (0022 §73 item 13). Known limit (0022 §77 item 4): the per-host caches (management
challenges and stored replies, A1.4, A1.10; set-up packets, B5.1a), each of 4 entries with
least-recently-used eviction, can be churned by a source inside the subnet that sends from
several addresses or ports; a legitimate host then loses its stored reply (a retransmission
gets status 33, A1.4), its challenge (a new `GET_CHALLENGE` is needed) or its kept set-up (it
sends its packets again, B5.4). None of this keys, arms or loosens a limit (A1.9, A3.10).

B11.15 **Sources outside the subnet** (0022 §49; Annex X1, D29). The G7250 MUST discard
(B11.1) every C&C packet, DUC I&Q packet and memory-mapped packet whose source address is
outside the radio's own subnet (address and netmask of A1.6 C-1), as the management channel
does (A1.10, S4): such a source starts no session, takes nothing over, keeps no set-up (B5.1a)
and gets no register answer. Discovery is answered as before (B3.2). Set IP is accepted from
outside the subnet only when sent to the limited broadcast 255.255.255.255, which a router
does not forward; unicast and directed-broadcast Set IP from outside the subnet is ignored
(B3.9; 0022 §70). A host behind a router cannot open a P2 session or change the radio's address;
it needs a bridged link.

---

## Part C — Level 2: TCI

Part C, the single G7250 TCI profile followed by both AetherSDR on level 1 and the firmware's
TCI server on level 2 (0014), is in `spec/part-c-tci-profile.md`, with its choices in 0021. Base: [TCI] Transceiver Control Interface,
protocol 2.1 (`sources.md`, read 2026-11-19 in part). Level 2 is built only after gate G6
(0011 point 4). Level 2 sockets share the CH394Q with the management socket, which stays open
(0008, Socket budget; A1.2).

---

## Annex M — Management payloads

This annex fixes the payload of every command of Table A1-2 (A1.4; 0039). It adopts the
layouts the reference model used for its first tests (`product/refmodel/docs/mgmt-payloads.md`,
testbench finding SF-08) and adds the update, configuration and calibration commands. All
values are big-endian. TLV = tag (1 byte), length (1 byte), value; a reader MUST skip a TLV
with an unknown tag, so that tags can be added. A field marked "u8", "u16", "u32" or "u64" is
an unsigned integer of that size. A reply payload is given for status 0; a reply with another
status has an empty payload unless stated.

**Authentication** (A1.10). The challenge is 16 bytes from the hardware random-number
generator ([HIS] HIS 10). The trailer of an authenticated request is
HMAC-SHA256(secret, challenge ‖ bytes 0 … 17 + *N*) [RFC2104], [FIPS180-4]: the challenge
first, then the header and the payload. The secret is 32 bytes.

Table M-1 — Payloads of the management commands

| Command | Request payload | Reply payload (status 0) |
|---|---|---|
| `IDENTIFY` 0x0001 | empty | TLV list of Table M-2 |
| `GET_STATS` 0x0002 | empty | repeated: name length (u8), name (ASCII), value (u64) |
| `GET_CHALLENGE` 0x0010 | empty | challenge, 16 bytes |
| `SET_FIRST_SECRET` 0x0011 | secret, 32 bytes | empty |
| `CHANGE_SECRET` 0x0012 | new secret XOR HMAC-SHA256(old secret, "G7NS" ‖ challenge), 32 bytes; the challenge is the one this request is authenticated with | empty |
| `END_WRITE_SESSION` 0x0013 | empty | empty |
| `GET_CONFIG` 0x0020 | empty | TLV list of Table M-3, every item |
| `SET_CONFIG` 0x0021 | TLV list of Table M-3, the items to change; validated as a whole (A1.6 C-4) | TLV list of Table M-3, every item, after the change |
| `FACTORY_RESET` 0x0022, `RESTART` 0x0024 | empty | empty |
| `SWITCH_LEVEL` 0x0023 | target level (u8: 1 or 2) | empty |
| `READ_LOG` 0x0030 | first sequence number wanted (u32); 0 = oldest | repeated, as many as fit in 1400 bytes: sequence (u32), uptime ms (u32), text length (u8), text (ASCII) |
| `CLEAR_LOG` 0x0031 | empty | empty |
| `UPDATE_BEGIN` 0x0040 | flags (u8: bit 0 foreign image, bit 1 downgrade), then the package header of Table M-4 | block size (u16, 512), block count (u32), one-way (u8: 1 for a foreign image, A1.8a item 3) |
| `UPDATE_BLOCK` 0x0041 | block number (u32, from 0), CRC-32 [IEEE802.3] of the data (u32), data (512 bytes; the last block may be shorter) | block number (u32), blocks received so far (u32) |
| `UPDATE_VERIFY` 0x0042 | empty | SHA-256 computed over the received or written image (32 bytes) |
| `UPDATE_ACTIVATE` 0x0043 | flags (u8: bit 0 foreign image) | permanent (u8: 1 for a foreign image, no trial boot, A1.8a item 5) |
| `UPDATE_ABORT` 0x0044 | empty | restart needed (u8: 1) |
| `UPDATE_STATUS` 0x0045 | empty | state (u8: 0 idle, 1 receiving, 2 verified, 3 installer before the first erase, 4 installer writing), blocks received (u32), block count (u32), sectors written (u8) |
| `ROLLBACK` 0x0046 | empty | empty |
| `RETURN_TO_STOCK` 0x0050, `ENTER_VENDOR_RECOVERY` 0x0051 | empty | none: always status 56 (A1.8a items 2, 4) |
| `GET_CALIBRATION` 0x0060 | empty | calibration set id (SHA-256 of the set, 32 bytes, zeros if none), date (u32, seconds since 1970-01-01 UTC), source length (u8), source (ASCII), then the entries of Table M-5 |
| `SET_CALIBRATION` 0x0061 | date (u32), source length (u8), source (ASCII), then the entries of Table M-5; replaces the stored set as a whole (A1.12) | calibration set id (32 bytes), TX calibrated (u8) |
| `CLEAR_CALIBRATION` 0x0062 | empty | empty |

Table M-2 — `IDENTIFY` TLVs (A1.5 items I-1 to I-6)

| Tag | Length | Content |
|---|---|---|
| 1 | var | model, ASCII "G7250" (I-1) |
| 2 | 6 | MAC address (I-1) |
| 3 | var | serial number, only if the hardware has one (I-1; HIS-15) |
| 4 | 4 | running image version: major (u8), minor (u8), build (u16) (I-2) |
| 5 | 4 | other slot version, 0 = empty (I-2) |
| 6 | 4 | boot stage version (I-2) |
| 7 | 4 | hardware revision, 0 = not identified (I-2; HIS-18) |
| 8 | 1 | bit 0 stock image retained, bit 1 vendor recovery supported; both 0 in this version (I-2a) |
| 9 | 2 | integrity of the running image and of the other slot: 0 none, 1 good, 2 bad (I-3) |
| 10 | 1 | update state: 0 normal, 1 trial boot, 2 rolled back, 3 installer (I-3) |
| 11 | 1 | secret set (I-3a) |
| 12 | 3 | first-secret window open (u8), seconds left (u16) (I-3a) |
| 13 | 1 | active level (I-4) |
| 14 | 6 | owner IP address and UDP port, zeros if none (I-4) |
| 15 | 4 | `TX_ARM_STATE` word (I-4) |
| 16 | 4 | uptime in ms (I-5) |
| 17 | 1 | last reset cause, 0 unknown (I-5; P-SYS-2) |
| 18 | 5 | versions: management major, minor; P2 (44); record major, minor (I-5) |
| 19 | 1 | controller state: 0 unknown, 1 not detected, 2 detected (I-6) |
| 20 | 1 | TX calibrated: record `tx` bit 4 (I-7; A1.12) |
| 21 | 36 | calibration set id (32 bytes, zeros if none) and date (u32) (I-7; A1.12) |
| 22 | 1 | sectors written by the streaming installer (I-3; A1.8a item 3) |
| 23 | 1 | running slot: 1 slot A, 2 slot B (I-2; A1.8; 0043) |
| 24 | 1 | link-local address in use without a complete RFC 3927 probe (u8: 0 no, 1 yes) (A1.11; 0057) |

Table M-3 — Configuration items (A1.6)

| Tag | Length | Item |
|---|---|---|
| 1 | 1 | C-1 address mode: 0 DHCP (with link-local fallback, A1.11), 1 static |
| 2 | 4 | C-1 static address |
| 3 | 4 | C-1 netmask |
| 4 | 4 | C-1 gateway |
| 5 | 1 | C-2 default level at power-on (1 or 2) |
| 6 | 1 | C-6 region (0 in this version, Table A3-0) |
| 7 | 4 | C-3 maximum continuous TX time, ms |
| 8 | 3 | C-3 power limit of one band: `tx_band` index (u8), limit in 0.1 W (u16); one TLV per band |
| 9 | 2 | C-3 SWR limit S2 × 100 |
| 10 | 2 | C-3 PA temperature limit T3, 0.1 °C |
| 11 | var | C-7 TCI allow-list: 0 to 8 entries of address (4 bytes) and prefix length (u8) |

A `SET_CONFIG` that writes tag 9 or 10 before the calibration set holds S2 or T3 (P-PWR-2)
is refused with status 62 (A1.12; 0039).

Table M-4 — Package header (A1.8), the payload of `UPDATE_BEGIN` after the flags byte

| Bytes | Field |
|---|---|
| 0–3 | magic 0x4737504B ("G7PK") |
| 4 | header version, 1 |
| 5 | component: 1 main-unit image (the only one in this version, 0028) |
| 6–7 | target hardware id (0x00C8 for the G7250) |
| 8–9 | lowest hardware revision supported, 0 = any |
| 10–13 | image version: major (u8), minor (u8), build (u16); 0 for a foreign image |
| 14–17 | image length in bytes |
| 18–49 | SHA-256 of the image (for a foreign image: the value the owner supplies) |
| 50–51 | signature length, 0 in this version (A1.8, U-4) |
| 52 | target slot: 0 either slot (or a foreign image), 1 slot A, 2 slot B (A1.8, slot-linked images; 0043) |

Table M-5 — Calibration entries (A1.12)

| Field | Content |
|---|---|
| id length (u8), id (ASCII) | a parameter id of Table P-1, for example "P-RF-2" |
| value length (u16), value | the value; its encoding is fixed in Table P-1 when the parameter is set from measurements (Annex P) |

## Annex P — Pending hardware parameters

This annex lists every value that only the real radio can settle (0035; [HIS] HIS 90,
`9d10e72`). The normative text names each value by its id ("pending hardware (P-RF-2)"). A
value marked "pending G6" waits for the level-2 feasibility gate (0011). Until a value is set
from measurements, the safe placeholder in the last column applies. The measured values of a
unit are written as its calibration set (A1.12); fixing the encoding or a default of a value
is a spec change, made in this table.

**Freeze rule** (0035). Transmit stays disabled until every parameter of groups P-RF, P-PWR,
P-SEN and P-TIM, and P-RX-2, P-TX-1, P-ATU-1, P-ANT-1 and P-SYS-9, is set (the configuration frame of P-RX-2
may hold a transmit enable, [HIS] HIS 80): the RF path in transmit is only known
once the ATU and antenna latches have known safe values ([HIS] HIS 85). Until then record `tx`
bit 4 is 0, and every arm is rejected with `NOT_CALIBRATED` (Table B8-3; B10.2 item 3; part C
C5.5.6 item 7). The ATU latches stay unwritten and `ATU_STATE` reads 5 (unknown) until P-ATU-1
is set (A3.9b; 0032); the antenna is not selectable until P-ANT-1 is set. The way back to stock
over the network is closed until P-SYS-5 is set (A1.8a item 3, "Gate").

Discovery, the register map, the management channel and level-1 sessions are fully specified
without these values. Reception is specified too, but DDC0 is not offered until P-RX-2 (the
FPGA configuration frame) is set (B6.1; 0042), and once it is, the receive front end depends on
P-RF-1 and P-RF-5: until those are set, reception may be degraded or, if no chain B frame is
known, unavailable; a host sees this as a weak or silent DDC0 stream, not as an error (B6).

Table P-1 — Pending hardware parameters

| Id | Parameter | Used in | HIS | Safe placeholder until set |
|----|-----------|---------|-----|----------------------------|
| P-RF-1 | Filter plan: which TX frequencies share one TX low-pass filter and relay state; which RX band-pass segment and preamplifier setting each receive frequency needs (SPI1 chains A and B, [HIS] HIS 85) | A3.1b, A3.9, part C C7.6 | HIS-07 | each `tx_band` entry is its own RF-path state; RX filters chosen per band from the chain B segments only once mapped; until then RX uses the widest segment the mapping allows, or none if unknown |
| P-RF-2 | Relay settle time `t_settle`; whether a relay may be commanded again before it has settled | A3.9a items 2, 3a; B10.5; Annex X1 D27 | HIS-07 | not applicable while TX is disabled |
| P-RF-3 | RF-gone guard: forward-power threshold `P_rf_off` or guard time `t_rf_off`, with `t_rf_off` + `t_settle` ≤ 98.75 ms | A1.9, A3.9a items 1, 1a, 3; B10.5 | HIS-07, HIS-12 | not applicable while TX is disabled; a commanded reset waits the full `t_reset_wait` = 2 s if RF could be present |
| P-RF-4 | RX step attenuator and preamplifier steps (chain B bits), attenuation during TX | A3.6, B7, part C C5.9 | HIS-07 | preamplifier off, attenuator at its first known position or unwritten; `G7250_ATT` absent |
| P-RF-5 | RX default state: filters for the default receive frequency, preamplifier, attenuation | A3.6, B5.5 | HIS-07 | chain B output disabled (OE high, [HIS] HIS 85) until a known frame exists; then the P-RF-4 placeholder |
| P-ANT-1 | Antenna selection (most likely PD13 latch bits 4–5, [HIS] HIS 85) and the default antenna after reset | A3.6, B8 `ANT_STATE`, part C C5.8 | HIS-06 | latch unwritten, its strobe PD13 held inactive (high) and never pulsed ([HIS] HIS 85); `ANT_STATE` = 0 (unknown); `G7250_ANT` reports 0 and a set is answered with `READ_ONLY` |
| P-ATU-1 | ATU latch meanings and safe values; tune power of the ATU profile; tune-time margin; where tuned states are stored | A3.4 rule 5, A3.9b, Table A3-2, B10.9, part C C5.7 | HIS-08, HIS-12 | latches unwritten, strobes PD13, PG2, PG5 held inactive (high) and never pulsed ([HIS] HIS 85); state unknown (`ATU_STATE` 5); `ATU_TUNE` rejected with `NOT_CALIBRATED` (0032) |
| P-PWR-1 | Mapping of P2 drive level 0–255 and TCI `DRIVE` 0–100 to forward power (DAC channel 1, [HIS] HIS 85) | A3.4 rule 4, B10.8, part C C4 | HIS-12 | not applicable while TX is disabled |
| P-PWR-2 | Limit values: SWR S1, S2, `P_min`; temperatures T1–T4 and their hysteresis; under-voltage `REDUCE` and `UNKEY` values; fold-back caps `P_swr`, `P_temp`; `P_ssb_mean`; maximum power at 50 MHz | Table A3-2, A3.4a, A3.4c, A3.5, B4.8 `tx_band` | HIS-11, HIS-12 | not applicable while TX is disabled |
| P-PWR-3 | Margin on the forward-power bound for the digital crest-factor PEP estimate | Table A3-2, A3.4c | HIS-11, HIS-12 | not applicable while TX is disabled |
| P-PWR-4 | Whether the PA tolerates the reaction times of Table A3-2 or needs faster protection in hardware | Table A3-2 | HIS-12 | not applicable while TX is disabled |
| P-PWR-5 | Safe transmit bias per unit (DAC channels 2, 7, 8, [HIS] HIS 85) | A3.6 (firmware) | HIS-12 | bias channels at 0 (HIS 85) |
| P-SEN-1 | Detector laws and calibration of forward and reflected power; PEP or average detector; coupler position (record field `coupler`) | A3.4c, B4.9, B9 | HIS-08, HIS-11 | `coupler` = 0 (not reported); forward and reflected power reported as 0 with `SENSOR_VALID` clear |
| P-SEN-2 | Sensor read rates and response times; detection time of a missing, stuck or out-of-range sensor | Table A3-2 | HIS-11 | sensors read, but not used for limits while TX is disabled |
| P-SEN-3 | "RF while not keyed": threshold (above the carrier leakage with all-zero DUC samples, at or below `P_rf_off`), blanking time `t_blank`, shortest CW element gap where the check runs | Table A3-2, A3.9a item 6, part C C8 | HIS-11, HIS-12 | the check runs with the forward-power sensor once P-SEN-1 is set; until then it is not available, which is one more reason TX stays disabled |
| P-SEN-4 | Time constant of the PA temperature filter | Table A3-2, A3.4a | HIS-11, HIS-12 | not applicable while TX is disabled |
| P-SEN-5 | Calibration of the receive signal level in dBm | part C C4 `RX_SENSORS` | HIS-11 | level reported uncalibrated, relative to full scale |
| P-TIM-1 | Delay from the DUC input to the coupler | A3.4c, A3.9a item 6 | HIS-18 | not applicable while TX is disabled |
| P-TIM-2 | DUC FIFO capacity while keyed; exact TX underrun time (within the bounds of B10.5: at least the key latency + 1.25 ms, at most `KEYED_TIMEOUT`) | B10.5, B10.7, B11.7, Annex X1 D8, annex AetherSDR | HIS-18 | not applicable while TX is disabled: part of the freeze rule (group P-TIM), and DUC I&Q is discarded until `tx` bit 4 = 1 (B10.7), so no FIFO capacity is used before it is set |
| P-NET-1 | Highest SPI clock to the CH394 that works on the radio (datasheet maximum 62 MHz in SPI mode 0 or 3 [CH394], manual V1.2, features list, VERIFIED in [HIS] HIS 40, `6e2208a`; about 17 MHz seen in use on the board, MEDIUM, the safe bound until the board link is measured) | B6, 0004 | HIS-14 | DDC0 rates 48, 96 and 192 ksps; 384 ksps offered only once ≥ 50 MHz is shown |
| P-NET-2 | Whether the CH394 MAC address is unique per unit ([HIS] HIS 40) | A1.5, B3 | HIS-15 | the CH394 value is used as it is |
| P-NET-3 | Whether the CH394 reports the destination address (unicast, directed or limited broadcast) of a received UDP datagram ([HIS] HIS 40, HIS 90) | B3.9 (Set IP) | HIS-14 | treat every Set IP as unicast or directed broadcast: own subnet only, any source while link-local (B3.9) |
| P-SYS-1 | Pin states between reset and the boot stage's GPIO init (board pull resistors on the RF control pins of [HIS] HIS 85, and what the vendor bootloader leaves); whether they hold the safe levels in that window | A3.6, A3.5 | HIS-13 | outside this spec before the boot stage runs; the boot stage sets the safe state within 20 µs of its first instruction (A3.6; 0045) |
| P-SYS-2 | Reset cause (power-on, software, watchdog), and whether the log survives a warm reset | A1.7, Table B8-3 `WDT_RESET_AT_START`, Annex M tag 17 | HIS-13, HIS-18 | reset cause reported as 0 (unknown); the log is lost at every reset |
| P-SYS-3 | Power-up path without the controller (HIS 20 H1/H2) | part A scope (0029) | HIS-17 | none; observation T0-P2 |
| P-SYS-4 | Behaviour of the stock controller's panel with a silent main unit | part A scope (0023) | HIS-17 | CTRL idle ([HIS] HIS 70) |
| P-SYS-5 | Drill F1: the vendor bootloader starts an image of this firmware, and the vendor path installs the stock firmware into an empty application area; whether the vendor bootloader uses an application sector as staging ([HIS] HIS 15 rule 5) | A1.8 (slot layout), A1.8a items 3, 9 (0028) | HIS-19 | no first install before F1; foreign `UPDATE_BEGIN` gets status 56 |
| P-SYS-6 | Power draw in idle, for a later standby (0029) | part A scope | — | quiet idle only |
| P-SYS-7 | Start mode of the independent watchdog (IWDG1_SW option byte, [HIS] HIS 10, HIS 90) | A1.8a item 3 (installer), firmware | HIS-13 | the firmware and the installer service the watchdog whenever it may run |
| P-SYS-8 | SEND and TUNER interfaces on the main unit: whether and how the firmware drives or reads them | A3.6, A3.7, B4.8 `atu` bit 4, `regmap_groups` bit 7 | HIS-09 | released (inactive) and not offered |
| P-SYS-9 | Whether AXI SRAM content kept by the firmware survives a reset without power loss, through the vendor bootloader that runs before the boot stage (fault entry counters, reset counters, `TRIP_COUNT`), [HIS] HIS 10, HIS 90 (`72591aa`: a system reset does not clear SRAM and AXI SRAM has a 64-bit ECC word, both MEDIUM; whether the stage before 0x08020000 writes the firmware's region 0x2401FFE0–0x2401FFFF is not known). Radio check: write a pattern, then a software reset (`RESTART`) and a watchdog reset, and read it back in the boot stage | A3.5 "Counters", Table A1-4, `TRIP_COUNT` | HIS-13, HIS-18 | part of the freeze rule: TX stays locked; the counters are kept as far as the hardware keeps them, and nothing relies on them |
| P-RX-1 | Physical receive paths behind the two receive tuning words ([HIS] HIS 80) | B2, B3, 0004 | HIS-02 | one DDC |
| P-RX-2 | FPGA configuration frame (command 0x01, [HIS] HIS 80): meaning and safe values of the status byte 0 and of bytes 9–18 (per-rate calibration among them), position of the spectral-inversion flag; which tuning word (offset 1 or 5) is DDC0; I/Q order of the 0x04 samples; bus set-up, strobe width and turnaround | B6.1, B6.7, B4.8 `streams` | HIS-01, HIS-02 | DDC0 not offered (`streams` bit 0 = 0, `STREAM_STATUS` reason 2); no configuration frame written outside the A3.6 safe state |
| P-TX-1 | The third tuning word as the transmit word ([HIS] HIS 80, HYPOTHESIS) | B10 (firmware) | HIS-02 | not applicable while TX is disabled |
| P-L2-1 | Level 2: receiver bandwidth for `IF_LIMITS`, TCI `KEYER` jitter budget | part C C3, C4 | HIS-01, G6 | level 2 not offered before G6 |
| P-L2-2 | Level 2: stream budget and socket buffer split | part C C6 | HIS-14, HIS-18, G6 | level 2 not offered before G6 |
| P-L2-3 | Level 2: TX chain values of the on-board DSP | part C C8 | G6 | level 2 not offered before G6 |

**Tests.** Every test of Annex X3 and of part C runs against the reference model, with
simulated values for the parameters of Table P-1 (0019). Against the radio, a test that keys
the transmitter, tunes the ATU or selects an antenna runs only once the parameters it uses are
set; these are the keying tests of A3, B10 and part C C5.5–C5.8, C7 and C8. Until then those
tests check, against the radio, only that the arm is rejected with `NOT_CALIBRATED`, the ATU
tune with `NOT_CALIBRATED` and an antenna set with `READ_ONLY` (0035).

## Annex X1 — Differences from standard P2

Every deviation of G7250-P2 from [P2] (B1.5). Decisions 0003–0008 and 0017 are accepted; 0020
and 0022 are proposed for ON8ST review at rc1. Only hardware-dependent values (with their HIS id) remain
open in the sections concerned.

| Id | Deviation | [P2] behaviour | G7250 behaviour | Decision | Spec section |
|----|-----------|----------------|-----------------|----------|--------------|
| D1 | No DDC audio stream | Host may send DDC audio to port 1028 ([P2] DDC Audio packet; General bytes 13–14) | No socket on port 1028; General bytes 13–14 ignored; record `streams` bit 2 = 0 | 0017 | B2.3, B4 |
| D2 | Memory-mapped register port defined | Default port "xxxx", undefined ([P2] General bytes 29–32) | Default port 7250, one port for both directions; destination default (bytes 31–32 = 0) is the owner's source port; carries the G7250 register map | 0005, 0003, 0020 | B2.3, B8.1 |
| D3 | TX needs an explicit token arm | High Priority byte 4 bit [1] enables transmit ([P2]) | No TX request (PTT, CWX, KEY, ATU) keys unless armed by token handshake; keying only on an edge after confirmation; arm dropped by the events of Table A3-1 | 0015, 0007 | B10.1, B10.2, A3.2 |
| D4 | Hardware timer forced on while armed | Host may clear General byte 38 bit [0] ([P2]) | Clear honoured only while not armed; arming requires the bit set | 0015, 0007 | B5.6 |
| D5 | Takeover drops the arm | Second host's C&C takes over ([P2] Discovery reply byte 4 NOTE) | Takeover always allowed, but unkeys and drops the arm first; the old owner gets the `TAKEOVER` notice, also while unarmed and also a standard host, which is expected to ignore packets from port 7250 by source-port demultiplexing (a host SHOULD do so; 0022 §88 item 6); new owner starts unarmed, with mode class, session limits and the TX settings of PA enable and the DUC-specific packet reset; RUN and RX settings kept | 0007, 0022 §3, §64, §79, §87 item 2 | B5.7, A2.4, A3.6a |
| D6 | Owner is IP address and UDP port | "different Host" not defined ([P2]) | A C&C packet from the owner's IP on another source port is a takeover; host must use one source port per session | 0007 | B5.5, A2.2; X2 C7 |
| D7 | Keyed timeout | Only the 1 s watchdog ([P2] "Detailed operation") | While keyed, High Priority at least every 100 ms; none within `KEYED_TIMEOUT` (500 ms) unkeys, arm kept | 0007 | B10.5 |
| D8 | TX IQ underrun unkeys | No such rule ([P2]) | DUC underflow longer than the underrun time (at least key latency + 1.25 ms, at most `KEYED_TIMEOUT`; exact value pending hardware, P-TIM-2; HIS-18) unkeys whenever the TX source is PTT, in every mode class, except a PTT transmission whose RF the keyer forms with CW selected and break-in clear; the underflow is timed from the key if the FIFO is empty then | 0007, 0022 §25, §57, §85 | B10.5, B10.5b |
| D9 | TX mode class required | No such field ([P2]) | Arm refused without `TX_MODE_CLASS` | 0007 | B10.3 |
| D10 | Safety core overrides PTT | Bit [1] enables transmit ([P2]) | Limits may reduce drive or unkey while PTT is set; 5 min maximum continuous TX; no key outside the TX ranges; keying only on a 0→1 edge when every condition holds, refused requests not latched; a retune while keyed that needs an RF-path change (ATU memory recall included), a `TX_MODE_CLASS` change or PA enable cleared while keyed, unkeys; RF-path fields are deferred while keyed and relays move only after RF is gone; `ATU_TUNE` while keyed is refused; at a key the RF path is first switched to the state for the TX frequency (A3.1b); without a PEP detector, power is bounded on a PEP estimate from the crest factor of the DUC samples | 0007, 0011, 0022 §1, §2, §14, §22–§24, §26, §28, §33, §62 | A3.1a, A3.1b, A3.4, A3.4c, A3.9–A3.9c, B10.4, B10.10 |
| D11 | Memory-mapped packets are not C&C | "any C&C packet" feeds the watchdog; not stated for memory-mapped packets ([P2]) | No memory-mapped packet feeds the watchdog or takes over; a `CAP_REQUEST`-only packet does not start a session; its answer goes to the request's source when the host is not the owner | 0003, 0007 | B4.4, B4.5, B5.6; X2 C6 |
| D12 | Erase and Program ignored | Erase/Program reprogram the EEPROM; Command Reply ([P2]) | Ignored without reply; V3.8 checksum not claimed; update only via the management channel | 0008 | B3.9, A1.8 |
| D13 | Unsupported requests refused | No negative acknowledgement; radio follows the host ([P2]) | Unoffered DDC rate or size, format, time stamp, VITA-49, VNA mode, wideband parameters, DUC rate/bits or EER: stream does not run, reason in `STREAM_STATUS`; never clamped | 0004, 0020 | B6.3, B6.6, B6.7, B6.10, B7, B10.6, B11.2 |
| D14 | Wideband parameters | Defaults 512 / 16 bit / 20 ms / 32 packets; zero selects the default only for byte 26; byte 27 range 0–255 ms ([P2] General bytes 24–28) | From M2, fixed 512 / 16 bit / 50 ms / 8 packets; zero in bytes 24–25, 27, 28 means these values; other values refused | 0004, 0022 §21 | B6.10 |
| D15 | Session refused below 100 Mbit/s | No link-speed rule ([P2]) | Discovery answered, no stream started, reason in `LINK_STATUS` | 0004 | B5.8 |
| D16 | PTT1–PTT3 never key | Table lists PTT1–PTT3 in byte 4 bits [2]–[4] ([P2]) | Bits [2]–[7] ignored | 0007 | B5.9; X2 C5 |
| D17 | TUNE bit does not tune | Client control bit [0] TUNE for accessory hardware ([P2]) | Information only; internal ATU via `ATU_TUNE` | 0007, 0005 | B7, B10.9 |
| D18 | Open-collector outputs ignored | High Priority byte 1401 drives outputs ([P2]) | Ignored (no such outputs) | 0007 | B7 |
| D19 | Power fields filled without Alex | Status bytes 14–15, 22–23 "Set to zero if Alex not selected" ([P2]) | Always filled from the PA coupler | 0006 | B9.2 |
| D20 | Conversion constants per unit | Constants per board type in Appendix A ([P2]) | Constants of this unit (per band where needed) from the capability record; words stay raw ADC values | 0006 | B9.3 |
| D21 | No port relocation onto the management port | Host may move ports with General bytes 5–32 ([P2]) | A P2 port is not moved onto the management port; default kept, conflict logged | 0008 | B2.3, A1.3 |
| D22 | Non-default radio-side ports not honoured | Host may move every radio-side port with General bytes 5–22, 29–30 ([P2]) | Zero or the default value honoured; any other value: default kept, logged, `LINK_STATUS` bit 17; bytes 31–32 (host side) honoured | 0020 §13 | B2.3, B7, B11.12 |
| D23 | No P2 session while level 2 is active | C&C from another host takes over ([P2] Discovery reply byte 4 NOTE) | While level 2 is active, P2 C&C starts no session and no takeover; discovery answers 0x03; level changes only over management | 0021 §16, 0020 §25 | A2.5 |
| D24 | DUC I&Q only from the owner | No source rule for DUC I&Q ([P2] DUC I&Q Data packet) | DUC I&Q from any source other than the owner's IP address and port is discarded and counted; the host sends it from its session socket | 0022 §4 | B10.7, B11.7 |
| D25 | Answer rate limits | Every discovery is answered ([P2] Discovery packet) | Discovery, `CAP_REQUEST`, `NOT_OWNER` and management answers are rate-limited per source, with total caps; excess requests dropped and counted; no answer to a discovery from source port 1024 or to a non-owner's register packet from source port 7250 | 0022 §16, §71, §73 item 20 | B11.14, B3.2, B8.5 |
| D26 | CWX key-down capped at 1 s | No such rule ([P2] High Priority byte 5) | With CWX selected (byte 5 bit [0]), a dot or dash bit (bits [1], [2]) held continuously for more than 1 s unkeys, arm kept; a new 0→1 edge is needed | 0022 §25, §36 | B10.5a |
| D27 | Key waits for relays to settle | Bit [1] enables transmit at once ([P2]) | A PTT or CWX 0→1 edge keys only after the relays have settled (`t_settle`, pending hardware P-RF-2; HIS-07): when it arrives with an RF-path change, while an earlier change still moves or settles, right after an unkey, or when the key itself needs the TX state of A3.1b; bounded by `t_rf_off` + `t_settle` | 0022 §23, §33, §37 | A3.1b, A3.9a |
| D28 | Stale High Priority packets discarded | The host's sequence number is 0x00000000; nothing on order ([P2] High Priority packet, "Sequence Number") | After three consecutive increasing non-zero host numbers, a High Priority packet not newer than the last accepted one is discarded and counted; five such packets in a row, or a jump back by more than 2¹⁶, resynchronise (logged; no 0→1 edge from that packet); a PTT or CWX bit at 0 in a discarded packet still unkeys; otherwise, and with numbers 0, a PTT or CWX edge counts only 10 ms or more after the latest packet with the bit at 1 | 0022 §35, §55, §56, §72 | B11.6a, B2.7 |
| D29 | P2 only from the radio's own subnet | No source rule ([P2]) | C&C, DUC I&Q and memory-mapped packets from outside the radio's subnet are discarded; no session or takeover from there; Set IP from outside the subnet only by limited broadcast | 0022 §49, §70 | B11.15, A2.3, B3.9 |
| D30 | PA disabled blocks all RF | General byte 58 bit [0] enables the PA ([P2] General packet byte 58); [P2] does not say that transmit stops without it | While bit [0] is 0 no TX request keys and there is no exciter-only output; clearing it while keyed unkeys, arm kept; cleared at every session end and takeover | 0020 §22, 0022 §14, §64, §73 item 15 | B7, B10.10, A3.1a, A3.6a |
| D31 | CW keyer elements under the key rule; hang delay counts as keyed | DUC-specific bytes 11–12 give the CW hang delay ([P2] DUC specific packet); [P2] says nothing on relays or settings during it | Every keyer element (CWX, KEY, TCI `KEYER` and CW text) is a key under every condition of A3.1a, and a failed condition stops the keyer until a new edge; during the hang delay the radio counts as keyed for RF-path changes: a TX RF-path, relay or mode class change or PA enable cleared in a gap ends the hang delay, and relays move only after RF gone, SEND release and the T/R return; the hang delay is clamped to 2000 ms and the keyed timeout ends it | 0022 §74, §83 item 1 | A3.1a, A3.4d, A3.9, A3.9a, B10.5, B10.5a, B10.6, B10.9 |
| D32 | Restart with the run bit keeps RX settings only | The session "can be re-started … by sending a High Priority Packet with the RUN bit set, if the same network session is still active" ([P2] "Detailed operation"); nothing on settings being reset | After a stop, watchdog standby or link loss, a High Priority packet with run = 1 from the last owner restarts the session with its RX-side settings; its TX settings (Table A3-4), mode class, keyed timeout and session limits are reset, and an arm needs a General packet of the new session or its set-up phase; after a reset, update, level or address change, and for another host, nothing carries over | 0022 §78 | A2.3, B5.4, A3.6, A3.6a |
| D33 | CWX with iambic clear is a straight key | High Priority byte 5 bit [0] CWX: "A set bit will send a dot or dash at the speed selected by the DUC Specific packet" ([P2] High Priority packet byte 5, p. 34) | With iambic clear (DUC-specific byte 5 bit [3]) a dot or dash bit is key-down for as long as it is held, with no element timing and no weight, under the 1 s cap; with iambic set, elements at the keyer speed. Known limit: bit [3] is shared by the hardware keyer from KEY and by CWX, so iambic KEY and straight-key CWX (or the reverse) cannot be used at the same time | 0022 §82, §86 item 9 | B10.5a |

Not deviations, but G7250 choices within [P2]: board type 200 (0002); short discovery reply
only, bytes 14–19 and 24–59 zero (0003); one DDC (0004; HIS-02); a contiguous subset of the
[P2] DDC rates (0004); byte 22 = 0x00, Big-Endian 3-byte only (0004); time stamps not offered,
bytes 4–11 zero (0004); DUC 192 ksps 24-bit as current [P2] practice (0004); microphone data
only if HIS-17 allows (0004); raw ADC words in the status fields (0006); P2 Set IP accepted as
[P2] specifies, with logging that is not visible on the wire (0008); the [P2] 1 s watchdog
(0007).

## Annex X2 — Clarifications of [P2]

| Id | [P2] text | G7250 reading |
|----|-----------|---------------|
| C1 | "Hardware to Host Protocol": "Byte order shall be MSB first (little Endian)" | MSB first (big-endian), consistent with "Host to Hardware Protocol" and with General byte 39 = 0 meaning Big-Endian (B2.5) |
| C2 | Discovery, Erase, Set IP, General, DDC-specific, DUC-specific and High Priority packets: sequence number "set to 0x00000000"; other packets: per-port counter, reset at "power on" (DDC, High Priority status, microphone, wideband packets) or "program start" (DDC audio, DUC I&Q, both memory-mapped packets); the High Priority status text says both "set to 0x00000000" and "rolls over after exceeding 0xFFFFFFFF" | G7250 sends per-port counters on all outgoing streams, the status packet included, reset at power-on and every reset, on stop, at a takeover and at a session start by another host, and continued across the other session ends (wideband: per frame, C16; 0022 §83 item 4); discovery reply always 0; incoming sequence numbers are not checked (B2.7; 0022 §21) |
| C3 | Discovery reply byte 4: "0xFE (0xFF if running)" means a hardware configuration follows, "Appendix A"; Appendix C instead uses byte 4 = 0x02/0x03 and byte 11 = 255 | The G7250 uses neither: byte 4 is only 0x02 or 0x03, never board type 254/255, no Appendix C reply (0003 §1; B3.3, B3.4) |
| C4 | General byte 37: DSP clock "as part of the Discovery response as specified in Appendix B"; Appendix B is "To be written"; Appendix C bytes 27–30 carry the DSP clock | For board type 200 the DSP clock of the phase-word formula is the fixed virtual clock 122 880 000 Hz, reported in record field `dsp_clock_hz` with the meaning of Appendix C bytes 27–30; the hardware clock may differ and is converted by the firmware (0003 §3; 0022 §81; B3.8) |
| C5 | High Priority packet table: byte 4 "[1] = PTT0…[4] = PTT3"; text: "Bit [1] enables transmit … Bits [2] to [7] are reserved for future use" | Inconsistent in [P2]. The G7250 has one transmitter [G7250-MAN]: bit [1] is the transmit bit; bits [2]–[7] are ignored under either reading and never key (0007 rule 4; Resolved 11; B5.9) |
| C6 | "Detailed operation": "any C&C packet" feeds the watchdog; the term is not listed in one place | C&C packets are the four packets headed "Control Elements - PC to Hardware": General, DDC-specific, DUC-specific, High Priority (0022 §21). Memory-mapped packets ("Memory Mapped") are not C&C (0007 sub-decision 2.1; 0003 §2; B5.6) |
| C7 | Discovery reply byte 4: 0x03 if "connected to a different Host"; takeover by "the PC sending the C&C commands" | A host is identified by source IP address and UDP port; the owner's IP on another port is a different host (0007, Resolved 4; B3.4, B5.7) |
| C8 | Discovery reply byte 4: 0x03 when "running and already connected to a different Host" | While level 2 is active the RF chain is in use by another controller; the G7250 answers 0x03 (0003, Resolved; B3.4) |
| C9 | General byte 39: 0 selects Big-Endian 3-byte; bits [0] Big-Endian and [2] 3-byte select options "if indicated in the Discovery reply" | Provisional (no decision): 0x00, and values with only bits [0] and/or [2] set, select the offered format; any other bit refuses (B6.3) |
| C10 | General bytes 29–30 "memory mapped data from the PC will be sent to", bytes 31–32 "to the PC will be sent to" | Bytes 29–30 name the G7250's receive port, bytes 31–32 the host's destination port; the G7250 sends from the same local port it receives on (0005, Consequences; B8.1) |
| C11 | Set IP Address packet: success "can be verified by sending a Discovery Command"; Command reply packet: "Following a successful Set IP command, the hardware will respond with a Discovery reply packet" | Both hold: the G7250 sends a Discovery reply after a Set IP, and answers later discoveries with the new address (0008 point 6; B3.9) |
| C12 | Streams go to "the Source Port of the Host that initiated the Discovery Packet" (status, microphone, wideband "Destination Port") or "… the Discovery Command" (DDC packet "Destination Port"); the General packet is sent from that same port (General packet "Source Port") | The destination is the owner's source IP address and port (A2.2), fixed at session start or takeover (0022 §3, §12); for a P2 host this is the discovery port. A discovery from another socket of the same host does not move the streams, and a run bit without a discovery starts the session normally (0020 §14; B2.4, B5.2) |
| C13 | Set IP Address packet: "Broadcast to Port 1024", destination 255.255.255.255 | Also accepted when sent to the unit's unicast or the subnet's directed broadcast address, but those only from a source inside the radio's own subnet; the limited broadcast is accepted from any source, as a router does not forward it; the MAC in bytes 5–10 identifies the unit (0020 §21; 0022 §70; B3.9) |
| C14 | High Priority Status bytes 39–40 headed "ADC0 max magnitude" with the note "Max ADC1 magnitude"; text: "for each of ADC0 and ADC1 … up to 32768" | Bytes 39–40 carry ADC0, 0–32767, bit 15 = 0; the "ADC1" note is read as ADC0 (0022 §21; B9.2) |
| C15 | Memory-mapped packets shown with 240 address/data pairs, no count field | A packet carries 1 to 240 pairs and is 4 + 6*n* bytes long; the length is the count (0020 §4; 0022 §21; B8.1) |
| C16 | Wideband Data packet "Sequence Number": "set to zero at power on or whenever a block of raw samples has been sent" and "incremented in each sequential packet until the number of samples set in the General packet has been sent" (pp. 52–53) | The wideband counter is 0–7 within each 8-packet frame and restarts at 0 for every frame, not a continuous counter (0022 §21; B2.7, B6.10) |
| C17 | High Priority packet: frequency, Alex words, step attenuator and PTT in one packet; no order of application stated within a packet | RF-path fields of the packet are applied first, relays settle, then the PTT or CWX edge keys (A3.9a item 3; 0022 §23) |
| C18 | "Detailed operation" and the packet introductions: General, DDC-specific and DUC-specific packets are sent before the High Priority packet that sets run | Packets received from a host while no session runs are kept per source (set-up phase) and count for the session that host starts with its run bit; the set-up of other sources is discarded at that moment (0022 §34; B5.1a) |
| C19 | High Priority packet byte 5 (p. 34): bit [0] CWX, which selects CW from the host; bits [1] and [2] dot and dash, sent at the keyer speed; DUC-specific byte 5 keyer bits (reverse, iambic, mode B, strict spacing) without saying whether they apply to CWX | Bits [1] and [2] are the paddle inputs of the radio's keyer (DUC-specific bytes 9–10); the TX request is asserted while bit [0] and bit [1] or [2] are set; iambic, mode B and strict spacing apply to CWX, reverse keys does not; with iambic clear a dot or dash bit is key-down for as long as it is held, under the 1 s cap (0022 §36, §82; B10.5a) |
| C20 | High Priority Status byte 30 bit [2] "DUC FIFO underflow"; nothing on what the DUC sends meanwhile or when an underflow begins | While the DUC FIFO is empty the DUC gets zero samples; an underflow begins at the key if the FIFO is empty then, else when it runs empty while keyed (0022 §57; B10.5); while unkeyed the FIFO keeps at most the latest 25 ms of the owner's I&Q (0022 §63; B10.7) |
| C21 | "Detailed operation": the session is restarted either by a High Priority packet with the RUN bit set "if the same network session is still active", or by a Discovery exchange followed by re-initiation of all C&C settings and a High Priority packet with RUN set; "same network session" is not defined | The same network session is still active for the last owner (same source IP address and UDP port) after a stop, a watchdog standby or a link loss, and not after a reset, a firmware update, a level change or a change of the radio's address; no 0→1 edge of the host's run bit is needed. What the G7250 resets at the restart is a deviation, Annex X1 D32 (0022 §78; A2.3, B5.4) |
| C22 | DUC-specific byte 5 bit [7] "Break_in" among the CW options; no meaning stated, and nothing on CW together with PTT ([P2] DUC specific packet byte 5; High Priority packet bytes 4, 5) | Break-in set: KEY and CWX key by themselves with the hang delay, and PTT and CW never share a transmission (the first source keeps it). Break-in clear: KEY and CWX form RF only inside the owner's PTT transmission; T/R relay and SEND follow the PTT; with CW selected the keyer alone forms the RF, DUC I&Q is not sent and the underrun check is waived, so a PTT transmission without elements is silent (host note, B10.5b item 2.3; 0022 §87 item 6); keyer RF starts only from the PTT key, the moment RF is enabled after the RF delay (0022 §87 item 1); without PTT they key nothing, not latched (0022 §85; B10.5b) |

## Annex X3 — Conformance tests

To be implemented in g7250-testbench, first against the reference model, then on the radio
(0001 item 5; design note §5 principle 6). Test ids are stable; descriptions are one line.
Unless a test says otherwise, a test in which KEY or CWX keys runs with break-in set
(DUC-specific byte 5 bit [7] = 1; B10.5b; 0022 §85).

### Management channel, ownership, safety (A1–A3)

| Id | Description |
|----|-------------|
| T-A1-01 | The management port answers identification at both levels, also to a broadcast request. |
| T-A1-02 | No management request keys the radio, arms it, loosens a limit or switches an RF-path relay; `RESTART`, `FACTORY_RESET`, `SWITCH_LEVEL` and a C-3 change while armed and keyed unkey and disarm in the order of A3.2 before acting; every other request leaves `TX_STATE` and `TX_ARM_STATE` unchanged. |
| T-A1-03 | A write without a valid HMAC response is refused and logged; a replayed exchange is refused. |
| T-A1-04 | The first-secret window opens only in factory state, closes after its time and after a secret is set. |
| T-A1-05 | An update is refused while armed or transmitting. |
| T-A1-06 | An older image is refused without the downgrade flag and accepted, logged, with it. |
| T-A1-07 | A block with a bad CRC-32 is rejected and requested again; an image for other hardware is refused before writing. |
| T-A1-08 | Power loss at each step of an update to an image of this firmware leaves a bootable image (reference model). The foreign image of A1.8a item 3 is excluded; it has its own test. |
| T-A1-09 | A trial boot without confirmation rolls back and logs it. |
| T-A1-10 | A bad network configuration or an address conflict falls back to the previous one. |
| T-A1-11 | A General packet that moves a P2 port onto the management port leaves the management socket in place and logs the conflict. |
| T-A1-12 | Requests from outside the subnet are not answered, except identification. |
| T-A1-13 | Wrong magic is dropped; a short or length-mismatched request gets status 17; an unknown major version gets status 18; an unknown command gets status 16. |
| T-A1-14 | A repeated request (same request id, command and payload) gets the stored reply and is not executed twice; a request with the same id but another command or payload is executed and gets its own reply (A1.4). |
| T-A1-15 | A second host gets status 35 for writes while a write session is held; the session ends after 60 s idle; `GET_STATS`, `IDENTIFY`, `GET_CHALLENGE`, other unauthenticated requests and a request with a wrong HMAC from the holder every 30 s do not keep it open, an authenticated, verified request from the holder does (A1.10). |
| T-A1-16 | The first-secret window closes 300 s after network up; a restart inside the window reopens it for 300 s. |
| T-A1-17 | A region change (C-6) is refused with status 57 while a P2 session runs; the factory default is region 0, the ranges of Table A3-0 (0039). |
| T-A1-18 | With a non-empty TCI allow-list, a TCI connection from an address outside it is refused and logged; the management port stays reachable. |
| T-A1-19 | A request with the right magic and 12–17 bytes gets status 17; one below 12 bytes gets no reply. |
| T-A1-20 | Two hosts (same IP, different ports) each get their own challenge; a request authenticated with the other host's challenge fails with status 33; a challenge works once. |
| T-A1-21 | A repeated request id within 5 s gets the cached reply; after 5 s a replayed authenticated request fails with status 33. |
| T-A1-22 | `CHANGE_SECRET` carries the masked new secret; the captured datagram does not contain the new secret; the new secret works afterwards. |
| T-A1-23 | After `UPDATE_BEGIN` the session ends and RX stops; a run request gets `STREAM_STATUS` reason 10 at most every 100 ms; register writes get `BUSY`; after `UPDATE_ABORT` the state lasts until `RESTART`. |
| T-A1-24 | During an unconfirmed trial boot RX works and an arm is rejected with `MAINTENANCE`; after confirmation it is accepted. |
| T-A1-25 | A region change is refused with status 50 while armed or keyed on either level and with status 57 while a P2 session runs. |
| T-A1-26 | More than 20 management requests per second from one source are answered at most 20 times per second; the excess is counted in `GET_STATS`. |
| T-A1-27 | `ROLLBACK` while armed, and while keyed, is refused with status 50 and leaves `TX_STATE` and `TX_ARM_STATE` unchanged; unarmed it is accepted. |
| T-A1-28 | A C-3 write that loosens a tightened persistent limit back toward the built-in value is accepted with status 0 and logged with old value, new value and source; a value looser than the built-in value gets status 49; no command changes a built-in value; `FACTORY_RESET` leaves the persistent limits unchanged. |
| T-A1-29 | `RESTART` and `FACTORY_RESET` while keyed at full power: the unkey comes first, and no relay moves and the reset does not start until forward power is below `P_rf_off` or `t_rf_off` has passed (reference model; A1.9). |
| T-A1-30 | A `SET_CONFIG` that changes the radio's address while armed and keyed unkeys and drops the arm with `SESSION_CHANGED` before the reply, and applies the address only after RF is gone; the session ends; a C-3 change while armed and keyed unkeys and drops the arm with `MAINTENANCE`, applies the limit and then replies (A1.6, A1.9; 0022 §73 item 1). |
| T-A1-31 | `SWITCH_LEVEL` during a firmware update gets status 58. |
| T-A1-32 | A region change while a P2 session runs and TX is armed gets status 57; on level 2 while armed it gets status 50 (after G6). |
| T-A1-33 | `UPDATE_STATUS` while armed and keyed is answered with status 0 and changes nothing; every other update command gets status 50. |
| T-A1-34 | After `UPDATE_BEGIN` and 10 min without an update command, the update is aborted and logged and the radio restarts into the running image; RX works afterwards (reference model). |
| T-A1-35 | While armed and keyed, a DHCP renewal with another address, a lease expiry, a DHCPNAK and an address conflict each unkey, drop the arm with `SESSION_CHANGED` and end the session, and the radio uses the new address (or stops using the old one) only after forward power is below `P_rf_off` or `t_rf_off` has passed; a renewal with the same address and netmask changes nothing (reference model; A1.11a). |
| T-A1-36 | With the reference model holding forward power above `P_rf_off` after the unkey, `RESTART` while keyed unkeys and disarms, does not reset, logs the event, enters the fault state with `RF_NOT_KEYED` and replies with status 59 after `t_reset_wait` (2 s); `UPDATE_ACTIVATE` and `ROLLBACK` in the same state get 59 as well (A1.9). |
| T-A1-37 | After `UPDATE_BEGIN` and 10 min without an update command, with forward power held above `P_rf_off`, the radio does not restart, logs it and stays in the update state and in fault (reference model; A1.8, A1.9). |
| T-A1-38 | `RESTART`, `FACTORY_RESET` and `SWITCH_LEVEL` while keyed: the `TX_STATE` push showing unkeyed and the `TX_ARM_STATE` push showing the drop leave the radio before the management reply; a retransmission of `RESTART` with the same request id during the wait for RF gone is answered with status 1 and does not start a second reset (A1.9, A2.5a). |
| T-A1-39 | After `FACTORY_RESET` with every item of Table A1-4 changed beforehand: C-1, C-2, C-5, C-6, C-7 and the TCI port are at their factory defaults, the first-secret window is open, and the C-3 limits, the log, the fault and reset counters, `TRIP_COUNT` and the firmware images are unchanged (A1.6a). |
| T-A1-40 | An arm write and `UPDATE_BEGIN` sent so that they reach the radio within 1 ms of each other, repeated 1000 times: each time either the arm is confirmed and `UPDATE_BEGIN` gets status 50, or `UPDATE_BEGIN` succeeds and the arm is rejected with `MAINTENANCE`; never both (reference model; A1.8). |
| T-A1-41 | With forward power held above `P_rf_off` (reference model), `FACTORY_RESET` while keyed gets status 59 and changes nothing of Table A1-4: the secret still works, no first-secret window opens, and C-1, C-2, C-6, C-7 and the TCI port are unchanged; the attempt is logged; without RF it restores Table A1-4 only with the reset (A1.6a). |
| T-A1-42 | Identification requests from 200 sources inside the subnet at 10 per second each get at most 50 replies per second in total; from sources outside the subnet at most 10 per second; all management replies together at most 100 per second; the excess is counted (A1.10). |
| T-A1-43 | A management datagram with the right magic and type 2 (a reply), also one of 12–17 bytes, gets no reply and is counted; two radios whose management ports are pointed at each other exchange no datagrams (A1.4). |
| T-A1-44 | After 5 failed authentications from one address within 10 s, its authenticated requests and `GET_CHALLENGE` get status 34 until 10 s without a failure; requests above the 20 per second reply limit get no reply at all (A1.10). |
| T-A1-45 | On level 2 with the arm held by a TCI client, a C-7 allow-list that excludes that client unkeys and drops the arm with `HOLDER_LOST` before the reply and before the connection closes; an allow-list that keeps it changes nothing in the TX state (after G6; A1.9). |
| T-A1-46 | After `UPDATE_ABORT`, and after `UPDATE_VERIFY` without `UPDATE_ACTIVATE`, with only `UPDATE_STATUS` for 10 min, the radio restarts into the running image, logs it, and RX works afterwards (reference model; A1.8). |
| T-A1-47 | A request whose header declares 18 + *N* (+ 32) above 1000 bytes gets status 19 and is not executed, also when the datagram arrives truncated; a datagram longer than 1000 bytes whose *N* declares less gets status 17 (A1.4). |
| T-A1-48 | While host A holds the write session, an authenticated `RESTART` from host B whose HMAC verifies is accepted and resets the radio, and the write session is gone afterwards; a `FACTORY_RESET` or `SET_CONFIG` from B in the same state gets status 35 (A1.10). |
| T-A1-49 | With forward power decaying slowly after the unkey (reference model), a `RESTART` while keyed is accepted; during its wait for RF gone a second `RESTART` from another host gets status 1 and a `FACTORY_RESET` gets status 35; exactly one reset follows (reset counter + 1, `GET_STATS`), and the secret and the configuration of Table A1-4 are unchanged (A1.9, one commanded reset at a time). |
| T-A1-50 | Host A opens the write session with an authenticated `SET_CONFIG` at *t* = 0 and repeats the same request (same id, command and payload) at *t* = 4 s, answered from the stored reply; a write from host B gets status 35 before *t* = 60 s and is accepted after the 60 s idle time counted from *t* = 0, not from the repeat (A1.4, A1.10). |
| T-A1-51 | With forward power decaying slowly after the unkey (reference model), host A's `RESTART` while keyed is accepted: a repeat from A during the wait for RF gone gets status 1, a repeat after the reply gets the stored status 0 reply; a `RESTART` from host B during the wait gets status 1, and B's stored reply of an earlier request is unchanged. With forward power held above `P_rf_off` for `t_reset_wait`, A gets status 59, and both A's repeat and B's retried `RESTART` are answered with status 59; no reset follows (A1.4, A1.9; 0022 §87 item 3). |
| T-A1-52 | A foreign-image `UPDATE_BEGIN` and `UPDATE_ACTIVATE`, `RETURN_TO_STOCK` and `ENTER_VENDOR_RECOVERY` while armed, and while keyed, get status 50 and leave `TX_STATE` and `TX_ARM_STATE` unchanged; during another firmware update they get status 58 (A1.8a item 6). |
| T-A1-53 | Otherwise, `RETURN_TO_STOCK` and `ENTER_VENDOR_RECOVERY` get status 56 and change nothing; identification reports that no stock image is retained and that vendor recovery is not supported (A1.5 I-2a, A1.8a items 2, 4; 0028). |
| T-A1-54 | With the F1 gate passed (reference model), an accepted foreign `UPDATE_BEGIN` ends the P2 session, stops the streams, logs the hand-over with source and target, replies after RF is gone with the one-way warning and hands over to the installer; the secret is still in storage until the first sector erase and is erased with it; with forward power held above `P_rf_off` it gets status 59 and nothing changes (reference model; A1.8a items 3, 6–8). |
| T-A1-55 | A synthetic opaque image sent with the "foreign image" flag is accepted only if the hardware id matches (else 51) and it fits sectors 1–7 (else 61); a block with a bad CRC gets 53 and is not written; `UPDATE_VERIFY` with a hash that differs from the owner-supplied SHA-256 gets 55; `UPDATE_ACTIVATE` without the flag gets 17, before a good verify 55; with both, the activation is permanent, with no trial boot and no rollback (reference model; A1.8a items 3, 5). |
| T-A1-56 | No update writes the vendor bootloader region or the option bytes; an update to an image of this firmware writes neither the boot stage nor the running slot; only the streaming installation writes sector 1 (reference model; A1.8a item 1). |
| T-A1-57 | With the controller disconnected, and with a controller that does not respond, discovery, RX IQ on level 1 and the management channel work; identification reports the controller state (reference model; part A scope, A1.5 I-6; 0023). |
| T-A1-58 | With the controller disconnected, an image of this firmware (staged in the other slot) and then a synthetic opaque stock image with the "foreign image" flag (streaming installer) are installed through the same management commands (`UPDATE_BEGIN`, `UPDATE_BLOCK`, `UPDATE_VERIFY`, `UPDATE_ACTIVATE`) and the same commanded-reset order; both succeed and no step waits for, or sends anything to, the controller (reference model; A1.8a "Installation paths"; 0025 R2, 0028). |
| T-A1-59 | For an image of this firmware and for a foreign image alike, a block with a wrong CRC-32 gets status 53, an image whose SHA-256 differs from the header gets status 55 at `UPDATE_VERIFY`, and an image for other hardware gets status 51 before anything is written or erased (reference model; A1.8, A1.8a; 0025 R3). |
| T-A1-60 | During a streaming installation (A1.8a item 3): `UPDATE_STATUS` reports the sectors written; `IDENTIFY` reports the installer state; any other management command than those listed gets 58, and discovery, P2 and TCI get no answer; the TX-side state of A3.6 is unchanged and toggling the local PTT and KEY inputs keys nothing; `UPDATE_ABORT` before the first erase restarts into this firmware with its secret and configuration unchanged, after it gets status 60; a transfer that stops for more than 10 min does not restart the main unit and resumes when blocks arrive again; sector 0 and the option bytes are never written, and no sector is written before all its blocks arrived with a good CRC (reference model; 0028). |
| T-A1-61 | Before the F1 gate (P-SYS-5) is set, a foreign `UPDATE_BEGIN` that passes every other check gets status 56 and changes nothing (also an oversized one: 56 precedes 61); after it is set (reference model) the same request is accepted (A1.8a item 3, "Gate"; 0028). |
| T-A1-62 | During a streaming installation, before the first sector erase, 10 min without an update command other than `UPDATE_STATUS` restart the main unit into this firmware with secret and configuration unchanged; a repeat of the accepted `UPDATE_BEGIN` is answered from the stored reply; `IDENTIFY` reports update state "installer" with the sectors written (reference model; A1.4, A1.5 I-3, A1.8a item 3). |
| T-A1-63 | `SET_CALIBRATION` while a P2 session runs gets 57, while armed 50, during an update 58; with an unknown parameter id 48 and nothing changes; a complete set sets record `tx` bit 4 from the next session on and `IDENTIFY` tags 20 and 21 report it; `FACTORY_RESET` keeps the set; `CLEAR_CALIBRATION` clears bit 4 from the next session on (reference model; A1.12; 0039). |
| T-A1-64 | A `SET_CONFIG` with the SWR limit (tag 9) or the PA temperature limit (tag 10) before the calibration set holds P-PWR-2 gets status 62; a P2 write of `LIMIT_SWR` or `LIMIT_PA_TEMP` gets write reject reason `NOT_CALIBRATED` (A1.12 item 4; 0039). |
| T-A1-65 | A region other than 0 in `SET_CONFIG` (tag 6) gets status 48; with region 0 a DUC0 frequency at each edge of every range of Table A3-0 is inside, 1 Hz outside each edge is outside (reference model with a complete calibration set; A1.6 C-6; Table A3-0; 0039). |
| T-A1-66 | With no DHCP server, the radio uses a link-local address, retries DHCP from 4 s doubling up to every 64 s, and lends the management socket for at most 2 s per retry; a management request repeated every 500 ms for 2.5 s around a retry gets an answer (reference model; A1.2, A1.11; 0041). |
| T-A1-67 | Running in slot A (`IDENTIFY` tag 23 = 1): `UPDATE_BEGIN` with a package for slot A gets 63 and nothing is written, one for slot B is accepted; during an unconfirmed trial boot `UPDATE_BEGIN` gets 58; a simulated write failure in the staging slot discards the transfer with status 55, logs it and leaves the running image and boot record unchanged (reference model; A1.8; 0043). |
| T-A1-68 | A trial image that meets the self-confirmation criterion confirms itself after 10 s without a host; one that keeps its network down rolls back after 60 s through a commanded reset; one that resets before confirming (simulated crash or power loss) starts the previous image at the next start; each rollback is logged (reference model or emulator; A1.8; 0044). |
| T-A1-69 | From link up: with a DHCP server that answers at once, `IDENTIFY` and P2 discovery are answered within 5 s; with no DHCP server, within 17 s, from the link-local address, after both announcements (reference model or emulator; A1.11 "Bound on start-up"; 0048, 0057). |
| T-A1-70 | At start-up with no DHCP server: the radio sends the RFC 3927 probes for its chosen link-local address with the section 9 timing before it answers discovery; an ARP reply claiming that address during the probe makes it choose another and probe again (reference model or emulator; A1.11; 0049). |
| T-A1-71 | After a successful probe the radio sends the RFC 3927 announcements, and only then answers discovery and `IDENTIFY` on the link-local address, within the start-up bound (A1.11; 0048, 0049). |
| T-A1-72 | While running on a link-local address, a conflict (IP_CONFLI) makes the radio give the address up at once without a defending ARP, end the session as an address change (A1.11a), probe a new address with discovery and management closed for at most 10 s, and log the conflict; after 10 conflicts it waits 60 s before the next probe (A1.11; 0049). |
| T-A1-73 | A conflict on a DHCP-leased address sends DHCPDECLINE, moves to link-local and restarts DHCP; a conflict on a static address falls back to the previous configuration (C-4), or to link-local if there is none; each is logged (A1.6 C-4, A1.11; 0049). |
| T-A1-74 | At power-on before any address, DHCPDISCOVER has IP source 0.0.0.0; while link-local, DHCPDISCOVER and DHCPREQUEST have the link-local address as IP source, `ciaddr` 0.0.0.0 and the BROADCAST flag set, and a server's broadcast OFFER and ACK give the radio its lease (emulator or reference model; A1.11; 0052). |
| T-A1-75 | While armed and during a P2 session: `SET_FIRST_SECRET` with a 31-byte payload and `SET_CALIBRATION` with a TLV that runs past the end get 17, not 50 or 57; `SET_CALIBRATION` with a well-formed payload and an unknown parameter id gets 57 during the session and, outside it while armed, 50; unarmed and outside a session, 48 (A1.4 "Malformed payload", Table A1-3; 0056). |
| T-A1-76 | With socket 0 failing to open in MACRAW (simulated): after three tries 10 ms apart the radio uses the chosen link-local address unprobed, answers `IDENTIFY` with tag 24 = 1, logs an entry containing `link-local <address> used without a complete RFC 3927 probe`, raises `GET_STATS` `ll_unprobed`, prints a `G7250 FAULT LINKLOCAL` console line, and still gives the address up on IP_CONFLI; no A3.5 fault is set (emulator; A1.11; 0057). |
| T-A2-01 | Identification and record `level` report the active level and the owner (IP and port). |
| T-A2-02 | A level change drops the arm with `LEVEL_CHANGE` (after G6). |
| T-A2-03 | While level 2 is active, P2 C&C starts no session and no takeover (after G6). |
| T-A2-04 | `SWITCH_LEVEL` while armed and keyed follows A2.5a in order: unkey, `TX_ARM_STATE` with `LEVEL_CHANGE` from port 7250 before it closes, reply status 1, streams stop, change logged, P2 sockets closed, ports 1024 and 7251 open (after G6). |
| T-A2-05 | `SWITCH_LEVEL` to the active level returns status 0 and changes nothing; to level 2 before G6, status 56. |
| T-A2-06 | While level 2 is active, discovery answers 0x03, port 7250 is closed, and management identification gives level 2 (after G6). |
| T-A2-07 | Takeover by a General packet while host A runs: host B becomes owner at once, DDC0 continues to B from the next packet with sequence 0, no stream packet or register push goes to A after its last `TX_ARM_STATE`, B's watchdog runs, and the record is sent once to B. |
| T-A2-08 | After a takeover `TX_MODE_CLASS` is 0, `KEYED_TIMEOUT` 500 ms, session limits are the persistent values, and the RX settings of A are kept until B writes its own. |
| T-A2-09 | Two hosts alternating C&C every 50 ms: neither can stay armed, every takeover is counted, and the log has at most one takeover entry per second for the pair. |
| T-A2-10 | At a takeover the old owner receives the final `TX_ARM_STATE` (`TAKEOVER`) exactly three times, about 10 ms apart, with identical register pairs and consecutive sequence numbers, and no stream packet or register push afterwards; a discovery from it is still answered. |
| T-A2-11 | After B takes over from A, each DUC I&Q or register packet from A within 5 s is answered with one copy of the `TAKEOVER` notice, at most once per 100 ms; a C&C packet from A takes the radio back, B receives three notices, and the streams to A restart at sequence 0. On port 7250 the notices to B continue B's session counter (the packet after B's last register packet), and answers sent to A while B was owner do not advance it (B2.7; 0051). |
| T-A2-12 | While level 2 is active, a General packet on port 1024 has no effect, is counted, and does not count for a later level-1 session (after G6). |
| T-A2-13 | Host A with General bytes 31–32 = 50000 is taken over by host B: the three copies and every repeated notice arrive at A's address on port 50000 from local port 7250 (with bytes 31–32 = 0: at A's source port); a register write from A within the 5 s gets its `NOT_OWNER` answer at its source port and a notice, at most one notice per 100 ms, and neither limit consumes the other (A2.4). |
| T-A2-14 | Host A sets PA enable, CW select, break-in, hang delay 500 ms, RF delay 20 ms, keyer speed 20 wpm and EER; host B takes over with a High Priority packet only: `TX_STATE` bit 4 shows PA disabled, KEY keys nothing, CWX forms no element, no EER refusal stands, and DUC rate and bits read 192 and 24; the same holds after A clears run and starts again with a run bit only (A3.6a). |
| T-A2-15 | At a takeover the new owner's first stream packet and the effects of the takeover packet do not wait for the second and third `TX_ARM_STATE` copy to the old owner (A2.4 step 1). |
| T-A2-16 | Host B takes over from host A while A is not armed: A receives the final `TX_ARM_STATE` three times, reading disarmed with drop reason 4 `TAKEOVER`, and the repeated notice for 5 s; the token in force afterwards differs from the one before (A2.4 step 1, A3.2). |
| T-A2-17 | Host A sends no memory-mapped packet and General bytes 31–32 = 0, unarmed, and is taken over by host B: A receives the three copies of `TX_ARM_STATE` (`TAKEOVER`) from port 7250 at its source port and, for its DUC I&Q within the 5 s, the repeated notice at most once per 100 ms, while `TX_ARM_STATE` read by B shows drop reason 4 `TAKEOVER` and a new token; a host A whose session was carried over at a restart by the run bit with General bytes 31–32 = 6000 receives the copies at port 6000 (A2.4, notice to every old owner; 0022 §87 item 2). |
| T-A3-01 | Each drop event of Table A3-1 unkeys, disarms, issues a new token and reports its reason (one test per row). |
| T-A3-02 | Each limit row of Table A3-2 gives its action and recovery, and the action takes effect within the maximum time from threshold to action of its row, measured from the injected sensor or detector sample beyond the threshold, also under full packet load (one test per row, reference model; A3.10). |
| T-A3-03 | A `LIMIT_<name>` write looser than the persistent value (C-3) is rejected with `LOOSER_LIMIT`, a stricter one accepted; writing 0 and the session end return to the persistent value; the answer carries the limit in force. |
| T-A3-04 | A class 1 fault clears only with the current code and only when the condition is gone; a clear does not re-arm. |
| T-A3-05 | A class 2 fault clears only by reset and is detected again at start-up if the cause persists. |
| T-A3-06 | After power-on and after every session end the state of A3.6 holds. After power-on the check starts 20 µs after the boot stage's first instruction at 0x08020000; from then on every RF control pin of HIS 85 is at its safe-state level and stays there (emulator or reference model; A3.6, "From when"; 0045). |
| T-A3-07 | Not in v1.0 (0030). Controller PTT and KEY inputs are reported and never key the radio without arm, with and without a P2 session while level 1 is active. |
| T-A3-08 | While keyed, a DUC0 retune inside the same transmit range and RF-path state is followed without unkey. |
| T-A3-09 | While keyed, a DUC0 change to another `tx_band` entry, or out of all ranges, unkeys before the frequency applies (`TX_STATE` reason 6, or 4 with `LIMIT_TX_FREQ`), keeps the arm, and switches relays only after the unkey; keying again needs a new 0→1 edge. |
| T-A3-10 | While keyed, an Alex word or step-attenuator change in the High Priority packet is applied only after the unkey; a register write that would switch an RF-path relay is rejected with `KEYED`; a controller ANT or ATU press is ignored and logged. |
| T-A3-11 | SWR above S1 caps forward power at `P_swr` to the end of the continuous transmission even if SWR falls back; with CW break-in the cap holds across element gaps shorter than the hang delay and lifts only after the hang delay has run out; the next continuous transmission starts uncapped (reference model; A3.4a). |
| T-A3-12 | PA temperature above T2 caps power at `P_temp`; the cap lifts below T2 minus the hysteresis; a short temperature spike below the filter time constant does not cap (reference model). |
| T-A3-13 | Under a received flood at line rate to every open port, the watchdog, keyed timeout and limit actions keep their timing (the unkey may come earlier, never later). |
| T-A3-14 | With a class 1 and a class 2 code flagged, `FAULT_CLASS` is 2 and `FAULT_CLEAR` is rejected with `NOT_CLEARABLE`, clearing nothing; with only class 1 codes whose conditions are gone, all flags clear. |
| T-A3-15 | A processor watchdog reset seen at start-up gives `WDT_RESET_AT_START`, class 1, clearable by `FAULT_CLEAR` without another reset; the radio never resets itself to clear a fault. |
| T-A3-16 | A PTT bit held at 1 while PA enable is 0, while EER or a DUC rate refusal stands, with DUC0 outside the ranges, while unarmed and while inhibited does not key when the condition clears; a new 0→1 edge then keys (A3.1a). |
| T-A3-17 | `ATU_TUNE` = 1 while the owner is keyed with PTT, CWX or KEY is rejected with `KEYED`, does not start after the unkey, and does not end the transmission (A3.9b). |
| T-A3-18 | In a tune cycle the drive is at the tune power before RF starts and before the first ATU relay moves; the cycle ends unkeyed in each end case (tuned, failed, abort, time limit, keyed timeout), and a PTT held at 1 through the cycle does not key after it (reference model). |
| T-A3-19 | After an unkey no RF-path relay moves until forward power is below `P_rf_off` or `t_rf_off` has passed; no key comes earlier than `t_settle` after the last relay change (reference model; A3.9a). |
| T-A3-20 | A High Priority packet with a DUC0 frequency in another `tx_band` entry and PTT 0→1, sent while unkeyed, switches the relays, waits `t_settle` and then keys; if PTT is 0 again before the end of the settle time, nothing keys. |
| T-A3-21 | At a key the T/R relay and the RX attenuator during TX reach their TX position before SEND and RF; at an unkey they return only after RF is gone; at a session end, takeover or fault entry while keyed the A3.6 relay set is applied only after RF is gone. |
| T-A3-22 | Decaying RF within `t_blank` after a normal unkey gives no `RF_NOT_KEYED` fault; RF above the threshold after `t_blank` while not keyed does (reference model). |
| T-A3-23 | `FAULT_CLEAR` less than 10 s after the latest fault entry is rejected with `NOT_CLEARABLE` even when the condition is gone; after 10 s it clears. |
| T-A3-24 | The third `SWR_REPEATED` entry since power-on gives `FAULT_CLASS` 2 and cannot be cleared by `FAULT_CLEAR`; after `RESTART` the counters are kept and a further entry is class 2 again; after a power cycle the count starts at 0. |
| T-A3-25 | While keyed, a retune inside one `tx_band` entry that would make the ATU recall another stored state unkeys with `TX_STATE` reason 6 before the recall; no ATU state changes while keyed (HIS-08; reference model). |
| T-A3-26 | After an unkey, a session end and a takeover, the DUC FIFO is empty: the first I&Q sent at the next key is from packets received after it. A host that streams DUC I&Q without pause for 10 s unkeyed and then keys: the FIFO never holds more than 4800 samples while unkeyed, and the first sample transmitted was received at most 25 ms before the key (B10.7). |
| T-A3-27 | With DDC0 at 7.1 MHz and DUC0 at 14.2 MHz, unkeyed, the RF path is in the 7 MHz state; a PTT 0→1 edge switches it to the 14 MHz state unkeyed, waits `t_settle` and then keys through the 14 MHz TX filter; after the unkey and RF gone it returns to the 7 MHz state (reference model; A3.1b). |
| T-A3-28 | Unkeyed, a DUC0 change into another `tx_band` entry moves no relay, apart from the TX low-pass filter where HIS-07 lets it follow the TX frequency; the switch comes at the next key (A3.1b). |
| T-A3-29 | A High Priority packet with a DUC0 change that needs other relays and PTT 0, followed 10 ms later by a packet with PTT 0→1, keys once the relays have settled, not earlier, and is not refused (A3.9a item 3). |
| T-A3-30 | A PTT 0→1 edge within `t_rf_off` after an unkey keys only after RF gone, the deferred changes and the settle time; a further RF-path change during the wait refuses the request, which does not key later (A3.9a item 3). |
| T-A3-31 | A limit trip with action `FAULT` (PA temperature above T4) drops the arm with drop reason 6 `FAULT` and `FAULT_CODE` `PA_OVERTEMP`, never with 0x40 + limit id (Table A3-1). |
| T-A3-32 | In class AM at full drive, a carrier with 100 % sine modulation and an unmodulated carrier each give a carrier estimate of at most 25 W plus the margin and a PEP of at most 100 W; the modulated carrier is not reduced below 25 W (reference model; A3.4c). |
| T-A3-33 | With a detector model without envelope samples, AM is held to a mean of 25 W; without PEP, SSB with a 6 dB crest factor is held to a mean of `P_ssb_mean`, and CW and FM with a constant envelope to a mean of 100 W (reference model; A3.4c). |
| T-A3-34 | CW break-in with element gaps shorter than the hang delay for 5 min unkeys at the maximum continuous TX time; with gaps longer than the hang delay the timer restarts at every key (A3.4d). |
| T-A3-35 | A key whose only relay movement is the T/R relay and the RX attenuator during TX is not held by the "relays settled" condition; it keys after the sequence of A3.9a item 4 (A3.1a). |
| T-A3-36 | With a mean-only detector model and `LIMIT_FWD_POWER` = 10 000 mW: class SSB with a signal of 6 dB PEP-to-mean ratio is held to a mean of 2.5 W (`P_ssb_mean` = 25 W), so its PEP stays at 10 W; class CW to a mean of 10 W; class AM to a mean of 2.5 W; the same bounds hold when the 10 W are the persistent C-3 value with no session value, and on TCI with `G7250_LIMIT:LIMIT_FWD_POWER,10000;` (reference model; Table A3-3). |
| T-A3-37 | With a session power limit below the ATU profile's tune power, the tune carrier of an `ATU_TUNE` cycle stays at the limit, set before RF and before the first ATU relay moves; with no stricter limit it is at the profile's tune power (reference model; A3.4 rule 5, A3.9b item 3). |
| T-A3-38 | After a power-on, a processor watchdog reset and a `RESTART` while armed, the first `TX_ARM_STATE` of the next session reads disarmed with drop reason 5 `RESET` until the next drop (Table A3-1 event 5); after the processor watchdog reset `SAFETY_STATE` is 1 with `FAULT_CODE` `WDT_RESET_AT_START`, and the drop reason still reads 5, not 6; the `RESTART` itself pushed drop reason 11 `MAINTENANCE` before the reset (A1.9; Table A3-1). |
| T-A3-39 | With a mean-only detector model, *L* = 100 W and class SSB, voice with a 12 dB crest factor at full drive is held to a PEP estimate of at most 100 W (mean about 6.3 W), not to the class bound of 25 W mean (reference model; A3.4c). |
| T-A3-40 | With a mean-only detector model and class CW declared, SSB I&Q with a 6 dB crest factor at full drive is held to a PEP estimate of at most *L* (mean about *L* / 4), not to the CW bound of mean ≤ *L* (reference model; A3.4c). |
| T-A3-41 | With a mean-only detector model and class AM, a 25 W carrier modulated to 150 % is held to a PEP estimate of at most *L* and a mean of at most 25 W; at 100 % sine modulation the bounds of Table A3-3 give the same result as before (reference model; A3.4c). |
| T-A3-42 | Not in v1.0 (0030). With level 1 active and armed: KEY keys only while the owner has set DUC-specific byte 5 bit [1] in this session; after a takeover or session end it keys nothing until the new owner sets the bit; a controller request that reaches the main unit never keys (A3.1a, B10.1). |
| T-A3-43 | With DDC0 at 7.1 MHz and DUC0 at 14.2 MHz, `ATU_TUNE` switches the RF path to the 14 MHz TX state and waits `t_settle` before the tune carrier; after the cycle and RF gone the path returns to the 7 MHz state, and the next PTT key switches to the 14 MHz state again (reference model; A3.9b items 2, 7). |
| T-A3-44 | With forward power held above `P_rf_off` after a normal unkey, after a session end and after a takeover (reference model), the radio enters the fault state with `RF_NOT_KEYED` no later than `t_reset_wait` after the unkey, no relay moves while RF is present, and the session end completes (A3.9a item 1a). |
| T-A3-45 | During the bounded wait of a PTT edge that needs the TX state, a DDC0 change alone does not refuse the key; it is applied after the next unkey; a DUC0 change to another `tx_band` entry during the wait refuses it (A3.9a item 3). |
| T-A3-46 | With CW break-in and a hang delay of 200 ms, SEND and the T/R relay stay in their TX position between elements 100 ms apart and return only after 200 ms without an element and RF gone; a drop event during the hang delay releases SEND at once after RF gone (A3.9a item 4). |
| T-A3-47 | A drop event while not armed that does not end the session (`TX_ARM` = 0, a C-3 change) changes no field of `TX_ARM_STATE` and issues no new token; one that ends it (the watchdog stop, `RUN_CLEARED`) changes no field of `TX_ARM_STATE` but issues a new token at the session end, pushed at the next session start; a takeover while not armed sets drop reason 4 `TAKEOVER` (A3.2, A3.6). |
| T-A3-48 | With CW break-in (hang delay 200 ms) and CWX, KEY and the hardware keyer: an element that comes while PA enable is 0, while inhibited or after a TX frequency outside the ranges does not key, the keyer stops, and it keys again only after a new paddle or KEY closure; on level 1 `ATU_TUNE` = 1 during the hang delay gets `KEYED` (A3.1a, A3.4d, A3.9b). |
| T-A3-49 | With CW break-in (hang delay 200 ms), in a 100 ms gap between elements: a DUC0 change to another `tx_band` entry, an Alex word change or a `TX_MODE_CLASS` change ends the hang delay with unkey reason 6 or 10, and RF gone, SEND released and the T/R return all come before the relay or class change; PA enable cleared ends it with reason 7; a DDC0 change alone keeps the hang delay and is applied after it (reference model; A3.4d, A3.9a item 4). |
| T-A3-50 | `ATU_TUNE` = 0 from the owner is accepted while the tune cycle keys (the cycle ends unkeyed), while unarmed and while no cycle runs (no-op, nothing changes) (A3.9b item 1). |
| T-A3-51 | With break-in and a hang delay of 5000 ms written, the hang delay in force is 2000 ms, the clamp is logged and counted; with High Priority packets stopping during the hang delay, the hang delay ends between `KEYED_TIMEOUT` and `KEYED_TIMEOUT` + 10 ms after the last packet, with `TX_STATE` unkey reason 2, SEND released after RF gone and the T/R relay returned (A3.4d, B10.5). |
| T-A3-52 | With DDC0 at 7.1 MHz and DUC0 at 14.2 MHz, a PTT 0→1 edge arriving while the RF path returns to the 7 MHz state after an unkey keys after one `t_settle` from the switch to the 14 MHz state, never later than `t_rf_off` + `t_settle` after the edge; a DDC0-only change deferred meanwhile is applied at the next unkey (reference model; A3.9a item 3a). |
| T-A3-53 | With `t_blank` below the gap between elements (reference model): with break-in and a hang delay of 500 ms, and with break-in clear inside a PTT transmission, RF above the "RF while not keyed" threshold injected in a gap after `t_blank` from the end of the last element's trailing ramp enters the fault state with `RF_NOT_KEYED`, although SEND and the T/R relay are in their TX position; RF that decays within `t_blank` after each element gives no fault (Table A3-2, A3.9a item 6). |
| T-A3-54 | With DDC0 changed while keyed (receive frequency only) and a PTT 0→1 edge right after the unkey, the edge keys after the TX-side deferred changes and the TX state, and DDC0 stays deferred to the next unkey; in a reference model whose relays cannot be commanded again before they settle, an edge during the return to the receive state keys once the relay has settled, been commanded to the TX state and settled again, within the bound of A3.9a item 3, and is refused when that bound is shorter (A3.9a items 3, 3a). |
| T-A3-55 | With break-in and a hang delay of 500 ms: High Priority packets in a gap that repeat the Alex word in force do not end the hang delay; an Alex word change received while an element is on is deferred, does not end the hang delay in the following gap and is applied when it ends; a different Alex word received in a gap ends it with unkey reason 6; a DUC0 frequency outside the transmit ranges received in a gap ends it with unkey reason 4 and limit id `LIMIT_TX_FREQ` (A3.4d). |
| T-A3-56 | With a DUC0 change that needs another RF-path state sent with a PTT 0→1 edge (reference model, `t_settle` 20 ms), a CWX paddle edge and an `ATU_TUNE` write 5 ms after the PTT edge are refused (the write with `KEYED`) and not latched; the PTT request keys after the settle; with the PTT request refused at the bound instead, the CWX bit still held does not key until a new edge (A3.9a item 3; 0022 §87 item 4). |
| T-A3-57 | Level 1, a P2 host sending CW as DUC I&Q with PTT, zero samples (I = Q = 0) between elements and a gap of 300 ms, `t_blank` below the gap (reference model): RF above the "RF while not keyed" threshold injected in a gap is detected, the fault state with `RF_NOT_KEYED` entered, no later than `t_blank` + 10 ms after the last non-zero sample (or after the injection, if later), while PTT stays set; RF that decays within `t_blank` after each element gives no fault; the same with the zero samples fed during an underflow shorter than the underrun time; with non-zero noise samples in the gaps the check does not run there (known limit) (Table A3-2, A3.9a item 6; 0022 §88 item 1). |
| T-A3-58 | v1.0 (0030): with level 1 active, armed and unarmed, with and without hardware CW selected, toggling the local PTT input (PH6) and the KEY contacts (PH3, PH4) keys nothing, and High Priority status byte 4 bits [0]–[2] stay 0 (reference model with simulated inputs; [HIS] HIS 85). |
| T-A3-59 | With a simulated supply-configuration mismatch, and with a time-out, the boot stage keeps the A3.6 safe state, holds CH394 RST# low, writes no flash, answers no discovery or management request, and sends `G7250 FAULT SUPPLY <8 upper-case hex digits> MISMATCH` (or `TIMEOUT`) with CR LF on USART1 at 115200 8N1 once per second (emulator or reference model; A3.5a; 0040). |
| T-A3-60 | Needs the radio (P-SYS-9): after two `SWR_REPEATED` entries, a `RESTART` and then a processor watchdog reset leave the fault entry counter and `TRIP_COUNT` unchanged, so the third entry is class 2; a power cycle clears them. Before P-SYS-9 is set the arm is rejected with `NOT_CALIBRATED` (A3.5; Annex P; 0050). |

### Discovery (B3)

| Id | Description |
|----|-------------|
| T-B3-01 | Broadcast discovery to 255.255.255.255:1024 yields exactly one reply. |
| T-B3-02 | Directed discovery to the unit's IP:1024 yields exactly one reply. |
| T-B3-03 | Discovery to the subnet broadcast address yields exactly one reply. |
| T-B3-04 | Reply comes from port 1024 to the host's IP and source port. |
| T-B3-05 | Reply payload is 60 bytes; bytes 0–3 are zero. |
| T-B3-06 | Byte 4 is 0x02 when idle. |
| T-B3-07 | Byte 4 is 0x03 when running for host A and discovery comes from host B. |
| T-B3-08 | Byte 4 is 0x02 when running for host A and discovery comes from host A on the same port. |
| T-B3-09 | Bytes 5–10 equal the unit's link MAC address and are stable across power cycles. |
| T-B3-10 | Byte 11 is 200 (0xC8). |
| T-B3-11 | Byte 12 is 44 and equals `p2_version`. |
| T-B3-12 | Byte 13 equals major × 10 + minor of `fw_version`. |
| T-B3-13 | Bytes 14–19 and 24–59 are zero. |
| T-B3-14 | Byte 20 equals `num_ddc` and the number of DDCs that can be enabled and streamed. |
| T-B3-15 | Byte 21 is 1; `dsp_clock_hz` is 122 880 000; a phase word 2^32 × 7 100 000 / 122 880 000 in High Priority bytes 9–12 tunes DDC0 to 7.1 MHz within the resolution of the hardware clock (B3.8). |
| T-B3-16 | Byte 22 is 0x00. |
| T-B3-17 | Discovery is answered in Big-Endian format after any General packet. |
| T-B3-18 | Discovery with non-zero bytes 5–59 is still answered. |
| T-B3-19 | Discovery from a non-owner is answered while streaming, without interrupting the stream. |
| T-B3-20 | Erase and Program packets get no reply and change neither firmware nor stored state. |
| T-B3-21 | No UDP port outside Table B2-1 answers or is open; port 1028 is closed. |
| T-B3-22 | Byte 4 is 0x03 when running for host A and discovery comes from host A's IP on another port. |
| T-B3-23 | Byte 4 is 0x03 while level 2 is active (after G6). |
| T-B3-24 | Byte 4 is never 0xFE or 0xFF, and byte 11 never 254 or 255. |
| T-B3-25 | Set IP with the unit's MAC sets and stores the address (0.0.0.0 selects DHCP), is answered with a Discovery reply, works while running and while armed, and is logged with its source. |
| T-B3-26 | Set IP with another MAC changes nothing. |
| T-B3-27 | A discovery from the owner while armed drops the arm with `REDISCOVERY`. |
| T-B3-28 | A discovery from the owner's IP on another port while armed is answered with 0x03 and does not drop the arm or move the streams. |
| T-B3-29 | Set IP with the unit's MAC sent to the unicast or directed broadcast address is accepted with the same effects as a broadcast Set IP. |
| T-B3-30 | Byte 13 equals major × 10 + minor of `fw_version`, capped at 255; byte 23 is non-zero only on a beta build. |
| T-B3-31 | 100 discoveries per second from one source get at most 10 replies per second; the excess is counted in `GET_STATS`. |
| T-B3-32 | Set IP with the unit's MAC to the unicast or directed broadcast address from a source outside the radio's subnet is ignored and counted. To 255.255.255.255 from the same source it is accepted where the controller reports the destination; where it does not (P-NET-3), it is ignored and counted too, except while the radio uses a link-local address, when it is accepted (B3.9, B11.15; 0039). |
| T-B3-33 | A discovery request from source port 1024 gets no reply and is counted; two G7250s on one LAN with a discovery reply injected toward one of them exchange no further datagrams (B3.2). |
| T-B3-34 | After a Set IP to an address inside the current subnet the netmask is unchanged; to an address outside it the netmask becomes 255.255.255.0 and a gateway outside the new subnet is cleared; 0.0.0.0 starts DHCP (B3.9; 0039). |

### Capability record (B4)

| Id | Description |
|----|-------------|
| T-B4-01 | A `CAP_REQUEST` write is answered with one memory-mapped packet holding the echo and the whole record. |
| T-B4-02 | `cap_magic` is 0x47373235 and `cap_length` equals the number of record registers sent (their values are checked with the version 1.1 fields, below). |
| T-B4-03 | `product_name` reads "G7250-P2". |
| T-B4-04 | A `CAP_REQUEST`-only packet from a host without a session starts no session, takes nothing over, does not feed the watchdog, and is answered to its source IP and port. |
| T-B4-05 | A `CAP_REQUEST` from host B while host A runs does not disturb A's session or arm. |
| T-B4-06 | Writes to the record range are ignored and answered with the current values. |
| T-B4-07 | The record is sent once, unasked, per session: at the session start when the owner's set-up phase holds a General packet, otherwise after the first General packet of the session; also when the host's `CAP_REQUEST` was answered less than 1 s before the session start (B4.4 item 2). |
| T-B4-08 | The record does not change during a session. |
| T-B4-09 | `ddc_rates`, `num_ddc`, `streams`, `duc_format`, `wb_format`, `wb_frame`, `link` and `tx` match the behaviour tested in B5, B6 and B10. |
| T-B4-10 | Unused bits and unused range, band and calibration entries are zero. |
| T-B4-11 | `CAP_REQUEST` with a value other than 1 is rejected with `BAD_VALUE` and sends no record. |
| T-B4-12 | `cap_version` is 1.1, `cap_length` is 207, `mgmt` is 0x1C530001, `regmap_version` is 1.0, and the version 1.1 fields match the behaviour tested in B9. |
| T-B4-13 | A `CAP_REQUEST` from the future session socket before the first General packet is answered to that socket and does not make it the owner. |
| T-B4-14 | `CAP_REQUEST`s from a non-owner source at 10 per second are answered at most once per second; from 20 sources, at most 10 answers per second in total. |
| T-B4-15 | General packets from a source without a session get no record; under a takeover ping-pong between two hosts the unasked record reaches each destination at most once per second. |
| T-B4-16 | On a 10 Mbit/s link a `CAP_REQUEST` is answered with the record and with `LINK_STATUS` and `STREAM_STATUS`; the status answers are at most one per 100 ms per source and 10 per second in total (B5.8). |
| T-B4-17 | A `CAP_REQUEST` from a non-owner repeated 1 s after an answered one is answered again; one repeated after 200 ms is dropped by the rate limit and counted (B4.4 items 3, 4). |
| T-B4-18 | An r1.0 firmware reports record `tx` bits 0–3 = 1 and bit 4 = 0 without a complete calibration set; an arm that meets every other condition gets `NOT_CALIBRATED` (12), not `TX_NOT_IMPLEMENTED` (11). A build with bit 1 = 0 reports bit 4 = 0 and rejects the arm with 11, which precedes 12 (B4.8, B10, B10.2 item 3; 0036, 0039). |
| T-B4-19 | The unasked record at a session start arrives in one to three packets to the owner's register destination, in ascending address order, with consecutive port 7250 sequence numbers, within 50 ms, together holding every record register once with the values of a `CAP_REQUEST` answer in the same session; the `CAP_REQUEST` answer is one packet (B4.4 items 1, 2; 0059). |

### Session, ownership, watchdog (B5)

| Id | Description |
|----|-------------|
| T-B5-01 | Without C&C from the owner for more than one second in RUN, the unit goes to standby, stops streaming and drops the arm. |
| T-B5-02 | Memory-mapped packets alone do not keep the session alive past one second. |
| T-B5-03 | C&C packets from a non-owner do not feed the owner's watchdog; the first one is a takeover. |
| T-B5-04 | Takeover while armed and keyed unkeys, drops the arm with `TAKEOVER`, sends a last `TX_ARM_STATE` to the old owner, and serves the new host unarmed. |
| T-B5-05 | A C&C packet from the owner's IP on a new source port is a takeover. |
| T-B5-06 | General byte 38 bit [0] clear disables the watchdog while not armed; it is ignored while armed; `WATCHDOG_STATE` reports both values. |
| T-B5-07 | An arm with General byte 38 bit [0] clear is rejected with `WATCHDOG_DISABLED`. |
| T-B5-08 | Clearing the run bit stops all streams, resets sequence counters and drops the arm with `RUN_CLEARED`. |
| T-B5-09 | On a 10 Mbit/s link discovery is answered, no stream starts, and `LINK_STATUS` gives the reason. |
| T-B5-10 | With C&C every 900 ms the session stays; after the last C&C the unit is in standby within 1100 ms. |
| T-B5-11 | A run bit without a preceding discovery, after a DDC-specific packet that enables DDC0, starts the streams to the sender's source port. |
| T-B5-12 | On a 10 Mbit/s link no High Priority status is sent; each run request and `CAP_REQUEST` is answered with `LINK_STATUS` and `STREAM_STATUS`, at most once per 100 ms. |
| T-B5-13 | A link renegotiation from 100 to 10 Mbit/s while armed and keyed unkeys, disarms with `LINK_DOWN`, stops the streams and sets `LINK_STATUS` refusal reason 1; back at 100 Mbit/s nothing restarts until a High Priority packet with run = 1, which from the last owner restarts with its RX settings (B5.4). |
| T-B5-14 | An arm before any General packet of the session or its set-up phase is rejected with `WATCHDOG_DISABLED`; after the B5.1 order (General with byte 38 bit [0] set, DDC-specific, DUC-specific, then run) it is accepted without a further General packet, also with a refused byte 39; a General packet with bit [0] set from another source before the run does not count. |
| T-B5-15 | Host A sends General (byte 58 bit [0] = 1) and DUC-specific packets, then run: PA enable and the DUC set-up are in force without resending; set-up kept from host B is discarded at A's session start; after A's session ends, a run bit alone from A needs a new General packet before an arm is accepted (B5.1a). |
| T-B5-16 | After a session end by an address change, a level change and a reset, a run bit alone from the same host gives `STREAM_STATUS` reason 1 for DDC0, PA disabled and the DUC settings of Table A3-4; after it resends the General, DDC-specific and DUC-specific packets, DDC0 streams and an arm is accepted (B5.4 item 3). |
| T-B5-17 | Host A streams DDC0 at 192 ksps on 7.1 MHz with PA enable and CW select set; its C&C is cut for 1.5 s (watchdog standby), after which it continues its High Priority keepalive with run = 1 unchanged: the first packet restarts the session without a run 0→1 edge, DDC0 resumes at 192 ksps on 7.1 MHz with sequence numbers continuing from before the standby, PA enable and CW select read cleared (Table A3-4), and an arm is rejected with `WATCHDOG_DISABLED` until A sends a General packet with byte 38 bit [0] set, then accepted (A2.3, B5.4, B2.7). The same after a link loss and return to 100 Mbit/s; after a stop and run the counters start at 0. |
| T-B5-18 | After host A's session ends by a watchdog standby, a run bit alone from host B gives DDC0 `STREAM_STATUS` reason 1 and counters starting at 0; A's RX settings are not used for B (B5.4 item 3, B2.7). |
| T-B5-19 | Host A with General bytes 31–32 = 50000 stops (run = 0) and restarts with a High Priority packet with run = 1 only: the unasked capability record arrives at port 50000 at the session start, unless an unasked record went there less than 1 s before (B4.4 item 2, B5.4). |
| T-B5-20 | On port 1027: two High Priority packets back to back at wire speed, run = 1 then run = 0, and the reverse, with no third packet within 2 ms: both are processed, so the session ends (or starts) as the last one says; a sequence of 20 High Priority packets 1 ms apart: none is lost; in each case `GET_STATS` `hp_rx` rises by exactly the number of packets sent (emulator, at the SPI clock known to work, with no management write, store compaction or firmware update in progress; B2.3 "High Priority bursts", Table B2-2; 0053, 0054, 0058). |
| T-B5-21 | While armed: `SET_FIRST_SECRET`, `CHANGE_SECRET`, `FACTORY_RESET` (as a commanded reset, A1.9) and `SET_CALIBRATION` get status 50 or first unkey and disarm as their rules say, and no flash work starts while TX is armed or keyed; unarmed, a `SET_CONFIG` is answered only after its flash write has ended (emulator; B2.3 "Flash work"; 0055). |

### Streams (B6)

| Id | Description |
|----|-------------|
| T-B6-01 | No DDC0 data before a High Priority packet with run set. |
| T-B6-02 | After General, DDC-specific and run, DDC0 packets arrive from port 1035 to the owner's source IP address and port, also when the host's discovery came from another port. |
| T-B6-03 | Every DDC0 payload is 1444 bytes with 24-bit samples. |
| T-B6-04 | Bytes 12–13 are 24 and bytes 14–15 are 238. |
| T-B6-05 | Sequence numbers start at 0 and increase by one per packet, without gaps on a clean link. |
| T-B6-06 | Sequence counter restarts at 0 after stop and run. |
| T-B6-07 | Sequence number rolls over from 0xFFFFFFFF to 0 (reference model; forced counter). |
| T-B6-08 | Packet rate matches the selected sample rate (rate / 238 per second) within tolerance, per offered rate. |
| T-B6-09 | Each offered rate (48, 96, 192, and 384 if offered) streams without loss for a set duration, with and without TX IQ. |
| T-B6-10 | A DDC0 rate outside `ddc_rates` (including 768 and 1536) stops DDC0, is not clamped, and is reported in `STREAM_STATUS`; a valid rate resumes it. With P-RX-2 set (reference model); before it, reason 2 (Table B8-3 order; 0042). |
| T-B6-11 | A known test tone at a set offset from the DDC0 frequency appears at the right frequency and sign (I before Q). |
| T-B6-12 | Samples are signed two's complement, MSB first (full-scale positive and negative test signals). |
| T-B6-13 | Clearing DDC-specific byte 7 bit [0] stops DDC0 data; setting it resumes. |
| T-B6-14 | Clearing the run bit stops all streams within a defined time. |
| T-B6-15 | Changing between offered rates while running takes effect without a stop. |
| T-B6-16 | Fields listed as ignored in Table B6-2 have no effect on the stream. |
| T-B6-17 | A host that does not know board type 200 and never writes the register map still receives DDC0 data (0015), tuned to the frequency it set as a phase word for a 122.88 MHz clock (B3.8, B4.11). |
| T-B6-18 | A sample size other than 0 or 24 stops DDC0 with a reason. With P-RX-2 set (reference model); before it, reason 2 (Table B8-3 order; 0042). |
| T-B6-19 | General byte 39 asking for little-endian, float or double stops DDC0 with a reason. The reason is 5, also before P-RX-2 (Table B8-3 order; 0042). |
| T-B6-20 | General byte 37 bit [0] or [1] set stops DDC0 with a reason; time stamp bytes 4–11 are otherwise zero. The reason is 6, also before P-RX-2 (Table B8-3 order; 0042). |
| T-B6-21 | Before M2, a wideband enable sends nothing; from M2, ADC0 wideband gives 8 packets of 1028 bytes every 50 ms, with sequence numbers 0–7 in every frame. |
| T-B6-22 | From M2, wideband parameters other than 0 or the fixed values stop wideband with a reason. |
| T-B6-23 | No microphone packets are sent unless `streams` bit 3 is set. |
| T-B6-24 | The DDC0 sequence counter does not restart on a rate change, nor on a resume after a refusal or after DDC-specific byte 7 bit [0] was cleared. |
| T-B6-25 | Each stream comes from its local port of B2.4a. |
| T-B6-26 | A run bit with no DDC-specific packet from that host sends High Priority status but no DDC0 I&Q, `STREAM_STATUS` DDC0 reason 1; a DDC-specific packet with byte 7 bit [0] set then starts DDC0 (B5.2). |
| T-B6-27 | Before P-RX-2 is set: record `streams` bit 0 = 0; a session that enables DDC0 gets `STREAM_STATUS` DDC0 not running with reason 2 and no DDC0 packets; discovery, `CAP_REQUEST` and the management channel answer as usual; with P-RX-2 set (reference model) DDC0 runs (B6.1; 0042). |

### Field table and register map (B7, B8)

| Id | Description |
|----|-------------|
| T-B7-01 | Every field marked "ignored" in Tables B7-1, B7-2 and B10-1 has no effect. |
| T-B7-02 | High Priority byte 4 bits [2]–[7], the TUNE bit and byte 1401 never key or switch anything. |
| T-B7-03 | A non-default radio-side port in General bytes 5–22 or 29–30 is not honoured: the default stays, `LINK_STATUS` bit 17 is set and the request is logged; non-zero bytes 31–32 redirect register packets. |
| T-B7-04 | General byte 37 bit [2] set stops DDC0 with reason 11. Also before P-RX-2 (Table B8-3 order; 0042). |
| T-B7-05 | General byte 58 bit [0] = 0 keeps a TX request from keying and sets `TX_STATE` bit 4. |
| T-B7-06 | A General packet whose byte 37 bit [3] differs from discovery byte 21 stops DDC0 with `STREAM_STATUS` reason 13, keeps PTT from keying and is logged; a General packet with the matching bit ends it (B3.8). |
| T-B8-01 | All registers of Table B8-2 marked "on change" or "event" are pushed within 5 ms of a change. |
| T-B8-02 | `TX_ARM` from a non-owner is rejected; `CAP_REQUEST` from any host is answered. |
| T-B8-03 | `TX_ARM_TOKEN` is never 0, 0xFFFFFFFF or `ARM_MAGIC` and changes on every drop and every new session. |
| T-B8-04 | Every written pair is answered with the value in force within 5 ms, in write order. |
| T-B8-05 | Writes to an unknown address, a read-only register, with a bad value or a looser limit are answered with the value in force and `WRITE_STATUS` with the reason of B8.4. |
| T-B8-06 | Writes from a non-owner, other than `CAP_REQUEST`, are rejected with `NOT_OWNER`, answered to the source, and cause no takeover or watchdog feed. |
| T-B8-07 | Padding pairs (0xFFFF, and 0x0000 with data 0) and a trailing incomplete pair are ignored and not answered. |
| T-B8-08 | Register packets come from port 7250 and go to the owner's source port, or to General bytes 31–32 when non-zero. |
| T-B8-09 | A non-owner's `TX_ARM` write gets `WRITE_STATUS` `NOT_OWNER` to the writer and leaves the owner's `TX_ARM_STATE` (last reject reason included) unchanged; an owner's failed arm precondition is reported in `TX_ARM_STATE` only. |
| T-B8-10 | During a firmware update a non-owner's write gets `BUSY`, not `NOT_OWNER`. |
| T-B8-11 | `NOT_OWNER` answers to one source are at most one per 100 ms. |
| T-B8-12 | With an inhibit active while armed, `TX_ARM_STATE` shows state 2 without a new generation and returns to 1 when it lifts; while disarmed an arm is rejected with `INHIBITED`. |
| T-B8-13 | `LIMIT_INHIBIT` bit 3 is set while the PA temperature is above T3 and bit 5 during supply over-voltage, each cleared when its inhibit lifts; `TX_STATE` bit 5 equals the OR of the bits. |
| T-B8-14 | An under-voltage `UNKEY` sets `LIMIT_INHIBIT` bit 5; a TX request does not key until the supply has been inside the range for 2 s; an under-voltage `REDUCE` sets no inhibit (reference model). |
| T-B8-15 | A non-owner's write to `TX_ARM_TOKEN` is answered with the current token and `NOT_OWNER`; the owner's write with the token and `READ_ONLY`; neither changes it (B8.4a). |
| T-B8-16 | `ATU_TUNE` = 1 while armed and unkeyed but inhibited, in fault or with DUC0 outside the ranges is rejected with `NOT_KEYABLE` and does not start later. |
| T-B8-17 | The last `TX_ARM_TOKEN` before a `RESTART`, a processor watchdog reset and a power cycle differs from the first token after each (reference model with forced token values; B10.2 item 5). |
| T-B8-18 | A `LIMIT_SWR` write looser than the persistent S2 is rejected with `LOOSER_LIMIT`; one between S1 and S2 replaces S2 and leaves S1; one below S1 lowers both; an `ATU_TUNE` = 1 that is both unarmed and from a non-owner gets `NOT_OWNER`, unarmed and with DUC0 outside the ranges gets `NOT_ARMED`, not `NOT_KEYABLE` (A3.9b item 1). |
| T-B8-19 | A register write from a non-owner whose source port is 7250 gets no answer and is counted; from another port it gets `NOT_OWNER` (B8.5). |
| T-B8-20 | Writing 0 to `LIMIT_FWD_POWER` after 10 000 mW returns the answer with the limit in force at the persistent value, and RF is not stopped (B8.6). |
| T-B8-21 | During A's session: register writes and `CAP_REQUEST`s from host C get answers on port 7250 numbered by the non-owner counter (0, 1, 2, …), and A's register stream shows no gap; the non-owner counter is not reset by A's session ending or by a takeover, only by a reset of the radio (B2.7; 0051). |

### Telemetry (B9)

| Id | Description |
|----|-------------|
| T-B9-01 | High Priority status arrives every 50 ms, every 1 ms while transmitting, and immediately (at most one per ms) on an input change. |
| T-B9-02 | Bytes listed as 0 in Table B9-1 are zero; bytes 39–40 never exceed 32767. |
| T-B9-03 | Known supply and power inputs, converted with the record constants and the Appendix A formulas, match within the stated accuracy. |
| T-B9-04 | Event registers arrive before or with the next status packet after a change. |
| T-B9-05 | Fast registers arrive every 10 ms during TX; a snapshot of all telemetry registers arrives every 1 s. |
| T-B9-06 | An absent sensor is flagged absent in the record and has no register. |

### Transmit (B10)

| Id | Description |
|----|-------------|
| T-B10-01 | With `tx` bit 1 = 0, no arm is accepted and nothing keys. |
| T-B10-02 | An arm with the correct `token XOR ARM_MAGIC` is confirmed by `TX_ARM_STATE` with a new generation; any other value is rejected. |
| T-B10-03 | Each arm precondition of B10.2 item 3, when violated, rejects the arm with its reason. |
| T-B10-04 | A PTT bit already 1 at confirmation does not key; a later 0→1 edge does. |
| T-B10-05 | A stale token from an earlier arm or session does not arm. |
| T-B10-06 | An arm without `TX_MODE_CLASS` is rejected with `NO_MODE_CLASS`; with class AM at full drive and 100 % sine modulation, the carrier estimate settles at or below 25 W plus the margin and the PEP at or below 100 W within 200 ms, or the radio unkeys; the 100 ms mean is then about 37.5 W and is not reduced to 25 W (A3.4c). |
| T-B10-07 | While keyed, no High Priority packet for 500 ms (or the written shorter value) unkeys and keeps the arm. |
| T-B10-08 | `KEYED_TIMEOUT` values above 500 ms or below 100 ms are not accepted. |
| T-B10-09 | Continuous TX for 5 min (or the stricter value) unkeys and keeps the arm. |
| T-B10-10 | A DUC rate other than 192 or bits other than 24 refuses TX IQ and keeps the PA off, with a reason. |
| T-B10-11 | With TX source PTT and CW select clear (DUC-specific byte 5 bit [1] = 0), a DUC underflow longer than the underrun time unkeys with `TX_STATE` reason 3 and keeps the arm; with `KEYED_TIMEOUT` = 100 ms the unkey comes no later than 110 ms after the underflow starts. |
| T-B10-12 | A TX request with DUC0 outside the transmit ranges does not key. |
| T-B10-13 | SEND is asserted only while armed and keyed, before RF, and released on every drop event, standby and fault. |
| T-B10-14 | `ATU_TUNE` without arm does not tune; with arm it tunes within its time limit. |
| T-B10-15 | Not in v1.0 (0030). KEY keys hardware CW only when armed and CW is selected. |
| T-B10-16 | A `KEYED_TIMEOUT` write outside 100–500 is rejected with `BAD_VALUE`; the value returns to 500 at session end; unkey happens between T and T + 10 ms. |
| T-B10-17 | DUC-specific byte 5 bit [0] (EER) set refuses TX IQ with reason 12. |
| T-B10-18 | DUC-specific bytes 14–15 = 0 and byte 16 = 0 are read as 192 ksps and 24 bits. |
| T-B10-19 | `TX_MODE_CLASS` values 0 and above 4 are rejected; the class returns to 0 at session end. |
| T-B10-20 | A repeated correct arm write while armed is answered and does not raise the generation. |
| T-B10-21 | DUC I&Q from a source other than the owner's IP address and port (including the owner's IP on another port) is discarded, never fills the FIFO, has no effect on an armed owner's transmission, and is counted in `GET_STATS`. |
| T-B10-22 | `ATU_TUNE` = 1 while not armed is rejected with `NOT_ARMED` and does not start after a later arm. |
| T-B10-23 | During ATU tuning a PTT edge neither keys nor ends the tune; no High Priority packet for `KEYED_TIMEOUT` ends the tune with `ATU_STATE` = 4. |
| T-B10-24 | With TX source PTT, CW select clear and `TX_MODE_CLASS` = 2 (CW), a DUC underflow unkeys after the underrun time, as in every other class; with TX source CWX or KEY no underrun check applies. |
| T-B10-25 | General byte 58 bit [0] cleared while keyed unkeys with `TX_STATE` reason 7, arm kept; setting it again does not key without a new 0→1 edge. |
| T-B10-26 | With byte 5 bit [0] set, a dash bit held at 1 for 720 ms is not cut; held for more than 1 s it unkeys between 1000 and 1010 ms with `TX_STATE` reason 9, arm kept, also while the keyer forms elements; it keys again only after both bits are 0 and a new 0→1 edge. |
| T-B10-27 | A `TX_MODE_CLASS` write with another value while keyed unkeys first with `TX_STATE` reason 10, arm kept, and then applies; a write of the value in force while keyed changes nothing. |
| T-B10-28 | `ATU_TUNE` = 1 while a tune cycle runs is a no-op and the cycle continues; `ATU_TUNE` = 0 aborts it and the cycle ends unkeyed. |
| T-B10-29 | With byte 5 bit [0] clear, bits [1] and [2] never key; with bit [0] set, a dot or dash 0→1 keys the keyer, whose elements follow the keyer speed of DUC-specific byte 9; bit [0] alone never keys. |
| T-B10-30 | After a tune cycle that fails, is aborted or ends by a takeover, the RF path returns after RF gone to the state for the receive frequency, the ATU to its stored state for that frequency or to bypass, and nothing is stored for the TX frequency; `ATU_STATE` stays 4 until the next tune, and a TX request still keys (reference model; A3.9b item 7). |
| T-B10-31 | `ATU_TUNE` = 1 repeated 100 ms after a cycle ended is a no-op answered with `ATU_STATE`; 2 s after the end a new cycle starts. |
| T-B10-32 | With TX source PTT, CW select clear and no DUC I&Q at the key, the radio unkeys with `TX_STATE` reason 3 between the underrun time and the underrun time + 10 ms after the key; a host that sends its first DUC I&Q packet 1.25 ms after the PTT edge is not cut; while the FIFO is empty the DUC gets zero samples (reference model); the underrun time is at least `t_rf_off` + `t_settle` + 1.25 ms and at most `KEYED_TIMEOUT` (B10.5). |
| T-B10-33 | With CWX selected and keyer speed 20 wpm: with iambic set, both bits held give alternating dots and dashes, mode B adds the opposite element after a squeeze release, and reverse keys set changes nothing; with iambic clear, a dot bit held for 300 ms gives one key-down of 300 ms without element timing, and held for more than 1 s unkeys with reason 9; with keyer speed 0 CWX does not key in either mode (B10.5a). |
| T-B10-34 | Break-in clear, CW selected, armed: a PTT 0→1 edge keys (T/R relay, SEND, `TX_STATE` TX source 1) with no RF until a KEY closure or CWX paddle edge; the elements form RF, SEND and the T/R relay stay in TX between them; no DUC I&Q for 1 s gives no underrun unkey and DUC I&Q sent meanwhile is not transmitted and is counted; a KEY closure already held at the PTT edge forms no RF until it is closed again; with an RF delay of 20 ms (byte 13), a KEY closure or CWX paddle edge after the PTT edge but before the T/R relay has settled and the RF delay has elapsed forms no RF before the PTT key nor after it until a new closure or edge (0022 §87 item 1); at the PTT release the element ends with its trailing ramp, then SEND is released after RF gone and the T/R relay returns (B10.5b). |
| T-B10-35 | Break-in clear, armed, no PTT: KEY closures and CWX paddle edges form no RF, key nothing and are reported in status byte 4; a PTT 0→1 edge after them sends no element; with CW not selected, a CWX paddle edge inside a PTT transmission forms no RF, is not latched, and the DUC I&Q is transmitted (B10.5b). |
| T-B10-36 | Break-in set, armed: inside a PTT transmission a KEY closure or CWX paddle edge forms no RF, the keyer stops, and the PTT transmission continues; during the hang delay of a CWX transmission a PTT 0→1 edge is refused and, still held after the hang delay, does not key until a new 0→1 edge; a change of byte 5 bit [7] or bit [1] while keyed applies only after the unkey (B10.5b). |
| T-B10-37 | Not in v1.0 (0030). Break-in set, armed, hardware CW selected, hang delay 500 ms: a CWX element, then within the hang delay a KEY element, then a CWX element form one CW transmission (`TX_STATE` keyed throughout, SEND and the T/R relay in TX, the maximum continuous TX time counted from the first element); a KEY dot and a CWX dash held together form the elements of the iambic keyer as one paddle pair (B10.5b item 1; 0022 §87 item 5). |
| T-B10-38 | With TX source PTT and CW select clear, a DUC-specific packet with EER set, then one with DUC0 rate 384, arriving while keyed: the transmission continues and DUC I&Q is still transmitted; after the PTT release `STREAM_STATUS` shows the refusal and a new PTT 0→1 edge does not key until a valid DUC-specific packet (B10.6, refusal arriving while keyed; 0022 §87 item 8). |
| T-B10-39 | With break-in clear, CW selected and `t_settle` 20 ms (reference model): a CWX paddle bit raised during the settle and held, then the other paddle bit raised while the first is still held, forms no RF; after both bits at 0 and a new rise it forms RF; with CW not selected, a CWX paddle edge in the PTT transmission forms no RF and is not latched (B10.5b items 2.2, 2.3; 0022 §88 item 3). |
| T-B10-40 | With break-in clear, CW selected and no element, a PTT bit held by High Priority packets every 50 ms with no DUC I&Q is not unkeyed by the underrun check and is unkeyed by the maximum continuous TX time (set stricter, 10 s), counted from the key of A3.1a, not from the end of the RF delay (B10.5, A3.4d; 0022 §88 items 2, 4). |
| T-B10-41 | With record `tx` bit 1 = 1 and bit 4 = 0 (transmit parameters of Annex P not set), an arm that meets every other condition of B10.2 item 3 is rejected with `NOT_CALIBRATED`, and `ATU_TUNE` = 1 with write reject reason `NOT_CALIBRATED`; with bit 4 = 1 (reference model, simulated values) the arm is accepted (Annex P; 0035, 0036). |
| T-B10-42 | With record `tx` bit 4 = 0, DUC I&Q from the owner is discarded and counted, `STREAM_STATUS` DUC0 reads not running with reason 14, and PTT and CWX key nothing and are not latched (B10, B10.7; 0039). |
| T-B10-43 | With CWX, iambic set, *S* = 20 wpm and *W* = 50: a held dot paddle gives marks of 60 ms and spaces of 60 ms, a held dash paddle marks of 180 ms and spaces of 60 ms, within the tolerance of B10.5a item 6; with *S* = 60 the unit is 20 ms; *S* = 70 is used as 60, a log entry contains `CW speed 70 used as 60` and `GET_STATS` `cw_speed_clamped` rises by one per packet; a dot paddle edge while idle keys within 5 ms of the packet's arrival (reference model with a complete calibration set, or emulator; B10.5a items 1, 4; 0046). |
| T-B10-44 | Weight: at *S* = 20 wpm, *W* = 66 gives dot marks of 79.2 ms with spaces of 40.8 ms and *W* = 33 dot marks of 39.6 ms with spaces of 80.4 ms, a dot plus space staying 120 ms; *W* = 80 is used as 66, *W* = 0 as 50 (B10.5a item 3; 0046). |
| T-B10-45 | Strict character spacing: with bit [6] set, a paddle pressed 1.5 *T* after the end of a mark and held starts its element 3 *T* after that end; one pressed at 1.5 *T* and released at 2.5 *T* starts nothing; with bit [6] clear a paddle pressed at 1.5 *T* starts its element at once, within 5 ms (B10.5a items 4, 5; 0046). |
| T-B10-46 | Iambic, dash paddle held: (a) dot paddle also pressed during the dash mark and both released before its end: mode A ends after the dash, mode B sends one dot after it and stops; (b) dot paddle pressed and released during the dash mark while the dash paddle stays held: mode A sends another dash, mode B a dot and then dashes; (c) dot paddle pressed only during the space after the dash: no dot from memory in either mode, the decision point sees what is held (B10.5a item 4; 0046). |
| T-B10-47 | An ATU tune cycle (reference model with a complete calibration set) that ends tuned gives `TX_STATE` unkey reason 11 and `ATU_STATE` 3; one that ends failed by itself reason 11 and `ATU_STATE` 4; one aborted by `ATU_TUNE` = 0, and one aborted by a stop request, reason 1 and `ATU_STATE` 4; one stopped by the ATU tune time reason 4 with limit id 8 (A3.9b item 6; 0047). |

### Error handling (B11)

| Id | Description |
|----|-------------|
| T-B11-01 | Each packet type shorter than Table B11-1 is discarded with no effect (no watchdog feed, no takeover). |
| T-B11-02 | A truncated High Priority packet with PTT set does not key. |
| T-B11-03 | Unknown command bytes on port 1024 get no reply and change nothing. |
| T-B11-04 | Packets with arbitrary or repeated sequence numbers are accepted; gaps on ports 1027 and 1029 are counted in `GET_STATS`. |
| T-B11-05 | Under forced RX overrun only whole DDC0 packets are lost, the gap shows in the sequence numbers, and status byte 30 bit [0] is set. |
| T-B11-06 | DUC I&Q arriving faster than 192 ksps loses whole packets only. |
| T-B11-07 | A DDC0 frequency outside `rx_range` is tuned without refusal. |
| T-B11-08 | Datagrams to port 1028 and other closed ports get no reply from the firmware; whether the CH394Q sends ICMP is recorded. |
| T-B11-09 | With the hardware timer disabled and the owner gone, the session stays until a takeover; an ICMP port unreachable is logged and does not end it. |
| T-B11-10 | With a host that numbers its High Priority packets, a duplicate or older packet (PTT 1 after a newer PTT 0) is discarded and counted and does not key; numbering across 0xFFFFFFFF → 1 is accepted as newer; a packet numbered 0 after 0xFFFFFFFF is discarded (B11.6a). |
| T-B11-11 | With unnumbered High Priority packets, PTT 1, 0, 1 within 5 ms keys once only; a 1 that arrives 10 ms or more after the latest packet with PTT 1 gives a new key (B11.6a). |
| T-B11-12 | C&C, DUC I&Q and register packets from outside the radio's subnet start no session, take nothing over and get no answer; discovery from there is still answered (B11.15). |
| T-B11-13 | The test vector of B11.6a, sent unnumbered after an arm confirmed at 0 ms, gives exactly the stated results on the reference model; on the radio the 8 ms and 12 ms cases give the stated results (B11.6a). |
| T-B11-14 | Numbered packets 1, 2: unnumbered rule still applies (a PTT edge 5 ms after a packet with PTT 1 is ignored); after 3 the mode is numbered and a duplicate of 2 is discarded and counted; a single packet with number 0 after 3 is discarded (B11.6a). |
| T-B11-15 | In numbered mode after 1000 a host restarts at 1: packets 1 to 4 are discarded and counted, packet 5 resynchronises (logged, counted) and its PTT 0→1 does not key, a later edge keys under the 10 ms rule; a packet older by more than 2¹⁶ resynchronises at once; a 0→1 edge in a discarded packet never keys (B11.6a). |
| T-B11-16 | In numbered mode while keyed by PTT, an older packet with PTT 0 is discarded but unkeys (`TX_STATE` reason 1, arm kept); later accepted packets with PTT 1 do not key; an accepted PTT 0 and then 1 keys (B11.6a). |

---

## Sources

- [P2] openHPSDR Ethernet Protocol V4.4: "Detailed operation" (session, watchdog, standby,
  restart, NOTE2); "openHPSDR Host to Hardware Protocol"; "openHPSDR Hardware to Host
  Protocol"; Discovery packet; Erase, Program, Set IP Address packets; General packet bytes
  4–59; DDC specific packet bytes 4–1443; DUC specific packet bytes 4–59; High Priority data
  packet bytes 4–1443; DUC I&Q Data packet; Microphone Data packet; Wideband Data packet;
  Memory mapped registers from PC / from hardware; Discovery reply bytes 0–59; Command reply
  packet (including "Set IP"); DDC packet bytes 0–1443; High Priority Status packet bytes
  0–59; Appendices A, B, C; revision history (V2.7, V3.8).
- [CH394] WCH CH394 manual 1, V1.2: sections 1–3, 7.1.2–7.1.7, 7.1.13–7.1.15 (UNIP, UNPORT,
  PHY_CFG), 7.2, 12.2.4.
- [G7250-MAN] Xiegu G7250 Operation Manual rev 0.1; [G7250-PUDX], [G7250-RIGPIX] (50 MHz
  power, secondary).
- [TCI] Transceiver Control Interface 2.1 (read in part).
- [FIPS180-4] NIST FIPS 180-4 (SHA-256); [RFC2104] HMAC (`sources.md`).
- [AETHER-SRC] AetherSDR source, commit 9f7cbeb; [PIHPSDR] piHPSDR source, commit 422bec5
  (`sources.md`; used for the DDC rate unit and host behaviour, B3.2, B6.5, 0020).
- `analysis/ch394-network-budget.md`; `open-questions-his.md` (HIS-01 to HIS-20).
- Decisions 0001, 0002, 0003, 0004, 0005, 0006, 0007, 0008, 0009, 0011, 0012, 0014, 0015,
  0016, 0017, 0018, 0019, 0020 (proposed), 0021 (§2, §8, §10, §14, §16), 0022 (proposed);
  design note §6.
- RFC 2119, RFC 8174 (BCP 14); RFC 1982 (serial-number arithmetic, B11.6a); RFC 1812 section
  5.3.5.1 (limited broadcast not forwarded by routers, B3.9).
