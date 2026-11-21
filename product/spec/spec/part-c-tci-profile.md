# Part C — Level 2 and the G7250 TCI profile

- Version: 1.0, 2026-11-21, accepted by ON8ST; changes in `spec/CHANGELOG.md`
- Status: version 1.0 (tag `spec-v1.0`); applies 0021 and 0022, accepted by ON8ST, and the
  later decisions that concern level 2 (`product-handover.md`). This file is part C of the
  network interface specification; `spec/network-interface.md` points to it ("Part C — Level
  2: TCI"). Level 2 is not offered before gate G6 (0011).
- © 2026 ON8ST, CC BY 4.0 (see `LICENSE.md`); quoted TCI material © Expert Group LLC under its
  MIT notice.
- Builds on: 0011 (two levels, single owner, shared safety core, level 2 only after G6), 0012
  (full functionality), 0013 (AetherSDR), 0014 (one G7250 TCI profile), 0004 (rates), 0006
  (telemetry), 0007 (watchdog and safety), 0008 (management channel, socket budget), 0015 (TX
  arm), 0017 (no audio on the main unit), 0019 (reference model), 0021 (profile choices),
  0022 (rc1 review fixes), and `spec/network-interface.md` 1.0
  (parts A and B, with 0022 §62–§88; Table B8-3 is the single list of code names, 0022 §19).

Sources are cited by key (see `sources.md`) or by decision number (see `decisions/`).
Citations of [TCI] give the section of TCI Protocol 2.1 and the command name, for example
"[TCI] §4.2 DDS". Values that depend on an open question for the HIS or on gate G6 are
written with the HIS id or "G6" and are listed in Annex C-A. Design choices are recorded in
0021 and cited as "0021 §n". Statements marked **[unverified]** rest on an assumption that has
not been checked against a public source; the profile rule stands meanwhile, and they are
listed in Annex C-D.

**Notice for quoted material.** Short quotations from [TCI] in this part are taken from "TCI
Protocol, Ver. 2.1", September 2026, which carries this notice: "Copyright (c) 2023 "Expert
Group" LLC, Taganrog. Permission is hereby granted, free of charge, to any person obtaining a
copy of this software (the "Software"), to deal in the Software without restriction, including
without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense,
and/or sell copies of the Software, and to permit persons to whom the Software is furnished to
do so, subject to the following conditions: The above copyright notice and this permission
notice shall be included in all copies or substantial portions of the Software. THE SOFTWARE
IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT
LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE."
Whether the notice must accompany short quotations is not settled; it is kept here to be safe
(`sources.md`, [TCI], Terms).

---

## C1 Scope and conformance language

C1.1 This part defines the **G7250 TCI profile**: the TCI version, the commands, arguments and
streams used, and the G7250 extensions (0014). It applies to two implementations (0014,
Decision):

- (a) **Level 1:** the TCI server of AetherSDR, when AetherSDR controls a G7250 over G7250-P2
  (0013, 0016). AetherSDR's TCI server announces protocol 2.1 [AETHER].
- (b) **Level 2:** the TCI server in the G7250 main-unit firmware, with DSP on board (0011
  point 1).

C1.2 Level 2 is built only after feasibility gate G6 (HIS answers on compute, memory and
network stack; 0011 point 4; design note §6). Milestone M6 covers receive, M7 transmit. Until
G6, every requirement in this part that is marked **(L2)** is a target, not a commitment.
Requirements marked **(L1)** apply to AetherSDR only. Unmarked requirements apply to both.

C1.3 "The server" means a conforming TCI server of either level. "A client" means a TCI client
connected to it. "The profile" means this part. "The safety core" means the watchdog, TX arm,
TX limits and fault state of 0007, which sit below both levels (0011 point 3).

C1.4 The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD
NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY" and "OPTIONAL" in this part are to be interpreted
as described in BCP 14 (RFC 2119, RFC 8174) when, and only when, they appear in all capitals,
as shown here (as in part B, B1.2).

C1.5 **Normative reference.** [TCI] Transceiver Control Interface, protocol 2.1, September
2026 (`sources.md`). The server MUST behave as [TCI] specifies for every command it supports,
unless this part states a deviation (C1.7). [TCI] describes the behaviour of ExpertSDR3; where
it names ExpertSDR3 as the server, the profile reads "the server".

C1.6 **Extensions.** [TCI] states that it "uses an extensible architecture and can be
supplemented with new functions and commands, while keeping the old ones operational", and
that "the extensibility of the TCI interface allows you to create an individual set of
commands and functions for each device model, while maintaining the basic command set inherent
to all transceivers" ([TCI] §1.4). G7250 functions that no TCI 2.1 command describes with
identical meaning are carried by G7250 extension commands (C5), all with the prefix `G7250_`.
A standard TCI command MUST NOT be given a G7250 meaning that differs from [TCI] (the same rule
as 0005 for P2). The reserved commands of [TCI] §4.6 MUST NOT be used for G7250 functions.

C1.7 **Deviations and clarifications.** Every deviation from [TCI] is stated where it applies
and listed in Annex C-B. Where [TCI] is silent or ambiguous, the reading the profile follows is
listed in Annex C-C.

C1.8 **Profile version.** This draft defines profile version `1`. The version is reported by
`G7250_PROFILE` (C5.3).

---

## C2 Transport

C2.1 The server MUST offer TCI as "a full duplex web socket protocol that runs on top of a TCP
connection", with the transceiver as the server ([TCI] §1.4). WebSocket is [RFC6455].

C2.2 Commands are ASCII strings ([TCI] §3.1) and MUST be sent in WebSocket text messages;
stream blocks ([TCI] §3.4) MUST be sent in WebSocket binary messages ([TCI] §3.4: "commands are
transmitted as strings and audio streams are transmitted as byte streams"). Clarification
(Annex C-C, C-C1): the server MAY put several complete commands in one text message and MUST
accept several complete commands in one message from a client; a command MUST NOT be split over
messages, and a binary message carries exactly one stream block; a binary message from a client
that does not hold exactly one complete block is discarded (Annex C-B, C-B20; 0021 §43 item
17), as is a block whose length or header C6.3.5 does not accept; a message discarded for its form is
not an application frame (C8.2; 0021 §44 item 13). A message MAY be fragmented
into several frames ([RFC6455] §5.4); the server MUST accept fragmented messages, and a message
counts, for every rule of this part (application frames and the timers of C8.2), only when its
last fragment has arrived (0021 §40). In this part "frame" in "application frame", "text frame"
and "binary frame" means a complete WebSocket data message; Ping, Pong and Close are control
frames ([RFC6455] §5.5).

C2.3 No TLS and no WebSocket subprotocol are used. [TCI] defines neither. The server MUST
accept the HTTP upgrade on any request path and gives the path no meaning (0021 §17.1; Annex
C-C, C-C2).

C2.4 **Port.** [TCI] does not state a default TCP port (`sources.md`, [TCI]). The port MUST be
configurable. The default is **50001** on both levels (0021 §2). This is a profile choice,
not a TCI standard; it is the default of AetherSDR's TCI server ([AETHER-SRC], per
`annex-aethersdr.md` §2.4), so an application finds both levels on the same port. **(L2)** On
level 2 the port is set through the management channel configuration (0008); a change applies
as C2.10 item 4. The port MUST NOT
collide with the management port or with port 1024 (0008, Transport point 2). **(L1)** On
level 1 the port is set in AetherSDR.

C2.4.1 **Reach (L2).** The server MUST accept TCI connections only from source addresses
inside the radio's own subnet (as 0008 S4), and, when the TCI allow-list is not empty, only
from addresses that match it. The allow-list holds up to 8 IPv4 addresses or prefixes and is
set through the management channel as an authenticated configuration write (0008 S2; 0021
§10). A refused connection MUST be closed without the WebSocket handshake and logged with its
source address (0008 L-2). **(L1)** AetherSDR SHOULD offer an equivalent restriction (0021
§10).

C2.5 **Number of clients, level 2.** The CH394Q has 8 hardware sockets [CH394], and each TCP
connection takes one socket (0008, Socket budget, [CH394] §7.2.2; `analysis/ch394-network-budget.md`,
Addendum, "TCI (level 2)"). While level 2 is active:

| Socket use | Sockets | Source |
|------------|---------|--------|
| Management channel, always reserved | 1 | 0008, Transport point 1 |
| P2 port 1024; discovery stays answerable on level 2 | 1 | 0003 (discovery reply byte 4); part B B3.1; 0021 §3 |
| Spare TCI listener, so that a further client can always be accepted and refused cleanly | 1 | 0008, Socket budget |
| TCI client connections | 5 | 8 − 3 |

- C2.5.1 **(L2)** The server MUST serve **5** simultaneous clients (`clients_max` = 5; 0021
  §3), on the assumption of C2.5.3; if HIS-14 shows otherwise, the number is recomputed from
  the same table. The number MUST be reported in `G7250_PROFILE` (C5.3).
- C2.5.2 **(L2)** When all client sockets are in use, the server MUST accept the next
  connection on the spare listener, complete the WebSocket handshake, send
  `G7250_REFUSED:busy;` in a text frame, close with WebSocket status 1008 (RFC 6455 §7.4.1),
  and return the socket to listening. The extension command is sent before the opt-in of C5.2
  on purpose, because the connection ends at once.
- C2.5.3 **[unverified]** That a CH394Q socket in TCP server mode turns into the connected
  socket on accept, so that a new listener needs another socket, is assumed from how such
  socket chips generally work; [CH394] does not spell it out (`analysis/ch394-network-budget.md`,
  Addendum). To be checked in the full CH394 documentation (HIS-14).
- C2.5.4 DHCP lease renewal borrows the management socket (0008, Socket budget) and does not
  reduce the number of TCI clients. An address change by DHCP is C5.5.5 row 14 (A1.11a; 0022
  §54).
- C2.5.5 **(L1)** AetherSDR runs on the host and is not bound by the CH394Q socket budget. It
  MAY serve any number of clients.

C2.6 **Throughput, level 2.** All TCI traffic crosses the same SPI link as level 1 traffic, so
the SPI design target of at most 60 % busy in the worst case applies (0004, Decision point 6;
`analysis/ch394-network-budget.md`, Addendum). The WebSocket framing and the TCI commands run
on the G7250 processor, not in the CH394Q; client frames are masked (RFC 6455 §5.3) and must be
unmasked by the processor (`analysis/ch394-network-budget.md`, Addendum; HIS-18, G6). Stream
budgets are in C6.4.

C2.7 **Level change.** **(L2)** The TCI server listens only while level 2 is the active level
(0011 point 2). Level selection is done through the management channel and part A (0011;
0008, C-2 and I-4; A2.5a), not over TCI. On a switch from level 2 to level 1, after the
validation of A2.5a step 1, the server MUST send, in this order, items 1 and 2 before the
management reply of A2.5a step 3 (0021 §25; 0022 §9, §61):

1. if keyed: unkey, and the unkey broadcast of C7.7 (`TRX:0,false;`, `TUNE:0,false;`);
2. if armed: disarm with reason `LEVEL_CHANGE` (0007, drop event 10), then
   `G7250_TX_ARM_STATE` and then the new `G7250_TX_ARM_TOKEN` to opted-in clients, and
   `TX_ENABLE:0,false;` to all clients (A2.5a step 2; message order of C5.5.5);
3. no further stream blocks; `G7250_STREAM_STATE` `stopped`, reason `level_change`, for each
   stream that was running, to the client that had it (A2.5a step 4);
4. `G7250_PROFILE` with `active` false to opted-in clients (C5.3);
5. a WebSocket close with status 1001 "going away" on every connection (RFC 6455 §7.4.1);
   then the listener closes (A2.5a step 6).

**(L1)** On a switch from level 1 to level 2 while armed or keyed, AetherSDR receives the P2
drop `LEVEL_CHANGE` before port 7250 closes (A2.5a step 2), sends items 1–4 to its TCI
clients, and keeps its TCI connections open (0021 §25). While unarmed and unkeyed the radio
sends no drop (A2.5a step 2 applies only while armed or keyed), so AetherSDR sees only its
streams stop. Before it reports a loss of ownership (C7.2.1), AetherSDR MUST check the active
level, which the radio shows after the switch (A2.5a step 7: management identification I-4),
and when level 2 is active it MUST report the end as a level change: items 3 and 4 with reason
`level_change`, and `STOP;` to all clients (C2.9a), never `takeover` (0021 §41 item 9; §43 item 18).

C2.8 **Firmware update while level 2 is active** (L2) (0022 §45; A1.8). `UPDATE_BEGIN` is
accepted only while unarmed and unkeyed (A1.8, status 50 otherwise). A `G7250_TX_ARM` and
`UPDATE_BEGIN` serialise as A1.8 states: one wins atomically, the other is refused (arm:
`MAINTENANCE`; update: status 50) (0022 §61). From `UPDATE_BEGIN` on, the server MUST:

1. stop every stream and send `G7250_STREAM_STATE` `stopped`, reason `update`, for each
   stream that was running, to the client that had it;
2. if the device was started, stop it and send `STOP;` to all clients (C2.9a). No unkey, drop
   or `TX_ENABLE` message is needed: `UPDATE_BEGIN` is accepted only while unarmed and
   unkeyed, so `TX_ENABLE` is already `false` (C5.5.6 item 8, unarmed; `TX_ENABLE` does not
   depend on item 3, 0021 §41 item 4, §44 item 1);
3. keep the TCI connections open and keep answering, but follow no change: a set request of a
   standard command is answered with the value in force, and the opted-in requester receives
   `G7250_WRITE_STATUS` with `BUSY`; an extension set request is rejected with `BUSY`;
   `G7250_TX_ARM` is rejected with `MAINTENANCE` in `G7250_TX_ARM_STATE`; `START` is not
   followed; `IQ_START`, `AUDIO_START` and `LINE_OUT_START` are answered with
   `G7250_STREAM_STATE` `refused`, reason `update`; notifications (`G7250_SAFETY`,
   `HW_SENSORS`) continue;
4. just before the restart (after `UPDATE_ACTIVATE`, `RESTART`, `ROLLBACK` or the
   abandoned-update timeout of A1.8), close every connection with WebSocket status 1001 "going
   away" (RFC 6455 §7.4.1), then carry out the commanded reset of A1.9. The close comes
   after RF is gone and the reply is sent (A1.9 steps 2–3); if the reset is refused because
   RF stays (status 59, A1.9 "RF not gone"; 0022 §59), the connections stay open and
   `G7250_SAFETY` reports the fault.

The state lasts until the restart, also after `UPDATE_ABORT` (A1.8). After the restart, while
the new image is an unconfirmed trial boot, RX works and arm requests are rejected with
`MAINTENANCE` (A1.8; C5.5.6 item 5). **(L1)** On level 1 the P2 session of AetherSDR ends at
`UPDATE_BEGIN` (A1.8), which the radio accepts only while unarmed and unkeyed; AetherSDR sends
its TCI clients items 1 and 2 and keeps the connections open.

C2.9 **Link below 100 Mbit/s** (L2) (0022 §51; B5.8). On a link below 100 Mbit/s the server
keeps listening and accepts connections but does not start the device: `START` is not
followed, stream starts are answered with `G7250_STREAM_STATE` `refused`, reason `link`, and arm
requests are rejected with `NOT_RUNNING`. A renegotiation below 100 Mbit/s while started is
drop event 9 of C5.5.5 (`LINK_DOWN`), stops the streams (`G7250_STREAM_STATE` `stopped`,
reason `link`) and stops the device, with `STOP;` to all clients (C2.9a); while a client is connected, a
return to 100 Mbit/s restarts nothing by itself and sends no `START;` (automatic start: C4
`START`). This is the level-1 rule
(B5.8; 0004 point 9); an IQ stream does not fit a 10 Mbit/s link (C6.4).

C2.9a **Device stopped by the server** (0021 §41 item 4). `START` and `STOP` are bidirectional
state ([TCI] §4.2 START, STOP), synchronised to all clients ([TCI] §3.1). Whenever the device
stops without a client `STOP` (drop event 9 `LINK_DOWN`, `UPDATE_BEGIN` of C2.8, and (L1)
every end of AetherSDR's P2 session other than one caused by a client `STOP`: a takeover, a
watchdog standby, a link loss, an inferred loss, a level change or a firmware update, C7.2.1,
C2.7, C2.8; 0021 §46 item 5), the server MUST send `STOP;` to all clients after the messages of the section
that applies, so that a generic client sees that the device no longer runs. A later start
follows C4 `START`: (L2) the automatic start where its conditions hold (after a reset, C8.5,
after a switch to level 2, at the end of the TCI session, and while no client is connected on the return of the
link to 100 Mbit/s), otherwise a client `START`; (L1) a client `START` under the confirmation
rule of C4 `START` (0021 §45 item 4).

C2.10 **Commanded reset and configuration changes while level 2 is active** (L2) (0021 §35;
A1.6, A1.6a, A1.9).

1. **`RESTART` and `FACTORY_RESET`.** The management channel accepts them while armed or
   keyed (A1.9). The server MUST then, before the management reply (A1.9 steps 1–3): (a) if
   keyed, unkey and send the unkey broadcast of C7.7; (b) if armed, disarm with reason
   `MAINTENANCE` (C5.5.5 row 11 for `RESTART`, row 15 for `FACTORY_RESET`), also when the
   reset is then refused with status 59, and send the drop messages of C5.5.5; after the reset
   the state reads `RESET` (row 6) (A1.9; 0022 §73 item 2). After RF is gone and the reply is sent (A1.9 steps
   2–3) it stops every stream, closes every connection with WebSocket status 1001 "going
   away" (RFC 6455 §7.4.1), and the reset follows (A1.9 steps 4–5). After a factory reset
   the TCI port is 50001 and the allow-list is empty (A1.6a, Table A1-4).
2. **RF not gone** (A1.9, "RF not gone"; 0022 §59). If the reset is refused with status 59,
   the connections stay open, TX stays unkeyed and disarmed, RX and the RX streams continue,
   and `G7250_SAFETY` reports the fault (`RF_NOT_KEYED`); arm requests are rejected with
   `IN_FAULT` (C8.4). Every later commanded reset follows the same rule.
3. **Allow-list change (C-7).** When a new allow-list is applied, every open TCI connection
   from an address it no longer allows MUST be closed with WebSocket status 1008 (RFC 6455
   §7.4.1) and logged (A1.6 C-7). If that connection is the keying client, the server unkeys
   (`unkey` 8; C8.2); if it holds the arm, the server drops the arm as drop event 3
   (`HOLDER_LOST`), unkeying first. Both happen with the messages of C5.5.5 and C7.7 before the
   management reply and before the connection is closed, as A1.9 requires for a C-7 list that
   closes the holder's or the keying client's connection (A1.9; 0022 §73 item 18).
   Connections the new list still allows are not touched, and the TX state stays unchanged
   when no such connection is closed.
4. **TCI port change** (C2.4). A new TCI port applies after the management reply (A1.6, "a
   network change is applied after the reply"): the one listening socket (C2.5: a single
   socket listens at any time, and once all client sockets are in use it is the spare
   listener) moves to the new port; open connections on the old port MUST be kept, with their
   arm and key state, until they close. A client that reconnects uses the new port (0021 §44
   item 9).

---

## C3 Initialization

C3.1 On every new connection the server MUST send the initialization commands of [TCI] §4.1,
then `READY;`, and then the current state of every supported bidirectional parameter ([TCI]
§3.1: "the client receives the current status ... first sending initialization commands, then
parameters to set the status"; [TCI] §4.1 READY: "Sent after the initialization commands").

C3.2 Initialization commands and their G7250 values, in this order:

| # | Command | G7250 value | Status | Source / reason |
|---|---------|-------------|--------|-----------------|
| 1 | `PROTOCOL:ExpertSDR3,2.1;` | the same on both levels; arg1 is a compatibility value, not an identification of the G7250 | MUST | [TCI] §4.1 PROTOCOL ("arg1 — program name", "arg2 — version of the protocol"); [AETHER-SRC] (AetherSDR sends this value, `annex-aethersdr.md` §2.4); 0021 §4 |
| 2 | `DEVICE:<name>;` | (L2) `G7250` MUST; (L1) `G7250` SHOULD, AetherSDR MAY keep `AetherSDR` where a client needs it | MUST | [TCI] §4.1 DEVICE; [TCI] §1.4 ("maintaining the device model designation"); [AETHER-SRC] (`device:AetherSDR` for WSJT-X behaviour, `annex-aethersdr.md` G-13); 0021 §5. Clients recognise the profile by `G7250_EXT` (C5.1.3), not by `DEVICE`. |
| 3 | `RECEIVE_ONLY:<bool>;` | `false` when transmit is implemented and enabled; `true` on level 2 before M7, and on level 1 if AetherSDR's G7250 backend cannot transmit yet | MUST | [TCI] §4.1 RECEIVE_ONLY; design note §6 (M6/M7, M4); 0012 (RX-only only as an intermediate milestone) |
| 4 | `TRX_COUNT:1;` | 1 | MUST | [TCI] §4.1 TRX_COUNT; one transmitter [G7250-MAN] (0007, rule 4); one receiver in this version (0004; HIS-02, [HIS] HIS 80) |
| 5 | `CHANNELS_COUNT:2;` | 2 (channel A and B in receiver 0) | MUST | [TCI] §4.1 CHANNELS_COUNT. Channel B is needed as the split TX frequency (C4, SPLIT_ENABLE). Whether channel B can also be demodulated is separate (C4, RX_CHANNEL_ENABLE). The old name `CHANNEL_COUNT` MUST NOT be sent ([TCI] §4.1 note). |
| 6 | `VFO_LIMITS:500000,54000000;` | 0.5–54 MHz | MUST | [TCI] §4.1 VFO_LIMITS; receive range 0.5–30 MHz and 50–54 MHz [G7250-MAN]. VFO_LIMITS has a single range, so 30–50 MHz is inside it but not receivable; see C3.3. |
| 7 | `IF_LIMITS:<-f>,<+f>;` | ± half the receiver bandwidth in use (L2: pending G6, P-L2-1; HIS-01; L1: AetherSDR's DSP) | MUST | [TCI] §4.1 IF_LIMITS ("for VFOA only"; "Sent when you connect a device or change the sample rate") |
| 8 | `MODULATIONS_LIST:...;` | see C3.4 | MUST | [TCI] §4.1 MODULATIONS_LIST; modes [G7250-MAN] |
| 9 | `READY;` | — | MUST | [TCI] §4.1 READY |

C3.3 **Frequencies outside the receive range.** A `VFO` or `DDS` set request whose frequency
lies in 30–50 MHz (outside the receive range of [G7250-MAN]) or outside `VFO_LIMITS`, and an
`IF` set request that would move the channel frequency (`DDS` + `IF`) there, MUST be
treated as not applicable (0021 §41 item 3): the server MUST NOT tune there and MUST reply with the current value
(the same pattern [TCI] uses for unsupported values, for example §4.3 IQ_SAMPLERATE note).
Listed as a clarification (Annex C-C, C-C3).

C3.4 **Modulation list.** [TCI] does not give a complete list of mode names; the names below are
those that appear in [TCI] (§4.1 MODULATIONS_LIST example "AM,LSB,USB,FM", §4.2 MODULATION
example "NFM", §4.2 RX_BIN_ENABLE note "AM, LSB, USB, CW and NFM", §4.2 DIGL_OFFSET/DIGU_OFFSET,
§3.4 "DIGL/DIGU"). The G7250 modes are CW, AM, SSB, FM, WFM [G7250-MAN].

| TCI name | G7250 mode [G7250-MAN] | RX | TX | TX mode class (C5.6) | Status |
|----------|-------------------------|----|----|----------------------|--------|
| `AM` | AM | yes | yes, 25 W max [G7250-MAN] | `am` | MUST (0021 §6) |
| `LSB`, `USB` | SSB | yes | yes, 100 W [G7250-MAN] | `ssb` | MUST (0021 §6) |
| `CW` | CW | yes | yes, 100 W [G7250-MAN] | `cw` | MUST (0021 §6) |
| `NFM` | FM (narrow) | yes | yes, 100 W ("NFM" in [G7250-MAN] TX power) | `fm` | MUST (0021 §6, §13) |
| `WFM` | WFM | yes | **no**: no WFM transmit power is given in [G7250-MAN] | `none` (arm and TX refused) | SHOULD (L2: G6, compute; 0021 §6) |
| `DIGL`, `DIGU` | SSB used for data modes | yes | yes, 100 W [G7250-MAN] | `ssb` | SHOULD ([TCI] §4.2 DIGL_OFFSET; 0021 §6) |

The narrow FM mode is announced as `NFM`; a set request with `FM` (the name in the [TCI] §4.1
example) MUST be accepted as `NFM`, and the reply is `NFM` (0021 §13). Mode names are sent in
upper case; set requests are case-insensitive ([TCI] §3.1).

C3.5 **G7250 extension state.** Extension commands are not part of initialization. They are
sent only to a client that has opted in (C5.2), so that generic clients see only standard
TCI 2.1 traffic.

---

## C4 Command subset

C4.1 The table lists every TCI 2.1 command of [TCI] §4 and its status in the profile.
"Supported" commands MUST behave as [TCI] specifies, with the G7250 notes given. For a command
marked NOT SUPPORTED, the server MUST ignore it without a reply, as [TCI] does for an unknown
command ([TCI] §4.7 note 5), and MUST NOT send it. For a MAY or SHOULD command that a server
does not implement, the same rule applies. Every MAY or SHOULD command, and every optional
part of a command (demodulation of channel B, a MAY part of the MUST `RX_CHANNEL_ENABLE`; audio
rates below 48 kHz on level 2, a SHOULD part of the MUST `AUDIO_SAMPLERATE`; IQ at 384 kHz,
a conditional part of the MUST `IQ_SAMPLERATE`; the microphone sources of the MUST `TRX`,
conditional on HIS-17; `fast`, a MAY part of the SHOULD `AGC_MODE`), has exactly one token in
Table C5-1, and the server MUST list in `G7250_FEATURES` (C5.4) the token of each one it
implements, so a client can discover it (0014, Consequences; 0021 §44 item 6, §45 item 10). A
missing whole command is ignored without a reply; a missing part is refused as the row of the
command states (0021 §45 item 3).

C4.2 "trx" is the transceiver number, always 0 (`TRX_COUNT:1`); "ch" is the channel, 0 (A) or
1 (B). Commands with a trx or receiver number out of range are ignored ([TCI] §4.7 note 5).

### Device and frequency

| Command | Status | G7250 note | [TCI] |
|---------|--------|------------|-------|
| `START` | MUST | Starts receive. On level 1, AetherSDR maps it to its P2 session start. (L1) If the radio answers discovery with byte 4 = 0x03 (another host has a session, or level 2 is active), AetherSDR MUST NOT start a session, which would take the radio over, on a TCI `START` without confirmation by its own user; until then `START` is not followed (0022 §47; A2.4 host guidance; [P2] Discovery reply byte 4 NOTE). (L2) Not followed during a firmware update (C2.8) or on a link below 100 Mbit/s (C2.9). (L2) **Automatic start** (0021 §43 item 9; target until G6, C1.2): generic clients do not send `START`, so the device is started without a client `START` at power-on with level 2 active, after every switch to level 2, and when a TCI session ends (C5.10.1 item 4); while no client is connected, a return of the link to 100 Mbit/s starts it as well. An automatic start needs a link of 100 Mbit/s and no firmware update (C2.8, C2.9); otherwise the device stays stopped until a client `START`. A client `STOP` stays in force until a client `START` or the end of the TCI session. The state is sent at connection (C3.1). | §4.2 START |
| `STOP` | MUST | Stops the device. Unkeys and drops the arm, reason `STOP` (analogue of P2 RUN cleared, 0007 drop event 2; Table B8-3 drop reason 13). (L1) AetherSDR clears the P2 run bit (High Priority byte 4 bit [0] [P2]); the radio's drop `RUN_CLEARED` unkeys first (A3.2), and AetherSDR reports `STOP`, not `RUN_CLEARED`, to its TCI clients (0022 §32). | §4.2 STOP |
| `DDS` | MUST | Receiver centre frequency. Range C3.3. Moves the effective TX frequency when the channel frequency follows it; changes while keyed: C7.6. | §4.2 DDS |
| `IF` | MUST | Offset of channel A/B within the receiver passband, inside `IF_LIMITS`. For the TX channel it moves the effective TX frequency; changes while keyed: C7.6. | §4.2 IF |
| `VFO` | MUST | Channel frequency (= DDS + IF). Range C3.3. Changes while keyed: C7.6. | §4.2 VFO |
| `RX_ENABLE` | MUST | Only trx 0, always `true`; "The first transceiver is always enabled". | §4.5 RX_ENABLE |
| `RX_CHANNEL_ENABLE` | MUST for ch 0; MAY for ch 1 | Ch 0 is always enabled. Demodulating ch 1 at the same time needs a second demodulator (L2: G6). If not implemented, a set to `true` for ch 1 is refused with a reply `RX_CHANNEL_ENABLE:0,1,false;`. | §4.2 RX_CHANNEL_ENABLE |
| `LOCK` | MAY | Tuning lock; synchronised state only. | §4.2 LOCK |
| `VFO_LOCK` | MAY | Notification, server to client. | §4.4 VFO_LOCK |
| `TX_FREQUENCY` | MUST | Sent on every change of the effective TX frequency (channel A, or channel B with split, plus the XIT offset when XIT is on; the commands that move it are listed in C7.6.1). `DIGL_OFFSET` and `DIGU_OFFSET` do not move it (0021 §34). | §4.4 TX_FREQUENCY |

### Mode and filters

| Command | Status | G7250 note | [TCI] |
|---------|--------|------------|-------|
| `MODULATION` | MUST | Names of C3.4; one mode per receiver, shared by channels A and B ([TCI] §4.2 MODULATION arg1). Sets the TX mode class (C5.6). Refused while keyed (C7.6). | §4.2 MODULATION |
| `RX_FILTER_BAND` | MUST | Demodulator passband. | §4.2 RX_FILTER_BAND |
| `DIGL_OFFSET`, `DIGU_OFFSET` | SHOULD | Required if `DIGL`/`DIGU` are in the modulation list. 0–4000 Hz ([TCI]). In `DIGL`/`DIGU` the suppressed carrier is at the channel frequency, as in `LSB`/`USB`; the offset shifts only the audio passband of the demodulator and of the modulator relative to it, so it moves neither the effective TX frequency nor `TX_FREQUENCY`, and the TX frequency limit acts on the channel frequency as in SSB (0021 §34) **[unverified against ExpertSDR3; Annex C-D, D9]**. Followed while keyed (C7.6.4). | §4.2 DIGL_OFFSET, DIGU_OFFSET |
| `AGC_MODE` | SHOULD | At least `normal` and `off`; `fast` MAY (token `agc_fast`). Unknown modes, and `fast` where not implemented, are ignored. | §4.2 AGC_MODE |
| `AGC_GAIN` | MAY | -20…120 dB. | §4.2 AGC_GAIN |
| `RX_NB_ENABLE`, `RX_NB_PARAM` | MAY | L2: G6 (compute). | §4.2 |
| `RX_NR_ENABLE`, `RX_ANC_ENABLE`, `RX_ANF_ENABLE`, `RX_APF_ENABLE`, `RX_DSE_ENABLE`, `RX_NF_ENABLE`, `RX_BIN_ENABLE` | MAY | L2: G6. Mode restrictions as in [TCI]. | §4.2 |
| `SQL_ENABLE`, `SQL_LEVEL` | SHOULD | Squelch; applies to the RX audio stream (C6). | §4.2 SQL_ENABLE, SQL_LEVEL |

### Split, RIT, XIT

| Command | Status | G7250 note | [TCI] |
|---------|--------|------------|-------|
| `SPLIT_ENABLE` | SHOULD | TX on channel B frequency. The TX frequency limit (`LIMIT_TX_FREQ`, Table A3-2) applies to the effective TX frequency. Changes while keyed: C7.6. | §4.2 SPLIT_ENABLE |
| `RIT_ENABLE`, `RIT_OFFSET` | SHOULD | RX only; no TX effect. | §4.2 RIT_ENABLE, RIT_OFFSET |
| `XIT_ENABLE`, `XIT_OFFSET` | SHOULD | The XIT offset is part of the effective TX frequency for the TX frequency limit (`LIMIT_TX_FREQ`, Table A3-2). Changes while keyed: C7.6. | §4.2 XIT_ENABLE, XIT_OFFSET |

### Transmit

| Command | Status | G7250 note | [TCI] |
|---------|--------|------------|-------|
| `TX_ENABLE` | MUST | Server-controlled permission, not activity: `true` exactly when the `TX_ENABLE` conditions of C5.5.6 hold, which do not include item 3, so it stays `true` while keyed (0021 §44 item 1). A client set is ignored and the current state is replied ([TCI] §4.3 note). | §4.3 TX_ENABLE |
| `TRX` | MUST (M7 on L2) | `TRX:0,true,tci;` keys with TX audio from the client's TCI stream. [TCI] takes the TCI stream only "if the TCI audio stream is enabled, otherwise the microphone signal is taken" ([TCI] §4.2 TRX note): a `TRX:0,true,tci;` from a client without a running `AUDIO_START` is therefore a microphone request (Annex C-C, C-C21; 0021 §41 item 10). `TRX:0,true;` (microphone) and `mic1`/`mic2`/`micpc`/`ecoder2`: refused unless a microphone path exists (HIS-17; token `mic`, 0021 §45 item 10); refused means the server replies `TRX:0,false;`. `TRX:0,true,tci;` is also refused when the client's TX audio block period exceeds half the keyed timeout in force (C6.3.3). Every key needs the arm (C5.5) and follows the key rule of A3.1a: a refused `TRX` is not latched (0022 §24). A `TRX:0,true` from the keying client while it keys with `TRX` changes nothing but restates the request (C8.2.3). Keyed liveness: Table C8-1, for every source and mode. | §4.2 TRX |
| `TUNE` | MUST (M7 on L2) | Tune carrier at `TUNE_DRIVE`, under the arm. It does **not** start the internal ATU: the meaning is not identical (as P2 TUNE, 0007 Sub-decision 6). ATU tuning is `G7250_ATU_TUNE` (C5.7). A client that has not opted in is bound by the 1 s generic keyed window unless it restates `TUNE:0,true;` (Table C8-1, C8.2.3; 0021 §18, §28). `TUNE:0,false;` during an ATU tune cycle aborts the cycle (C5.7). | §4.2 TUNE |
| `DRIVE` | MUST | 0–100. Mapping to power pending hardware (P-PWR-1; HIS-12). Whatever the `DRIVE` or `TUNE_DRIVE` value, forward power stays within the bound of the limit in force, built-in, persistent or `G7250_LIMIT`, by the mapping of Table A3-3 for the detector in use (A3.4c; 0022 §52): without a PEP detector on the PEP estimate from the digital crest factor for every mode, the class-based mean bounds acting in addition (A3.4c; 0022 §62; C8.1). A `REDUCE` limit action lowers power on the radio side (A3.3, A3.4a: drive lowered for forward power and supply, fixed caps `P_swr` and `P_temp` for SWR and PA temperature) and does not change the `DRIVE` or `TUNE_DRIVE` value (0007, Sub-decision 4 rule 4; 0021 §21). | §4.2 DRIVE |
| `TUNE_DRIVE` | MUST | 0–100, as `DRIVE`. | §4.2 TUNE_DRIVE |
| `TX_FOOTSWITCH` | SHOULD | Reports a PTT input that reaches the main unit (controller microphone PTT, HIS-17). It never keys by itself, on either level (0007, Sub-decision 6; 0021 §9; C7.3). In v1.0 the local PTT input is not used (0030): always `false`. | §4.4 TX_FOOTSWITCH |
| `MON_ENABLE`, `MON_VOLUME` | MAY | TX monitor in the RX audio stream. | §4.2 MON_ENABLE, MON_VOLUME |

### Streams

| Command | Status | G7250 note | [TCI] |
|---------|--------|------------|-------|
| `IQ_START`, `IQ_STOP` | MUST | Receiver 0 only. Budget C6.4. | §4.3 IQ_START, IQ_STOP |
| `IQ_SAMPLERATE` | MUST | 48000, 96000, 192000, 384000 as [TCI] lists; 384000 only where 0004 point 6 allows it (token `iq_384`, 0021 §45 item 10). Unsupported value: ignored, current rate replied ([TCI] §4.3 note). | §4.3 IQ_SAMPLERATE |
| `AUDIO_START`, `AUDIO_STOP` | MUST | RX audio of receiver 0. | §4.3 AUDIO_START, AUDIO_STOP |
| `AUDIO_SAMPLERATE` | MUST | 48000 MUST; 8000, 12000, 24000 MUST on L1, SHOULD on L2 (G6; token `audio_rates_all`). Unsupported value: ignored, the rate in force replied, as `IQ_SAMPLERATE` (0021 §45 item 3). Applies to RX and TX audio of this client ([TCI] §4.3 note). | §4.3 AUDIO_SAMPLERATE |
| `AUDIO_STREAM_SAMPLE_TYPE` | MUST | int16, int24, int32, float32; default float32 ([TCI] §4.3 note). | §4.3 AUDIO_STREAM_SAMPLE_TYPE |
| `AUDIO_STREAM_CHANNELS` | MUST | 1 or 2; default 2. | §4.3 AUDIO_STREAM_CHANNELS |
| `AUDIO_STREAM_SAMPLES` | MUST | 100…2048; defaults per rate as [TCI]. | §4.3 AUDIO_STREAM_SAMPLES |
| `TX_STREAM_AUDIO_BUFFERING` | MUST (M7 on L2) | 50…500 ms; default 50 ms. | §4.3 TX_STREAM_AUDIO_BUFFERING |
| `LINE_OUT_START`, `LINE_OUT_STOP` | MAY | Duplicate of the RX audio stream ([TCI] §3.4). L2: budget C6.4. | §4.3 LINE_OUT_START, LINE_OUT_STOP |
| `LINE_OUT_RECORDER_START`, `_SAVE`, `_BREAK` | NOT SUPPORTED on L2; MAY on L1 | Needs a file system on the server; the main unit has none that is known (HIS-18). | §4.3 |
| `VOLUME`, `MUTE`, `RX_MUTE`, `RX_VOLUME`, `RX_BALANCE` | MAY | L2: the main unit cannot play audio (0017); if supported, the values are synchronised and MUST NOT change the level of the TCI RX audio stream (0021 §17.2; [TCI] does not say whether ExpertSDR3 applies them to the stream). | §4.2 |

### Sensors

| Command | Status | G7250 note | [TCI] |
|---------|--------|------------|-------|
| `RX_SENSORS_ENABLE` | MUST | Interval 30–1000 ms. | §4.4 RX_SENSORS_ENABLE |
| `RX_SENSORS`, `RX_CHANNEL_SENSORS` | MUST, MAY for ch 1 | Signal level in dBm in the RX filter. L2: calibration pending hardware (P-SEN-5; HIS-11). | §4.4 RX_SENSORS, RX_CHANNEL_SENSORS |
| `TX_SENSORS_ENABLE` | MUST (M7 on L2) | Interval 30–1000 ms. | §4.4 TX_SENSORS_ENABLE |
| `TX_SENSORS` | MUST (M7 on L2) | arg2 "microphone signal level, dBm": RMS level of the TX audio in use in dB relative to full scale of the TCI TX audio stream, `-140` without TX audio (no microphone on the main unit, 0017; 0021 §15). arg3 RMS output power in W: the forward power averaged over the sliding 100 ms window of A3.4c; arg4 peak output power in W: the PEP over the same window, or, if the detector gives no PEP (HIS-11), the PEP estimate of A3.4c (measured mean × digital crest factor), the value the safety core enforces (0022 §62; 0021 §42); arg5 SWR: the value the safety core uses (`TLM_SWR`). All are the safety core's own values (A3.4c; 0006; 0022 §17; 0021 §21). | §4.4 TX_SENSORS |
| `HW_SENSORS_ENABLE`, `HW_SENSORS` | SHOULD | Supply voltage V, current A, temperature °C from the 0006 telemetry (supply voltage, PA or supply current, PA temperature), HIS-11. A sensor that does not exist is reported as `0` and flagged absent in `G7250_FEATURES` (C5.4). | §4.5 HW_SENSORS_ENABLE, HW_SENSORS |

### CW

| Command | Status | G7250 note | [TCI] |
|---------|--------|------------|-------|
| `CW_MACROS`, `CW_MSG` | SHOULD | Text to CW, generated in the server. Each start of transmission is a TX request under the arm (C5.5). Text received while it cannot key (unarmed, inhibited, refused) is discarded, not queued (A3.1a; 0022 §24). Outside `CW` ([TCI] §3.2 "Working in CW mode") CW text is refused as `KEYER` is: ignored without a reply, not queued and not latched, so a later change to `CW` does not send it (0021 §44 item 15). While the text is sent, the CW text liveness of C8.2 applies to every keying client, opted in or not, and the keyed timeout and the generic keyed window do not run (Table C8-1; 0022 §25; 0021 §29 item 1).  | §3.2, §4.5 CW_MACROS, CW_MSG |
| `CW_MACROS_STOP` | MUST | Accepted from any client; unkeys at once. | §4.5 CW_MACROS_STOP |
| `CW_TERMINAL` | SHOULD | Terminal mode keeps TX keyed after the macro; still bound by the maximum TX time and the keyed liveness of Table C8-1 (keyed timeout for an opted-in client, 1 s generic keyed window otherwise, fed by a further `CW_MACROS` or `CW_MSG`; 0021 §18, §28). Per client ([TCI] §4.5 CW_TERMINAL: "Unidirectional control"; C7.1): a client's terminal mode applies to the CW text that client keys; a `CW_TERMINAL:0,false;` from another client does not end it (any client ends the transmission with a stop request, C7.4); it ends when the client's connection closes (0021 §43 item 14). | §3.2.1, §4.5 CW_TERMINAL |
| `CW_MACROS_EMPTY`, `CALLSIGN_SEND`, `CW_MON` | SHOULD | Notifications, as [TCI]. | §4.5 |
| `CW_MACROS_SPEED`, `CW_MACROS_SPEED_UP`, `CW_MACROS_SPEED_DOWN`, `CW_MACROS_DELAY` | SHOULD | As [TCI]. | §4.2, §4.3 |
| `CW_KEYER_SPEED` | SHOULD | Speed of the radio's own keyer for the KEY input on the controller, if it reaches the main unit (HIS-17); token `cw_keyer` (Table C5-1). Not the client-timed `KEYER` below (token `keyer`). | §4.2 CW_KEYER_SPEED |
| `KEYER` | MAY | Client-timed CW key ([TCI] §4.4 KEYER: arg3 is "for a press – the pause before it, for a release – how long the key was held"; the device "plays each state for the given duration"). "Works in CW mode only" ([TCI]): outside `CW` every `KEYER` command is ignored and not latched, so a later change to `CW` does not key (0021 §33; A3.1a). The first press is a TX request under the arm. An arg3 above 1000 ms is clamped to 1000 ms (0021 §33). Every `KEYER` command restates the request for the generic keyed window (C8.2.3); a key-down from a client that has not opted in ends after the 1 s generic keyed window without a further `KEYER` (0021 §18). The key-down cap acts on the key-down as played: a key-down that has been played for more than 1 s from the start of its playback, without the release being played, ends from every client, opted in or not, whatever frames it sends (C8.2; part B B10.5a; `unkey` 9; 0022 §25; 0021 §33). Jitter over TCP/WebSocket on level 2: pending G6 (P-L2-1). | §4.4 KEYER |

### Spots, focus, panels, reserved

| Command | Status | G7250 note | [TCI] |
|---------|--------|------------|-------|
| `SPOT`, `SPOT_DELETE`, `SPOT_CLEAR` | MAY on L2, SHOULD on L1 | L2 has no panorama of its own; spots MAY be stored and relayed. AetherSDR has a panorama. | §4.3 SPOT, SPOT_DELETE, SPOT_CLEAR |
| `CLICKED_ON_SPOT`, `RX_CLICKED_ON_SPOT` | MAY on L1; NOT SUPPORTED on L2 | No panorama on L2. | §4.4 |
| `APP_FOCUS`, `SET_IN_FOCUS` | MAY on L1; NOT SUPPORTED on L2 | No application window on L2. | §4.4 APP_FOCUS, SET_IN_FOCUS |
| `ECODER_SWITCH_RX`, `ECODER_SWITCH_CHANNEL` | NOT SUPPORTED | No E-Coder panel. | §4.5 |
| `PREAMP`, `PREAMP_PARAM`, `CTCSS_*`, `RX_NF_PARAM`, `RX_SMETER`, `TX_POWER`, `TX_SWR` | MUST NOT be sent or accepted | Reserved: "the TCI server does not accept or send them" ([TCI] §4.6). Front-end control: `G7250_PREAMP`, `G7250_ATT` (C5.9). | §4.6 |

C4.3 **Parameter monopoly and tick.** The server MUST apply the 200 ms monopoly (500 ms for CW
speed and delay) and the 30 ms notification tick of [TCI] §4.7 notes 6 and 7 to the parameters
of [TCI], with these exceptions (0021 §38):

1. The safety core is never subject to the monopoly: a stop request of C7.4 (`TRX` false,
   `TUNE` false, `CW_MACROS_STOP`, a disarm, `STOP`, and `G7250_ATU_TUNE:0,abort;`) takes
   effect immediately.
2. A tightening extension write is never dropped by the monopoly: a stricter `G7250_LIMIT`, a
   lower `G7250_KEYED_TIMEOUT` and `G7250_TX_POLICY:holder`. A loosening extension write is
   subject to it, as a standard parameter.
3. Messages that report a server-side unkey, a drop or a safety change (the unkey broadcast of
   C7.7, `TX_ENABLE:0,false;`, `G7250_TX_STATE`, `G7250_TX_ARM_STATE`, `G7250_SAFETY`) MUST
   be sent at once, not on the 30 ms tick, and MUST NOT be merged away: a `TRX` that goes
   from `true` to `false` and back to `true` within one tick is reported as `TRX:0,false;`
   followed by `TRX:0,true;`.

---

## C5 G7250-specific mapping and extension commands

### C5.1 Rules for extensions

C5.1.1 An extension command has the form of [TCI] §3.1 (`NAME:arg1,arg2;`, reserved characters
`:`, `,`, `;`, case-insensitive), with a name that starts with `G7250_`. Logical arguments
follow [TCI] §4.7 note 3; part C's own enumerations are lower-case words, and Table B8-3 code
names are sent verbatim in upper case (C5.5.1); in set requests both are accepted in any case
([TCI] §3.1) (0021 §43 item 15). Hexadecimal values are exactly 8 hex digits, case-insensitive
(the one exception is the disarm value of C5.5.3).

C5.1.2 Extension commands follow the four TCI command types of [TCI] §3.3. For each one, the
type is given. Bidirectional extension state is synchronised to all opted-in clients, as [TCI]
§3.1 does for standard commands.

C5.1.3 A client that talks to a server without the profile gets no reply to an extension
command, because "an unknown command" is "ignored without a reply" ([TCI] §4.7 note 5). A
client recognises a profile server by the reply to `G7250_EXT` (C5.2), not by `DEVICE` or
`PROTOCOL` (0021 §4, §5).

C5.1.4 Every extension MUST have the same meaning on both levels (0014), and MUST be
implemented on both levels, except: an extension whose hardware is not yet known MAY be
absent until the HIS answers (`G7250_ANT` HIS-06, `G7250_ATU` selection HIS-08,
`G7250_PREAMP` and `G7250_ATT` HIS-07) and, on level 1, until a part B register or field
carries it (part B B7, B8.3: group 5 has only the read-only `ANT_STATE`, group 6 is reserved,
the Alex words and step attenuator are ignored until HIS-06 and HIS-07). The server MUST
announce what is present through `G7250_FEATURES` (C5.4); an absent extension command is
ignored without a reply, as an unknown command ([TCI] §4.7 note 5) (0021 §23). Where level 1
maps an extension onto G7250-P2, the mapping is given.

### C5.2 Opt-in: `G7250_EXT`

- Type: bidirectional, but per client (the only bidirectional extension that is not
  synchronised).
- Set: `G7250_EXT:<bool>;`. Reply: `G7250_EXT:<bool>;`.
- After `G7250_EXT:true;` the server MUST send to that client, in this order: the reply
  `G7250_EXT:true;`, `G7250_PROFILE`, `G7250_FEATURES`, and the current state of every
  extension it implements, and from then on every extension notification. To
  `G7250_EXT:false;` the server MUST send the reply `G7250_EXT:false;` as the last extension
  message to that client (0021 §43 item 2b). Before the opt-in, and after that reply, the
  server MUST NOT send extension commands to that client, except `G7250_REFUSED` (C2.5.2) and
  the reply to a further `G7250_EXT` of its own.
- Reason: [TCI] does not say how a client treats an unknown command from the server
  **[unverified for existing clients; Annex C-D]**; the opt-in keeps generic clients on pure
  TCI 2.1.
- Extension set requests from a client that has not opted in MUST still be accepted, with
  two exceptions: a `G7250_ATU_TUNE` start from a client that has not opted in MUST be ignored
  without a reply and is not latched, because that client is bound by no keyed timeout and
  would see no `G7250_ATU_STATE` (C5.7; 0021 §29 item 2); and a `G7250_TX_ARM` with any value
  other than the disarm value (C5.5.3) from such a client MUST be ignored without a reply and
  is not latched, because it would hold an arm whose `G7250_TX_ARM_STATE` and token it cannot
  see (0021 §44 item 20). The disarm value is accepted from every client, opted in or not
  (C5.5.5 row 1).
- **Opt-in state while keyed** (0021 §29 item 3). The opt-in state that selects the column of
  Table C8-1 for the keying client is fixed at the key: a `G7250_EXT:true;` or
  `G7250_EXT:false;` from the keying client while keyed changes which extension messages it
  receives at once, but its keyed-liveness column only from its next key (the row: C7.5 item
  7). A `G7250_EXT:false;` from the arm holder is drop
  event 13 (C5.5.5), which unkeys.
- **Resend of `G7250_FEATURES`** (0021 §41 item 6). The server MUST send `G7250_FEATURES` again
  to every opted-in client whenever the set of tokens changes, for example when transmit
  becomes available or unavailable, when a sensor becomes absent, and (L1) when AetherSDR's P2
  session starts or ends or a new capability record arrives (B4.4).

### C5.3 Identification and active level: `G7250_PROFILE`

- Type: notification. `G7250_PROFILE:<profile>,<level>,<active>,<clients_max>;`
- `profile`: profile version, `1` for this draft (C1.8). `level`: `1` (AetherSDR over
  G7250-P2) or `2` (firmware). `active`: `true` if this level currently owns the RF chain
  (0011 point 2). `clients_max`: C2.5 (L1: `0` = no fixed limit).
- (L1) `active` is `true` while AetherSDR has a running G7250-P2 session; it becomes `false`
  at every end of that session: when the radio changes to level 2 or the session is taken
  over (0007, drop events 4 and 10), and also on a TCI `STOP` or a stop in AetherSDR's own user
  interface (run cleared, C4 `STOP`), on expiry of the P2 watchdog, at the start of a firmware
  update (C2.8), on a link below 100 Mbit/s (C2.9a) and at every other session end of A2.3
  (0021 §44 item 10). It becomes `true` again when AetherSDR's next session starts.
- (L2) While the server is listening, level 2 is active; `active:false` is sent only just
  before closing on a level change (C2.7).
- The level is also reported in the capability block and the management channel (0011 point 2;
  part B B4).

### C5.4 Discovery of optional functions: `G7250_FEATURES`

- Type: notification. `G7250_FEATURES:<token>,<token>,...;` lists what this server implements
  (0014, Consequences: "a way for the application to discover which ones are available").
- Tokens (profile 1): the complete list is Table C5-1, one token per MAY or SHOULD command or
  group of C4, per optional part of a command (C4.1), and per optional extension (C4.1, C5.1.4; 0021
  §44 item 6, §45 item 10). A command of C4 that has no token in Table C5-1 is MUST on that level and is
  always implemented, except a command marked NOT SUPPORTED on that level, which has no token
  and is never implemented (C4; 0021 §46 item 10). No other token is sent in profile 1; a client ignores a token it does
  not know.
- A token appears only if the function works now; for example `tx` is absent on level 2
  before M7, and on level 1 `ant`, `atu_select`, `preamp` and `att` are absent until HIS-06,
  HIS-07, HIS-08 and a part B register exist (C5.1.4; 0021 §23).

Table C5-1 — Feature tokens (profile 1)

| Token | Present when the server implements, and it works now | C4 status |
|-------|------------------------------------------------------|-----------|
| `tx` | transmit: `TRX`, `TUNE`, `DRIVE`, `TUNE_DRIVE`, `TX_STREAM_AUDIO_BUFFERING`, `TX_SENSORS`, TX audio and the arm of C5.5 (L2 from M7; L1 from M4 and while a P2 session runs) | MUST (M7 on L2) (C4) |
| `iq` | `IQ_START` can be followed now (L1: a P2 session runs) | MUST (C4) |
| `audio` | `AUDIO_START` can be followed now (L1: a P2 session runs) | MUST (C4) |
| `audio_rates_all` | `AUDIO_SAMPLERATE` 8000, 12000 and 24000 besides 48000 | MUST on L1, SHOULD on L2 (C4) |
| `iq_384` | `IQ_SAMPLERATE` 384000 (where 0004 point 6 allows it, C6.4) | conditional part of a MUST (C4) |
| `mic` | the microphone sources of `TRX` (Table C8-1 row 3): a microphone path reaches the TX chain (HIS-17) | conditional part of a MUST (C4) |
| `line_out` | `LINE_OUT_START`, `LINE_OUT_STOP` | MAY (C4) |
| `recorder` | `LINE_OUT_RECORDER_START`, `_SAVE`, `_BREAK` (L1 only) | MAY on L1 (C4) |
| `volume` | `VOLUME`, `MUTE`, `RX_MUTE`, `RX_VOLUME`, `RX_BALANCE` as synchronised values | MAY (C4) |
| `ch_b_rx` | demodulation of channel B: `RX_CHANNEL_ENABLE` ch 1 `true`, `RX_CHANNEL_SENSORS` ch 1 | MAY (C4) |
| `lock` | `LOCK` and the `VFO_LOCK` notification | MAY (C4) |
| `split` | `SPLIT_ENABLE` | SHOULD (C4) |
| `rit` | `RIT_ENABLE`, `RIT_OFFSET` | SHOULD (C4) |
| `xit` | `XIT_ENABLE`, `XIT_OFFSET` | SHOULD (C4) |
| `dig` | modes `DIGL`, `DIGU` and `DIGL_OFFSET`, `DIGU_OFFSET` | SHOULD (C4) |
| `wfm` | mode `WFM` (receive only) | SHOULD (C4) |
| `agc` | `AGC_MODE` with at least `normal` and `off` | SHOULD (C4) |
| `agc_fast` | `AGC_MODE` `fast` | MAY part of a SHOULD (C4) |
| `agc_gain` | `AGC_GAIN` | MAY (C4) |
| `nb` | `RX_NB_ENABLE`, `RX_NB_PARAM` | MAY (C4) |
| `nr`, `anc`, `anf`, `apf`, `dse`, `nf`, `bin` | `RX_NR_ENABLE`, `RX_ANC_ENABLE`, `RX_ANF_ENABLE`, `RX_APF_ENABLE`, `RX_DSE_ENABLE`, `RX_NF_ENABLE`, `RX_BIN_ENABLE`, one token each | MAY (C4) |
| `sql` | `SQL_ENABLE`, `SQL_LEVEL` | SHOULD (C4) |
| `mon` | `MON_ENABLE`, `MON_VOLUME` | MAY (C4) |
| `footswitch` | `TX_FOOTSWITCH`: a PTT input reaches the main unit (HIS-17); absent in v1.0 (0030) | SHOULD |
| `cw_text` | CW text: `CW_MACROS`, `CW_MSG`, `CW_MACROS_SPEED`, `CW_MACROS_SPEED_UP`, `CW_MACROS_SPEED_DOWN`, `CW_MACROS_DELAY` and the notifications `CW_MACROS_EMPTY`, `CALLSIGN_SEND`, `CW_MON` | SHOULD (C4) |
| `cw_terminal` | `CW_TERMINAL` | SHOULD (C4) |
| `cw_keyer` | the radio's own keyer for the KEY input on the controller and its speed, `CW_KEYER_SPEED` (HIS-17); not `KEYER` | SHOULD |
| `keyer` | the client-timed CW key `KEYER` of [TCI] §4.4; not the radio's keyer | MAY (C4) |
| `spots` | `SPOT`, `SPOT_DELETE`, `SPOT_CLEAR` | MAY on L2, SHOULD on L1 (C4) |
| `spot_click` | `CLICKED_ON_SPOT`, `RX_CLICKED_ON_SPOT` (L1 only) | MAY on L1 (C4) |
| `focus` | `APP_FOCUS`, `SET_IN_FOCUS` (L1 only) | MAY on L1 (C4) |
| `hw_sensors` | `HW_SENSORS_ENABLE`, `HW_SENSORS` | SHOULD (C4) |
| `absent_voltage`, `absent_current`, `absent_temperature` | with `hw_sensors`: that `HW_SENSORS` field has no sensor and is reported as `0` | — |
| `atu` | `G7250_ATU_TUNE` and `G7250_ATU_STATE` (C5.7) | extension, C5.1.4 |
| `atu_select` | `G7250_ATU` settable (C5.7) | extension, C5.1.4 |
| `ant` | `G7250_ANT` settable (C5.8) | extension, C5.1.4 |
| `ant_state` | `G7250_ANT` reported only (C5.8) | extension, C5.1.4 |
| `preamp` | `G7250_PREAMP` (C5.9) | extension, C5.1.4 |
| `att` | `G7250_ATT` (C5.9) | extension, C5.1.4 |

### C5.5 TX arm on TCI

C5.5.1 **Same safety core.** The arm, its preconditions, its drop events and the limits are
those of 0007, which sits below both levels (0011 point 3). TCI changes only how the arm is
requested and reported. A TCI client is never the P2 owner: on level 1 the P2 owner is
AetherSDR, which becomes the owner by its run bit or by a takeover (A2.3, A2.4; 0022 §3); on
level 2 the owner of the RF chain is the firmware's TCI level itself (0011 point 2), and the
TCI clients share control of it as [TCI] §3.1 describes. Code names on TCI are the Table B8-3
names, verbatim and in upper case as Table B8-3 writes them; part C's own enumerations, which
have no Table B8-3 name (states, TX sources, mode classes, stream reasons), are lower case
(part B B8.7; 0021 §22; 0022 §19, §32).

C5.5.2 **Why an arm on TCI at all.** [TCI] has no arm: `TX_ENABLE` is server-controlled
information ([TCI] §4.3 TX_ENABLE). Without an arm, any TCI client on the LAN could key 100 W.
The profile keeps the rule of 0015: only software that knows the G7250 can enable transmit.
Once armed, generic TCI clients may key with the standard commands (`TRX`, `TUNE`, CW), which
is what full functionality for third-party applications needs (0012), unless the TX policy
restricts keying to the arm holder (C5.5.8). This matches level 1, where AetherSDR arms over
P2 and its TCI clients key through AetherSDR (0021 §1).

C5.5.3 **Commands.**

| Command | Type | Form | Meaning |
|---------|------|------|---------|
| `G7250_TX_ARM_TOKEN` | notification | `G7250_TX_ARM_TOKEN:0,<hex8>;` | Current token; never `00000000`, `FFFFFFFF` or `ARM_MAGIC` (with the token equal to `ARM_MAGIC` the arm value would be `00000000`, which disarms), different from the previous one (0007 Decision 1; part B B10.2 item 5). Sent on opt-in and whenever it changes. |
| `G7250_TX_ARM` | unidirectional control | `G7250_TX_ARM:0,<hex8>;` | `token XOR ARM_MAGIC` arms; `00000000`, also accepted as the single digit `0`, disarms, from any client (drop row 1; while disarmed it changes nothing). Any other value, a value that is not 8 hex digits (other than `0`) and a missing value are rejected with `BAD_TOKEN` (0021 §43 item 4). When several conditions fail, the reason is the first in this order (0021 §41 item 5, amended by §43 items 1 and 12): `BAD_TOKEN`; then the no-op of a valid arm while armed (C5.5.5.1; also while keyed, so an armed radio never answers `PTT_ACTIVE`); then C5.5.6 item 5 (`MAINTENANCE`, before item 1, because a firmware update stops the device, C2.8 item 2); then items 1, 2 (`IN_FAULT` before `INHIBITED`), 3, 4, 6 and 7; (L1) when AetherSDR knows from management identification I-4 (A1.5; A2.5a step 7) that level 2 is active, item 4 (`LEVEL_NOT_ACTIVE`) comes before item 1, because no P2 session can run then and item 1 would always hide it (0021 §44 item 4); item 5 (`MAINTENANCE`), where AetherSDR knows from management identification I-3 that an update or an unconfirmed trial boot is in progress (A1.5), still comes before item 4, because the update dominates: it ends level 2 as well (A1.8; 0021 §46 item 9). From a client that has not opted in, every value but the disarm value is ignored without a reply (C5.2; 0021 §44 item 20). `ARM_MAGIC` = `47375458` is the constant of part B (B8.6; 0007 Decision 1). A correct arm while already armed is a no-op (C5.5.5.1). |
| `G7250_TX_ARM_STATE` | notification | `G7250_TX_ARM_STATE:0,<state>,<generation>,<reject>,<drop>,<holder>;` | `state`: `disarmed`, `armed`, `inhibited` (armed, but a limit inhibit is active so TX requests do not key; no new generation; 0022 §13; part B `TX_ARM_STATE`). `generation`: +1 on every arm, 0–255, wraps. `reject`: per client, the reject reason of this connection's own last rejected arm request (Table B8-3, C5.5.6), `none` if it has none; a rejection of another client's request never changes it, so a non-holder's `BAD_TOKEN` is not shown to the holder (0021 §41 item 1). `drop`: last drop reason, a Table B8-3 drop reason name (C5.5.5; `STOP` and `HOLDER_LOST` are Table B8-3 drop reasons 13 and 14, not part C additions), `none` if none (0021 §43 item 3). `holder`: `true` if this connection is the arm holder. Sent per client on every change. |
| `G7250_TX_STATE` | notification | `G7250_TX_STATE:0,<keyed>,<source>,<mine>,<inhibits>,<unkey>,<limit>;` | `keyed`: bool (PA actually keyed); with CW (`KEYER`, CW text, `CW_TERMINAL`) it follows each element and is `false` in the gaps between elements, so it toggles per element, unlike `TRX` (C7.4.1); in a gap `source` keeps the transmission's source and `mine` stays `true` for the keying client until the transmission ends, the hang delay included (C7.6; 0021 §45 item 13); whether RF may be present in a gap is the break-in rule of part B (A3.1a "Keyer elements", A3.4d). (L1) TCI CW reaches P2 as DUC I&Q with PTT (C8.2.5), so P2 `TX_STATE` shows the PTT transmission, not the elements; AetherSDR derives `keyed` from its own element timing, `true` from the start of each element's leading ramp to the end of its trailing ramp (0021 §46 item 7). `source`: `none`, `trx`, `tune`, `cw`, `keyer`, `atu`. `mine`: `true` if this connection made the current TX request. `inhibits`: hex8, bit *n* set while the limit with limit id *n* of Table B8-3 holds an inhibit (bit 3 `LIMIT_PA_TEMP` above T3; bit 5 `LIMIT_SUPPLY_VOLTAGE` after an over-voltage or under-voltage `UNKEY`, until the supply has been inside the range for 2 s; Table A3-2; 0022 §42, §61), the bit numbering of `LIMIT_STATUS` bits 0–15. `unkey`: last unkey reason, the number of part B `TX_STATE` [23:16] (0 none, 1 TX request released, 2 keyed timeout, 3 TX IQ underrun, 4 limit, 5 arm dropped, 6 retune needs RF-path change, 7 PA disabled by the host, 8 keying client lost, 9 CW key-down longer than 1 s, 10 TX mode class changed while keyed, 11 ATU tune cycle ended by itself; 0047), with the TCI causes of C8.2 (0022 §32). (L1) `inhibits` is the P2 `LIMIT_INHIBIT` register (Table B8-2; 0022 §32). `limit`: the limit id name (Table B8-3) when `unkey` is 4, else `none`. Sent per client on every change. |
| `G7250_KEEPALIVE` | unidirectional control | `G7250_KEEPALIVE;` | An application frame with no other effect and no reply (C8.2; 0021 §18). MUST be implemented. |

C5.5.4 **Procedure.** (1) The client opts in (C5.2) and receives the token. (2) It sends
`G7250_TX_ARM:0,<token XOR ARM_MAGIC>;`. (3) The server checks the value (else `BAD_TOKEN`)
and the preconditions (C5.5.6 items 1–7), and either pushes `G7250_TX_ARM_STATE` = `armed`
with a new generation or pushes the reject reason. (4) **After the confirmation, the radio keys only on a TX request that arrives
after it** (0007 Decision 1 rule 2); a `TRX` state that was already `true` does not key. Every
TX request follows the key rule of A3.1a: a 0→1 request at a moment when every key condition
holds; a request refused for any reason is not latched (0022 §24). (5) The
server sends `TX_ENABLE:0,true;` to all clients when the `TX_ENABLE` conditions of C5.5.6
hold; a key and an unkey do not change it (0021 §44 item 1).

C5.5.5 **Arm holder and drop events.** The connection that armed is the **arm holder**. Each
drop unkeys first, then disarms and draws a new token (0007 Decision 1 rule 5). The messages
then follow in this order, the same in C2.7, C7.2.1 and T-C2-04: the unkey broadcast of C7.7
if it was keyed; `G7250_TX_ARM_STATE` with the reason; `G7250_TX_ARM_TOKEN` with the new
token (both to opted-in clients); `TX_ENABLE:0,false;` to all clients (0021 §41 item 2). Drop
events on TCI:

| # | Event | Reason | 0007 drop event |
|---|-------|--------|-----------------|
| 1 | Any client sends `G7250_TX_ARM:0,00000000;` | `HOST_DISARM` | 1 |
| 2 | Any client sends `STOP;` | `STOP` (Table B8-3 drop reason 13); (L1) the P2 drop is `RUN_CLEARED` (C4, `STOP`) | 2 (RUN cleared) |
| 3 | The arm holder's connection closes (TCP close or reset, WebSocket close) | `HOLDER_LOST` (Table B8-3 drop reason 14); if the holder was also the keying client, the unkey that comes first is reported as `unkey` 8, not 5 (C8.2; 0021 §29 item 4) | 3 (analogue of the watchdog) |
| 4 | Connection watchdog: no frame of any kind, Pong included, from the arm holder for 1 s while armed (C8.2) | `WATCHDOG` | 3 |
| 5 | (L1) Any drop of the P2 arm between AetherSDR and the radio, with the P2 reason passed on unchanged, except a `RUN_CLEARED` caused by a TCI `STOP`, which is reported as `STOP` (row 2; C4 `STOP`; 0021 §43 item 9) | P2 reason | the P2 drop reasons of Table B8-3 (0022 §61) |
| 6 | Any reset | `RESET` (state after reset: disarmed; a commanded reset while armed drops first with row 11 or 15, C2.10 item 1) | 5 |
| 7 | Entry into the fault state, a limit trip with action `FAULT` included; the cause is the `fault_code` of `G7250_SAFETY` (0022 §41) | `FAULT` | 6 |
| 8 | A limit trip with action `DISARM` (no limit of Table A3-2 has it in this version; 0022 §41) | `LIMIT_<name>` (limit id of Table B8-3) | 7 |
| 9 | Ethernet link down or below 100 Mbit/s (HIS-14; C2.9) | `LINK_DOWN` | 8 |
| 10 | Change of active level | `LEVEL_CHANGE` | 10 |
| 11 | Start of a firmware update, a change of the persistent limits over the management channel (C-3; built-in limits are never writable, A3.4 rule 2; 0008; 0022 §6, §31), or a `RESTART` while armed (C2.10 item 1; A1.9; 0022 §73 item 2) | `MAINTENANCE` | 11 |
| 12 | A TX path that must be blocked is found not blocked | `FAULT` | 13 |
| 13 | The arm holder sends `G7250_EXT:false;` | `HOST_DISARM` | 1 |
| 14 | (L2) A change of the radio's address or netmask (management C-1, P2 Set IP, or DHCP: another address or netmask at renewal, end of lease, DHCPNAK, address conflict), applied after RF is gone, and at the latest `t_reset_wait` after the unkey (A3.9a item 1a); the TCI connections on the old address end with it (A1.6, A1.11a; 0022 §44, §54, §68) | `SESSION_CHANGED` | 9 |
| 15 | (L2) A factory reset (management `FACTORY_RESET` or the local path, A1.6a); the reset itself follows as row 6 (C2.10) | `MAINTENANCE` | 11 (A1.9) |

0007 drop events 4 (takeover) and 12 (rediscovery) have no TCI equivalent on level 2, because
TCI has no session takeover; event 9 is row 14. On level 1 all three reach TCI through row 5
(C7.2.1).
The drop reasons `STOP` (13) and `HOLDER_LOST` (14) are listed in Table B8-3 as TCI-only drop
reasons, never set over P2 (0021 §22; 0022 §30); part C adds no code name of its own.

C5.5.5.1 **Holder rules** (0021 §24, refined by 0022 §46).

1. A valid `G7250_TX_ARM` from a client while another TCI client holds the arm MUST be a
   no-op: no new generation, the holder stays the same, and the requester receives
   `G7250_TX_ARM_STATE` with `holder` false (as part B B10.2 item 5). A TCI client can become
   the holder only while no other TCI client holds the arm; on level 2 that is after the arm
   has been dropped.
2. **(L1) Two holders of one arm.** On level 1 the one P2 arm of AetherSDR, the P2 owner
   (C5.5.1), can have two holders at the same time: AetherSDR's own user interface and at most
   one TCI client. This is the only exception to item 1:
   1. A TCI client becomes the TCI holder by a valid `G7250_TX_ARM` while no TCI client holds
      the arm. If the P2 arm is not set, AetherSDR performs the P2 handshake and confirms on
      TCI only after the radio has confirmed, with the new generation (C5.5.7). If AetherSDR's
      user interface already holds the P2 arm, the client joins: no P2 write, no new
      generation, and the client receives `G7250_TX_ARM_STATE` with `holder` true.
   2. AetherSDR's user interface becomes a holder by its own arm action; while a TCI client
      holds the arm it joins in the same way, without a P2 write or a new generation.
   3. Rows 3, 4 and 13 end only the TCI client's holding and unkey any transmission that
      client keyed; AetherSDR MUST write `TX_ARM` = 0 over P2 only when no holder remains, and
      otherwise the arm stays, with no new generation. Rows 1 and 2, AetherSDR's own disarm
      and every drop of row 5 end the arm for both holders.
3. After every drop, and every unkey that no client requested, the server sends the unkey
   broadcast of C7.7.

C5.5.6 **Conditions.** An arm request is accepted only if items 1–7 hold. `TX_ENABLE` is a
permission, not an activity flag: it is `true` exactly when items 1, 2 and 4–9 hold, and item
3 applies only to the arm request, so `TX_ENABLE` stays `true` while keyed and across every
key and unkey that changes none of these items ([TCI] §4.3 TX_ENABLE: "transmission
allowed (true) / transmission denied (false)"; 0021 §44 item 1):

1. The device is started (`START`), (L1) AetherSDR has a running P2 session with the watchdog
   effectively on (0007 Decision 2.3; part B B10.2 item 3).
2. Not in fault (0007 Sub-decision 5; A3.5); no limit inhibit active (0007 Sub-decision 4;
   Table A3-2).
3. Not keyed, and the TCI `TRX` and `TUNE` states are `false` (0007 Decision 1 rule 1); for
   the arm request only, not for `TX_ENABLE`.
4. This level is the active level (0011 point 2): (L2) level 2; (L1) level 1.
5. No firmware update in progress and no unconfirmed trial boot (0008; A1.8; C2.8).
6. The TX mode class is defined (C5.6): the modulation of the TX channel is not `WFM`. Reject
   reason `NO_MODE_CLASS` (0007 Decision 1 rule 1).
7. Transmit is implemented (L2: M7; L1: M4) and calibrated: the calibration set holds the
   transmit parameters of part B Annex P (record `tx` bit 4; part A A1.12; 0035, 0036, 0039).
8. Armed.
9. The effective TX frequency (C4, split and XIT) lies inside a transmit range of [G7250-MAN]
   for the configured region (`LIMIT_TX_FREQ`, Table A3-2; A1.6 C-6).

Reject reasons, the Table B8-3 names verbatim (0021 §22; 0022 §19):
`BAD_TOKEN` (wrong value, C5.5.3); `NOT_RUNNING` (item 1); `IN_FAULT` (item 2, fault; first
when a fault and an inhibit both hold);
`INHIBITED` (item 2, a limit inhibit while disarmed; 0022 §13); `PTT_ACTIVE` (item 3);
`LEVEL_NOT_ACTIVE` (item 4; level 1 only, because on level 2 the server listens only while
level 2 is active, C2.7, so item 4 always holds there; on level 1 it is reachable by the order
of C5.5.3, 0021 §44 item 4); `MAINTENANCE` (item 5, also during an unconfirmed trial boot,
0022 §10; (L1) a trial boot reaches TCI as the P2 reject reason `MAINTENANCE` of a running
session; during a firmware update the P2 session has ended (A1.8) and P2 shows AetherSDR no
update state, so AetherSDR is not required to report `MAINTENANCE` then and MAY report
`NOT_RUNNING`; the update case is tested on level 2 only, T-C5-44); `NO_MODE_CLASS` (item 6); `TX_NOT_IMPLEMENTED` (item 7, not implemented); `NOT_CALIBRATED` (item 7, not calibrated; after `TX_NOT_IMPLEMENTED`, and after every other reason, as in part B B10.2 item 3). (L1) Any P2 reject reason
is passed on unchanged, including `WATCHDOG_DISABLED` (part B B10.2 item 3); on level 2 the
connection watchdog of C8.2 is always on, so `WATCHDOG_DISABLED` is not used. `NOT_OWNER` is
not used on TCI (Table B8-3: reserved). The order among several failing conditions is that of
C5.5.3: `MAINTENANCE` comes before `NOT_RUNNING` (0021 §43 item 1).

C5.5.7 **(L1) Mapping.** AetherSDR generates its own TCI token and keeps its own TCI arm
state. A valid `G7250_TX_ARM` from a TCI client makes AetherSDR perform the P2 arm handshake
(0007 Decision 1) and confirm on TCI only after the radio has confirmed on P2. AetherSDR MAY
also arm from its own user interface; that counts as an arm by a holder that never disconnects
while AetherSDR runs. An AetherSDR arm, a P2 arm handshake included, MUST be triggered only by
an explicit arm action of the user in AetherSDR's own user interface or by a valid
`G7250_TX_ARM` from a TCI client; never by a TX request, whether a TCI `TRX`, `TUNE`, `KEYER`
or CW text, a CAT or rigctl command, or a key in AetherSDR's own user interface. While nothing
is armed such a TX request is refused (C5.5.4) and causes no P2 `TX_ARM` write (0015; 0007
Decision 1; 0021 §30; `annex-aethersdr.md` Q-O3). Several arm sources follow C5.5.5.1 item 2 (0021 §24; 0022 §46). Listed for the
annex "Changes needed in AetherSDR" (0014, Consequences; `annex-aethersdr.md` G-13).

C5.5.8 **TX policy: `G7250_TX_POLICY`** (0021 §1).

- Type: bidirectional, synchronised to opted-in clients. `G7250_TX_POLICY:<any|holder>;` (set,
  reply), `G7250_TX_POLICY;` (read). Default `any`.
- `any`: while armed, every client MAY send a TX request (C7.4). `holder`: only the arm holder
  MAY, and on level 1 with two holders (C5.5.5.1 item 2) either holder; a TX request from
  another client is refused as in C7.5 item 2.
- Any client MAY set `holder` while armed, opted in or not (C5.2: extension sets are
  accepted; a tightening, 0021 §41 item 6). While disarmed a set to `holder` is refused: the
  value in force, `any`, is replied and an opted-in requester receives `G7250_WRITE_STATUS`
  `NOT_ARMED`, because the policy returns to `any` whenever no holder remains and a new arm
  always starts with `any` (below; 0021 §44 item 17).
- **`holder` set while a non-holder keys** (0021 §44 item 5). When the policy becomes
  `holder` while the keying client is not an arm holder, the server MUST unkey that
  transmission at once (`unkey` 1, TX request released; arm kept; an ATU tune cycle ends as
  `failed`, C5.7; queued CW text is discarded) and send the unkey broadcast of C7.7; keying
  again needs a TX request from a holder. `unkey` 1 here means that the policy has withdrawn
  the non-holder's TX request, which then counts as released, as a client's `TRX:0,false;`
  would release it; part B `TX_STATE` [23:16] has no policy reason, and part C adds none
  (0021 §45 item 6). A transmission keyed by a holder
  continues. Only the **policy owner** MAY set `any`: the arm
  holder; on level 1 with two holders, the holder that set `holder`, or the remaining holder
  once that one has stopped holding (0022 §46). A set to `any` from another client is ignored
  and the current value is replied (tighten only, as 0007 Sub-decision 4 rule 1).
- The policy returns to `any` when the last client disconnects, on a level change and on reset,
  as tightened limits do (C5.10.1), and whenever no arm holder remains: at every drop or disarm
  (C5.5.5; 0021 §24). A new arm therefore always starts with policy `any`.
- Stop requests (C7.4) are accepted from every client under either policy.
- (L1) AetherSDR implements the policy in its own TCI server. When only AetherSDR's user
  interface holds the arm, `holder` means that only the user interface keys; when a TCI client
  has joined as the second holder, the user interface and that client key (0022 §46).

### C5.6 TX mode class: `G7250_TX_MODE_CLASS`

- Type: notification. `G7250_TX_MODE_CLASS:0,<class>;`, `class` one of `cw`, `ssb`, `am`, `fm`,
  `none`: the lower-case form of the `TX_MODE_CLASS` values of part B (B10.3: 1 SSB, 2 CW,
  3 AM, 4 FM; `none` for 0), a part C enumeration, since Table B8-3 gives these values no code
  names (0007, Sub-decision 4 rule 3; 0021 §14, §22).
- On TCI the class is **derived by the server** from `MODULATION` of receiver 0 by the table
  of C3.4, never declared by a client (0021 §6).
- With class `none` (`WFM`), an arm request is rejected with `NO_MODE_CLASS` (C5.5.6 item 6).
  If `MODULATION` changes to `WFM` while armed and not keyed, the arm is kept, TX requests are
  refused (`TRX:0,false;`, `TUNE:0,false;`) and `TX_ENABLE` becomes `false` (0021 §6). On level 2 the DSP is on board, so the radio knows the mode
  (0011 point 1); the reason for a declared class on level 1, that the radio does not know the
  host's mode (0007, Sub-decision 4 rule 3), does not apply.
- (L1) **Source of the class on level 1** (0021 §31). The class AetherSDR writes to
  `TX_MODE_CLASS` over P2 MUST be the class, by the table of C3.4, of the mode of the signal
  AetherSDR actually transmits, whichever source set that mode: AetherSDR's own user interface
  or a TCI `MODULATION`. AetherSDR MUST write it before arming and again on every change of
  the TX mode from any source (0007, Sub-decision 4 rule 3). TCI receiver 0 is AetherSDR's TX
  slice (`annex-aethersdr.md` G-13), so `MODULATION` of receiver 0 and the TX mode are the
  same value, and a mode change in the user interface is reported to TCI clients as
  `MODULATION` and `G7250_TX_MODE_CLASS`. While keyed, a TCI `MODULATION` is refused (C7.6.3);
  a mode change in the user interface is a `TX_MODE_CLASS` write that unkeys first (A3.9c;
  `unkey` 10, arm kept), then applies.
- The 25 W AM limit applies whenever the class is `am` ([G7250-MAN]; 0007); it is carrier
  power, measured by the carrier estimate of A3.4c (or, when the detector gives no envelope
  samples, held by the mean bounds of Table A3-3), with PEP also held to 100 W (A3.4c; 0022
  §17, §43). The 100 W limit of the other classes applies to PEP. Without a PEP detector the
  PEP bound acts on the PEP estimate of A3.4c for every class, so the class derived from
  `MODULATION` never stands in for the signal sent; the class-based mean bounds are not
  conservative on their own and act only in addition (A3.4c, Table A3-3; 0022 §62). The power
  measurement applies whatever the class (part B B10.3).

### C5.7 ATU

TCI 2.1 has no ATU command; `TUNE` is a tune carrier ([TCI] §4.2 TUNE: "Switching between
receive and Tune modes"), not an ATU tune cycle.

| Command | Type | Form | Meaning |
|---------|------|------|---------|
| `G7250_ATU` | bidirectional | `G7250_ATU:0,<bypass\|inline>;` (set), `G7250_ATU:0,<bypass\|inline\|unknown>;` (reply), `G7250_ATU:0;` (read) | Internal ATU out of or in line ([G7250-MAN], ATU: short press switches the tuner in). Present only with token `atu_select` (C5.1.4); without it a set is answered with the value in force and `G7250_WRITE_STATUS` `READ_ONLY` when token `atu` is present, and ignored otherwise (0021 §23). Refused while keyed with `KEYED` (C7.6). The reported value follows part B `ATU_STATE` (0006): 5 (unknown, P-ATU-1) → `unknown`, and a set while it is unknown is answered with `unknown` and, to an opted-in client, `G7250_WRITE_STATUS` `NOT_CALIBRATED`, which takes precedence over `READ_ONLY` (0032); 0 → `bypass`; 1, 2 (tuning) and 3 (tuned) → `inline`; 4 (failed) → the in/out state the server knows (L2), else the value reported before the cycle started, until HIS-08 shows whether a failed cycle leaves the ATU in line (0021 §43 item 13). |
| `G7250_ATU_TUNE` | unidirectional control | `G7250_ATU_TUNE:0;` (start), `G7250_ATU_TUNE:0,abort;` (abort) | Starts a tune cycle (A3.9b; 0022 §22). Only from an opted-in client; from a client that has not opted in a start is ignored without a reply (C5.2; 0021 §29 item 2). A TX request under the arm, with its own tune profile and time limit (`LIMIT_ATU_TIME`; 0007 Sub-decision 4 rule 5, Sub-decision 6, "Internal ATU tuning"). Rejections, sent to an opted-in requester in `G7250_WRITE_STATUS` (0022 §51): without the arm `NOT_ARMED`; while keyed by any other source, the CW hang delay between elements included (C7.6; A3.4d, A3.9b item 1; 0022 §74), from any client including the keying client, `KEYED`; armed and unkeyed but with another condition of A3.1a failing (fault, inhibit, effective TX frequency outside the ranges, `WFM`, a non-holder under policy `holder`) `NOT_KEYABLE`; during a firmware update `BUSY`; before the ATU parameters are set `NOT_CALIBRATED` (A3.9b; 0032); none is latched (A3.1a). Where several apply, the answer carries the first in the order of A3.9b item 1 (`BUSY`, `NOT_CALIBRATED`, `NOT_ARMED`, the no-op, `KEYED`, `NOT_KEYABLE`; `NOT_OWNER` and `BAD_VALUE` do not arise for this command on TCI; 0022 §73 item 6). A repeated start while the cycle runs, or within 2 s after the end of the previous cycle, is a no-op (A3.9b item 1; 0022 §39). Before RF the cycle applies pending RF-path changes, switches the RF path to the TX state of A3.1b for the effective TX frequency in force (`TX_FREQUENCY`: channel A, or channel B with split, plus XIT) and waits until the relays have settled, so that with split cross-band the tune carrier goes through the TX filter of the TX frequency, not the receive state of `DDS` (A3.9b item 2; C7.6.1 item 4; 0022 §66). The tune power, the lower of the ATU profile's tune power and the forward-power bound in force (A3.4 rule 5, Table A3-3; 0022 §53), is set before RF and before any ATU relay moves; while tuning, a further TX request (`TRX` true, `TUNE` true, `KEYER` press, CW text) neither keys nor ends the tune (A3.9b item 4); a stop request of C7.4 from any client (`TRX` false, `TUNE` false, `CW_MACROS_STOP`, a disarm, `STOP`), and `G7250_ATU_TUNE:0,abort;` from any client, aborts the cycle at once: unkey, `G7250_ATU_STATE` `failed` (A3.9b items 6–7; 0021 §32); the keyed liveness of Table C8-1 applies to the requesting client and ends the tune as `failed`; the cycle always ends unkeyed (part B B10.9; 0022 §14, §22). During the cycle `TUNE` is `true` to all clients, and `TUNE:0,false;` follows its end (C7.4.1). An abort while no cycle runs has no effect and no reply. At every end of the cycle, once RF is gone, the RF path returns to the unkeyed state of A3.1b for the receive frequency (`DDS`), as after every unkey; a cycle that ended `tuned` leaves its result stored for the TX frequency where the ATU stores states (HIS-08), and the next key recalls it under A3.1b; a cycle that did not end `tuned` stores nothing, the ATU is at the stored state for the receive frequency or bypassed, `G7250_ATU_STATE` stays `failed` until the next tune or ATU change, and keying is not blocked (A3.9b item 7; 0022 §38, §66). |
| `G7250_ATU_STATE` | notification | `G7250_ATU_STATE:0,<state>;` | `bypass`, `inline`, `tuning`, `tuned`, `failed`, `unknown`: part B `ATU_STATE` 0–5 (0006, "ATU state"); `failed` as A3.9b item 7; `unknown` while the ATU latches are not written (P-ATU-1; 0032). |

(L1) Mapped to the `ATU_TUNE` and `ATU_STATE` registers of group 4 (Table B8-2; token `atu`
when group 4 is present, HIS-08): a start writes `ATU_TUNE` = 1, an abort and every stop
request of C7.4 during a cycle write `ATU_TUNE` = 0, which aborts the cycle (Table B8-2;
0021 §32); so do the expiry of the requesting client's keyed liveness (Table C8-1 row 8),
the loss of its connection (0021 §43 item 7) and the policy unkey of C5.5.8 when the tune
cycle is a non-holder's (0021 §46 item 8). The switch to the TX state before the carrier and the return to the receive state
after the cycle are done by the radio for the P2 DUC0 and DDC0 frequencies (A3.9b items 2, 7;
B10.9); AetherSDR sets both before the write, as for a key. Part B has no register for ATU selection, so `atu_select`
is absent on level 1 until one exists (0021 §23). Tune power of the profile pending hardware (P-ATU-1; HIS-08,
HIS-12); the tune power used is never above the forward-power bound in force (A3.4 rule 5;
0022 §53), so a `G7250_LIMIT` on `LIMIT_FWD_POWER` below it lowers the tune carrier.

### C5.8 Antenna

| Command | Type | Form | Meaning |
|---------|------|------|---------|
| `G7250_ANT` | bidirectional | `G7250_ANT:0,<1\|2>;` (set, reply), `G7250_ANT:0;` (read) | ANT1 or ANT2 ([G7250-MAN], "ANT SELECT"). The reply reports the antenna actually selected (0006; part B `ANT_STATE`), `G7250_ANT:0,0;` while it is not known (`ANT_STATE` 0, "unknown", Table B8-2; 0021 §41 item 6); a set of `0` is answered with the value in force and `BAD_VALUE`. Settable only with token `ant`; with token `ant_state` only, it is a notification and a set is answered with the value in force and `G7250_WRITE_STATUS` `READ_ONLY`; with neither, absent (C5.1.4; 0021 §23). Refused while keyed with `KEYED` (C7.6). Default after reset: pending hardware (P-ANT-1; HIS-06; 0007 Decision 5); until set, the reply is `G7250_ANT:0,0;` and a set gets `READ_ONLY` (Annex P). |

(L1) Part B group 5 has only the read-only `ANT_STATE` (Table B8-2) and the P2 Alex words
are ignored until HIS-06 (part B B7), so AetherSDR announces at most `ant_state` until a
writable register exists (0021 §23).

### C5.9 Front end (optional)

| Command | Type | Form | Meaning |
|---------|------|------|---------|
| `G7250_PREAMP` | bidirectional | `G7250_PREAMP:0,<bool>;` | Preamplifier, if it exists (HIS-07). The TCI `PREAMP` command is reserved ([TCI] §4.6) and so not used. Present only with token `preamp`. Refused while keyed with `KEYED` (C7.6). |
| `G7250_ATT` | bidirectional | `G7250_ATT:0,<dB>;` | Attenuation in dB, steps pending hardware (P-RF-4; HIS-07). Present only with token `att`. Refused while keyed with `KEYED` (C7.6). |

Both are absent on both levels until HIS-07 is answered and, on level 1, a part B register
exists (group 6 is reserved, the step attenuator byte 1443 is ignored until P-RF-4 is set (HIS-07), part B B7, B8.3) (C5.1.4;
0021 §23).

### C5.10 Safety and fault state

| Command | Type | Form | Meaning |
|---------|------|------|---------|
| `G7250_SAFETY` | notification | `G7250_SAFETY:<state>,<fault_code>,<fault_class>,<fault_flags>,<trips>;` | The part B fault registers (Table B8-2; A3.5; 0021 §22). `state`: `normal` or `fault` (`SAFETY_STATE` 0, 1). `fault_code`: first cause, the Table B8-3 name (`SWR_REPEATED`, `PA_OVERTEMP`, `SENSOR_IMPLAUSIBLE`, `RF_NOT_KEYED`, `TX_PATH_NOT_BLOCKED`, `WDT_RESET_AT_START`, `SELFTEST_FAILED`, `INTERNAL_ERROR`), `none` if none (`FAULT_CODE`). `fault_class`: `0` none, `1` host-clearable, `2` latched until reset, the highest class among the flagged codes (`FAULT_CLASS`; 0022 §5). `fault_flags`: hex8, bit *c* set for every fault code *c* since the last clear (`FAULT_FLAGS`). `trips`: decimal, limit trips with action `UNKEY` or stronger since power-on, saturating (`TRIP_COUNT`). Sent on every change. |
| `G7250_FAULT_CLEAR` | unidirectional control | `G7250_FAULT_CLEAR:<fault_code>;` | `fault_code`: the Table B8-3 name of the current `fault_code` of `G7250_SAFETY`, as part B `FAULT_CLEAR` takes the current code (0022 §32); another name, or none, is rejected with `G7250_WRITE_STATUS` `BAD_VALUE`. Clears all flags only if every flagged code is class 1 (not escalated), its condition is gone and at least 10 s have passed since the latest fault entry (A3.5; 0022 §5, §27); otherwise nothing is cleared and the requester receives `G7250_WRITE_STATUS` `NOT_CLEARABLE`. A clear does not re-arm (0007 Decision 5, option 5D). Any client MAY send it, opted in or not (C5.2); in fault no client holds the arm, because the fault drops it (A3.5), so the holder rule of C5.10.1 item 3 does not arise. Rejections go to an opted-in requester in `G7250_WRITE_STATUS`: `BAD_VALUE`, `NOT_CLEARABLE`, or `BUSY` during a firmware update (0022 §51). (L1) AetherSDR, the P2 owner, writes the code to `FAULT_CLEAR` and passes the P2 answer on. |
| `G7250_LIMIT` | bidirectional | `G7250_LIMIT:<id>,<value>;` (set, reply), `G7250_LIMIT:<id>;` (read) | TCI session value of a host-tunable limit. `id`: `LIMIT_TX_TIME` (ms), `LIMIT_FWD_POWER` (mW, PEP; enforced through Table A3-3 like every tier, so with a mean-only detector it bounds the PEP estimate of A3.4c for every class and, in addition, the mean: SSB value × `P_ssb_mean` / 100 W, AM min(value / 4, 25 W), CW and FM the value; A3.4c; 0022 §52, §62), `LIMIT_SWR` (1000 × SWR), `LIMIT_PA_TEMP` (m°C): the Table B8-3 limit ids, in the units of their part B registers (Table B8-2; B8.6). Another id: `BAD_VALUE`. Rules C5.10.1. The reply always carries the limit in force. |
| `G7250_KEYED_TIMEOUT` | bidirectional | `G7250_KEYED_TIMEOUT:<ms>;` (set, reply), `G7250_KEYED_TIMEOUT;` (read) | Keyed timeout of C8.2 for opted-in and TX-audio keying clients. Not a limit (0021 §20). 100–500 ms, other values `BAD_VALUE`; default 500 ms (0007 Resolved 2; part B `KEYED_TIMEOUT`). A lower value is accepted from any client; a higher one only as C5.10.1 item 3, otherwise it is rejected with `LOOSER_LIMIT` and the value in force is replied (0021 §44 item 16). Back to 500 ms at TCI session end, on a level change and on reset. (L1) AetherSDR applies it to its TCI-side timer and MAY write it to the P2 `KEYED_TIMEOUT` register; if it does, it SHOULD NOT write a value below twice the period of its own High Priority keep-alive (100 ms, C8.2.4), so 200 ms, because the radio's keyed timeout is fed by that keep-alive and one late packet would unkey; the TCI-side timer uses the TCI value whatever is written to P2 (0021 §44 item 19). |
| `G7250_LIMIT_STATUS` | notification | `G7250_LIMIT_STATUS:<hex8>;` | The part B `LIMIT_STATUS` layout: bit *n* limit id *n* limiting (`WARN` or `REDUCE`); bit 16 + *n* limit id *n* tripped (`UNKEY` or stronger) since the last TX request; limit ids of Table B8-3 (0006, "limit status"; 0021 §22). |

C5.10.1 **Limit tiers on TCI** (0021 §20; 0022 §6; A3.4 rules 1–2).

1. The TCI session value is the third tier of A3.4 rule 1. The **limit in force** is the
   strictest of the built-in value, the persistent value (management C-3, 0008) and the TCI
   session value; it is the only value that acts and the only value a reply carries.
2. A `G7250_LIMIT` write MUST be accepted only if it is equal to or stricter than the
   persistent value; a looser write MUST be rejected (not clamped) with `LOOSER_LIMIT`, and
   the reply carries the limit in force.
3. Writing 0 returns the TCI session value to the persistent value. Because the session value
   is shared by all clients, a write of 0, or of a value looser than the current session
   value, MUST be accepted only from the arm holder, or from any client while no client holds
   the arm; otherwise it is rejected with `LOOSER_LIMIT`. (L1) AetherSDR's own user interface
   counts as an arm holder (C5.5.5.1 item 2, C5.5.7): while it alone holds the arm, no TCI
   client is a holder, so such a write from a TCI client is rejected with `LOOSER_LIMIT`
   (0021 §44 item 18).
4. The TCI session ends when the last client disconnects, on a level change and on reset; the
   session values then return to the persistent values (after a reset, the stored ones, A3.6).
5. A change of the persistent values (C-3) while armed drops the arm with `MAINTENANCE`
   (C5.5.5 row 11) and then applies at once; a session value looser than the new persistent
   value is replaced by it. Every tier of `LIMIT_FWD_POWER` acts through the same bounds of
   Table A3-3, also with a detector that gives only the mean, then on the PEP estimate with
   the class-based mean bounds in addition (A3.4c; 0022 §52, §62).
6. (L1) AetherSDR writes to the P2 `LIMIT_<name>` register of the same name (Table B8-2) the
   strictest of its own user interface's value, where its user interface sets one, and the
   TCI session value. A TCI write looser than the user interface's value, otherwise
   acceptable by items 2 and 3, is rejected with `LOOSER_LIMIT`, because it could not act, and
   the TCI session value stays as it was. The TCI reply always carries the limit in force as
   the P2 answer reports it; a P2 `LOOSER_LIMIT` is passed on to TCI (0021 §43 item 3).
   AetherSDR writes this value at every change of either input and at every start of its P2
   session, before it arms, because the radio returns the P2 session limits at every session
   end (A3.6; B5.4; C8.5a item 3; 0022 §78; 0021 §45 item 1). Before it sets the P2 PTT bit
   it MUST have received the P2 write echo of each such write (part B B8.4), so that no
   transmission starts under a limit that is not yet in force (0021 §46 item 4).
7. The 1 s generic keyed window, the 1 s key-down cap and the 30 s CW text liveness of C8.2
   are built in and have no command (0022 §25).
8. (L1) When the TCI session ends (item 4), AetherSDR MUST write the P2 `LIMIT_<name>`
   registers it had written for TCI, and `KEYED_TIMEOUT` if it had written it, back to the
   values of its own user interface (0 for a register where its user interface sets no
   session value, which returns it to the persistent value, A3.4 rule 2; `KEYED_TIMEOUT` to 500 ms, Table B8-2), so that no TCI session
   value stays in force on P2 after the last TCI client has gone (0021 §41 item 8).

### C5.11 Stream state: `G7250_STREAM_STATE`

- Type: notification, per client. `G7250_STREAM_STATE:0,<stream>,<state>,<reason>;`,
  `stream` one of `iq`, `audio`, `line_out`, `tx_audio`; `state` `running`, `stopped`,
  `refused`; `reason` `none`, `budget`, `rate`, `not_supported`, `level_change` (C2.7),
  `takeover` (L1, C7.2.1), `update` (C2.8), `link` (C2.9) (0022 §45, §51). These are part C enumerations: the stream reasons of Table B8-3
  have numbers but no symbolic names.

### C5.12 Write rejections: `G7250_WRITE_STATUS`

- Type: notification, per client, sent only to the opted-in client whose request was
  rejected. `G7250_WRITE_STATUS:<command>,<reason>;`, `command` the name of the rejected
  command, `reason` a write reject reason of Table B8-3, verbatim: `READ_ONLY`, `BAD_VALUE`,
  `LOOSER_LIMIT`, `BUSY` (firmware update in progress), `NOT_CLEARABLE`, `KEYED` (RF-path
  change, `MODULATION` or `G7250_ATU_TUNE` while keyed, or a TX request while keyed by another
  source or client), `NOT_ARMED`, `NOT_KEYABLE` (a TX request or `G7250_ATU_TUNE` while
  another key condition fails, C5.7), `NOT_CALIBRATED` (`G7250_ATU_TUNE` before the ATU
  parameters are set, C5.7; 0032) (0021 §22; part B B8.4; 0022 §22, §51).
- Part B owns Table B8-3, which lists these names for P2; their use on TCI, `KEYED`,
  `NOT_ARMED` and `NOT_KEYABLE` for TX requests included, is defined here in C5.12 and
  nowhere else (0021 §44 item 12).
- The TCI counterpart of the P2 `WRITE_STATUS` register. The reply with the value in force is
  sent as well, to all clients as usual.
- **Coverage** (0021 §43 item 11). It MUST be sent to an opted-in requester for every request,
  standard or extension, that a rule of part C refuses or does not follow: `KEYED` for a change
  refused while keyed (C7.6.3, C5.7) and for a TX request (`TRX`, `TUNE`, a `KEYER` press, CW
  text) refused because the radio is keyed by another client or source (C7.5 items 2 and 7);
  `NOT_ARMED` for a TX request while unarmed (C5.5.4); `NOT_KEYABLE` for a TX request while
  armed with another condition of A3.1a failing (fault, inhibit, the effective TX frequency
  outside the ranges, `WFM`, policy `holder`, a refusal at the end of the wait of C7.6.5);
  `BAD_VALUE` for a `TRX` source without a path (C4 `TRX`), a TX audio block period that is too
  long (C6.3.3), a frequency of C3.3 and a value outside a command's range; `BUSY` during a
  firmware update (C2.8); `READ_ONLY`; `LOOSER_LIMIT` for a looser limit (C5.10.1), for a higher
  `G7250_KEYED_TIMEOUT` not allowed by C5.10.1 item 3 (C5.10) and for
  `G7250_TX_POLICY:any` from a client that is not the policy owner (C5.5.8); `NOT_ARMED` for
  `G7250_TX_POLICY:holder` while disarmed (C5.5.8); `NOT_CLEARABLE` (C5.10). Where several
  reasons apply to one TX request, the answer carries the first in the order `BUSY`,
  `NOT_ARMED`, `KEYED`, `BAD_VALUE`, `NOT_KEYABLE`: the order of C5.7 for `G7250_ATU_TUNE`,
  with `BAD_VALUE`, a fault of the request itself, before the key conditions of A3.1a
  (0021 §45 item 11, §46 item 6). The command name is that of the refused request (for CW text `CW_MACROS` or
  `CW_MSG`). Not covered: arm requests (`G7250_TX_ARM_STATE`), stream starts
  (`G7250_STREAM_STATE`), commands ignored without a reply (unknown, unsupported or absent
  commands, [TCI] §4.7 note 5; `KEYER` and CW text outside `CW`; a `G7250_ATU_TUNE` start or
  an arm value of `G7250_TX_ARM` from a client that has not opted in), a client set of `TX_ENABLE` and writes dropped by the 200 ms
  monopoly ([TCI] §4.7 note 6). A client that has not opted in receives none (C5.2).
- `UNKNOWN_ADDRESS` and `NOT_OWNER` are not used on TCI: an unknown command is ignored without
  a reply ([TCI] §4.7 note 5), and TCI clients have no owner (C5.5.1).
- Arm rejections are reported only in `G7250_TX_ARM_STATE` (as part B B8.4; 0022 §13).
- [TCI] has no reply to `IQ_START` or `AUDIO_START` ([TCI] §4.3), so a client cannot see that a
    stream was refused. This notification is that report, the TCI counterpart of the stream
  status register of 0004 (Consequences).

### C5.13 Connection refused: `G7250_REFUSED`

- Type: notification. `G7250_REFUSED:<reason>;`, `reason` a part C enumeration; profile 1
  defines only `busy` (all client sockets in use, C2.5.2) (0021 §41 item 6).
- Sent once, in a text message, to a connection that the server is about to close, before
  any opt-in and as an exception to C5.2 (the other is the reply to `G7250_EXT`); the WebSocket close follows at once. A client
  that does not know the profile ignores it as an unknown command ([TCI] §4.7 note 5).
- Not sent for a connection refused by the reach rule or the allow-list (C2.4.1), which is
  closed without the handshake, nor for an allow-list change (C2.10 item 3).

---

## C6 Streams

C6.1 **Block format.** Every stream block MUST use the 64-byte header of [TCI] §3.4
(`receiver`, `sample_rate`, `format`, `codec` = 0, `crc` = 0, `length`, `type`, `channels`,
`count`, `reserv[7]` = 0), followed by `length` samples. Clarifications (Annex C-C):

- C-C4 **Byte order.** [TCI] does not state it. The header fields and the samples MUST be
  little-endian (0021 §12) **[unverified: assumed from the C structure on common PC
  platforms; Annex C-D]**.
- C-C5 **Block size.** The binary frame MUST carry the header and `length` samples only, not
  the full `data[16384]` (0021 §12) **[unverified; Annex C-D]**; the data part MUST NOT exceed
  16384 bytes ([TCI] §3.4).
- C-C6 **count.** Incremented per block within a stream; kept separately for RX audio and
  TX_CHRONO and reset on each switch between TX and RX ([TCI] §3.4). The profile also keeps it
  per stream for IQ and line-out.

C6.2 **IQ stream** (`type` = 0, IQ_STREAM).

| Item | Value | Source |
|------|-------|--------|
| Receiver | 0 | `TRX_COUNT:1` (C3.2); HIS-02 |
| Rates | 48, 96, 192, 384 kHz | [TCI] §4.3 IQ_SAMPLERATE; 0004 Decision point 1 (the same four rates on level 1); 384 kHz only where 0004 point 6 allows it |
| Channels | 2 (I and Q); complex samples = `length` / `channels` | [TCI] §3.4 |
| Format | `FLOAT32` (3) on both levels; no `INT24` IQ in profile 1; `format` MUST state it | [TCI] §3.4 SampleType; 0021 §11. [TCI] has no command to choose the IQ format, and its audio default is float32 ([TCI] §4.3 AUDIO_STREAM_SAMPLE_TYPE). |
| Samples per block | at most 16384 bytes of data: 2048 complex samples at FLOAT32 | [TCI] §3.4 |
| Rate per client | [TCI] says "The rate is individual for each client" ([TCI] §4.3 IQ_SAMPLERATE). (L2) With one DDC the server MAY offer only one IQ rate at a time; a different rate requested while another client streams IQ is "ignored, the server replies with the current rate" ([TCI] §4.3), and `G7250_STREAM_STATE` reports `rate`. | 0004 (one DDC); HIS-02 |

(L2) IQ comes from the on-board DDC, the same chain as level 1 (0004 Decision point 1); the
conversion from the 24-bit DDC output to FLOAT32 runs on the processor (HIS-18, G6).

C6.3 **Audio streams.**

| Stream | `type` | Direction | Rates | Format, channels, samples | Notes |
|--------|--------|-----------|-------|---------------------------|-------|
| RX audio | 1 | server → client | `AUDIO_SAMPLERATE` (C4) | `AUDIO_STREAM_SAMPLE_TYPE`, `_CHANNELS`, `_SAMPLES` (C4) | (L2) Demodulated **on board** (0011 point 1). The main unit has no speaker or audio output (0017), so TCI is the only audio output on level 2. (L1) Demodulated in AetherSDR. |
| TX audio | 2 | client → server | as RX audio of this client ([TCI] §4.3 AUDIO_SAMPLERATE note) | as RX audio | Sent by the client in answer to TX_CHRONO, with the number of samples in TX_CHRONO's `length` ([TCI] §3.4), counted over all channels (C-C25); a block of another length: C6.3.5; an answer is required while keyed (C8.2.1). (L2) Modulated on board. |
| TX_CHRONO | 3 | server → client | — | header only; `length` = the client's `AUDIO_STREAM_SAMPLES` (samples over all channels, C-C25), `channels` its `AUDIO_STREAM_CHANNELS`, `sample_rate` its `AUDIO_SAMPLERATE`, `format` its `AUDIO_STREAM_SAMPLE_TYPE` | Sent "without waiting for a response from the client" ([TCI] §3.4). Sent to the keying client while it keys with source `tci` and a running `AUDIO_START`, in a mode other than `CW` (C7.5; C6.3.4). |
| Line out | 4 | server → client | as RX audio | fixed ("you cannot change its parameters", [TCI] §3.4) | MAY (C4). |

C6.3.1 There is no microphone stream from the radio: the microphone is on the controller and
reaches the main unit only if HIS-17 shows a path (0004 Decision point 3; 0017).

C6.3.2 With `DIGL`/`DIGU` and 2 channels the RX audio stream is complex; with 1 channel it is
real ([TCI] §3.4).

C6.3.3 **TX audio block period** (0021 §39, §43 item 2a). `length` and `AUDIO_STREAM_SAMPLES`
count samples over all channels, and a block holds `length` / `channels` frames (C-C25; [TCI]
§3.4: `length` is the "number of samples", for IQ "the number of real samples", complex samples
being "Stream.length/Stream.channels"; [TCI] §4.3 AUDIO_STREAM_SAMPLES: the "sample amount
indicated in a Stream.length field"; **[unverified]** against a client, Annex C-D, D10). The TX
audio block period of a client is therefore

    period = AUDIO_STREAM_SAMPLES / (AUDIO_STREAM_CHANNELS × AUDIO_SAMPLERATE)

of that client ([TCI] §4.3: the sample amount "will be applied to every sample rate"); the
server MUST send TX_CHRONO to the keying client with `length` equal to that sample amount and
`channels` equal to its channel count, at that period, and the client's TX_AUDIO_STREAM block
carries `length` samples over `channels` channels. A `TRX:0,true,tci;` from a client whose
block period exceeds half the keyed timeout in force MUST be refused (`TRX:0,false;`; to an
opted-in requester `G7250_WRITE_STATUS` `TRX`, `BAD_VALUE`) and is not latched (A3.1a), so
that at least two blocks fit in every keyed timeout. With the default 500 ms (half: 250 ms) this
refuses, among the values [TCI] allows, only 1 channel at 8 kHz with more than 2000 samples
(2048 samples: 256 ms); with 2 channels, 2048 samples at 8 kHz last 128 ms and key. A client
then lowers `AUDIO_STREAM_SAMPLES`. A
`G7250_KEYED_TIMEOUT` lowered while keyed applies at once (C5.10) and is checked against the
block period at the next key.

C6.3.4 **`TRX:0,true,tci;` in `CW`** (Table C8-1 row 2; 0021 §43 item 8). In `CW` the TX
audio stream is not used: the radio sends the CW carrier at `DRIVE`, with the CW ramp, for as
long as the transmission lasts, as a key-down; the server sends no TX_CHRONO and discards TX
audio blocks, `TX_SENSORS` arg2 is `-140`, and the TX audio underrun does not act.

C6.3.5 **TX audio block of another length or header** (0021 §44 item 2, §45 item 2; Annex C-C, C-C30). [TCI] §3.4
asks the client to answer TX_CHRONO "with the specified number of samples in the
Stream.length" and does not say what the server does with another number. A TX_AUDIO_STREAM
block whose `length` differs from that of TX_CHRONO MUST be accepted when `length` is a
multiple of its `channels` and its data part is at most 16384 bytes ([TCI] §3.4; C-C5): the
server plays all `length` / `channels` frames within the buffer bound of C6.3.5 ("Buffer
bound"; 0021 §45 item 2), and the block feeds the TX audio underrun timer
and, as an application frame, the keyed timeout, as a block of the TX_CHRONO length does. A
block that fails either condition is no block (C2.2): it is discarded,
it is not an application frame and it feeds neither timer. A client that answers with shorter
blocks than asked sends less audio than the period needs; the gap is played as silence (as
the DUC sends zeros while its FIFO is empty, B10.5) and is
not an underrun as long as blocks keep arriving within `TX_STREAM_AUDIO_BUFFERING` + 500 ms
(Table C8-2). A block with `length` = 0 is accepted (0 is a multiple of every `channels`): it
plays nothing and feeds both timers, as the zero-sample answer of C8.2.1 ([TCI] §3.4: a block
of zero samples "corresponds to no signal"; 0021 §45 item 2).

**Header mismatch** (0021 §45 item 2). A TX_AUDIO_STREAM block whose `channels`,
`sample_rate` or `receiver` differs from the client's settings (`AUDIO_STREAM_CHANNELS`,
`AUDIO_SAMPLERATE`, transceiver 0; C6.3, C4.2) MUST be discarded: it is no block in the sense
of C2.2, is not an application frame and feeds neither timer, because the server cannot play
it at the period it planned. `format` is taken from the block (C7.5 item 8); other header
fields follow C6.3.

**Buffer bound** (0021 §45 item 2). The server MUST NOT buffer more TX audio of the keying
client than `TX_STREAM_AUDIO_BUFFERING` plus one block of the TX_CHRONO length (C6.3.3). When
an accepted block would exceed this bound, the oldest buffered frames not yet played are
dropped, as many as are needed for the block's frames to fit, so that the newest audio is
kept and the lag stays within the bound (0021 §46 item 11). Such a block still counts as an
application frame and feeds the TX audio underrun timer, because it arrived well-formed; a
client that sends faster than the period therefore loses audio but is not unkeyed, and the
audio on the air never lags the client by more than the bound.

C6.4 **Stream budget, level 2.** Payload rates, from sample rate × channels × bytes per
sample × 8 (64-byte header and WebSocket framing per block left out, under 1 % at 2048-sample
blocks):

| Stream | Payload (Mbit/s) |
|--------|------------------|
| IQ 384 kHz, FLOAT32 | 24.6 |
| IQ 192 kHz, FLOAT32 | 12.3 |
| IQ 48 kHz, FLOAT32 | 3.1 |
| RX audio 48 kHz, 2 channels, float32 | 3.1 |
| RX audio 48 kHz, 1 channel, float32 | 1.5 |
| TX audio 48 kHz, 1 channel, float32 | 1.5 |

For comparison, DDC0 at 384 ksps on level 1 is 18.6 Mbit/s payload and 32.9 % SPI busy at
62 MHz, the datasheet maximum [CH394] (more at the clock that works on the board, part B P-NET-1), and DUC0 is 9.2 Mbit/s (0004, "Per stream"). Level 2 has no DUC IQ, but FLOAT32 IQ is
a third larger than 24-bit IQ.

- C6.4.1 **(L2)** The server MUST keep the aggregate of all streams to all clients within the
  SPI design target of 0004 Decision point 6, with the WebSocket and TCP overhead included
  (pending G6, P-L2-2; HIS-14, HIS-18). A stream start that would exceed it MUST NOT be followed, and
  `G7250_STREAM_STATE` reports `budget`. Streams already running are never stopped to make
  room.
- C6.4.2 **(L2)** Proposed provisional limit, until G6 gives numbers: at most one IQ stream at
  a time, and RX audio to all clients. On these numbers a 384 kHz IQ stream (24.6) plus five
  stereo float32 RX audio streams (15.4) plus TX audio (1.5) is 41.5 Mbit/s of payload, about
  72 % of the 57.5 Mbit/s effective SPI rate at 62 MHz, the datasheet maximum (part B P-NET-1; `analysis/ch394-network-budget.md`;
  0004, "Assumptions") before register accesses: over the target. So at 384 kHz the server
  needs either fewer audio clients, mono or int16 audio, or a lower IQ rate. The IQ format
  stays FLOAT32; if the budget is short, the IQ rate offered is reduced (0021 §11). Numbers to
  be settled at G6.
- C6.4.3 **(L1)** AetherSDR is not bound by C6.4.1; its own P2 stream to the radio is bound by
  0004.

C6.5 **CH394Q buffers.** In TCP mode the CH394Q handles retransmission, so the 0004 buffer
split for UDP no longer applies on level 2; a level-2 split is pending G6 (P-L2-2; HIS-14).

---

## C7 Multiple clients and ownership

C7.1 **Synchronisation.** The server synchronises all clients: a parameter change from one
client is applied and sent to all ([TCI] §3.1; §3.3: "the server will synchronize the
parameters of all connected clients"). Unidirectional commands are per client ([TCI] §3.3),
including stream starts and rates (C6).

C7.2 **Single owner, levels.** Only one level controls the RF chain at a time, and only that
owner may transmit (0011 point 2). TCI clients do not compete with the level owner: on level 1
they control the radio through AetherSDR, the P2 owner; on level 2 they share the firmware's TCI
level. No TCI command changes the active level (C2.7).

C7.2.1 **(L1) P2 ownership and takeover** (0021 §26; 0022 §3; A2.3, A2.4). AetherSDR becomes
the P2 owner by setting the run bit while no session runs, or by a takeover. When another P2
host takes over, AetherSDR receives a last `TX_ARM_STATE` with `TAKEOVER`, three times about
10 ms apart, and nothing more (A2.4 step 1; 0022 §29). AetherSDR SHOULD also treat ownership as
lost when, while it believes it owns the radio, it receives neither DDC0 data nor High
Priority status packets for more than 200 ms, or when its DDC0 or status stream restarts at
sequence number 0 without its own stop and run (A2.4, host guidance; 0022 §29, §48); on a
`TX_ARM_STATE` with `TAKEOVER` it stops sending C&C at once. In each case it then stops sending
C&C and acts as below. AetherSDR MUST then send its TCI clients, in this order: the unkey broadcast
of C7.7 if keyed; `G7250_TX_ARM_STATE` (disarmed, `TAKEOVER`), a new `G7250_TX_ARM_TOKEN` and
`TX_ENABLE:0,false;` if armed; `G7250_STREAM_STATE` `stopped`, reason `takeover`, for each
running stream; `G7250_PROFILE` with `active` false; then `STOP;` (C2.9a). It keeps the TCI
connections open. The same `STOP;` follows every other end of its P2 session that no client
`STOP` caused, a watchdog standby, a link loss and an inferred loss (below) included (C2.9a;
0021 §46 item 5).
**Inferred loss** (0021 §45 item 5). When AetherSDR infers the loss only from the 200 ms
silence or from a sequence restart, without a `TX_ARM_STATE` with `TAKEOVER`, it cannot tell a
takeover from an outage (a link loss, a watchdog standby of the radio, a reset); it MUST then
send the same sequence with the outage indication instead of the takeover one: drop reason
`LINK_DOWN` (C5.5.5 row 9) instead of `TAKEOVER` and stream reason `link` instead of
`takeover`, so that no client is told of a takeover that may not have happened. Recovery from
an inferred loss needs a restart by the user, as after a takeover: AetherSDR MUST NOT set the
run bit again by itself, because another host may own the radio and a run = 1 would take it
over (0022 §3 point 6); a user restart from the same IP address and port after an outage finds
the RX-side settings kept (B5.4). AetherSDR MUST NOT take the radio back by itself (no ping-pong, 0022 §3 point 6); a new session
is started by the user, and a TCI `START` is not such a user action while discovery answers
0x03 (C4, `START`; 0022 §47). At a new session AetherSDR MUST write `TX_MODE_CLASS`, the TCI session
limits (C5.10.1 item 6) and the keyed timeout to P2 again before arming, and send the General packet
with PA enable and the DUC-specific packet again, because the takeover returned them to the
persistent values and defaults and the TX settings of Table A3-4 to their session-start values
(A2.4 step 1; A3.6a; C8.5a item 3; 0022 §64); the same holds after every other session end,
a stop, a watchdog standby or a link loss included, as C8.5a item 3 states (0022 §78; 0021
§45 item 1). After a takeover AetherSDR also sends its
DDC-specific packet again: the radio keeps RX-side settings only for the host whose session
ended by a stop, a watchdog standby or a link loss, and only for its IP address and port,
which AetherSDR's session after a takeover is not (B5.4; C8.5a item 3; 0022 §78 items 2, 4;
0021 §44 item 11).

C7.3 **Priority.** [TCI] gives "the highest priority" to the server's own operator and, among
clients, priority to the first one ([TCI] §3.5). In the profile, the safety core has the highest
priority, above any client and above the server's own user interface (0011 point 3).

C7.3.1 **Controller on level 2** (0021 §9). In profile version 1 the controller (head) has no
control on level 2, as on level 1: inputs that reach the main unit (HIS-17) are reported
(`TX_FOOTSWITCH`) and MUST NOT key the radio (0007 Sub-decision 6, Resolved 6). The priority
[TCI] §3.5 gives to the server's own operator is not given to the controller. Any change needs
answers to HIS-10 and HIS-17, M7, a new decision and a new profile version.

C7.3.2 **P2 while level 2 is active** (0021 §16). A P2 host cannot take the RF chain from level
2: the P2 C&C ports have no socket on level 2 (0008, Socket budget), discovery answers byte 4 =
0x03 (0003; part B B3.1), and the level changes only through the management channel (0008
C-2; C2.7).

C7.4 **Who may transmit.** While armed, any connected client MAY send a TX request (`TRX`
true, `TUNE` true, `KEYER` press, `CW_MACROS`, `CW_MSG`, `G7250_ATU_TUNE`), unless
`G7250_TX_POLICY` is `holder`, in which case only the arm holder MAY (C5.5.8; 0021 §1). The client whose
request keys the radio is the **keying client** (`G7250_TX_STATE` `mine` = `true`). Any client
MAY end a transmission (`TRX` false, `TUNE` false, `CW_MACROS_STOP`, `G7250_TX_ARM:0,00000000;`,
`STOP`, and during an ATU tune cycle `G7250_ATU_TUNE:0,abort;`, which these also abort, C5.7);
these are never blocked by the 200 ms monopoly ([TCI] §4.7 note 6) and take effect
immediately. Making TX stop is always allowed, as tightening is always allowed (0011 point 3).

C7.4.1 **`TRX` and `TUNE` by TX source** (0021 §43 item 5). `TRX` and `TUNE` are synchronised
state ([TCI] §3.1); for each TX source of Table C8-1 they show:

| Row of Table C8-1 | TX source | While the source keys | Server-side unkey (C7.7) reported as |
|-------------------|-----------|-----------------------|---------------------------------------|
| 1–3 | `TRX` | `TRX` `true`, `TUNE` `false` | `TRX:0,false;` |
| 4 | `TUNE` | `TUNE` `true`, `TRX` `false` | `TUNE:0,false;` |
| 5 | `KEYER` | `TRX` `true` from the first key-down that keys ([TCI] §4.4 KEYER: "The first press switches the transceiver to transmission") until the return to receive after the hang delay ("returns to receive when the CW core of the device goes idle after the last release"); no toggle per element | `TRX:0,false;` |
| 6, 7 | CW text, `CW_TERMINAL` | `TRX` `true` from the first element until the queue is empty, terminal mode has ended and the hang delay after the last element has run out ([TCI] §3.2.1: the transceiver switches "to receive mode when the macro is finished") | `TRX:0,false;` |
| 8 | ATU tune cycle | `TUNE` `true` for the cycle, `TRX` `false` | `TUNE:0,false;` |

Each `true` is sent to all clients when the transmission keys (after the relay sequence of
C7.6.5), each `false` when it ends, at once (C4.3 item 3). `TRX:0,false;` and `TUNE:0,false;`
from any client are stop requests for every source (C7.4): for `KEYER` they end the
transmission at once, the hang delay included, and no further key-down is played; for CW text
they end it as `CW_MACROS_STOP`, discard the queue and end the keying of terminal mode (the
client's terminal mode setting stays, C4 `CW_TERMINAL`); for an ATU tune cycle they abort it
(C5.7). Sent while nothing is keyed they have no effect.

C7.5 **Two clients key.** Only one TX request can be active, because there is one transmitter
([G7250-MAN]; `TRX_COUNT:1`):

1. The first TX request that the server processes keys the radio, following [TCI] §3.5 ("the
   priority is behind the first one").
2. A TX request from another client while keyed MUST NOT change the TX source or the TX audio.
   For `TRX` and `TUNE` the server replies with the current state; `G7250_ATU_TUNE` while
   keyed is rejected with `KEYED` from every client, the keying client included (C5.7;
   A3.9b); `CW_MACROS` and `CW_MSG` from another client are ignored; `KEYER` from another
   client is ignored.
3. CW queueing of [TCI] §3.5 (a `CW_MACROS` added in line, a `CW_MSG` that replaces a macro)
   applies only to requests from the keying client.
4. TX audio from any client other than the keying client is discarded, and TX_CHRONO is sent
   only to the keying client.
5. Because `TRX` is synchronised, every client sees `TRX:0,true;`; a client finds out whether
   the transmission is its own from `G7250_TX_STATE` (C5.5.3). A generic client cannot tell,
   which is a known limit of [TCI]; the profile proposes no fix for generic clients.
6. When the keying client's connection closes, the server unkeys (C8.3) and sends the unkey
   broadcast of C7.7.
7. **TX source fixed while keyed** (0021 §43 item 4). While keyed (C7.6: the hang delay and
   queued CW text included) the TX source never changes, also for the keying client: a TX
   request of another source from it (`TUNE:0,true;` while keyed by `TRX`; `TRX:0,true` with
   another source, or while keyed by `TUNE`, `KEYER`, CW text or an ATU tune cycle; a `KEYER`
   press while keyed by CW text, `TRX` or `TUNE`; CW text while keyed by `KEYER`, `TRX` or
   `TUNE`) is refused as in item 2 (`TRX` and `TUNE` answered with the current state, `KEYER`
   ignored, CW text discarded; `G7250_WRITE_STATUS` `KEYED` to an opted-in requester, C5.12) and
   is not latched (A3.1a). CW queueing (item 3) and the restatements of C8.2.3 are the same
   source. The opt-in state that selects the column of Table C8-1 is fixed at the key (C5.2)
   and does not change until the unkey. The row is fixed at the key as well, with one
   exception: CW text in terminal mode moves from row 6 to row 7 when its text has been sent
   and the radio stays keyed, and back to row 6 when a further `CW_MACROS` or `CW_MSG` starts
   new text (C8.2.3); both rows are the same TX source (0021 §44 item 7).
8. **TX audio parameters while keyed with TCI audio** (0021 §43 item 4). While the keying
   client keys with source `tci` and a running `AUDIO_START` (Table C8-1 rows 1 and 2), its
   `AUDIO_STOP` and every change of its `AUDIO_SAMPLERATE`, `AUDIO_STREAM_SAMPLES` or
   `AUDIO_STREAM_CHANNELS` unkeys first (`unkey` 1, TX request released; arm kept; unkey
   broadcast of C7.7), then applies; keying again needs a new TX request, which C6.3.3 checks
   against the new block period. A change of `AUDIO_STREAM_SAMPLE_TYPE` is followed while
   keyed, because every block states its `format` ([TCI] §3.4). An `AUDIO_START` while keyed by
   `TRX:0,true,tci;` without a running `AUDIO_START` (row 3) does not switch the transmission to
   TCI audio: no TX_CHRONO is sent before the next key.

C7.6 **Changes while keyed** (A3.9; 0022 §1, §2; 0021 §8, §19). In C7.6 and C5.7, "keyed" has
the meaning part B gives it for RF-path purposes: the continuous transmission of A3.4d, the CW
hang delay included (part B A3.1a "Keyer elements", A3.4d, A3.9; 0022 §74). For CW text it lasts
from the first element until the queue is empty and the hang delay after the last element has
run out, also across a gap longer than the hang delay (0021 §43 item 6). While keyed:

C7.6.1 **Effective TX frequency.** The commands that move the effective TX frequency are:
`VFO` of the TX channel (channel A, or channel B with split on); `IF` of the TX channel;
`DDS` of receiver 0 when the channel frequency follows it (`VFO` = `DDS` + `IF`; **[unverified]**
whether [TCI] keeps `IF` or `VFO` on a `DDS` change; Annex C-D); `SPLIT_ENABLE`; `XIT_ENABLE`;
`XIT_OFFSET` while XIT is on; and (L1) any server action that sets one of these, such as a
click on a spot ([TCI] §4.4 RX_CLICKED_ON_SPOT). `RIT_ENABLE`, `RIT_OFFSET`, `DIGL_OFFSET` and
`DIGU_OFFSET` do not move it (C4; 0021 §34).
Before applying such a change the server MUST evaluate the new effective TX frequency:

1. If it lies outside the transmit ranges in force, or needs another TX filter, band or relay
   state than the one in use, an ATU memory recall included (A3.9; 0022 §28), the server MUST
   unkey first (`UNKEY`, arm kept; `G7250_TX_STATE`
   `unkey` 6 "retune needs RF-path change", or `unkey` 4 with `LIMIT_TX_FREQ` when the cause
   is the range), then apply the change and switch relays while unkeyed, then send the unkey
   broadcast of C7.7. Keying again needs a new TX request. In a gap of a CW transmission the
   change ends the hang delay as C7.6.7 states, and queued CW text is discarded (C7.7).
2. Otherwise the change MUST be followed while keyed, without unkey.
3. Which frequencies share one RF-path state is the filter plan, pending hardware (P-RF-1; HIS-07); until it is
   known, each transmit range of the `tx_band` list (part B B4.8) counts as its own state, on
   both levels.
4. **At the key** (A3.1b; 0022 §33). While unkeyed the RF path follows the receive frequency
   (`DDS`). Before every key the server checks the RF-path state against the effective TX
   frequency and, if it differs, switches while unkeyed, waits until the relays have settled
   and then keys (C7.6.5). With split cross-band (channel A 7.1 MHz, channel B 14.2 MHz,
   split on) the radio receives through the 7 MHz state, switches to the 14 MHz state before
   each key, an ATU tune cycle included (C5.7), and returns after each unkey once RF is gone,
   with CW break-in after the hang delay (C7.6.7).
5. **(L1) DUC0 is the effective TX frequency** (0021 §44 item 3). The radio applies the TX
   frequency limit (`LIMIT_TX_FREQ`, Table A3-2) and the RF-path state at the key (A3.1b) to
   the P2 DUC0 frequency (High Priority bytes 329–332 [P2]; glossary "Effective TX
   frequency"). AetherSDR MUST therefore set its DUC0 frequency at all times to the effective
   TX frequency of C4 (channel A, or channel B with split on, plus the XIT offset when XIT is
   on) and MUST NOT realise split, XIT or any other TX offset as a frequency shift of the
   baseband DUC I&Q: a shift in the baseband would put RF outside the range the radio checks,
   for example across a band edge, or through the filter of another `tx_band` entry. The
   offsets of `DIGL_OFFSET` and `DIGU_OFFSET`, which shift only the audio passband (C4), are
   not TX offsets in this sense. `TX_FREQUENCY` and the DUC0 frequency are the same value.

C7.6.2 **Other frequency changes.** A change that does not move the effective TX frequency
(channel A with split on, channel B with split off, `RIT_OFFSET`) is followed; if it needs
another receive filter or relay state, the relay change is deferred until the unkey, before
any new key, as A3.9 defers resent P2 state fields (0021 §19). At the unkey the path returns
to the state for the receive frequency then in force (A3.1b; 0022 §33); with split cross-band
this is the return after every transmission.

C7.6.3 **Refused while keyed.** The server MUST refuse, from any client, while keyed in the
sense of C7.6 (the hang delay and queued CW text included), changes of
`MODULATION` (0021 §8: the TX mode class must not change under a transmission; the TCI form of
A3.9c, 0022 §26), of the RF-path relays `G7250_ANT`, `G7250_ATU`, `G7250_PREAMP` and
`G7250_ATT` (A3.9; 0022 §2), and `G7250_ATU_TUNE` (A3.9b; 0022 §22). It replies with the
value in force and sends the opted-in requester `G7250_WRITE_STATUS` with `KEYED` (C5.12).
These refusals are the TCI form of the command rule of A3.9 ("a change requested by a command
is refused"), and they hold in a gap between CW elements as well; on P2 an owner's relay or
mode-class change in a gap ends the hang delay instead (A3.4d; 0022 §74). A refused change
leaves the transmission and its queued CW text as they are; a `MODULATION` change can be
accepted only after the transmission has ended, and then no queued text remains, because text
that cannot key is discarded (A3.1a; 0021 §43 item 6).
(L1) A mode change in AetherSDR's own user interface while keyed, the hang delay included, is
a `TX_MODE_CLASS` change on P2, which unkeys first (A3.9c; A3.4d), then applies; AetherSDR
discards queued TCI CW text with that unkey (C7.7).

C7.6.4 **Followed while keyed.** `DRIVE`, `TUNE_DRIVE`, `RX_FILTER_BAND`, `DIGL_OFFSET`,
`DIGU_OFFSET` and other settings that switch no relay and do not move the effective TX
frequency (0021 §34).

C7.6.5 **Relay sequence** (A3.9a; 0022 §23). The guard and settle times of A3.9a apply on
both levels: after an unkey no relay moves until RF is gone, and the radio keys only after the
relays have settled. A TX request (`TRX:0,true`, `TUNE:0,true`, a `KEYER` press, CW text)
that arrives while relays move or settle, whatever the cause (a frequency or relay command in
the same frame or an earlier one, the switch to the TX state of A3.1b, the return after an
unkey), waits as A3.9a item 3: relays, settle, then key if the request still stands; the wait
is bounded, and a further change of the TX RF-path state during it (an effective TX frequency
that needs another state, `G7250_ANT`, `G7250_ATU`, `G7250_PREAMP`, `G7250_ATT`) refuses the request, which is
not latched (0022 §37). A change that moves only the receive frequency during the wait
(`DDS`, or channel A with split on, while the effective TX frequency stays in its state) does
not refuse it: its relay change is deferred to the next unkey, as while keyed (C7.6.2; C7.6.1 for when `DDS` moves the TX frequency; A3.9a
item 3; 0022 §73 item 10). (L1) AetherSDR sends both in one High Priority packet or in
any order; the radio applies A3.9a.

C7.6.6 **Bound on the wait for RF gone** (A3.9a item 1a; 0022 §68). Every wait for RF gone on
TCI, whatever caused the unkey (a client's `TRX:0,false;`, a server-side unkey of C7.7, a drop
of C5.5.5, a TCI session end, a level change of C2.7, an address change of C5.5.5 row 14, a
commanded reset of C2.10), is bounded by `t_reset_wait` = 2 s from the unkey. If RF is still
present at the bound, the radio enters or stays in the fault state with `RF_NOT_KEYED` (class
2), reported by `G7250_SAFETY` and, if armed, as drop row 7 (`FAULT`); relays still do not move
while RF is present. The actions that move no relay then proceed: the session end, the level
change (C2.7 items 3–5), the address change (row 14); a commanded reset is refused (C2.10 item
2). The same holds on level 1 for AetherSDR's P2 session (A3.9a item 1a).

C7.6.7 **CW break-in and the hang delay** (A3.9a item 4; A3.4d; 0022 §73 item 9). With CW
break-in (`KEYER`, CW text), between elements RF stops but SEND, the T/R relay and the RX
attenuator during TX stay in their TX position for the hang delay; only when the hang delay
has run out without a new element are SEND released, RF being gone, and both returned, and the
deferred changes of C7.6.2 applied. A drop of C5.5.5 (a disarm and `STOP` included), a fault entry, a
TCI session end or a level change during the hang delay ends it at once (A3.9a item 4). The SWR cap holds over
the same span (C8.1; A3.4a). In a gap between elements (A3.4d, A3.9; 0022 §74; 0021 §43 item 6):

1. a change of C7.6.1 item 1 (another TX RF-path state, or outside the ranges) ends the hang
   delay: unkey (`unkey` 6, or 4 with `LIMIT_TX_FREQ`), RF gone (C7.6.6), SEND released, the
   T/R relay and the RX attenuator during TX returned, and only then the change is applied and
   relays switch. The reply with the new value and its synchronisation to all clients
   (C7.1) are sent only once the change has been applied, as part B answers such a write only
   after the change (A3.9; 0022 §74); the unkey broadcast of C7.7 comes first;
2. a change of C7.6.3 is refused, as while RF is on;
3. a change of the receive frequency alone (C7.6.2) stays deferred until the hang delay ends;
4. a controller ANT or ATU button press stays ignored, as while keyed (A3.9; C7.3.1).

Each element is a key under every condition of A3.1a (part B A3.1a "Keyer elements"; 0022 §74).
If a condition fails at an element (an inhibit, the effective TX frequency outside the ranges,
the arm dropped), that element does not key and the keyer stops: for `KEYER` the key-downs
already received are not played, for CW text the queue is discarded; the transmission ends with
the unkey broadcast of C7.7, and keying again needs a new request edge (a new `KEYER` press or
new CW text), the stopped request not being latched.

C7.7 **Unkey broadcast** (0021 §24). When the server unkeys without a request from a client
(keyed timeout or generic keyed window, TX audio underrun, a limit, the maximum TX time, a
retune of C7.6.1, a drop of C5.5.5, a level change, a takeover, the keying client's
connection closing, the key-down cap, a mode class change, a TX audio parameter change of C7.5
item 8, a keyer element whose condition fails, C7.6.7, a `G7250_TX_POLICY:holder` set while
a non-holder keys, C5.5.8), it MUST send `TRX:0,false;` if
`TRX` was `true` and `TUNE:0,false;` if `TUNE` was `true`, to all clients, at once (C4.3 item 3), by the TX source as C7.4.1 states, as the
synchronisation of [TCI] §3.1 requires for every change of state, and MUST discard queued CW
text. At every unkey, requested or not, the server MUST discard buffered TX audio, as part B
empties the DUC FIFO (B10.7; 0022 §32). Opted-in clients also receive
`G7250_TX_STATE` with the unkey reason. A client that sends `TRX:0,true;` afterwards makes a
new TX request.

---

## C8 Safety: limits and timeouts

C8.1 **Limits.** All limits of 0007 Sub-decision 4 apply on both levels with the same built-in
values, actions (WARN, REDUCE, UNKEY, DISARM, FAULT) and recovery (0007 rule 5; 0011 point 3).
They are reported over TCI by `G7250_LIMIT_STATUS`, `G7250_TX_STATE` and `G7250_SAFETY`, and
tightened by `G7250_LIMIT` (C5.10). `REDUCE` and the power measurement are those of part A
(A3.3, A3.4a, A3.4c; 0022 §7, §17; 0021 §21); TCI only reports them. A TCI power limit is
mapped as every other tier (Table A3-3; 0022 §52) and bounds the tune power (0022 §53).
Without a PEP detector every bound acts on the PEP estimate of A3.4c, the measured mean ×
the digital crest factor of the samples fed to the DUC (0022 §62). **(L2)** These are the
samples the level-2 DSP feeds to the DUC: the output of the on-board modulator for TCI TX
audio and for a microphone path, the CW waveform of `KEYER` and CW text, and the `TUNE` and
ATU tune carriers. A level-2 TX path whose RF does not come from samples fed to the DUC has
no estimate and does not key without a PEP detector; whether one exists: HIS-18 (A3.4c). **(L1)** They are AetherSDR's DUC I&Q after the drive scaling, as for any P2
host. The SWR cap `P_swr` holds to the end of the continuous transmission of A3.4d, not only
to the next unkey: with CW break-in (`KEYER`, CW text) it holds across element gaps shorter
than the hang delay (A3.4a; 0022 §67). The maximum continuous TX
time (`LIMIT_TX_TIME`) is 5 min built in, `UNKEY` only, arm kept (0007, Resolved item 3); it
applies to every TX source, including `CW_TERMINAL` and `TUNE`; "continuous" is as A3.4d (with
CW break-in, gaps shorter than the hang delay do not end a transmission; 0022 §51).

C8.2 **What replaces the P2 watchdog** (0021 §18). The P2 watchdog is "no C&C packet for 1 s"
[P2] (0007 Decision 2.1), and the keyed timeout is fed only by the High Priority packet, which
carries PTT, so that a host whose High Priority packets stop while other C&C continues is
unkeyed (0007 Decision 2, option 2C; option 2B rejected). The keyed timeout does not catch a
host whose I/O thread keeps sending a cached PTT or CWX bit while its TX logic has hung; on P2
the backstops for that case are the DUC underrun check, the CWX key-down cap and the maximum
TX time (part B B10.5, B10.5a; 0022 §25). On TCI the same role is played by the TX audio
underrun, the key-down cap, the CW text liveness and the generic keyed window, which only a
restatement of the TX request feeds (C8.2.3). TCI has no keep-alive of its own. The profile
separates the two timers:

- **Application frame.** A complete WebSocket text message that carries at least one complete
  TCI command ([TCI] §3.1), or a complete binary message that carries exactly one
  TX_AUDIO_STREAM block ([TCI] §3.4) that C2.2 and C6.3.5 accept; a fragmented message counts
  when its last fragment arrives (C2.2). A binary message discarded for its form (C2.2, C6.3.5)
  is not an application frame; a well-formed block whose audio is not used (C6.3.4, C7.5 item
  4) still is (0021 §44 item 13). WebSocket control frames (Ping, Pong, Close; RFC 6455 §5.5)
  are never application frames.
- **Connection watchdog.** Fed by any frame, Pong included (RFC 6455 §5.5.2: an endpoint that
  receives a Ping "MUST send a Pong frame in response"), so that an arm holder needs no
  traffic of its own to stay connected while it is idle (0022 §32). It only drops the arm; it
  never keeps the radio keyed.
- **Keyed liveness.** Fed only by the keying client: the keyed timeout by any application frame
  from it, the generic keyed window only by a restatement of its TX request (C8.2.3). A Pong
  MUST NOT feed either.

**Keyed liveness by TX source** (0021 §27, §29). Table C8-1 is total: for every TX
source, in every mode, the keying client is bound by exactly one keyed-liveness rule, chosen by
the source and by whether the client had opted in (C5.2) when the transmission keyed (C5.2,
"Opt-in state while keyed"). The other timers of Table C8-2 act in addition where the table
says so.

Table C8-1 — Keyed-liveness rule of the keying client

| # | TX source (what keyed the radio) | Opted-in keying client | Keying client not opted in | Also acting | Source |
|---|----------------------------------|------------------------|----------------------------|-------------|--------|
| 1 | `TRX:0,true,tci;` with a running `AUDIO_START`, any mode other than `CW` | keyed timeout | keyed timeout (its TX audio blocks are application frames) | TX audio underrun | 0021 §7, §18 |
| 2 | `TRX:0,true,tci;` with a running `AUDIO_START`, in `CW` | keyed timeout | generic keyed window | — | 0021 §18 |
| 3 | `TRX:0,true;` without source, or with `mic1`, `mic2`, `micpc`, `ecoder2`, or `tci` without a running `AUDIO_START` (C4); any mode; possible only where a microphone path exists (HIS-17) | keyed timeout | generic keyed window | — | 0021 §27 |
| 4 | `TUNE:0,true;` | keyed timeout | generic keyed window | — | 0021 §18 |
| 5 | `KEYER` key-down and the key-up gaps of the same transmission (`CW` only, C4) | keyed timeout | generic keyed window | key-down cap | 0021 §18, §33; 0022 §25 |
| 6 | CW text while the server sends it (`CW_MACROS`, `CW_MSG`) | CW text liveness | CW text liveness | — (keyed timeout and generic keyed window do not run) | 0022 §25; 0021 §29 item 1 |
| 7 | `CW_TERMINAL` after its text has been sent, radio still keyed | keyed timeout | generic keyed window | — | 0021 §18 |
| 8 | ATU tune cycle (`G7250_ATU_TUNE`) | keyed timeout; ends the cycle as `failed` | — (a start is ignored, C5.2) | `LIMIT_ATU_TIME` (C5.7) | 0021 §29 item 2 |

In every row the maximum TX time (C8.1), the loss of the keying client's connection and, for
the arm holder, the connection watchdog act as well (Table C8-2).

Table C8-2 — Timers

| Timer | Applies to | Server duty | Timeout and action | 0007 counterpart |
|-------|------------|-------------|--------------------|------------------|
| Connection watchdog | the arm holder, while armed | send a WebSocket Ping at least every 250 ms | no frame of any kind (Pong included) from the arm holder for **1 s** → drop the arm, `WATCHDOG` | Decision 2.1, base watchdog 1 s; drop event 3 |
| Keyed timeout | the keying client while keyed, where Table C8-1 names it | — | no application frame from the keying client within the keyed timeout in force (`G7250_KEYED_TIMEOUT`, default **500 ms**, floor 100 ms) → `UNKEY`, arm kept; `unkey` 2 | Decision 2, option 2C, `KEYED_TIMEOUT`; Resolved item 2 |
| Generic keyed window | the keying client while keyed, where Table C8-1 names it | — | no restatement of the TX request (C8.2.3) from the keying client for **1 s** (built in, not writable) → `UNKEY`, arm kept; `unkey` 2 | Decision 2.1 (the same 1 s) |
| TX audio underrun | the keying client, Table C8-1 row 1 | send TX_CHRONO ([TCI] §3.4) at the block period of C6.3.3 | no TX_AUDIO_STREAM block for longer than `TX_STREAM_AUDIO_BUFFERING` + **500 ms** → `UNKEY`, arm kept; `unkey` 3 (0021 §7) | analogue of Decision 2, TX IQ underrun; fixed in the profile, independent of a hardware FIFO |
| Key-down cap | the keying client, Table C8-1 row 5, opted in or not | — | a key-down played for more than **1 s** from the start of its playback without the release being played, whatever other frames arrive → `UNKEY`, arm kept; `unkey` 9 (C4 `KEYER`) | part B B10.5a (CWX cap); 0022 §25 |
| CW text liveness | the keying client, Table C8-1 row 6, opted in or not | — | no application frame from the keying client for **30 s** while text is sent → `UNKEY`, queued text discarded; `unkey` 2 | 0022 §25 (no P2 counterpart: P2 has no server-generated text) |
| Connection loss | arm holder / keying client | — | TCP close, reset or WebSocket close: holder → drop the arm (`HOLDER_LOST`); keying client → `UNKEY`, `unkey` 8 (keying client lost); both in one connection → `unkey` 8 and drop `HOLDER_LOST` (0021 §29 item 4) | drop event 3; 0022 §32 |

Every unkey of these tables is followed by the unkey broadcast of C7.7.

**Tolerance** (0021 §41 item 7). For every timer of Table C8-2 the server MUST NOT act before
the time stated and MUST have acted no later than 10 ms after it, as on P2 (part B B10.5,
B10.5a); a Ping interval of "at least every 250 ms" means no gap longer than 250 ms.

C8.2.1 **Deviation (Annex C-B, C-B1; 0021 §7).** [TCI] allows a client that is not ready to
"not send a response" to TX_CHRONO, and calls a block of zero samples "preferable" ([TCI]
§3.4). The profile requires the keying client to answer every TX_CHRONO while keyed with
source `tci`, with zero samples if it has no signal; no TX audio for longer than
`TX_STREAM_AUDIO_BUFFERING` + 500 ms unkeys and keeps the arm. The 500 ms equals the default
`KEYED_TIMEOUT` (0007, Resolved 2). **[unverified whether existing clients always answer;
Annex C-D]**; the 500 ms is the tolerance for late blocks.

C8.2.2 **Duty of an opted-in keying client.** While keyed, an opted-in keying client MUST send
an application frame at least every 100 ms, as the P2 host sends High Priority (0007 Decision
2; part B B10.5); `G7250_KEEPALIVE;` (C5.5.3) serves when it has nothing else to send. The
server MUST NOT unkey before the keyed timeout in force and MUST have unkeyed no later than
10 ms after it, as on P2 (part B B10.5). The 1 s and the keyed timeout are the same values as
on level 1 so that both levels behave alike (0014).

C8.2.3 **Generic keyed window** (0021 §18, §28; Annex C-B, C-B9). Only a **restatement** of
the TX request from the keying client restarts the 1 s: `TUNE:0,true;` while keyed by `TUNE`
(Table C8-1 row 4); `TRX:0,true` with the same source while keyed by `TRX` (rows 2, 3;
`TRX:0,true;` and `TRX:0,true,tci;` from a client without a running `AUDIO_START` are both
microphone requests, C4 `TRX` and C-C21, so they are the same source and each restates the
other; 0021 §44 item 14). Edge case (0021 §45 item 12): a `TRX:0,true,tci;` from a client
keyed in row 3 that has started `AUDIO_START` while keyed (C7.5 item 8) would be a row-1
request if it keyed now, but the row is fixed at the key (C7.5 item 7), so it is a restatement
of the row-3 request: it restarts the window and changes nothing else, the transmission stays
a microphone one, no TX_CHRONO is sent and the TX audio underrun does not act until a new key;
any
`KEYER` command while keyed by `KEYER` (row 5); a `CW_MACROS` or `CW_MSG` while keyed in
terminal mode (row 7), which starts new text and so row 6. A restatement while keyed changes
nothing else: it is not a new 0→1 request (A3.1a). Other commands, read requests and set
requests of other parameters included, do not restart the window, so a client whose polling
loop runs on while its TX logic has hung is unkeyed after 1 s. While the server itself sends CW text (`CW_MACROS`, `CW_MSG`), the transmission
ends with the text and the window does not run; it starts when the text has been sent and the
radio stays keyed (terminal mode). 1 s equals the P2 base watchdog (0007 2.1), and a dash at 5
words per minute (720 ms) fits in it. **[unverified]** whether existing clients resend a
command during a long `TUNE`; a generic `TUNE` therefore ends after 1 s unless the client
restates `TUNE:0,true;` (Annex C-D, D7). Long tune carriers need an opted-in client; ATU
tuning is `G7250_ATU_TUNE` (C5.7).

C8.2.3a **Known limit** (0021 §28, §43 item 7; 0022 §25 point 5). The keyed timeout is fed by
any application frame, as the P2 keyed timeout is fed by any High Priority packet (0007
Decision 2). A keying client whose I/O thread keeps sending frames while its TX logic has hung
is caught only by the maximum TX time (or a stricter `LIMIT_TX_TIME`, C5.10), as a P2 host that
keeps sending a cached PTT bit and cached I&Q (0022 §25 point 5), in these rows of Table C8-1:

- **Row 1, opted in or not.** A client that keeps answering TX_CHRONO, with zero samples or
  with cached audio, feeds both the keyed timeout and the TX audio underrun; the underrun
  catches only a client that stops answering (C8.2.1). In `AM` and `NFM` zero audio is not
  silence on the air: AM sends its full carrier (up to 25 W) and FM its full carrier (up to
  100 W) until the maximum TX time; in SSB zero audio gives no RF.
- **Rows 2–4 and 7 of an opted-in client.** `G7250_KEEPALIVE` or other frames hold `TUNE`,
  `TRX` in `CW` or a microphone `TRX`.

Part C so narrows the 0022 §25 statement that "each TX source that a cached bit or a queue can
hold on now has a bound well below the maximum TX time": that holds for a generic client in
rows 2–5 and 7 (generic keyed window), for every client for a `KEYER` key-down (key-down cap)
and for CW text (CW text liveness), and in row 1 only for a client that stops answering
TX_CHRONO. Profile 1 adds no further bound (0021 §43 item 7): an unkey on a run of all-zero TX
audio blocks in `AM` or `NFM` would end transmissions that [TCI] treats as normal (a block of
zero samples "corresponds to no signal" and "this option is preferable", [TCI] §3.4), and P2 has
the same limit for cached I&Q, so both levels behave alike (0014).

C8.2.4 **(L1)** AetherSDR's P2 keep-alive (High Priority every 100 ms from its I/O thread;
[AETHER-SRC] `P2Client.h:238–241`, per `annex-aethersdr.md` G-08c) does not depend on its TCI
clients, so the radio's keyed timeout protects only against AetherSDR itself stopping. AetherSDR
MUST apply C8.2 to its TCI clients, MUST NOT set the P2 PTT bit (High Priority byte 4 bit [1]
[P2]) for a TCI keying client whose keyed liveness has run out, and MUST clear it when the
TCI-side rule unkeys, before it forwards any further TX to P2 (0021 §18 point 6).
Independently, the radio applies the P2 watchdog and keyed timeout to AetherSDR (0007 Decision
2), so a crash of AetherSDR unkeys the radio even if AetherSDR's TCI side cannot act.

C8.2.5 **(L1) TCI CW onto P2** (0021 §36). AetherSDR MUST send the CW of a TCI `KEYER` and of
TCI CW text (`CW_MACROS`, `CW_MSG`, `CW_TERMINAL`) as DUC I&Q with PTT (part B B10.5), shaped
in its own TX chain, not with the CWX paddle bits of High Priority byte 5: the paddle bits drive
the radio's own keyer at its speed and weight, so they cannot play the client's straight-key
timing of [TCI] §4.4 KEYER, and a paddle bit held for a string longer than 1 s is cut by the
CWX cap (B10.5a, host note). With this mapping the caps that act on level 1 are: on the TCI
side, applied by AetherSDR, Table C8-1 and Table C8-2 (keyed timeout or generic keyed window,
the key-down cap on the played `KEYER` key-down, the CW text liveness); on the P2 side, applied
by the radio to AetherSDR, the watchdog, the keyed timeout, the DUC underrun check (AetherSDR
keeps DUC I&Q running during key-up, B10.5) and the maximum TX time, with CW gaps shorter than
the hang delay counted as continuous (A3.4d). The CWX cap does not act, because CWX is not
used. `TX_MODE_CLASS` is 2 (CW) (C5.6). Because the RF is formed from DUC I&Q, AetherSDR MUST
keep DUC-specific byte 5 bit [1] (CW from the radio's keyer) clear while it sends TCI CW this
way: with that bit set the radio's keyer forms the RF and drops DUC I&Q (part B B10.5b;
0022 §85). The transmission is then a PTT transmission for break-in purposes (0021 §46).

1. **Break-in and hang delay** (0021 §46 item 1). AetherSDR implements break-in and the hang
   delay of TCI CW itself, by holding the P2 PTT bit: PTT is set at the first element, stays
   set with DUC I&Q at zero between elements, and is cleared only when the hang delay has run
   out without a new element (or at an unkey of C7.7). The hang delay is the value AetherSDR
   writes to DUC-specific bytes 11–12, clamped to 2000 ms as part B A3.4d and Table B10-1
   bytes 11–12 clamp it; with a hang delay of 0, PTT is set and cleared per element. Within
   one PTT transmission the elements count as one continuous transmission (A3.4d), and
   `TRX` stays `true` until the PTT is cleared (C7.4.1).
   Note on a hang delay of 0 (0021 §47 item 2). Each element is then a PTT transmission of
   its own, so each element costs the key latency, the T/R settle `t_settle` plus the RF
   delay of DUC-specific byte 13 (part B A3.9a items 2, 4), and cycles the T/R relay, SEND
   and an external amplifier. I&Q that AetherSDR sends with the PTT edge waits for the key
   in the DUC FIFO, where samples are kept only within the 25 ms of part B B10.7, so the
   start of an element is cut when the key latency exceeds that. Two PTT edges also need the
   10 ms rule of part B B11.6a (Annex X1 D28): a 0→1 edge counts only 10 ms or more after
   the latest packet with the bit at 1. For break-in CW on level 1 a hang delay above 0
   is RECOMMENDED.
2. **Gap check** (0021 §46 item 2, as corrected by 0021 §47 item 1; 0022 §88 item 1). The
   check "RF while not keyed" runs in every gap between elements, also on level 1, where the
   elements of TCI CW are DUC I&Q of a PTT transmission: inside a PTT transmission with DUC
   I&Q, RF is commanded only up to the last non-zero DUC sample, and a run of all-zero
   samples longer than `t_blank` is a gap for that check (part B A3.9a item 6; Table A3-2;
   threshold and `t_blank` pending hardware P-SEN-3; HIS-11). RF in a gap is detected within `t_blank` plus the
   reaction time of 10 ms, while the PTT stays set. AetherSDR MUST send exact zeros
   (I = Q = 0) in the gaps between elements and during the hang delay of item 1. Remaining
   known limit: non-zero samples in a gap (noise, dither, a DC offset, a ramp tail running
   into the gap) command RF, so the check does not run in that gap; RF there that the
   samples do not command is then seen only at the next all-zero run longer than `t_blank`
   or after the unkey, at most the maximum continuous TX time later. The DUC underrun check
   and the limits act throughout (annex G-13).
3. **Hardware CW selected** (0021 §46 item 3). While the user has hardware CW from KEY
   selected in AetherSDR (DUC-specific byte 5 bit [1] set, part B B10.9), AetherSDR MUST
   refuse a TCI CW TX request (`KEYER` press, `CW_MACROS`, `CW_MSG`, `CW_TERMINAL` text) with
   `NOT_KEYABLE` (C5.12), not latched, and MUST NOT clear the bit by itself: with the bit set
   the radio would drop the DUC I&Q of the TCI CW (B10.5b item 2.3), and the user's choice of
   the radio's keyer stays as set.

C8.3 **Keying client gone, holder still there.** If the keying client disconnects while the arm
holder stays, the server unkeys and keeps the arm (UNKEY, analogue of the keyed timeout;
`unkey` 8, 0022 §32) and sends the unkey broadcast of C7.7. (L1) AetherSDR clears the P2 PTT
bit; the radio reports P2 `TX_STATE` reason 1, AetherSDR reports `unkey` 8 on TCI.

C8.4 **Fault state.** Entry, effect, classes, clearing (with the 10 s hold-off), escalation
after the third entry of a code and the counters as A3.5 (0007 Decision 5; 0022 §5, §27).
In fault, RX and RX streams continue, so clients can still see the radio and the fault (0007
Decision 5, "Effect"). `TX_ENABLE` is `false`, arm requests are rejected with `IN_FAULT`, and
the fault is reported by `G7250_SAFETY` (C5.10).

C8.5 **State after reset** is that of A3.6 (0007 Decision 5): not armed, new token, limits at the
persistent values read from the stored configuration, keyed timeout 500 ms, policy `any`.
(L2) After a reset the TCI server starts with `TRX` and `TUNE` `false`, `RECEIVE_ONLY` and
`TX_ENABLE` as C3.2 and C5.5.6, and the device started as C4 `START` (automatic start).

C8.5a **TX settings at a session end, a takeover and a level switch** (A3.6, A3.6a, Table A3-4;
0022 §64; 0021 §42).

1. **(L2)** The settings of Table A3-4 (PA enable, hardware CW from KEY, keyer options,
   break-in, EER, hang delay, RF delay, CW ramp, RX step attenuator during TX) have no TCI
   command; on level 2 the firmware sets them for its own TX chain, values pending G6 (P-L2-3). At every
   level switch, in both directions, they MUST return to the values of Table A3-4 after the
   unkey and before the new level's first packet or command is processed, as at a level-1
   session end (A2.5a step 4; A3.6a): a level-1 owner's values never carry into level 2, and
   level 2's never into level 1. At the end of a TCI session (C5.10.1 item 4) the TX-side
   state of the session ends as well: session limits (C5.10.1 item 4), `G7250_KEYED_TIMEOUT`
   500 ms (C5.10), `G7250_TX_POLICY` `any` (C5.5.8), queued CW text discarded (C7.7); no TCI
   client can change a setting of Table A3-4, so none is left behind by a client.
2. The standard TCI TX parameters (`DRIVE`, `TUNE_DRIVE`, `SPLIT_ENABLE`, `XIT_ENABLE`,
   `XIT_OFFSET`, `MON_ENABLE`, `MON_VOLUME`, the CW speed and delay commands of C4) are
   synchronised device state ([TCI] §3.1) and are kept across a TCI session end, as RX
   settings are kept (A2.4 step 4): every client receives their values at connection (C3.1),
   so no client keys with a TX setting it cannot see. Whatever their values, forward power
   stays within the bounds of C8.1.
3. **(L1)** The radio returns the settings of Table A3-4 at every end of AetherSDR's P2 session,
   a takeover included (A2.4 step 1; A3.6a). After a stop, a watchdog standby or a link loss
   it keeps the RX-side settings (every General packet field except byte 38 bit [0] and
   byte 58 bit [0], the DDC-specific packet and the receive state of the High Priority
   packet) for the same IP address and port, and a High Priority packet with run = 1 from
   that host restarts the session with them, with no 0→1 edge of the host's run bit needed
   (B5.4; A2.3; 0022 §78 items 2–4; 0021 §45 item 8). AetherSDR MUST therefore, after every
   session end, send its General packet with PA enable, which an arm needs in any case
   (a General packet of the new session with the watchdog bit, B10.2 item 3), and its
   DUC-specific packet (CW select, break-in, hang delay, RF delay, keyer, DUC rate) again, in
   its set-up phase or before it arms (B5.1a; C7.2.1). After every session end, with or
   without a takeover, AetherSDR MUST also write, before it arms, `TX_MODE_CLASS` (C5.6),
   every P2 `LIMIT_<name>` register as C5.10.1 item 6 sets it, and `KEYED_TIMEOUT` if it uses
   it (C5.10), because the radio returns them at every session end (A3.6; B5.4 restart item
   2; 0022 §78): a TCI session value in force (C5.10.1) is thus in force on P2 again before
   the arm (0021 §45 item 1). AetherSDR MUST wait for the P2 write echo of these writes (part
   B B8.4) before its first PTT 0→1 edge of the session (0021 §46 item 4). A resend of the DDC-specific packet is
   not needed when AetherSDR restarts from the same IP address and port after a stop, a
   watchdog standby or a link loss; after a takeover, a reset, a firmware update, a level
   change or an address change it is (C7.2.1; 0022 §78 item 4). Its TCI parameters of item 2 are AetherSDR's own state and
   are kept (0021 §44 item 11).

C8.6 **LAN security** (0021 §10). The arm prevents accidental transmission by software that does
not know the G7250; it is not security ("this is not security", 0007 Decision 1 rule 3). TCI
has no authentication [TCI], and the profile adds none to the TCI connection, so that generic
clients keep working (0012). Any accepted client can opt in and read the token. Level 2 is
meant for a trusted LAN; the mitigations are the reach rule and the optional allow-list of
C2.4.1, and the optional TX policy of C5.5.8. Traffic is in the clear, as on the management
channel (0008 S6).

---

## C9 Conformance tests

One line per test; tests run against the reference model on the product side (0019; 0021
§17.3) and against both implementations (0014, Consequences; 0001 item 5). Ids are stable.

| Id | Test |
|----|------|
| T-C2-01 | A WebSocket client connects on the configured port; commands arrive in text frames, streams in binary frames. |
| T-C2-02 | Several complete commands in one text frame are all executed. |
| T-C2-03 | (L2) `clients_max` clients connect; the next one receives `G7250_REFUSED:busy;` and close 1008; the listener accepts again afterwards. |
| T-C2-04 | (L2) A level change while keyed and armed sends, in this order, `TRX:0,false;`, `G7250_TX_ARM_STATE` with `LEVEL_CHANGE`, a new token, `TX_ENABLE:0,false;`, `G7250_STREAM_STATE` `stopped` `level_change`, `G7250_PROFILE` with `active` false, and then closes every connection with 1001 (C2.7). |
| T-C2-05 | Without configuration the server listens on TCP port 50001; after a port change through the management channel (L2) it listens on the new port only. |
| T-C2-06 | The WebSocket upgrade succeeds on `/` and on an arbitrary other path. |
| T-C2-07 | (L2) A connection from outside the radio's subnet is closed without handshake and logged. |
| T-C2-08 | (L2) With a non-empty allow-list, a connection from a non-listed address in the subnet is closed without handshake; a listed address connects. Setting the list without authentication fails. |
| T-C2-09 | (L2) `clients_max` in `G7250_PROFILE` is 5 while port 1024 still answers discovery with byte 4 = 0x03. |
| T-C2-10 | (L2) After `UPDATE_BEGIN`, running streams report `stopped` `update`, `TX_ENABLE` is false, a `VFO` set is answered with the value in force and `BUSY`, a valid `G7250_TX_ARM` is rejected with `MAINTENANCE`, not `NOT_RUNNING`, although the device is stopped (C5.5.3), `IQ_START` gets `refused` `update`; before the restart every connection closes with 1001; during the unconfirmed trial boot after it a valid arm is rejected with `MAINTENANCE` (C2.8). |
| T-C2-11 | (L2) On a 10 Mbit/s link connections are accepted, `START` is not followed, `IQ_START` gets `refused` `link`, and an arm is rejected with `NOT_RUNNING` (C2.9). |
| T-C2-12 | (L2) A `G7250_TX_ARM` and `UPDATE_BEGIN` that reach the radio within 1 ms of each other, repeated 1000 times: either the arm is confirmed and `UPDATE_BEGIN` gets status 50, or the update starts and the arm is rejected with `MAINTENANCE`, never with `NOT_RUNNING`; never both (C2.8; A1.8; C5.5.3). |
| T-C2-13 | (L2) With forward power held above `P_rf_off` (reference model), `RESTART` gets status 59, no connection is closed and `G7250_SAFETY` reports `RF_NOT_KEYED`; otherwise the connections close with 1001 only after the reply (C2.8 item 4; C2.10 item 2; A1.9). |
| T-C2-14 | (L2) `RESTART` while keyed and armed: `TRX:0,false;`, `G7250_TX_ARM_STATE` with `MAINTENANCE`, a new token and `TX_ENABLE:0,false;` arrive before the management reply, every connection closes with 1001 only after it; with RF held (status 59) the drop reason is `MAINTENANCE` as well; with `FACTORY_RESET` the drop reason is `MAINTENANCE`, and afterwards the server listens on 50001 with an empty allow-list (C2.10 item 1; A1.6a, A1.9). |
| T-C2-15 | (L2) An allow-list change that no longer allows the arm holder's address closes that connection with 1008 and drops the arm with `HOLDER_LOST`; a keying client so closed is unkeyed with `unkey` 8; in both cases the unkey broadcast and the drop messages arrive, and the radio is unkeyed, before the management reply and before the close; allowed connections stay open, and a list that closes no such connection leaves the TX state unchanged (C2.10 item 3; A1.9). |
| T-C2-16 | (L2) A TCI port change with connections open keeps them open with their arm and key state; new connections are accepted only on the new port (C2.10 item 4). |
| T-C2-17 | (L2) Drop event 9 while started, and `UPDATE_BEGIN` while started, send `STOP;` to all clients, generic clients included; a return to 100 Mbit/s sends no `START;` (C2.9a). |
| T-C2-18 | A command fragmented over three frames is executed once; for an opted-in keying client the keyed timeout restarts at the last fragment, not at the first (C2.2). |
| T-C2-19 | (L1) A 1→2 level switch while unarmed makes AetherSDR send its TCI clients `G7250_STREAM_STATE` `stopped` `level_change`, `G7250_PROFILE` with `active` false and `STOP;`, never `takeover` (C2.7). |
| T-C2-20 | A binary message from a client that carries two TX audio blocks, or half of one, is discarded; a message with one block is used (C2.2). |
| T-C2-21 | (L2) After drop event 9 while started and the last client leaving, the return of the link to 100 Mbit/s starts the device without a client `START`, and the next client finds `START;` in the state after `READY;`; with a client connected throughout, the return starts nothing and the device stays stopped until a client `START` (C2.9a, C4 `START`; 0021 §45 item 4). |
| T-C3-01 | Initialization order is PROTOCOL, DEVICE, RECEIVE_ONLY, TRX_COUNT, CHANNELS_COUNT, VFO_LIMITS, IF_LIMITS, MODULATIONS_LIST, READY, then state. |
| T-C3-02 | Values match C3.2; `CHANNEL_COUNT` is never sent. |
| T-C3-03 | `IF_LIMITS` is resent after a receiver sample-rate change. |
| T-C3-04 | A `VFO` or `DDS` set to 40 MHz or outside VFO_LIMITS is not followed; the current value is replied. |
| T-C3-05 | No `G7250_` command reaches a client that has not opted in. |
| T-C3-06 | `PROTOCOL:ExpertSDR3,2.1;` is sent on both levels; (L2) `DEVICE:G7250;` is sent. |
| T-C3-07 | `MODULATION:0,FM;` selects narrow FM and is replied as `MODULATION:0,NFM;`; `MODULATIONS_LIST` contains `NFM` and not `FM`. |
| T-C3-08 | An `IF` set that would move channel A to 40 MHz is not followed; the current value is replied (C3.3). |
| T-C4-01 | Each MUST command of C4 is accepted and its state replied and synchronised to a second client. |
| T-C4-02 | NOT SUPPORTED and reserved commands are ignored without a reply and never sent. |
| T-C4-03 | Commands with trx 1 or a channel number above 1 are ignored without a reply. |
| T-C4-04 | `TX_ENABLE` set by a client is ignored and the current state is replied. |
| T-C4-05 | `TRX:0,true;` without `tci` is refused when no microphone path exists. |
| T-C4-06 | `TUNE` produces a carrier at `TUNE_DRIVE` and never starts the ATU. |
| T-C4-07 | Unsupported IQ or audio rates are ignored and the current rate replied. |
| T-C4-08 | 200 ms monopoly and 30 ms tick behave as [TCI] §4.7 notes 6 and 7, except for stop commands. |
| T-C4-09 | (L1) While the radio answers discovery with 0x03, a TCI `START` makes AetherSDR send no C&C until its user confirms; after confirmation it starts the session. |
| T-C4-10 | Within 200 ms of another client's change, a stricter `G7250_LIMIT`, a lower `G7250_KEYED_TIMEOUT` and `G7250_TX_POLICY:holder` take effect; a looser `G7250_LIMIT` is dropped; a `TRX` that goes true→false→true within 30 ms is reported as `TRX:0,false;` then `TRX:0,true;`, and the unkey broadcast is not delayed to the tick (C4.3). |
| T-C4-11 | In `USB`, `KEYER:0,true,0;` does not key and does not key after a change to `CW`; in `CW` a `KEYER:0,false,1500;` is played as a 1000 ms key-down (C4 `KEYER`). |
| T-C4-12 | In `DIGU`, a `DIGU_OFFSET` change leaves `TX_FREQUENCY` unchanged and, while keyed, is followed without unkey (C4; C7.6.4). |
| T-C4-13 | Without a microphone path, `TRX:0,true,tci;` from a client without a running `AUDIO_START` is refused with `TRX:0,false;`; with `AUDIO_START` it keys (C4 `TRX`). |
| T-C4-14 | (L2) After power-on with level 2 active, a generic client that never sends `START` receives `START;` in the state after `READY;` and RX audio after `AUDIO_START`; after a client `STOP;` and the last client leaving, the next client finds the device started; on a 10 Mbit/s link it is not started (C4 `START`; C2.9). |
| T-C4-15 | Client A sets `CW_TERMINAL:0,true;` and keys with `CW_MACROS`; client B's `CW_TERMINAL:0,false;` does not end A's terminal mode, and the radio stays keyed after the macro until A's `CW_TERMINAL:0,false;` or a stop request (C4 `CW_TERMINAL`). |
| T-C4-16 | Armed, in `USB`, `CW_MACROS:0,TEST;` from an opted-in client does not key and gets no `G7250_WRITE_STATUS`; after a change to `CW` the text is not sent; a new `CW_MACROS` in `CW` keys (C4 `CW_MACROS`; 0021 §44 item 15). |
| T-C5-01 | `G7250_EXT:true;` yields the reply `G7250_EXT:true;`, then PROFILE, FEATURES and full extension state in order; `G7250_EXT:false;` yields the reply `G7250_EXT:false;` and then no further extension message (C5.2). |
| T-C5-02 | `G7250_FEATURES` lists exactly the functions that work: (L2) before M7 `tx` is absent and `RECEIVE_ONLY:true;` is sent; (L1) without part B group 4, `atu` is absent and `G7250_ATU_TUNE` is ignored without a reply; with a reference model that has no current sensor, `absent_current` is listed and `HW_SENSORS` reports current `0`; `ant`, `atu_select`, `preamp` and `att` are absent on level 1 (C5.1.4, C5.4). |
| T-C5-03 | A correct `G7250_TX_ARM` arms with a new generation; a wrong value is rejected with `BAD_TOKEN`; the token is never `00000000`, `FFFFFFFF` or `ARM_MAGIC`. |
| T-C5-04 | Each precondition of C5.5.6 items 1–7, when false, rejects the arm with its Table B8-3 reason (`NOT_RUNNING`, `IN_FAULT`, `INHIBITED`, `PTT_ACTIVE`, `MAINTENANCE`, `NO_MODE_CLASS`, `TX_NOT_IMPLEMENTED`); (L1) with level 2 active, shown by management identification I-4, a valid arm is rejected with `LEVEL_NOT_ACTIVE`, not `NOT_RUNNING` (C5.5.3; 0021 §44 item 4). |
| T-C5-05 | After arming, a `TRX` state already `true` does not key; a new 0→1 request does. |
| T-C5-06 | Each drop event of C5.5.5 unkeys, disarms, issues a new token and reports the reason, with the Table B8-3 names, `STOP` and `HOLDER_LOST`. |
| T-C5-07 | `TX_ENABLE` is `true` exactly when C5.5.6 items 1, 2 and 4–9 hold, and is resent on every change; armed, a `TRX:0,true;` that keys and the `TRX:0,false;` that ends it send no `TX_ENABLE`, which stays `true` throughout (C5.5.6; 0021 §44 item 1). |
| T-C5-08 | `G7250_TX_MODE_CLASS` follows `MODULATION` by the table of C3.4 (`DIGL`/`DIGU` give `ssb`); `WFM` gives `none` and blocks arming with `NO_MODE_CLASS`. |
| T-C5-09 | In class `am` at `DRIVE:0,100`, *L* = 100 W, by detector model of Table A3-3: with "PEP and envelope samples", with and without 100 % modulation, the carrier estimate stays within 25 W and the PEP within 100 W, and a modulated signal is not held to a 25 W mean; with "PEP, no envelope samples" the mean stays within 25 W and the PEP within 100 W (unmodulated: carrier 25 W; 100 % sine modulation: carrier about 16.7 W, PEP about 67 W); with "mean only" the PEP estimate stays within 100 W and the mean within min(*L* / 4, 25 W) = 25 W, with the same carrier and PEP values (A3.4c; Table A3-3). |
| T-C5-10 | `G7250_ATU_TUNE` needs the arm, follows the tune time limit and reports `tuned` or `failed`. |
| T-C5-11 | `G7250_ANT`, `G7250_ATU`, `G7250_PREAMP`, `G7250_ATT` (where present) and `MODULATION` are refused while keyed, from any client, with the value in force and `G7250_WRITE_STATUS` `KEYED` to an opted-in requester; `DRIVE` while keyed is followed. |
| T-C5-12 | With a persistent value stricter than the built-in one, `G7250_LIMIT` accepts a value equal to or stricter than the persistent value, rejects a value between persistent and built-in with `LOOSER_LIMIT`, always replies the limit in force, returns to the persistent value on a write of 0 and after the last client leaves. |
| T-C5-13 | `G7250_FAULT_CLEAR` with the current code clears a class-1 fault only when the condition is gone and 10 s have passed since the latest entry, and never re-arms; a wrong or missing code is rejected with `BAD_VALUE`; the third entry of the same code since power-on is not clearable (A3.5). |
| T-C5-14 | (L1) A TCI arm causes a P2 arm handshake and is confirmed only after the radio confirms. |
| T-C5-15 | (L1) Every P2 drop event is reported on TCI with the P2 reason. |
| T-C5-16 | Armed and not keyed, a change to `WFM` keeps the arm, sets `TX_ENABLE` false and refuses `TRX` and `TUNE`; a change back re-enables TX without a new arm. |
| T-C5-17 | `G7250_TX_POLICY` defaults to `any`; while armed any client can set `holder`; only the arm holder can set `any`; it returns to `any` after the last client leaves, on a level change and on reset. |
| T-C5-18 | With policy `holder`, `TRX`, `TUNE`, CW and `G7250_ATU_TUNE` from a non-holder are refused, while its `TRX` false, `CW_MACROS_STOP`, disarm and `STOP` still take effect. |
| T-C5-19 | A valid `G7250_TX_ARM` from client B while client A holds the arm leaves the generation and the holder unchanged; B receives `holder` false. |
| T-C5-20 | With policy `holder`, a drop or disarm returns the policy to `any`; the next arm starts with `any`. |
| T-C5-21 | A `G7250_LIMIT` write of 0 or a looser session value from a client that is not the arm holder, while a holder exists, is rejected with `LOOSER_LIMIT`; from the holder it is accepted. |
| T-C5-22 | `G7250_LIMIT` takes only `LIMIT_TX_TIME`, `LIMIT_FWD_POWER`, `LIMIT_SWR`, `LIMIT_PA_TEMP` in part B units; another id is rejected with `BAD_VALUE`. |
| T-C5-23 | `G7250_KEYED_TIMEOUT` accepts 100–500 ms, rejects other values with `BAD_VALUE`, and returns to 500 ms after the last client leaves. |
| T-C5-24 | `G7250_SAFETY` carries the Table B8-3 fault code name, the highest class among the flagged codes and the `FAULT_FLAGS` bit layout; `G7250_FAULT_CLEAR:<current code>;` with a class-2 code flagged is rejected with `NOT_CLEARABLE` and clears nothing. |
| T-C5-25 | `G7250_ATU_TUNE` without the arm is rejected with `NOT_ARMED` and does not start later when the radio is armed. |
| T-C5-26 | An extension whose `G7250_FEATURES` token is absent is ignored without a reply; with `ant_state` only, a `G7250_ANT` set is answered with the value in force and `READ_ONLY`. |
| T-C5-27 | (L1) With AetherSDR's interface and a TCI client both holding the arm, the TCI client's disconnect leaves the P2 arm set; with the TCI client as the only holder, it writes `TX_ARM` = 0 over P2. |
| T-C5-28 | `G7250_ATU_TUNE` while keyed with `TRX`, `TUNE` or CW, from the keying client and from another client, is rejected with `KEYED`, does not start after the unkey and does not end the transmission; a repeated `G7250_ATU_TUNE` during a tune cycle is a no-op; every cycle ends unkeyed. |
| T-C5-29 | `TRX:0,true;` refused while unarmed, inhibited, outside the ranges or in `WFM` does not key when the condition clears; a new `TRX:0,false;` then `TRX:0,true;` keys. `CW_MACROS` sent while unarmed is discarded and not sent after a later arm. |
| T-C5-30 | `G7250_TX_STATE` `unkey` reports 8 after the keying client's connection closes, 9 after the key-down cap, and never 10 as the result of a TCI command (`MODULATION` is refused while keyed); (L1) `inhibits` equals the P2 `LIMIT_INHIBIT` register. |
| T-C5-31 | Drop reasons on TCI are `STOP` for `STOP;` and `HOLDER_LOST` for the holder's connection loss, as Table B8-3 drop reasons 13 and 14; (L1) a TCI `STOP` makes AetherSDR clear the P2 run bit and its TCI clients see `STOP`. |
| T-C5-32 | (L1) With the P2 arm held by AetherSDR's user interface, a valid `G7250_TX_ARM` from a TCI client makes it a holder without a P2 write or a new generation; a second TCI client's arm is then a no-op with `holder` false. |
| T-C5-33 | (L1) With two holders, the one that set policy `holder` can set `any` and the other cannot; when the first stops holding, the remaining holder can; under `holder` both holders key and a third client cannot. |
| T-C5-34 | `G7250_ATU_TUNE` while armed and unkeyed with the effective TX frequency outside the ranges, or inhibited, is rejected with `NOT_KEYABLE`; repeated 100 ms after a cycle ended it is a no-op; after a failed cycle `G7250_ATU_STATE` stays `failed` and `TRX` still keys. |
| T-C5-35 | A `G7250_FAULT_CLEAR` from a client that has not opted in clears a clearable fault; a wrong name from an opted-in client gets `BAD_VALUE` and a class-2 fault `NOT_CLEARABLE`. |
| T-C5-36 | With a mean-only detector model, `G7250_LIMIT:LIMIT_FWD_POWER,10000;` holds class `ssb` with 6 dB PEP-to-mean ratio to a mean of 2.5 W, `cw` to 10 W and `am` to 2.5 W, at `DRIVE:0,100`; the reply carries 10000 (C5.10.1 item 5; Table A3-3). |
| T-C5-37 | With `G7250_LIMIT:LIMIT_FWD_POWER` below the ATU profile's tune power, `G7250_ATU_TUNE` tunes at the limit, not at the profile power (C5.7; A3.4 rule 5). |
| T-C5-38 | (L2) While armed and keyed, a DHCP renewal with another address unkeys, drops the arm with `SESSION_CHANGED` (row 14), and the new address applies only after RF is gone; a renewal with the same address changes nothing (C5.5.5; A1.11a). |
| T-C5-39 | After an under-voltage `UNKEY` with inhibit, `G7250_TX_STATE` `inhibits` has bit 5 set until the supply has been inside the range for 2 s, and `G7250_TX_ARM_STATE` shows `inhibited` while armed (C5.5.3). |
| T-C5-40 | (L1) With nothing armed, a generic TCI client's `TRX:0,true;` (and a CAT or rigctl TX request) is refused with `TRX:0,false;` and causes no P2 `TX_ARM` write and no P2 PTT (C5.5.7). |
| T-C5-41 | (L1) After a TCI client set `MODULATION:0,USB;`, AetherSDR's user interface sets its TX slice to AM: P2 `TX_MODE_CLASS` becomes 3, and TCI clients receive `MODULATION:0,AM;` and `G7250_TX_MODE_CLASS:0,am;`; the same change while keyed unkeys first with `unkey` 10 (C5.6). |
| T-C5-42 | During an ATU tune cycle, `TRX:0,true;` from any client neither keys nor ends it; `TUNE:0,false;`, `TRX:0,false;` or `G7250_ATU_TUNE:0,abort;` from any client ends it at once, unkeyed, with `G7250_ATU_STATE` `failed`; (L1) AetherSDR writes `ATU_TUNE` = 0 (C5.7). |
| T-C5-43 | A `G7250_ATU_TUNE:0;` from a client that has not opted in, while armed and unkeyed, starts no cycle and gets no reply (C5.2; C5.7). |
| T-C5-44 | Client B's `G7250_TX_ARM` with a wrong value gives B `reject` `BAD_TOKEN` and leaves the holder's `reject` unchanged; with the device stopped and in fault, an arm is rejected with `NOT_RUNNING`, and a wrong value with `BAD_TOKEN`; (L2) during a firmware update (device stopped) a valid arm is rejected with `MAINTENANCE`; during an unconfirmed trial boot a valid arm is rejected with `MAINTENANCE` (L1: the P2 reject reason passed on); in fault with an inhibit active the reason is `IN_FAULT`; armed and keyed, a valid arm from the holder is a no-op, not `PTT_ACTIVE` (C5.5.3). |
| T-C5-45 | At every drop of C5.5.5 the messages arrive in the order unkey broadcast (if keyed), `G7250_TX_ARM_STATE`, `G7250_TX_ARM_TOKEN`, `TX_ENABLE:0,false;`. |
| T-C5-46 | `G7250_FEATURES` is resent to opted-in clients when `tx` disappears (L2: transmit disabled; L1: P2 session end); with `ANT_STATE` 0, `G7250_ANT` reports `0` (C5.2; C5.8). |
| T-C5-47 | (L1) After the last TCI client leaves, the P2 `LIMIT_FWD_POWER` and `KEYED_TIMEOUT` hold the values of AetherSDR's user interface, not the former TCI session values (C5.10.1 item 8). |
| T-C5-48 | While armed, a client that has not opted in sets `G7250_TX_POLICY:holder;`; the policy becomes `holder` (C5.5.8). |
| T-C5-49 | With a mean-only detector model and *L* = 100 W: (L2) TCI TX audio in `USB` with a 12 dB crest factor at `DRIVE:0,100` is held to a PEP estimate of at most 100 W (mean about 6.3 W), not to the class bound of 25 W mean, and `TX_SENSORS` arg4 reports the estimate; a `KEYER` carrier and a `TUNE` carrier in `CW` are held to a mean of *L*; (L1) AetherSDR's DUC I&Q of SSB with a 6 dB crest factor sent while its mode class is CW is held to a mean of about *L* / 4 (C8.1; A3.4c; reference model). |
| T-C5-50 | With split on, channel A at 7.1 MHz and channel B at 14.2 MHz, `G7250_ATU_TUNE:0;` switches the RF path to the 14 MHz TX state and waits the settle time before the tune carrier; after a cycle that ended `tuned` and RF gone the path returns to the 7 MHz state, and the next `TRX:0,true` switches to the 14 MHz state with the tuned result; after an aborted cycle nothing is stored for 14.2 MHz (C5.7; A3.9b items 2, 7; reference model). |
| T-C5-51 | (L1) With AetherSDR's user interface at `LIMIT_FWD_POWER` 50 W: `G7250_LIMIT:LIMIT_FWD_POWER,30000;` makes AetherSDR write 30 000 mW to P2 and the reply carries 30000; a later `G7250_LIMIT:LIMIT_FWD_POWER,70000;` from the holder is rejected with `LOOSER_LIMIT` and the reply carries 30000; a write of 0 from the holder makes AetherSDR write 50 000 mW and the reply carries 50000 (C5.10.1 item 6). |
| T-C5-52 | An opted-in client receives `G7250_WRITE_STATUS:TRX,NOT_ARMED;` for `TRX:0,true;` while unarmed, `TRX,KEYED` while another client keys, `TRX,BAD_VALUE` for `TRX:0,true;` without a microphone path, `CW_MACROS,NOT_KEYABLE` while inhibited, `G7250_TX_POLICY,LOOSER_LIMIT` for `any` from a client that is not the policy owner; a client that has not opted in receives none, and `KEYER` outside `CW` gets none (C5.12). |
| T-C5-53 | `G7250_TX_ARM:0,0;` and `G7250_TX_ARM:0,00000000;` disarm from any client; `G7250_TX_ARM:0,1234;`, `G7250_TX_ARM:0,XYZ12345;` and `G7250_TX_ARM:0;` are rejected with `BAD_TOKEN` (C5.5.3). |
| T-C5-54 | With token `atu` and without `atu_select`, `G7250_ATU` reports `bypass` for `ATU_STATE` 0, `inline` for 1, 2 and 3, after a failed cycle (4) the value reported before the cycle, and `unknown` for 5; a set while the state is unknown gets `NOT_CALIBRATED`, before `READ_ONLY` (C5.7; 0032). |
| T-C5-55 | (L1) During an ATU tune cycle, the expiry of the requesting client's keyed timeout and the loss of its connection each make AetherSDR write `ATU_TUNE` = 0 and report `G7250_ATU_STATE` `failed` (C5.7). |
| T-C5-56 | Armed, client B (not the holder) keys with `TUNE`; client A sets `G7250_TX_POLICY:holder;`: B's transmission ends at once with `unkey` 1, `TUNE:0,false;` to all clients, the arm is kept, and B's next `TUNE:0,true;` is refused; with the holder keying instead, the same set leaves the transmission keyed; B's unkey is a server-side unkey of C7.7, reported to an opted-in B as `G7250_TX_STATE` `unkey` 1 (C5.5.8, C7.7; 0021 §44 item 5, §45 item 6). |
| T-C5-57 | A client that has not opted in sends a valid `G7250_TX_ARM` (the value derived from a token it saw before an opt-out): the radio stays disarmed, no reply and no `G7250_TX_ARM_STATE` change follow, and a later opt-in does not arm; its `G7250_TX_ARM:0,0;` while armed disarms (C5.2, C5.5.3; 0021 §44 item 20). |
| T-C5-58 | While disarmed, `G7250_TX_POLICY:holder;` is answered with `G7250_TX_POLICY:any;` and, to an opted-in requester, `G7250_WRITE_STATUS:G7250_TX_POLICY,NOT_ARMED;`; after the next arm the policy is `any` (C5.5.8; 0021 §44 item 17). |
| T-C5-59 | With client A the arm holder and `G7250_KEYED_TIMEOUT:200;` in force, client B's `G7250_KEYED_TIMEOUT:400;` is rejected with `LOOSER_LIMIT` and the reply carries 200; A's is accepted (C5.10; 0021 §44 item 16). |
| T-C5-60 | (L1) With only AetherSDR's user interface holding the arm and a TCI session value of `LIMIT_FWD_POWER` in force, a `G7250_LIMIT:LIMIT_FWD_POWER,0;` from a TCI client is rejected with `LOOSER_LIMIT` (C5.10.1 item 3; 0021 §44 item 18). |
| T-C5-61 | `G7250_FEATURES` contains only tokens of Table C5-1; for every MAY or SHOULD command of C4 that the server under test implements, its token is present, and for every one it does not implement the token is absent and, where the token stands for whole commands, the command is ignored without a reply; where it stands for a part of a command, that part is refused as C4 states: without `ch_b_rx`, `RX_CHANNEL_ENABLE:0,1,true;` is answered with `RX_CHANNEL_ENABLE:0,1,false;`; without `agc_fast`, `AGC_MODE:0,fast;` is ignored; without `audio_rates_all` or `iq_384`, an unsupported `AUDIO_SAMPLERATE` or `IQ_SAMPLERATE` is answered with the rate in force; without `mic`, a microphone `TRX` (Table C8-1 row 3) is answered with `TRX:0,false;`; where the server under test implements 384000 IQ or a microphone path, `iq_384` or `mic` respectively is present and `IQ_SAMPLERATE:384000;` or a microphone `TRX` is followed; `KEYER` support shows `keyer`, a KEY input keyer shows `cw_keyer` (C4.1, C5.4; 0021 §44 item 6, §45 items 3, 10). |
| T-C5-62 | (L1) A TCI `STOP;`, and separately a P2 watchdog expiry in the reference model, make AetherSDR send `G7250_PROFILE` with `active` false to opted-in clients; the next session start sends `active` true (C5.3; 0021 §44 item 10). |
| T-C5-63 | An opted-in client keys with `KEYER` in `CW` and sends two key-downs with a gap shorter than the hang delay: every client receives `G7250_TX_STATE` with `keyed` `true`, `false`, `true` and `false` (one toggle per element) while `TRX` stays `true` until the hang delay has run out; in the gap `source` is `keyer` and `mine` is `true` for the keying client and `false` for the others (C5.5.3 `G7250_TX_STATE`, C7.4.1; 0021 §45 item 13). |
| T-C5-64 | (L1) With TCI `KEYER` elements sent as DUC I&Q with PTT, `G7250_TX_STATE` `keyed` toggles per element by AetherSDR's own element timing while P2 `TX_STATE` stays keyed; with level 2 active (I-4) and a trial boot shown by I-3, a valid arm is rejected with `MAINTENANCE`, not `LEVEL_NOT_ACTIVE`; under the policy unkey of a non-holder's tune cycle AetherSDR writes `ATU_TUNE` = 0 (C5.5.3, C5.5.8, C5.7; 0021 §46 items 7–9). |
| T-C6-01 | Stream headers are 64 bytes, little-endian, with correct receiver, rate, format, length, type, channels and count; each frame is header plus `length` samples. |
| T-C6-02 | IQ streams at each offered rate without loss for a set duration; a test tone appears at the right offset and sign. |
| T-C6-03 | RX audio honours sample type, channels and samples per block. |
| T-C6-04 | TX_CHRONO goes only to the keying client; its TX audio is transmitted; others' TX audio is discarded. |
| T-C6-05 | (L2) A stream start over the budget is not followed and is reported as `budget`. |
| T-C6-06 | (L2) A second IQ rate requested while another client streams IQ is refused with the current rate. |
| T-C6-07 | IQ blocks carry `format` = 3 (FLOAT32) at every offered rate. |
| T-C6-08 | TX_CHRONO `length` equals the keying client's `AUDIO_STREAM_SAMPLES` and `channels` its `AUDIO_STREAM_CHANNELS`; at `AUDIO_SAMPLERATE` 8000 with 1 channel, 2048 samples (256 ms) make `TRX:0,true,tci;` refused at the default keyed timeout and 1024 samples (128 ms) key; with 2 channels 2048 samples (128 ms) key; with `G7250_KEYED_TIMEOUT:100;` 1 channel with 512 samples (64 ms) is refused and 2 channels with 512 samples (32 ms) key (C6.3.3; C-C25). |
| T-C6-09 | In `CW`, `TRX:0,true,tci;` from a client with a running `AUDIO_START` keys a CW carrier at `DRIVE`; no TX_CHRONO is sent, its TX audio blocks are discarded and `TX_SENSORS` arg2 is `-140` (C6.3.4). |
| T-C6-10 | A keying client (TX_CHRONO `length` 1024, 2 channels) answers with blocks of `length` 512: they are played in full within the buffer bound, the TX audio underrun does not act and the keyed timeout is fed; a block of `length` 511 with 2 channels, or with more than 16384 data bytes, is discarded, is not an application frame and feeds neither timer, so an opted-in client sending only such blocks is unkeyed at the keyed timeout; a block of `length` 0 is accepted and feeds both timers; a block with `channels` 1, `sample_rate` 8000 or `receiver` 1 against the client's 2 channels, 48 kHz and transceiver 0 is discarded and feeds neither timer; with `TX_STREAM_AUDIO_BUFFERING` 50 ms, blocks sent at twice the period are partly dropped, the oldest buffered frames first, so the TX audio on the air lags the client's blocks by no more than 50 ms plus one block, the TX audio underrun does not act and the keyed timeout is fed (C6.3.5, C8.2; 0021 §44 items 2, 13; §45 item 2). |
| T-C7-01 | A parameter set by one client is reported to all others. |
| T-C7-02 | With two clients, the first TX request keys; the second's `TRX`, `TUNE`, CW and KEYER are refused or ignored. |
| T-C7-03 | Any client can unkey with `TRX` false, `TUNE` false, `CW_MACROS_STOP`, disarm or `STOP`, immediately. |
| T-C7-04 | `G7250_TX_STATE` `mine` is true only for the keying client. |
| T-C7-05 | While keyed, each command of C7.6.1 (`VFO`, `IF`, `DDS`, `SPLIT_ENABLE`, `XIT_ENABLE`, `XIT_OFFSET`, channel B with split) that moves TX outside a transmit range, or into another `tx_band` entry, unkeys before the change applies (arm kept, `unkey` 4 with `LIMIT_TX_FREQ` or `unkey` 6); a small retune inside the same entry is followed without unkey. |
| T-C7-06 | Every server-side unkey of C7.7 sends `TRX:0,false;` (or `TUNE:0,false;`) to all clients, including clients that have not opted in. |
| T-C7-07 | While keyed with split on, a channel A change that needs another receive filter is followed and the relay switches only after the unkey. |
| T-C7-08 | (L1) After a P2 takeover of AetherSDR, its TCI clients receive the C7.2.1 sequence with `TAKEOVER`, the connections stay open, and AetherSDR does not retake the radio by itself. |
| T-C7-09 | A frame with `VFO` into another `tx_band` entry and `TRX:0,true;`, sent while unkeyed, switches relays, waits the settle time and then keys; after any unkey no relay moves until RF is gone (A3.9a; reference model). |
| T-C7-10 | While keyed, a frequency change inside one `tx_band` entry that would recall another ATU state unkeys with `unkey` 6 before the recall (A3.9; HIS-08). |
| T-C7-11 | After every unkey, buffered TX audio is discarded: the first TX audio transmitted at the next key is from blocks received after the new key. |
| T-C7-12 | (L1) When DDC0 data and High Priority status stop for more than 200 ms while AetherSDR is the P2 owner, AetherSDR stops sending C&C and sends its TCI clients the C7.2.1 sequence with `LINK_DOWN` and stream reason `link`, never `TAKEOVER`, and does not set the run bit again until its user restarts (C7.2.1 "Inferred loss"; 0021 §45 item 5). |
| T-C7-13 | With split on, channel A at 7.1 MHz and channel B at 14.2 MHz, unkeyed the RF path is in the 7 MHz state; `TRX:0,true` switches to the 14 MHz state, waits the settle time and keys; while keyed a channel A change is followed for receive and its relay change waits for the unkey; after the unkey and RF gone the path returns to the 7 MHz state (A3.1b; reference model). |
| T-C7-14 | A `VFO` frame into another `tx_band` entry followed 10 ms later by `TRX:0,true;` keys after the relays have settled and is not refused (C7.6.5). |
| T-C7-15 | (L1) When AetherSDR's DDC0 stream restarts at sequence 0 without its own stop and run, AetherSDR stops sending C&C and sends its TCI clients the C7.2.1 sequence with `LINK_DOWN` and stream reason `link`, never `TAKEOVER`, and does not set the run bit again until its user restarts (C7.2.1 "Inferred loss"; 0021 §45 item 5). |
| T-C7-16 | With forward power held above `P_rf_off` after a `TRX:0,false;` (reference model), the radio enters the fault state with `RF_NOT_KEYED` no later than `t_reset_wait` after the unkey, `G7250_SAFETY` reports it, the arm drops with `FAULT`, no relay moves while RF is present, and (L2) a level change in the same state completes with the messages of C2.7 (C7.6.6; A3.9a item 1a). |
| T-C7-17 | With CW break-in and a hang delay of 200 ms, `KEYER` elements 100 ms apart keep SEND and the T/R relay in their TX position; 200 ms after the last element and RF gone both return; a `STOP;` during the hang delay releases SEND at once after RF gone (C7.6.7; A3.9a item 4; reference model). |
| T-C7-18 | With split on, channel A at 7.1 MHz and channel B at 14.2 MHz, a channel A `VFO` change to 3.6 MHz (channel B unchanged) during the settle wait of `TRX:0,true` does not refuse the key, and its relay change is applied after the next unkey; a `VFO` of channel B into another `tx_band` entry during the wait refuses the key (C7.6.5; A3.9a item 3; reference model). |
| T-C7-19 | Keyed by `TUNE`, a `TRX:0,true,tci;` from the keying client leaves `TUNE` keyed, is answered with `TRX:0,false;` and (opted in) `G7250_WRITE_STATUS:TRX,KEYED;`, and does not key after `TUNE:0,false;`; during CW text a `KEYER` press from the keying client is ignored (C7.5 item 7). |
| T-C7-20 | Keyed with `TRX:0,true,tci;` in `USB`, an `AUDIO_STREAM_SAMPLES` change and, separately, `AUDIO_STOP` from the keying client unkey with `unkey` 1 and `TRX:0,false;` to all clients, arm kept; an `AUDIO_STREAM_SAMPLE_TYPE` change is followed without unkey (C7.5 item 8). |
| T-C7-21 | A first `KEYER` press sends `TRX:0,true;` to all clients; `TRX` stays `true` across element gaps shorter than the hang delay and becomes `false` after it; `CW_MACROS` keeps `TRX` `true` until the queue is empty and the hang has run out; an ATU tune cycle sends `TUNE:0,true;` then `TUNE:0,false;`; a key-down cap unkey sends `TRX:0,false;`; a generic `TRX:0,false;` during the hang ends it at once and during CW text discards the queue (C7.4.1). |
| T-C7-22 | With a hang delay of 200 ms, in a 100 ms gap between `KEYER` elements: `MODULATION` and `G7250_ANT` are refused with `KEYED` and the transmission continues; `G7250_ATU_TUNE` is rejected with `KEYED`; a `VFO` into another `tx_band` entry unkeys with `unkey` 6, and SEND is released and the T/R relay returned before the relays switch, with the `VFO` reply sent only after the change is applied; during CW text it also discards the queue; a `DDS` change for receive only is applied after the hang ends (C7.6.7; A3.4d; reference model). |
| T-C7-23 | During a `KEYER` sequence, an inhibit set between elements stops the keyer: the next key-down is not played, `TRX:0,false;` is sent, key-downs already received are not played after the inhibit clears, and the next `KEYER` press after the clear keys (C7.6.7; A3.1a). |
| T-C7-24 | (L1) With XIT on, `XIT_OFFSET` +3000 and channel A 1 kHz below the upper edge of a `tx_band` entry, `TX_FREQUENCY` and AetherSDR's P2 DUC0 frequency are channel A + 3 kHz, an opted-in, armed client with a running `AUDIO_START` and a block period within C6.3.3 sending `TRX:0,true,tci;` is refused (`TRX:0,false;`, `G7250_WRITE_STATUS:TRX,NOT_KEYABLE;`, outside the range), and AetherSDR's DUC I&Q carries no 3 kHz shift; with a block period beyond half the keyed timeout as well the reason is `BAD_VALUE` (order of C5.12); with `XIT_OFFSET` −3000 DUC0 is channel A − 3 kHz and the key is accepted; with split on DUC0 is channel B (+ XIT) (C7.6.1 item 5; C5.12; reference model; 0021 §44 item 3, §45 item 11). |
| T-C8-01 | Arm holder silent for 1 s (Pong suppressed) → arm dropped with `WATCHDOG`; an arm holder that answers only Pings keeps the arm. |
| T-C8-02 | Opted-in keying client that sends no application frame for the keyed timeout in force, while its WebSocket stack still answers every Ping, → unkey (not before the timeout, not later than 10 ms after it), arm kept. `G7250_KEEPALIVE` every 100 ms keeps it keyed. |
| T-C8-03 | An opted-in client keyed with `TRX:0,true,tci;` in `USB` stops answering TX_CHRONO but keeps sending other application frames (`G7250_KEEPALIVE` every 100 ms), so the keyed timeout does not act → unkey after `TX_STREAM_AUDIO_BUFFERING` + 500 ms (not before), `unkey` 3, arm kept; zero-sample answers keep the radio keyed (Table C8-2; 0021 §44 item 8). |
| T-C8-04 | Arm holder TCP reset → arm dropped with `HOLDER_LOST`; keying client TCP reset → unkey with `unkey` 8. |
| T-C8-05 | Maximum TX time (5 min built in, also with `CW_TERMINAL` and `TUNE`) unkeys and keeps the arm. |
| T-C8-06 | In fault, RX streams continue, `TX_ENABLE` is false and arm requests are rejected. |
| T-C8-07 | After reset, the server is disarmed with a new token, limits at the stored persistent values (C8.5), and `TRX`/`TUNE` false. |
| T-C8-08 | Not in v1.0 (0030). (L2) A controller PTT input (if HIS-17 gives a path) is reported by `TX_FOOTSWITCH` and never keys, armed or not. |
| T-C8-09 | `TX_SENSORS` arg2 reports the TX audio level in dB full scale while keyed with source `tci`, and `-140` without TX audio. |
| T-C8-10 | A generic client keyed with `TUNE` that sends no application frame, while answering Pings, is unkeyed after 1 s, arm kept; one that resends `TUNE:0,true;` every 500 ms stays keyed. |
| T-C8-11 | A generic `KEYER` key-down of 720 ms is not cut; a key-down held for 1 s without a further `KEYER` is unkeyed; an opted-in client's key-down held for more than 1 s is unkeyed with `unkey` 9 although it sends `G7250_KEEPALIVE` every 100 ms. |
| T-C8-12 | A generic client keyed with `TRX:0,true,tci;` in `USB` that stops sending TX audio blocks but answers Pings is unkeyed within the keyed timeout. |
| T-C8-13 | (L1) When the TCI-side keyed liveness of a TCI client runs out, AetherSDR's next P2 High Priority packet carries PTT = 0, although its P2 keep-alive continues. |
| T-C8-14 | `TX_SENSORS` arg3 is the 100 ms mean forward power and arg4 the PEP (with a mean-only detector model, the PEP estimate of A3.4c); in class `am` with an unmodulated carrier at full drive arg3 settles at or below 25 W plus the margin of Table A3-2, and with 100 % sine modulation arg3 is about 1.5 times the carrier and arg4 at most 100 W. |
| T-C8-15 | A `REDUCE` for SWR above S1 caps forward power at `P_swr` to the end of the continuous transmission, while `DRIVE` keeps the client's value; with `KEYER` and CW break-in the cap holds across element gaps shorter than the hang delay and lifts only after the hang delay has run out; the next continuous transmission starts uncapped (C8.1; A3.4a). |
| T-C8-16 | While the server sends CW text for a keying client that sends no application frame, the radio unkeys after 30 s, discards the queued text and reports `unkey` 2; a `CW_MACROS` every 20 s keeps it sending. |
| T-C8-17 | With a microphone path in the reference model, a generic client keyed with `TRX:0,true;` in `USB`, with `TRX:0,true,mic1;` in `AM` and with `TRX:0,true,tci;` in `CW`, that answers Pings but restates nothing, is unkeyed after 1 s (not before 1000 ms, by 1010 ms), arm kept; restating `TRX:0,true` with the same source every 500 ms keeps it keyed (Table C8-1 rows 2, 3). |
| T-C8-18 | A generic client keyed with `TUNE` that sends `VFO:0,0;` reads and `DRIVE` sets every 100 ms but no `TUNE:0,true;` is unkeyed after 1 s (C8.2.3). |
| T-C8-19 | An opted-in client's `CW_MACROS` text of 20 s, with no further frame from that client, is sent in full and not cut at the keyed timeout; at 40 s of text without a frame it is cut at 30 s (Table C8-1 row 6). |
| T-C8-20 | A keying client that is not the holder sends `G7250_EXT:false;` while keyed by `TUNE` and then `G7250_KEEPALIVE` every 100 ms: it stays keyed until the unkey; its next `TUNE` key is bound by the generic keyed window. A holder that is also the keying client loses its connection: `unkey` 8 and drop `HOLDER_LOST` (C5.2; Table C8-2). |
| T-C8-21 | Each timer of Table C8-2 acts not before its time and no later than 10 ms after it; the server's Ping gaps never exceed 250 ms (C8.2, Tolerance). |
| T-C8-22 | (L1) A TCI `KEYER` sequence and a 3 s TCI CW text string go to P2 as DUC I&Q with PTT, High Priority byte 5 stays 0, DUC-specific byte 5 bit [1] is clear in every DUC-specific packet AetherSDR sends while it keys TCI CW, and the string is not cut at 1 s; with hardware CW selected in AetherSDR (bit [1] set), a `KEYER` press and a `CW_MACROS` are refused with `NOT_KEYABLE`, no PTT is set and bit [1] stays set (C8.2.5 item 3; 0021 §46 item 3). |
| T-C8-23 | A `KEYER:0,true,0;` whose release never arrives, from an opted-in client sending `G7250_KEEPALIVE` every 100 ms, is unkeyed with `unkey` 9 between 1000 ms and 1010 ms after the start of the played key-down (Table C8-2, key-down cap). |
| T-C8-24 | (L2) After a level-1 host set PA enable, CW select, break-in and an RF delay of 20 ms, a 1→2 switch leaves level 2 with the values of Table A3-4 until the firmware sets its own, and after a 2→1 switch a P2 host that sends no DUC-specific packet finds PA disabled and no break-in; after the last TCI client leaves, `DRIVE` keeps its value and is sent to the next client at connection; (L1) after a takeover and a new session AetherSDR sends the General packet with PA enable and the DUC-specific packet again before it arms (C8.5a; A3.6a; reference model). |
| T-C8-25 | Known limit (documents C8.2.3a): a client keyed with `TRX:0,true,tci;` in `NFM` that answers every TX_CHRONO with zero samples stays keyed with full carrier until the maximum TX time, or a stricter `LIMIT_TX_TIME`, unkeys, arm kept. |
| T-C8-26 | A generic client in terminal mode keys with `CW_MACROS`: while the text is sent no keyed timeout or generic keyed window acts (row 6); after the text the 1 s generic keyed window acts (row 7); a further `CW_MACROS` within it starts new text under the CW text liveness (row 6 again), with no unkey (C7.5 item 7; C8.2.3; 0021 §44 item 7). |
| T-C8-27 | With a microphone path in the reference model, a generic client without `AUDIO_START` keyed with `TRX:0,true,tci;` that restates `TRX:0,true;` every 500 ms stays keyed, and one keyed with `TRX:0,true;` that restates `TRX:0,true,tci;` every 500 ms stays keyed (C8.2.3; 0021 §44 item 14). |
| T-C8-28 | (L1) After a TCI `STOP;` and a new `START;`, AetherSDR, restarting from the same IP address and port (no takeover, reset, update, level or address change in between), sends the General packet with PA enable and the DUC-specific packet before it arms, and DDC0 runs with the RX settings of the ended session without a new DDC-specific packet; after a takeover by another host and a new session it sends the DDC-specific packet as well; with a TCI session value `G7250_LIMIT:LIMIT_FWD_POWER,30000;` in force before the `STOP;` and no stricter value in AetherSDR's user interface, the P2 `LIMIT_FWD_POWER` register reads 30 000 mW, and `TX_MODE_CLASS` the class of the current mode, before AetherSDR writes the arm (C5.10.1 item 6, C7.2.1, C8.5a item 3; 0022 §78; 0021 §44 item 11, §45 item 1). |
| T-C8-29 | A client that has not opted in keys with `TRX:0,true,tci;` without a running `AUDIO_START` where a microphone path exists (row 3), then sends `AUDIO_START` and `TRX:0,true,tci;` every 500 ms: the radio stays keyed with the microphone, no TX_CHRONO is sent, the TX audio underrun does not act, and once the client stops restating the generic keyed window unkeys after 1 s (`unkey` 2); after `TRX:0,false;` a new `TRX:0,true,tci;` keys in row 1 (C8.2.3, C7.5 items 7 and 8; 0021 §45 item 12). |
| T-C8-30 | (L1) With a hang delay of 300 ms written to DUC-specific bytes 11–12, two TCI `KEYER` elements 200 ms apart and a third 600 ms after the second: the P2 PTT bit stays set with DUC I&Q at zero across the first gap and is cleared 300 ms after the second element (one P2 transmission for the first two, a new PTT 0→1 edge for the third); with a hang delay of 3000 ms AetherSDR writes 2000 and clears PTT 2000 ms after the last element; with a hang delay of 0 PTT is set and cleared per element (C8.2.5 item 1; 0021 §46 item 1). |
| T-C8-32 | (L1) With a hang delay of 300 ms, TCI `KEYER` elements 200 ms apart and `t_blank` below the gap (reference model): AetherSDR sends I = Q = 0 in every gap and in the hang delay; RF above the "RF while not keyed" threshold injected in a gap enters the fault state with `RF_NOT_KEYED` (`G7250_SAFETY`) no later than `t_blank` + 10 ms after the last non-zero sample of the element (or after the injection, if later), while the P2 PTT bit is still set; RF that decays within `t_blank` after each element gives no fault (C8.2.5 item 2; part B A3.9a item 6; 0021 §47 item 1; 0022 §88 item 1). |
| T-C8-31 | (L1) After a takeover, AetherSDR sets no PTT before the P2 write echoes of `TX_MODE_CLASS` and each `LIMIT_<name>` write have arrived (reference model delaying the echoes by 50 ms); after a watchdog standby, a link loss and an inferred loss every TCI client receives `STOP;` after the messages of C7.2.1 (C5.10.1 item 6, C8.5a item 3, C2.9a, C7.2.1; 0021 §46 items 4, 5). |

---

## Annex C-A — Open points (HIS and G6 only)

Every other open point of draft 1 is decided in 0021 (proposed for ON8ST review at rc1).

| # | Point | Depends on |
|---|-------|------------|
| A1 | Whether level 2 can be built at all: compute for DSP, WebSocket and TCI; memory; network stack. All (L2) requirements wait for it. | G6, HIS-18, HIS-14 |
| A2 | Number of receivers; whether `TRX_COUNT` stays 1 and whether channel B can be demodulated. | HIS-02, G6 |
| A3 | `IF_LIMITS` on level 2 (receiver bandwidth). | G6, HIS-01 |
| A4 | CH394Q TCP server behaviour: does the listening socket become the connection (C2.5.3), which sets `clients_max`? Socket buffer split in TCP mode (C6.5). | HIS-14 |
| A5 | Stream budget on level 2 (C6.4): number of IQ streams, maximum IQ rate, audio formats, aggregate with TCP and WebSocket overhead. | G6, HIS-14, HIS-18 |
| A6 | Calibration of `RX_SENSORS` dBm on level 2. | HIS-11, G6 |
| A7 | `DRIVE` 0–100 to power mapping; tune power. | HIS-12, HIS-08 |
| A8 | Microphone, PTT and KEY from the controller (C4, C7.3.1); a microphone path for `TRX` without `tci`. | HIS-17, HIS-10 |
| A9 | Preamplifier and attenuator steps (C5.9); until answered, and on level 1 until a part B register exists, `G7250_PREAMP` and `G7250_ATT` are absent (C5.1.4). | HIS-07 |
| A10 | Default antenna after reset (C5.8); antenna switching, until answered and on level 1 until a writable part B register exists `G7250_ANT` is at most reported (`ant_state`, C5.1.4). | HIS-06 |
| A11 | `KEYER` timing jitter over TCP and WebSocket on level 2 (C4). | G6 |
| A12 | Which optional DSP functions (NB, NR, ANC, ANF, APF, DSE, NF, BIN, second demodulator, `WFM` receive, audio rates below 48 kHz) level 2 can carry. | G6, HIS-18 |
| A13 | ATU selection in or out of line (`G7250_ATU`, token `atu_select`); absent until answered and, on level 1, until a part B register exists (C5.7). | HIS-08 |
| A14 | Filter plan: which TX frequencies share one RF-path state (C7.6.1); until answered, each `tx_band` entry is its own state. | HIS-07 |

## Annex C-B — Deviations from [TCI]

| # | Deviation | Section |
|---|-----------|---------|
| C-B1 | The keying client must answer every TX_CHRONO while keyed with source `tci`; no TX audio for `TX_STREAM_AUDIO_BUFFERING` + 500 ms unkeys (0021 §7). [TCI] §3.4 allows no answer. | C8.2.1 |
| C-B2 | Transmit needs a G7250 arm; `TRX`, `TUNE`, `KEYER` and CW text do not key without it. [TCI] has no arm. | C5.5 |
| C-B3 | A TX request from a second client while keyed does not change the TX source; stop requests are never held by the 200 ms monopoly. | C7.4, C7.5 |
| C-B4 | (L2) Only one IQ rate at a time may be offered, although [TCI] makes the rate individual per client; the refusal uses [TCI]'s own "ignored, current rate replied" rule. | C6.2 |
| C-B5 | `TRX` without the `tci` source is refused when there is no microphone path, instead of taking a microphone signal. | C4 |
| C-B6 | Changes of modulation, antenna, ATU selection, preamplifier and attenuator are refused while keyed (0021 §8, §19; A3.9). | C7.6.3 |
| C-B7 | (L2) TCI connections are accepted only from the radio's subnet and, if set, an allow-list (0021 §10). [TCI] has no such rule. | C2.4.1 |
| C-B8 | `G7250_TX_POLICY:holder` lets only the arm holder key (0021 §1); set while a non-holder keys, it unkeys that transmission (`unkey` 1, arm kept; 0021 §44 item 5). | C5.5.8 |
| C-B9 | While keyed, the keying client is bound by one keyed-liveness rule for every TX source and mode (Table C8-1): the keyed timeout (500 ms, any application frame) if it has opted in or keys with TCI audio; otherwise the 1 s generic keyed window, which only a restatement of the TX request feeds; a WebSocket Pong never counts. A generic `TUNE`, `KEYER` key-down, `CW_TERMINAL`, `TRX` in `CW` or microphone `TRX` in any mode that restates nothing ends after 1 s (0021 §18, §27, §28). [TCI] has no such timeout. | C8.2, C8.2.3 |
| C-B10 | While keyed, a change of the effective TX frequency that leaves the transmit range or needs another RF-path state unkeys before it applies (arm kept); [TCI] follows frequency changes without condition (0021 §19; A3.9). | C7.6.1 |
| C-B11 | A `KEYER` key-down held for more than 1 s ends from every client (0022 §25). [TCI] has no such limit. | C8.2 |
| C-B12 | While the server sends CW text, the keying client must send an application frame at least every 30 s, else the text ends (0022 §25). [TCI] has no such duty. | C8.2 |
| C-B13 | A TX request keys only after the relays have settled: when it arrives with or shortly after a band or relay change, and when the key itself needs the RF-path state for the TX frequency (split cross-band), an ATU tune cycle included; a receive-frequency change during that wait is deferred to the next unkey; `G7250_ATU_TUNE` while keyed is refused (0022 §22, §23, §33, §37, §66; §73 item 10; A3.1b, A3.9a, A3.9b). | C7.6.1, C7.6.5, C5.7 |
| C-B14 | (L2) During a firmware update streams stop, `START` and set requests are not followed and extension sets get `BUSY` (0022 §45). [TCI] has no such state. | C2.8 |
| C-B15 | (L2) On a link below 100 Mbit/s the device does not start and stream starts are refused (0022 §51). [TCI] has no link-speed rule. | C2.9 |
| C-B16 | `TRX:0,true,tci;` is refused when the client's TX audio block period exceeds half the keyed timeout in force (0021 §39). [TCI] allows every `AUDIO_STREAM_SAMPLES` value of 100–2048. | C6.3.3 |
| C-B17 | Stop requests and tightening extension writes are never dropped by the 200 ms monopoly, and unkey, drop and safety messages are sent at once and never merged, not on the 30 ms tick of [TCI] §4.7 notes 6 and 7 (0021 §38). | C4.3 |
| C-B18 | A `KEYER` key-down played for more than 1 s ends (key-down cap), and an arg3 above 1000 ms is clamped to 1000 ms (0021 §33; 0022 §25). [TCI] has neither limit. | C4 `KEYER`, C8.2 |
| C-B19 | (L2) A stream start (`IQ_START`, `AUDIO_START`, `LINE_OUT_START`) that would exceed the stream budget is not followed and is reported only by `G7250_STREAM_STATE` `budget` to opted-in clients; [TCI] has no refusal of a stream start and no reply to it ([TCI] §4.3) (0021 §43 item 17). | C6.4.1, C5.11 |
| C-B20 | A binary message carries exactly one stream block; a client message that does not hold exactly one complete block is discarded. [TCI] §3.4 defines the block, not its framing in messages (0021 §43 item 17). | C2.2 |
| C-B21 | While keyed the TX source never changes, also for the keying client; while keyed with TCI audio, `AUDIO_STOP` or a change of the client's audio rate, block size or channel count unkeys (0021 §43 item 4). [TCI] is silent. | C7.5 items 7, 8 |
| C-B22 | `MODULATION` and front-end changes are refused also in a gap between CW elements during the hang delay and while CW text is queued; a TX-frequency change that needs another RF-path state ends the hang delay and is replied only once applied (0021 §43 item 6; 0022 §74). | C7.6, C7.6.7 |

## Annex C-C — Clarifications of [TCI]

| # | Clarification | Section |
|---|---------------|---------|
| C-C1 | Commands are in text messages, several complete commands per message allowed, never split over messages; a message may be fragmented and counts when its last fragment arrives (0021 §40). | C2.2 |
| C-C2 | The WebSocket upgrade is accepted on any path (0021 §17.1). | C2.3 |
| C-C3 | A frequency inside `VFO_LIMITS` but outside the receive range is not followed, also when an `IF` set would move the channel there; the current value is replied (0021 §41 item 3). | C3.3 |
| C-C4 | Stream header and samples are little-endian **[unverified]**. | C6.1 |
| C-C5 | A stream frame carries the header and `length` samples only **[unverified]**. | C6.1 |
| C-C6 | `count` is kept per stream also for IQ and line-out. | C6.1 |
| C-C7 | [TCI] lists `TX_ENABLE` under §4.3 "Unidirectional Control Commands" but gives its type as "Bidirectional control"; the profile treats it as server-controlled state, synchronised to all clients, with client sets ignored, as its note says. | C4 |
| C-C8 | [TCI] lists `KEYER` under §4.4 "Notification Commands" but gives its type as "Unidirectional control", "Sent only by the TCI client"; the profile treats it as a client command. | C4 |
| C-C9 | `FM` in a `MODULATION` set request is read as `NFM` (0021 §13). | C3.4 |
| C-C10 | `TX_SENSORS` arg2 ("microphone signal level, dBm") carries the TX audio level in dB full scale, because the main unit has no microphone (0021 §15). | C4 |
| C-C11 | `PROTOCOL` arg1 is sent as `ExpertSDR3` as a compatibility value on both levels; it does not identify the G7250 (0021 §4). | C3.2 |
| C-C12 | `TX_SENSORS` arg3 ("RMS" power) is the 100 ms mean forward power and arg4 (peak power) the PEP over the same window, or without a PEP detector the PEP estimate, as the safety core measures and enforces them (A3.4c; 0021 §21, §42; 0022 §62). | C4 |
| C-C13 | An unkey that no client requested is reported to all clients as `TRX:0,false;` or `TUNE:0,false;`, as the synchronisation of [TCI] §3.1 implies (0021 §24). | C7.7 |
| C-C14 | The effective TX frequency is moved by `VFO`, `IF` and `DDS` of the TX channel, `SPLIT_ENABLE`, `XIT_ENABLE` and `XIT_OFFSET`; not by RIT (0021 §19). | C7.6.1 |
| C-C15 | A TX request that cannot key is not kept: a refused `TRX` or `TUNE` does not key later, and CW text received while unarmed is discarded ([TCI] is silent; 0022 §24). | C5.5.4, C4 |
| C-C16 | (L1) `STOP` ("Stops the device") clears AetherSDR's P2 run bit; the TCI drop reason is `STOP` (0022 §32). | C4 |
| C-C17 | (L1) `START` does not take over a radio that another host or level 2 uses (discovery byte 4 = 0x03) without confirmation by AetherSDR's user (0022 §47). | C4 |
| C-C18 | `DRIVE` and `TUNE_DRIVE` are kept as the client set them, but forward power, the ATU tune carrier included, never exceeds the bound of the power limit in force, mapped as Table A3-3 for the detector in use, without a PEP detector on the PEP estimate from the digital crest factor (0022 §52, §53, §62). | C4, C5.7, C5.10.1, C8.1 |
| C-C19 | `KEYER` outside `CW` ("Works in CW mode only", [TCI] §4.4 KEYER) is ignored and not latched (0021 §33). | C4 `KEYER` |
| C-C20 | The key-down cap of a `KEYER` key-down is measured on the key-down as played, from the start of its playback, not from the arrival of the command (0021 §33). | C4 `KEYER`, C8.2 |
| C-C21 | `TRX:0,true,tci;` without a running `AUDIO_START` of that client is a microphone request, as "This works if the TCI audio stream is enabled, otherwise the microphone signal is taken" ([TCI] §4.2 TRX note; 0021 §41 item 10). | C4 `TRX` |
| C-C22 | `VOLUME`, `MUTE`, `RX_MUTE`, `RX_VOLUME` and `RX_BALANCE` do not change the level of the TCI RX audio stream (0021 §17.2). | C4 |
| C-C23 | `DIGL_OFFSET` and `DIGU_OFFSET` move neither the effective TX frequency nor `TX_FREQUENCY`; the suppressed carrier stays at the channel frequency **[unverified]** (0021 §34). | C4, C7.6.1 |
| C-C24 | `STOP;` is sent to all clients whenever the device stops without a client `STOP` (link, firmware update, (L1) end of the P2 session) (0021 §41 item 4). | C2.9a |
| C-C25 | `Stream.length` and `AUDIO_STREAM_SAMPLES` count samples summed over all channels; a block holds `length` / `channels` frames, as [TCI] §3.4 states for IQ ("the number of real samples"; complex samples = "Stream.length/Stream.channels"); the TX audio block period is `AUDIO_STREAM_SAMPLES` / (`AUDIO_STREAM_CHANNELS` × `AUDIO_SAMPLERATE`) **[unverified]** (0021 §43 item 2a). | C6.3, C6.3.3 |
| C-C26 | `TRX` and `TUNE` show the transmission by TX source: `TRX` for `TRX`, `KEYER` ([TCI] §4.4: "The first press switches the transceiver to transmission") and CW text, `TUNE` for `TUNE` and an ATU tune cycle; `TRX:0,false;` and `TUNE:0,false;` stop every source (0021 §43 item 5). | C7.4.1 |
| C-C27 | `CW_TERMINAL` is per client, as a unidirectional command ([TCI] §3.3, §4.5) (0021 §43 item 14). | C4 `CW_TERMINAL` |
| C-C28 | (L2) The device starts by itself at power-on, at a switch to level 2 and at a TCI session end, because generic clients do not send `START` (0021 §43 item 9). | C4 `START` |
| C-C29 | `TRX:0,true,tci;` in `CW` keys a CW carrier; the TX audio stream is not used and no TX_CHRONO is sent (0021 §43 item 8). | C6.3.4 |
| C-C30 | A TX audio block whose `length` differs from TX_CHRONO's is played and feeds the timers when `length` is a multiple of `channels` (0 included) and the data part is at most 16384 bytes; otherwise, or when its `channels`, `sample_rate` or `receiver` differs from the client's settings, it is discarded and is not an application frame; buffered TX audio is bounded to `TX_STREAM_AUDIO_BUFFERING` plus one block (0021 §44 items 2, 13; §45 item 2). | C6.3.5, C8.2 |
| C-C31 | `TX_ENABLE` ("transmission allowed / transmission denied", [TCI] §4.3) is a permission: it does not depend on whether the radio is keyed and stays `true` while keyed (0021 §44 item 1). | C5.5.6 |
| C-C32 | CW text (`CW_MACROS`, `CW_MSG`) outside `CW` ([TCI] §3.2 "Working in CW mode") is ignored and not latched, as `KEYER` is (0021 §44 item 15). | C4 `CW_MACROS` |
| C-C33 | `TRX:0,true;` and `TRX:0,true,tci;` without a running `AUDIO_START` are the same (microphone) source, so each restates the other for the generic keyed window (0021 §44 item 14). | C8.2.3 |

## Annex C-D — Assumptions to verify against public sources

The profile rule stands meanwhile; none of these is a HIS question.

| # | Assumption | Rule meanwhile | Check against |
|---|------------|----------------|---------------|
| D1 | Stream header and samples are little-endian. | C-C4 (0021 §12) | a public TCI client implementation |
| D2 | A stream frame carries the header and `length` samples only. | C-C5 (0021 §12) | a public TCI client implementation |
| D3 | Existing clients tolerate, or at least do not need, the absence of unknown server commands. | opt-in, C5.2 | public client sources |
| D4 | Existing clients answer every TX_CHRONO while transmitting TCI audio. | C8.2.1 (0021 §7) | public client sources; AetherSDR conformance run |
| D5 | Clients accept FLOAT32 IQ. | C6.2 (0021 §11) | public client sources |
| D6 | Some clients expect `PROTOCOL:ExpertSDR3`; `device:AetherSDR` matters to WSJT-X. | C3.2 (0021 §4, §5) | [AETHER-SRC] comments; WSJT-X source |
| D7 | Generic clients send no command during a long `TUNE` or a held key, so the 1 s generic keyed window ends them. Compatibility cost of Table C8-1 (0021 §27, §28): a generic client that keys with a microphone `TRX` (any mode), `TRX` in `CW`, `TUNE`, `KEYER` or `CW_TERMINAL` and does not restate its TX request at least every second has its transmission ended after 1 s, even if it polls other commands; a voice operator holding PTT in such a client for 10 s is cut after 1 s. On the G7250 a microphone `TRX` exists only with a microphone path (HIS-17), so today the cost falls on `TUNE`, `TRX` in `CW`, `KEYER` and `CW_TERMINAL`. | C8.2.3 (0021 §18, §27, §28) | public client sources; AetherSDR conformance run |
| D8 | On a `DDS` change [TCI] keeps `IF`, so the channel frequency (`VFO`) moves with it. | C7.6.1 (0021 §19): checked as a TX-frequency change either way | [TCI] §4.2 DDS, IF; public client sources |
| D9 | In ExpertSDR3, `DIGL_OFFSET`/`DIGU_OFFSET` shift only the audio passband and leave the carrier at the channel frequency. | C-C23 (0021 §34): offsets do not move the effective TX frequency | public client sources; ExpertSDR3 behaviour as documented publicly |
| D10 | Audio `length` and `AUDIO_STREAM_SAMPLES` count samples over all channels, not frames per channel. [TCI] §4.3 AUDIO_STREAM_SAMPLES recommends 512 samples at 48 kHz for at least 10 ms, which fits frames of 1 channel but would be 5.3 ms with 2 channels, so the other reading is possible. | C-C25 (0021 §43 item 2a): samples over all channels | a public TCI client implementation (how it sizes TX audio blocks for a stereo TX_CHRONO) |
| D11 | Existing clients answer TX_CHRONO with exactly the `length` it asks for. | C6.3.5 (0021 §44 item 2): another length is accepted within the stated bounds | public client sources; AetherSDR conformance run |

## Sources

- [TCI] TCI Protocol, Ver. 2.1, September 2026, "Expert Group" LLC: §1.4, §2, §3.1–3.5, §4.1–4.7
  (`sources.md`); for rc3 in particular §3.2.1 (terminal mode), §3.4 (`Stream.length`), §4.3
  AUDIO_STREAM_SAMPLES, §4.4 KEYER, §4.5 CW_TERMINAL; for the rc4 draft §3.2 ("Working in CW
  mode"), §3.4 (TX_CHRONO and the TX audio answer), §4.3 TX_ENABLE. Read in full for this part, 2026-11-19, from the text extraction of the PDF.
- [AETHER] AetherSDR README (TCI server, protocol 2.1).
- [AETHER-SRC] AetherSDR source, commit 9f7cbeb (2026-11-18), through `annex-aethersdr.md`
  §2.4 and G-13 (TCI port, `PROTOCOL`, `DEVICE`).
- [G7250-MAN] Xiegu G7250 Operation Manual: receive and transmit ranges, modes, TX power, ATU,
  antennas, interfaces.
- [CH394] WCH CH394 manual 1, V1.2 (8 sockets), through `analysis/ch394-network-budget.md` and
  0008.
- [P2] openHPSDR Ethernet Protocol V4.4, through 0007 (watchdog) and part B (High Priority byte
  4 bit [1], PTT).
- [RFC6455] The WebSocket Protocol (§4.2.1, §5.2, §5.3, §5.4, §5.5, §5.5.2, §7.4.1).
- RFC 2119, RFC 8174 (BCP 14).
- Decisions 0001, 0003, 0004, 0006, 0007, 0008, 0011, 0012, 0013, 0014, 0015, 0017, 0019,
  0021, 0022.
- `spec/network-interface.md` rc3 (0022 §74): A3.1a ("Keyer elements"), A3.4d, A3.9, A3.9a
  item 4, A3.9b item 1; rc2: A1.6, A1.6a (Table A1-4), A1.8, A1.9, A2.3–A2.5a, A3.1a, A3.1b, A3.2–A3.6,
  A3.4c, A3.4d, A3.9–A3.9c, B4.8, B5.8, B7, B8.3, B8.4, B8.6, B8.7, Tables B8-2 and B8-3,
  B10.2, B10.3, B10.5, B10.5a, B10.7, B10.9; rc3 (0022 §62–§73): A1.9, A2.4 step 1,
  A3.4a, A3.4c with Table A3-3, A3.6a with Table A3-4, A3.9a items 1a, 3 and 4, A3.9b items 1,
  2 and 7, B10.9, B10.10; for the rc4 draft (0022 §75–§83): A1.5 (identification I-3, I-4),
  B5.4 (restart; RX-side settings kept for the same IP address and port), B10.2 item 3,
  B10.5, A3.1b and Table A3-2 (`LIMIT_TX_FREQ` on DUC0).
- `analysis/ch394-network-budget.md` (Addendum, "TCI (level 2)"); `open-questions-his.md`
  (HIS-02, -06, -07, -08, -10, -11, -12, -14, -17, -18); design note §6 (milestones, G6).
- [AETHER-SRC] also `P2Client.h:238–241` (P2 keep-alive), through `annex-aethersdr.md` G-08c.
