# Annex — Changes needed in AetherSDR

- Version: rc4, 2026-11-19
- Status: rc4, for review by ON8ST. Informative annex to `spec/network-interface.md`
  (0001, spec section 13; 0013, 0014, 0016).
- © 2026 ON8ST, CC BY 4.0 (see `LICENSE.md`, decision 0018).

Sources are cited by key (see `sources.md`) or by decision number (see `decisions/`).
AetherSDR source is cited as `path:line` relative to the root of the public repository
github.com/aethersdr/AetherSDR, at commit `9f7cbebcc976b7b798a33dd5a9fc491548d1bbaa`
(2026-11-18, `main`), read on 2026-11-19 from a shallow clone outside this repository.
AetherSDR is GPL-3.0 [AETHER]; this annex quotes no AetherSDR code, only describes it.
Line numbers are those of that commit and will drift.

Spec sections cited as "B3.4" and similar refer to `spec/network-interface.md` rc2 of
2026-11-19, and to its rc3, rc4 draft and rc4 for the sections added or changed by 0022 §62–§78, §81 and
§84–§88 (A3.4d, A3.6a, A3.9a items 1a and 6; B5.4 by 0022 §78, B3.8 by 0022 §81, A2.4 and
B10.5b by 0022 §85, §87).

---

## 1. Scope

This annex lists what AetherSDR [AETHER] needs so that it can drive a G7250 on level 1
(G7250-P2) with full functionality, including transmit (0012, 0013, 0016). It does three
things:

1. a survey of how AetherSDR is built for non-Flex radios today (section 2);
2. a gap list against the decisions and the spec (section 3);
3. a recommendation on where the G7250 code lives (section 4).

Section 5 records a check of the unit of the P2 DDC sample-rate field. Section 6 lists open
questions.

Nothing here is a request to the AetherSDR project. Any issue, RFC or pull request upstream
needs an explicit go from ON8ST (AGENTS.md rule 6; 0016, Consequences). The backend is
GPL-3.0 with ON8ST as copyright holder of contributed code (0018).

---

## 2. Survey of AetherSDR for non-Flex radios

### 2.1 The `IRadioBackend` seam

- `src/core/backends/IRadioBackend.h:161` declares `IRadioBackend`, a `QObject` with a
  vendor-neutral interface. Required members include `capabilities()` (line 367),
  `connectRadio()` (397), `disconnectRadio()` (404), `isConnected()` (405), slice frequency,
  mode, filter and AGC setters (478–497) and `setKeying()` (793). Most other requests have
  default implementations that answer "unsupported" (for example lines 164–200).
- Keying policy sits above the seam: "The decision to allow keying is made ABOVE this seam by
  the engine guard … A backend whose capabilities().canTransmit is false implements this as
  a no-op" (`IRadioBackend.h:787–792`).
- `RadioConnectRequest` (`IRadioBackend.h:70–73` ff.) carries host, port, serial, version and a
  free `params` map for family-specific options.
- `RadioCapabilities` (`src/core/backends/RadioCapabilities.h`) describes the radio to the
  engine: `canTransmit` and `txPowerMaxWatts` (lines 547–548), per-frequency power ceilings
  `txPowerBands` (550–553), `hasTuner` (757) and many more.
- A backend is chosen by the `family` string of the discovered radio:
  `src/models/RadioModel.cpp:776–777` creates `AnanBackend` for family `"anan"`; discovery
  sources are wired in `src/core/backends/LocalRadioDiscoverySource.cpp:23` and `:50`, and in
  `src/gui/MainWindow_Session.cpp:769–788`.

### 2.2 ANAN-G2 backend (openHPSDR Protocol 2)

Files in `src/core/backends/anan/`, built from `CMakeLists.txt:965–975`:

| File | Role |
|------|------|
| `P2Protocol.h/.cpp` | Socket-free P2 wire codec: discovery, General, DDC-specific, High Priority, DDC audio, High Priority status, DDC I&Q. Declared "RX-ONLY BY CONSTRUCTION: no DUC-Specific, TX drive or PTT encoder exists here" (`P2Protocol.h:17–20`). |
| `P2Client.h/.cpp` | Owns the UDP socket, session start and stop, the 100 ms keepalive, DDC I&Q ingest, sequence-gap detection, speaker-audio pacing. |
| `AnanDiscovery.h/.cpp` | Broadcast and directed P2 discovery for the radio picker. |
| `AnanBackend.h/.cpp` | `IRadioBackend` implementation; owns `P2Client` and one `AnanRxDsp` per receiver on an I/O thread (`AnanBackend.h:27–39`). |
| `AnanRxDsp`, `AnanPanAnalyzer`, `AnanDroop*` | Host DSP (WDSP), panadapter, DDC droop correction per gateware and rate. |
| `AnanSettings`, `AnanReceiverRoutes.h`, `AnanSpeakerPacing.h`, `AnanSliceAudio.h` | Settings, DDC routing, speaker pacing. |

What it implements:

- **Discovery.** 60-byte request with byte 4 = 0x02 (`P2Protocol.cpp:47–52`). The reply
  parser needs at least 24 bytes, bytes 0–3 zero and byte 4 = 0x02/0x03, and reads bytes
  5–13 and 20–23 (`P2Protocol.cpp:54–77`). Byte 13 is read as an integer build number, not
  tenths (`P2Protocol.h:107–109`). The picker lists only board type 10 (Saturn):
  `isSaturn()` (`P2Protocol.h:124`) is applied in `AnanDiscovery.cpp:332`. The header
  states that board type is a picker filter only and "must not decide what the backend
  does" once connected (`P2Protocol.h:118–123`; `AnanDiscovery.h:67–69`). Byte 4 = 0x03 is
  shown as "In_Use" (`AnanDiscovery.h:69`). Discovery sweeps every 5 s from its own socket
  (`AnanDiscovery.h:79`; `AnanDiscovery.cpp:240–245`, `:263`).
- **General packet.** 60 bytes; ports (bytes 5–22) zero = defaults; byte 37 = 0x08 (phase
  word); byte 38 = 0x01 (hardware watchdog on); byte 39 = 0 (big-endian, 3-byte)
  (`P2Protocol.cpp:79–93`). Wideband (bytes 23–28) and memory-mapped ports (29–32) stay 0.
- **DDC-specific packet.** 1444 bytes, port 1025 (`P2Protocol.h:30`). Byte 4 = number of
  ADCs (default 2), bytes 5–6 dither/random set for two ADCs (0x03), byte 7 enable bitmap,
  per DDC a 6-byte row at 17 + 6·n: ADC, rate (16-bit), CIC fields 0, sample size 24
  (`P2Protocol.cpp:95–131`). Up to 4 DDCs (`P2Protocol.h:166`).
- **High Priority packet.** 1444 bytes, port 1027 (`P2Protocol.h:31`). Byte 4 = run only, no
  PTT (`P2Protocol.cpp:201–202`); one phase word per DDC at 9 + 4·n (`:205–208`); Alex0
  always written with ANT1 (bit 24) plus a receive filter or HF bypass (`:209–218`); Alex1
  (`:219–225`); byte 1400 bit 1 speaker-amplifier mute (`:226–230`); step attenuators in
  bytes 1442–1443 (`:231–235`). Sent every 100 ms as keepalive (`P2Client.h:238–241`;
  `P2Client.cpp:547–556`).
- **DDC I&Q parsing.** Strict: bits per sample must be 24 and the datagram length must be
  exactly 16 + samples × 6, else rejected (`P2Protocol.cpp:303–329`). The DDC is identified
  by the sender port, base 1035 (`P2Protocol.h:51–63`; `P2Protocol.cpp:289–301`). Samples are
  24-bit big-endian, scaled by 2²³ − 1 (`P2Protocol.cpp:331–349`). Sequence gaps are counted
  per DDC (`P2Client.cpp:658–667`). The time stamp is ignored (`P2Protocol.cpp:312`).
- **Rates.** Six rates 48–1536 ksps (`P2Protocol.h:70`), used for capabilities, pan limits
  48 k–1536 k (`AnanBackend.cpp:678–679`) and zoom snapping (`AnanBackend.cpp:1492–1509`).
  Rates are not validated before sending (`P2Client.h:45–49`). A rate change is sent live on
  the running session, three times, without a restart (`P2Client.h:144–151`;
  `AnanBackend.cpp:1551–1670`).
- **Audio to the radio.** Optional DDC audio to port 1028, 48 kHz, 64 stereo frames per
  260-byte packet (`P2Protocol.h:33–41`, `:272–289`; `P2Client.cpp:378–380`), off by default
  (`P2Client.h:62–65`), switched by the connect parameter `anan.speakerAudioEnabled`
  (`AnanBackend.cpp:932–933`).
- **Telemetry.** High Priority status must be exactly 60 bytes; only byte 30 bit 3 (speaker
  underflow), bytes 37–38 (speaker FIFO) and bytes 57–58 (supply rail, raw) are decoded
  (`P2Protocol.cpp:275–287`). Supply is converted with a fixed Saturn scale
  (`P2Protocol.h:298–302`; `AnanBackend.cpp:2092`); bytes 49–50 are deliberately not used
  on the G2 (`P2Protocol.h:313–316`). Forward and reverse power are not decoded.
- **Frequency.** Phase word with a fixed DSP clock of 122.88 MHz (`P2Protocol.h:72–94`).
- **Socket model.** One `QUdpSocket` per session, bound to an ephemeral port on
  `AnyIPv4` (`P2Client.cpp:137–138`); discovery, General, DDC-specific and High Priority all
  leave from it (`P2Client.cpp:156–170`), and all radio traffic arrives on it, filtered by
  sender address (`P2Client.cpp:575–594`). A clean stop sends High Priority with run = 0
  (`P2Client.cpp:205–236`); `abandon()` closes without a stop packet (`P2Client.h:91–94`).
  Silence for 3 s ends the session (`P2Client.h:242–245`).
- **Transmit.** None: `canTransmit = false` (`AnanBackend.cpp:680`), `setKeying()` is a no-op
  (`AnanBackend.h:36–37`). TX is upstream's "Phase 3", wideband ("Gap B") is out of scope
  (`docs/architecture/anan-p2-backend-design.md:388–406`).

### 2.3 Hermes-Lite 2 backend (Protocol 1) as the model for transmit

`src/core/backends/hl2/` is the only host-modulated backend that transmits today.

- **TX DSP.** `Hl2TxDsp` takes processed TX audio at 24 kHz and produces baseband IQ through a
  WDSP TXA channel, output fixed at 48 kHz for EP2 (`Hl2TxDsp.h:24–56`).
- **Wiring.** Modulated IQ goes directly, on the I/O thread, to the wire client
  (`Hl2Backend.cpp:592–601`).
- **TX IQ queue.** `MetisClient::queueTxIq()` drains at the radio's frame rate; underflow is
  sent as silence, overflow drops the oldest samples, and the queue is primed at the start of
  each over (`MetisClient.h:421–439`). 126 samples per P1 packet (`MetisProtocol.h:471`).
- **Gate.** A host-side transmit gate (`MetisClient::enableTransmit()`, `MetisClient.h:379–391`)
  plus a refusal in the backend when the gate is closed or the session is not running
  (`Hl2Backend.cpp:5063–5100`). `canTransmit` reports the gate (`Hl2Backend.cpp:2478–2481`).
- **Power.** Forward and reverse power stay raw counts; `txPowerMaxWatts` is 0,
  "uncalibrated" (`Hl2Backend.cpp:2506`; `Hl2TelemetryService.cpp:217–218`).
- **Emergency stop.** A signal-handler path sends a precomputed stop datagram if the process
  dies (`Hl2EmergencyStop.h:17`; `Hl2EmergencyStop.cpp`).

What carries over to G7250-P2: the WDSP TX chain, the queue discipline, the two-level gate,
the emergency stop. What does not: P1 framing, the 48 kHz rate, and the gate being a host-side
boolean (on the G7250 the gate is a radio-side arm, 0007).

### 2.4 TCI server

- Default port 50001 (`src/core/TciServer.h:92`).
- Initialisation burst (`src/core/TciProtocol.cpp:345–535`): `vfo_limits:1000,75000000`
  (line 351), `if_limits`, `trx_count` (360), `channels_count:2` (368), `device:AetherSDR`
  (377), `receive_only:false` (378), a fixed `modulations_list` (379),
  `protocol:ExpertSDR3,2.1` (380), state lines, `audio_samplerate:48000` (517),
  `iq_samplerate` (522), `start` (528) and `ready` last (535).
- A GET of `vfo_limits` answers `30000,54000000` (`TciProtocol.cpp:612`), which differs from
  the burst (line 351). Neither comes from the radio.
- Commands handled (`TciProtocol.cpp:576–652`): `start`, `stop`, `dds`, `if`, `vfo`,
  `modulation`, `trx`, `tune`, `drive`, `tune_drive`, `mic_level`, RIT/XIT, `split_enable`,
  `rx_filter_band`, squelch, volume and mute, AGC, noise reduction and filter enables,
  `rx_channel_enable`, `rx_volume`, `rx_balance`, monitor, CW keyer and macros, spots,
  `lock`, record and play, `tx_gain`, `rx_ctun_ex` and others. IQ streaming and
  `iq_samplerate` are in `TciServer.cpp:1555–1610`; `audio_samplerate` in `:1482–1491`;
  `rx_sensors_enable` and `tx_sensors_enable` in `:1519–1535`; `tx_sensors` carries an extra
  AetherSDR field (`:4350–4356`).
- `tx_enable` is treated as server-to-client only (`TciProtocol.cpp:814–819`).
- No `hw_sensors` output was found (search of `src/core/Tci*`).
- AetherSDR extensions are opt-in through `aethersdr_ext_enable` (`TciServer.cpp:1539`;
  `TciExtensionSet.h:13–18`).

The TCI server works on the engine models (slices, transmit model), not on a backend, so a
G7250 backend gets TCI through the same path as every other backend.

---

## 3. Gap list for a G7250-P2 backend

Size: **S** = days, a contained change; **M** = one to a few weeks, new code within one
area; **L** = several weeks, new subsystem or cross-cutting. Estimates assume one developer
who knows Qt and the P2 wire, and exclude hardware bring-up time.

### 3.1 Already compatible

These parts of the ANAN P2 code already match the G7250 and need no change, only reuse:

| Item | AetherSDR | G7250 |
|------|-----------|-------|
| Discovery request and reply parse | `P2Protocol.cpp:47–77` | 60-byte short reply, bytes 0–3 zero (B3.2, B3.3; 0003 §1) |
| General byte 38 bit [0] = 1, byte 39 = 0 | `P2Protocol.cpp:87–90` | Compatible for these two bytes: watchdog on, required for arming (0007 §2.3); BE 3-byte (0004 point 1). The rest of the General packet is not: the packet is zero-initialised and only bytes 4, 37, 38 and 39 are set (`P2Protocol.cpp:79–93`; the backend is RX-only, `P2Protocol.h:17–20`, §2.2), so byte 58 bit [0] (PA enable) stays 0, and while it is 0 no TX request keys (B7 Table B7-1; B10.10). That is a gap, listed under G-08a and G-08b |
| DDC I&Q parse, 24-bit, length from bytes 14–15 | `P2Protocol.cpp:303–329` | 238 samples, 1444 bytes; host must read bytes 14–15 (B6.4) |
| DDC0 from source port 1035 to the discovery source port | `P2Protocol.h:43–63`; `P2Client.cpp:156` | B2.4, B6.1 |
| Sample size 24 written explicitly | `P2Protocol.cpp:127` | B6.5 Table B6-2, byte 22 |
| Live rate change without stop | `P2Client.h:144–151` | B6.5 (SHOULD take effect without a stop) |
| High Priority status exactly 60 bytes | `P2Protocol.cpp:278` | bytes 0–59 (0006) |
| Keepalive every 100 ms on the I/O thread | `P2Client.h:238–241` | 100 ms recommended [P2]; HP refresh duty while keyed (0007 §2) |
| One socket per session for all P2 traffic | `P2Client.cpp:137–170` | owner = IP address and UDP port (0007 point 4) |
| Phase word with a fixed DSP clock of 122.88 MHz, General byte 37 bit [3] = 1 | `P2Protocol.cpp:85`; `P2Protocol.h:72–94` | discovery byte 21 = 1 always, phase-word scale 122 880 000 Hz (B3.8; 0022 §81); see G-11 |

### 3.2 Gaps

#### G-01 Discovery of board type 200 — S

- **Required.** Board type 200 in discovery byte 11 (0002; B3.3). Byte 4 = 0x03 when another
  host has a session or level 2 is active (0003 §1). Byte 13 is decimal with one implied
  decimal place, 15 = 1.5 (B3.5); the full version is in the capability record (0003).
- **Today.** Only board type 10 is listed (`AnanDiscovery.cpp:332`, `P2Protocol.h:124`).
  Byte 13 is shown as an integer (`P2Protocol.h:107–109`; `AnanDiscovery.cpp:344`). Family `"anan"` routes to `AnanBackend`
  (`RadioModel.cpp:776–777`).
- **Change.** A discovery source that lists byte 11 = 200 under a new family (for example
  `"g7250"`), model "G7250"; route that family to the G7250 backend in `RadioModel.cpp` and
  `LocalRadioDiscoverySource.cpp`. Show the firmware version from the capability record, or
  byte 13 as tenths. Keep "In_Use" for byte 4 = 0x03 and, when the record says level 2 is
  active (`level` bits [7:4] = 2), say so instead of "in use".
- **Note.** The discovery sweep uses its own socket every 5 s (`AnanDiscovery.cpp:240–263`).
  With owner = IP + port (0007), the radio may answer that socket with 0x03 while AetherSDR's
  own session runs, so the picker would show the user's own radio as in use. See Q-S1.

#### G-02 Capability record (0003) — M

- **Required.** After discovery and before opening a session, the host reads the capability
  record: a memory-mapped write to `CAP_REQUEST`, answered with the whole record in one
  hardware-to-host memory-mapped packet (0003 §2). The host checks `cap_magic`,
  `cap_version` major and `cap_length` (0003 §4) and uses the record for sample rates,
  antennas, ATU, TX power and the TX-implemented flag (0003, Consequences). A request-only
  packet does not take over or start a session (0003 §2).
- **Today.** No memory-mapped packets at all. Everything the record carries is hard-coded
  for the G2: DSP clock (`P2Protocol.h:73`), rate list (`:70`), two ADCs and dither for both
  (`P2Protocol.cpp:100–106`), phase-word mode (`:85`), fixed `RadioCapabilities`
  (`AnanBackend.cpp:535–716`).
- **Change.** A record decoder (207 registers in record version 1.1, 0020 §26; B4) and a mapping into
  `RadioCapabilities`: `ddc_rates` → rate list and pan limits; discovery byte 21 and the
  phase-word scale are fixed and already match (G-11); `rx_range` → tuning limits; `tx_band` →
  `txPowerBands` (`RadioCapabilities.h:550–553`); `antennas`, `atu` → `hasTuner` (757) and
  antenna lists; `telemetry`, `cal_*` → G-07; `tx` bits → `canTransmit` (G-08); `link` and
  `mgmt` → status display. `RadioCapabilities` has no per-mode power ceiling, so the AM
  maximum (`tx_band` bits [15:0], 25 W [G7250-MAN]) needs a new field or a mode-aware
  lookup. Refuse to transmit when `cap_magic` is wrong or the major version is unknown
  (0003 §4 rule 4), but still allow receive.
- **Note.** The request should leave from the session socket, so that the answer and later
  register traffic use the one owner port (G-09).

#### G-03 G7250 register map over P2 memory-mapped packets on one port (0005) — M

- **Required.** All G7250 functions outside the standard fields, and all TX safety, are
  registers carried by P2 memory-mapped packets: up to 240 pairs of 16-bit address and 32-bit
  data per packet [P2]. The G7250 uses one local UDP port for both directions; the port is
  defined in part B: UDP 7250 (0020 §1; B2.3 Table B2-1 row 7). The spec defines how a
  write is confirmed (for example an echo) and the order against High Priority packets
  (0005, Consequences). Register packets do not feed the watchdog (0007 point 2).
- **Today.** None. General bytes 29–32 are sent as zero (`P2Protocol.cpp:79–93`). The ingest
  classifies datagrams by shape: DDC frame first, then 60-byte discovery reply, then 60-byte
  status (`P2Client.cpp:596–627`).
- **Change.** A codec for both memory-mapped packet directions; a send path to the G7250
  register port from the session socket; demultiplexing of the radio's register packets by
  **sender port before any shape test**, because a register packet of up to 1444 bytes could
  otherwise be tried as a DDC frame; a register cache with write confirmation, timeout and
  retry; an API for the backend (read, write, subscribe to pushed registers). General bytes
  31–32 must name the host's register port in a way the spec accepts (Q-S2). Event,
  fast and 1 s snapshot pushes (0006) must be handled without assuming they describe the same
  instant as status packets (0006, Consequences).

#### G-04 No DDC audio to the radio (0017) — S

- **Required.** The G7250 opens no socket on port 1028; a host SHOULD NOT send DDC audio
  (B2.3; 0017). The speaker FIFO fields in status are 0 (0017, Consequences).
- **Today.** Optional speaker stream to 1028 (`P2Client.cpp:378–380`), off by default
  (`P2Client.h:62–65`), enabled by `anan.speakerAudioEnabled` (`AnanBackend.cpp:932–933`);
  speaker-amplifier mute in High Priority byte 1400 (`P2Protocol.cpp:226–230`); speaker
  status decoded (`P2Protocol.cpp:282–283`). The README states that receive audio also
  plays through the radio's speaker [AETHER].
- **Change.** The G7250 backend never starts the speaker stream, offers no option for it,
  does not set byte 1400, and ignores the speaker fields of the status packet. Receive audio
  plays only on the host.

#### G-05 Sample rates 48–384 ksps and refusal handling (0004) — M

- **Required.** DDC0 rates 48, 96, 192, 384 ksps, or up to 192 if the SPI clock is below
  about 50 MHz; 768 and above never (0004 points 1, 6, 7). A request outside the offered set,
  or a sample size other than 24/0, stops DDC0 until a valid DDC-specific packet arrives; the
  reason is in a stream status register; no clamping (0004 point 7). On a 10 Mbit/s link the
  session is refused, discovery still answered, reason reported (0004 point 9).
- **Today.** Six rates including 768 and 1536 (`P2Protocol.h:70`), pan range 48 k–1536 k
  (`AnanBackend.cpp:678–679`), snapping over all six (`AnanBackend.cpp:1492–1509`), no
  validation before sending (`P2Client.h:45–49`). A refused rate shows only as "no IQ … within
  2000 ms" (`P2Client.cpp:557–563`, `P2Client.h:246–248`). DDC-specific byte 4 is 2 ADCs and
  bytes 5–6 set dither and random for two ADCs (`P2Protocol.cpp:100–106`).
- **Change.** Rate list and pan limits from `ddc_rates` (G-02); snap only within it; never
  send a rate outside it. On missing IQ, read the stream status register (G-03) and show the
  reason (rate, format, link speed). Send byte 4 = `num_adc` and dither/random only for ADCs
  that exist. After a live rate change, do not expect a sequence restart: counters continue (0020 §19); AetherSDR already resets its expectation on a rate change
  (`P2Client.cpp:534–543`).

#### G-06 Wideband frames, 4096 samples × 20 frames/s (0004, M2) — M

- **Required.** From M2, wideband ADC0 data out on port 1027: 512 samples per packet,
  16-bit, 8 packets per frame, one frame every 50 ms; enabled only while General bytes 24–28
  are 0 or equal to these fixed values; announced in `streams` bit 4, `wb_format`,
  `wb_frame` (0004 point 4 and point 7; 0003 §3).
- **Today.** No wideband in the P2 backend; it is out of scope upstream ("Gap B",
  `docs/architecture/anan-p2-backend-design.md:388–395`); General bytes 23–28 are zero. The HL2
  backend has a P1 bandscope (`src/core/backends/hl2/Hl2Spectrum.cpp`) that can serve as a
  display model.
- **Change.** If `streams` bit 4 is set: General byte 23 bit [0] and bytes 24–28 = 0 (or the
  fixed values); accept 1028-byte wideband packets from sender port 1027 on the session
  socket (demultiplexed by sender port, like G-03); reassemble 8 packets into a 4096-sample
  frame, discard incomplete frames; FFT and show a wideband panadapter. The frequency
  resolution depends on the ADC rate (0004 point 4; HIS-01).

#### G-07 Raw ADC telemetry with per-unit constants (0006) — M

- **Required.** Standard status fields carry raw ADC words: PA forward power bytes 14–15,
  PA reverse power 22–23, exciter power 6–7 if present, supply 49–50; inputs in bytes 4–5;
  FIFO state in 30–36; ADC peak in 39–40. Status arrives every 50 ms, every 1 ms during TX,
  and on input change (0006). The host converts with the unit's own constants from the
  capability record: `V = x / N × c`, `W = (x / N × c1)² / c2`, with *N* from `cal_info`,
  per band if `cal_info` bit 0 is set (0003 §3, 0x57–0xB8). G7250 telemetry registers
  (PA temperature, PA current, SWR used by the safety core, TX state, arm state, fault,
  limit status, ATU state, antenna, sensor validity) arrive as events, fast group and 1 s
  snapshot (0006).
- **Today.** Only bytes 30 bit 3, 37–38 and 57–58 are decoded (`P2Protocol.cpp:275–287`);
  supply from 57–58 with a fixed Saturn scale (`P2Protocol.h:298–302`; `AnanBackend.cpp:2092`),
  explicitly not 49–50 (`P2Protocol.h:313–316`); no power decoding. The HL2 backend keeps
  power as raw counts (`Hl2TelemetryService.cpp:217–218`).
- **Change.** A G7250 status decoder for the bytes above; conversion with the record's
  constants and the band set for the current TX frequency; forward power, reverse power, SWR
  and supply meters; PTT, key and overload indicators; the register telemetry mapped to
  meters and to the safety display (G-08d). Use the SWR register for anything
  safety-related; a host-computed SWR is display only (0006).

#### G-08 Transmit (0004, 0007, 0012, 0015) — L in total

##### G-08a DUC I&Q 24-bit at 192 ksps — L

- **Required.** DUC0 I&Q to port 1029, 192 ksps, 24 bits, 240 samples per packet, 1444-byte
  packets; a DUC-specific packet with rate 192 and 24 bits, else DUC I&Q is not accepted and
  TX is refused (0004 points 2 and 7; [P2] DUC I&Q packet, NOTE2). Drive level in High
  Priority byte 345 (0005). General byte 58 bit [0] (PA enable) set: while it is 0 no TX
  request keys, and clearing it while keyed unkeys (B10.10; Table B7-1; 0022 §14); the radio
  clears it at every session end and takeover, with every DUC-specific TX setting (A3.6a,
  Table A3-4; 0022 §64). A DUC FIFO
  underflow longer than a hardware-dependent time (pending hardware, part B Annex P P-TIM-2; HIS-18) unkeys, arm kept (0007 §2;
  B10.5). The underflow is timed from the key if no DUC I&Q has arrived by then, and the
  radio sends zeros while its FIFO is empty; the underrun time is at least the key latency
  plus 1.25 ms, so DUC I&Q started together with the PTT edge is in time (B10.5; 0022 §57).
- **Today.** No DUC code in the P2 backend (`P2Protocol.h:17–20`). The HL2 chain outputs
  48 kHz (`Hl2TxDsp.h:56`) into a P1 queue of 126-sample packets (`MetisProtocol.h:471`;
  `MetisClient.h:421–439`).
- **Change.** A TX chain at 192 kHz output (WDSP TXA as in `Hl2TxDsp`, with the output rate
  configurable); a DUC I&Q packet builder (sequence number plus 240 × 6 bytes); a pacer at
  800 packets/s on the I/O thread, with priming, silence on underflow and drop-oldest on
  overflow as in `MetisClient`; the DUC-specific packet to port 1026; the DUC frequency in
  High Priority bytes 329–332 in the encoding of G-11; drive level; General byte 58 bit [0]
  = 1 in the set-up General packet (not set today, §3.1). The DUC0 frequency MUST always be
  the effective TX frequency (channel A, or channel B with split, plus XIT when on): realise
  split and XIT by the DUC0 frequency, never by a frequency shift in the baseband DUC I&Q,
  because the radio checks the TX frequency limit and chooses the RF-path state on DUC0 (part C
  C7.6.1 item 5; A3.1b, Table A3-2 `LIMIT_TX_FREQ`; 0021 §44 item 3). A sideband/handedness
  check against an external receiver, as upstream plans for its own Phase 3
  (`docs/architecture/anan-p2-backend-design.md:402–406`).

##### G-08b TX arm handshake and TX mode class — M

- **Required.** The host reads `TX_ARM_TOKEN`, declares `TX_MODE_CLASS`, then writes
  `token XOR ARM_MAGIC` to `TX_ARM` and waits until `TX_ARM_STATE` shows armed with a new
  generation number. After that, the radio keys only on a 0→1 edge of High Priority byte 4
  bit [1] that arrives after the confirmation. Preconditions include RUN set, watchdog bit
  set in a General packet of the session or of its set-up phase, General byte 58 bit [0]
  (PA enable) set for keying, no fault, PTT bit 0 at the arm moment, level 1 active (B10.2
  item 3; A3.1a). Every drop issues a new token;
  the host must re-arm with it. The class must be updated on every mode change; AM class
  carries the 25 W limit. (0007 sub-decision 1, rules 1–5; sub-decision 4 rule 3.)
- **Today.** The gate is a host-side boolean (`MetisClient.h:379–391`) and the engine guard
  above the seam (`IRadioBackend.h:787–792`). Nothing reads a radio-side arm.
- **Change.** An arm state machine in the backend: disarmed → token known → class declared →
  arm written → confirmed → keyable; back to disarmed on any drop reason. `canTransmit` (or
  a readiness signal such as `independentTxReady()`, `IRadioBackend.h:794–795`) follows the
  confirmed arm and `tx` bit 1 of the record. Map each host TX mode to a mode class and
  write it before arming and on every change. Send PTT = 0 until the confirmation, then a
    fresh 0→1 edge. A user-visible arm control; arming only on that explicit action or a
  valid TCI `G7250_TX_ARM`, never on a key (Q-O3; part C C5.5.7).
- **Set-up phase** (B5.1a; 0022 §34). AetherSDR sends the General packet at set-up, before
  the run bit, and its keepalive is the High Priority packet (§2.2; `P2Client.cpp:156–170`,
  `:547–556`). The radio keeps that General packet (byte 38 bit [0], byte 58 bit [0], byte 39)
  and the DDC-specific and DUC-specific packets for the session the run bit starts, so
  AetherSDR need not resend them. At every session end, a takeover included, the radio
  returns the TX settings of Table A3-4 to their session-start values (A3.6a; 0022 §64); after
  a stop, a watchdog standby or a link loss it keeps the RX-side settings (every General packet
  field except byte 38 bit [0] and byte 58 bit [0], the DDC-specific packet and the receive
  state of High Priority) for the same IP address and port, and a High Priority packet with
  run = 1 from that host restarts the session with them, with no 0→1 edge of the run bit
  needed (B5.4; A2.3; 0022 §78). After a session end AetherSDR must therefore resend,
  before it arms again (for example after a restart with the run bit only): the General
  packet, because an arm needs a General packet of the new session with the watchdog bit
  (B10.2 item 3), and with PA enable; and the DUC-specific packet, for CW select, break-in,
  hang delay, RF delay, keyer and DUC rate (part C C8.5a item 3). After every session end,
  with or without a takeover, it must also write `TX_MODE_CLASS`, every P2 `LIMIT_<name>` it
  sets for its user interface or for a TCI session value (part C C5.10.1 item 6) and
  `KEYED_TIMEOUT` if it uses it, before it arms, because the radio resets them at every
  session end (A3.6; B5.4; 0022 §78; part C C8.5a item 3; 0021 §45 item 1). A resend of the DDC-specific
  packet is not needed when AetherSDR restarts from the same IP address and port after a stop,
  a watchdog standby or a link loss; after a takeover, a reset, a firmware update, a level
  change or an address change it is (part C C7.2.1, C8.5a item 3; 0022 §78 item 4). The capability record arrives once, unasked, at the session start
  (B4.4 item 2), possibly in up to three packets: collect the registers by address until the
  record is complete, or read it in one packet with `CAP_REQUEST` (0059).
- **High Priority sequence numbers** (B11.6a; 0022 §35, §55, §56). Number the High Priority
  packets from 1 upwards and restart at 1 only with a new session, so that the radio
  discards a reordered or duplicated packet; at the wrap skip 0 (0xFFFFFFFF → 1), because 0
  is never newer (B11.6a; 0022 §72). The radio engages numbered mode after three
  increasing non-zero numbers and resynchronises, with a log entry, after five packets in a
  row that are not newer or a jump back by more than 2¹⁶; a PTT edge in the packet that
  resynchronises does not key. In unnumbered mode (all numbers 0, and before numbered mode
  engages) a PTT edge counts only if it arrives 10 ms or more after the latest packet with
  PTT 1 (or after the arm confirmation).

##### G-08c High Priority refresh ≤ 100 ms while keyed; 500 ms keyed timeout — S

- **Required.** While keyed, the owner sends High Priority at least every 100 ms; if none
  arrives within `KEYED_TIMEOUT` (500 ms default, host may only shorten, floor 100 ms) the
  radio unkeys and keeps the arm. General byte 38 bit [0] must be set to arm and is forced on
  while armed (0007 point 2; sub-decision 2).
- **Today.** The 100 ms keepalive already exists on the I/O thread (`P2Client.h:238–241`;
  `P2Client.cpp:547–556`) and byte 38 = 1 is already sent (`P2Protocol.cpp:87`).
- **Change.** Carry the PTT bit in every keepalive; use a precise timer and keep the
  keepalive independent of the GUI and the DSP threads; on a keyed-timeout unkey reported in
  `TX_STATE`, drop the local key and require a new PTT action. Optionally write a shorter
  `KEYED_TIMEOUT`.

##### G-08d Takeover, disarm and fault display — M

- **Required.** Takeover is always allowed and always unkeys and disarms first; hosts should
  warn before taking over (discovery byte 4 = 0x03). Show arm state, fault and drop reasons
  (0007 points 3–5; Resolved 1 and 5). Faults in two classes, one host-clearable through
  `FAULT_CLEAR` (0007 point 5).
- **Today.** "In_Use" is shown in the picker (`AnanDiscovery.h:69`); AnanBackend uses a
  reconnect busy probe (`AnanBackend.h:4`, `ReconnectBusyProbe.h`). A session that goes
  silent for 3 s ends without a stop (`P2Client.h:242–245`). There is no arm, fault or
  limit display.
- **Change.** A confirmation before connecting to a radio that answers 0x03; a status area
  with arm state, last drop reason (13 events, 0007 sub-decision 1 rule 5), fault code and
  class, limit status (limiting/tripped per limit), keyed-timeout and maximum-TX-time unkeys;
  a "clear fault" action for host-clearable faults only. On a `TAKEOVER` drop, tell the user
  which situation occurred.

##### G-08e Emergency stop on host failure — S

- **Required.** Nothing beyond the radio's own protection: the keyed timeout and the
  watchdog end TX without the host (0007).
- **Today.** HL2 has a signal-handler stop (`Hl2EmergencyStop.cpp`); ANAN sends a clean stop
  on disconnect (`P2Client.cpp:205–236`).
- **Change (recommended).** The same for the G7250: on abnormal exit, send High Priority with
  run = 0 from the session socket, which unkeys and drops the arm (`RUN_CLEARED`, 0007 drop
  event 2). It shortens unintended carrier from up to 500 ms to the network delay.

#### G-09 Owner = IP address + UDP port (0007) — S

- **Required.** The owner is the IP address and UDP port of the host. C&C from the same IP on
  a new port is a takeover: unkey and disarm (0007 point 3; Resolved 4). Arm writes must come
  from the owner (0007 sub-decision 1 rule 1).
- **Today.** Yes, within one session: one socket on one ephemeral port carries all P2
  traffic (`P2Client.cpp:137–170`). But every `P2Client::start()` creates a new socket and so
  a new port (`P2Client.cpp:137–138`); this happens on every connect and reconnect.
  `startP2ClientSession()` still has a rate-change variant of `start()`
  (`AnanBackend.cpp:1094–1097`), but rate changes now take the live path, which keeps the
  socket (`AnanBackend.cpp:1551–1670`). Discovery sweeps use another socket
  (`AnanDiscovery.cpp:263`).
- **Change.** Keep one socket for the whole session, and send register traffic (G-03),
  capability request (G-02), wideband (G-06) and DUC I&Q (G-08a) from it too. Use only the
  live rate change. Treat any restart as a new session: expect the arm to be dropped
  (`TAKEOVER` or `REDISCOVERY`) and re-arm only on an explicit arm action of the user or a
  valid `G7250_TX_ARM` from a TCI client (part C C5.5.7; 0021 §30, §43 item 16). Make sure the picker's own
  discovery socket is not mistaken for a second host (G-01 note, Q-S1).

#### G-10 Antenna, filters, attenuator, ATU (0005, 0003, 0007) — M

- **Required.** Standard P2 fields only where the meaning is identical; Alex and Apollo fields
  are not used for G7250 functions (0005 option D rejected). The G7250 has no Alex (0006).
  Antenna, filters and ATU are registers (0005); ATU tuning is a TX request under the arm
  (`ATU_TUNE`, 0007 sub-decision 1 rule 4). The P2 field tables are in B7 (General, High Priority) and B10 (DUC-specific).
- **Today.** Alex0 is always written with ANT1 bit 24 and a receive filter or HF bypass,
  Alex1 too (`P2Protocol.cpp:209–225`); step attenuators in bytes 1442–1443 (`:231–235`).
  `hasTuner = false` (`AnanBackend.cpp:692`).
- **Change.** A G7250 High Priority builder that leaves the Alex words at zero (until B7 says
  otherwise) and fills only fields the G7250 uses; antenna selection, ATU in-line/bypass and
  tune through registers, with the ATU state register (0006) shown; `hasTuner` and antenna
  lists from the record (G-02). With split, set DDC0 and DUC0 before `ATU_TUNE`: the radio
  switches the RF path to the TX state for DUC0 before the tune carrier and back to the state
  for DDC0 after the cycle (A3.9b items 2, 7; 0022 §66).
- **v1.0** (0032, 0035). Until the ATU and antenna parameters are measured (part B Annex P,
  P-ATU-1, P-ANT-1), `ATU_TUNE` = 1 gets write reject reason `NOT_CALIBRATED`, `ATU_STATE` reads
  5 (unknown) and `ANT_STATE` reads 0 (unknown): show the tuner and the antenna as "not
  available" rather than hiding them, and do not retry.

#### G-11 Frequency encoding and DSP clock (B3.8) — compatible

- **Required.** Discovery byte 21 is always 1 (phase word), with the phase-word scale
  122 880 000 Hz; General byte 37 bit [3] must agree (B3.8; 0022 §81). A mismatch stops DDC0
  and blocks every TX request, `STREAM_STATUS` reason 13 (B3.8; 0022 §73 item 12).
- **Today.** Byte 37 bit [3] is always set and the clock is fixed at 122.88 MHz
  (`P2Protocol.cpp:85`; `P2Protocol.h:72–94`): the same encoding and scale.
- **Change.** None needed (§3.1). Recommended: check that discovery byte 21 is 1 and show
  `STREAM_STATUS` reason 13 if it is ever reported.

#### G-12 Management channel client (0008) — L as a tool, S in AetherSDR

- **Required.** A separate UDP request/response channel for identification, configuration,
  logs and firmware update, with HMAC challenge-response for writes (0008). "The AetherSDR
  backend does not need the management channel for M1–M4. A small separate tool (or a page
  in AetherSDR, needing a go from ON8ST) does updates and configuration" (0008,
  Consequences).
- **Today.** Nothing for radio management in the P2 backend; `firmwareUpdateSource` is unset
  (`AnanBackend.cpp:555`). AetherSDR's `src/updater` updates AetherSDR itself, not radios.
- **Change.** In AetherSDR, at most: show the `mgmt` port and protocol version from the
  record, and the active level. The client itself is the separate tool `g7250ctl` (0026,
  Q-O4); a firmware page in AetherSDR may follow on the same protocol. Such a page MUST
  present the return to stock as one-way (spec A1.8a item 3: a streaming installation, no way
  back once it starts, status 60 for an abort) and MUST NOT offer `RETURN_TO_STOCK` or
  `ENTER_VENDOR_RECOVERY`, which this version always answers with status 56 (0028).

#### G-13 TCI profile alignment (0014) — M, after part C

- **Required.** AetherSDR's TCI on level 1 follows the single G7250 TCI profile of part C,
  based on [TCI] 2.1 (0014). Part C is now written (`part-c-tci-profile.md`, choices in 0021).
- **Today.** AetherSDR announces `protocol:ExpertSDR3,2.1` (`TciProtocol.cpp:380`), the same
  version as [TCI] (`sources.md`). Several init values are fixed, not taken from the radio:
  `vfo_limits` (`:351`, and a different GET answer at `:612`), `receive_only:false`
  (`:378`), `modulations_list` (`:379`), `device:AetherSDR` (`:377`, chosen for WSJT-X
  behaviour, comment at `:370–376`). It sends both `trx_count` and `channels_count:2`
  (`:360`, `:368`). The burst starts with `vfo_limits` and sends `protocol` seventh, state
  lines and `start` before `ready`, which comes last (`:345–535`, §2.4). No `hw_sensors`.
  G7250 functions (ATU, antenna, safety state, active level) have no TCI mapping;
  AetherSDR's own extension mechanism exists (`TciExtensionSet.h:13–18`).
- **Change.** Most of this is per-radio data through `RadioCapabilities`, so it is a change
  to shared TCI code, not only to the backend. For a G7250 radio:
  - **Initialization order** (part C C3.1, C3.2, and the initialization test of C9): `PROTOCOL` first, then `DEVICE`,
    `RECEIVE_ONLY`, `TRX_COUNT`, `CHANNELS_COUNT:2`, `VFO_LIMITS`, `IF_LIMITS`,
    `MODULATIONS_LIST`, then `READY`, and only after `READY` the state lines (`start`,
    `audio_samplerate`, `iq_samplerate` and the others).
  - **Values** (C3.2): `TRX_COUNT:1`; `VFO_LIMITS:500000,54000000`, fixed by the profile, not
    taken from `rx_range`, and the GET answer the same; `RECEIVE_ONLY` is an initialization
    command ([TCI] §4.1), sent once per connection from whether AetherSDR's G7250 backend can
    transmit (part C C3.2; milestone M4), not from the capability record, which is not
    available before a P2 session and a TCI client may connect before one runs; record `tx`
    bit 1 (transmit implemented) and the session go to `G7250_FEATURES` token `tx` and to
    `TX_ENABLE`, never to `RECEIVE_ONLY`; while record `tx` bit 4 is 0 (not calibrated,
    part B Annex P) `TX_ENABLE` stays `false` and an arm gets `NOT_CALIBRATED` (C5.5.6 item 7;
    0035, 0036); `RECEIVE_ONLY` never follows the arm state, which
    `TX_ENABLE` carries (C5.5.6; 0021 §43 item 16);
    `modulations_list` as C3.4; `tx_enable` from the arm state and the band edges (C5.5.6);
    `hw_sensors` and `tx_sensors` from G-07.
  - **Receiver 0 is the TX slice** (C5.6; 0021 §31). With `TRX_COUNT:1` the one TCI receiver
    0 MUST be AetherSDR's TX slice, so that `MODULATION` of receiver 0 is the TX mode.
    `TX_MODE_CLASS` follows the mode of the signal AetherSDR actually transmits, from its user
    interface or from TCI, and is rewritten on every TX-mode change; a change while keyed
    unkeys on P2 (A3.9c). A TCI `MODULATION` while keyed is refused with `KEYED` (C7.6.3).
  - **Opt-in** (C5.2): G7250 extensions only after `G7250_EXT:true;`, next to AetherSDR's own
    `aethersdr_ext_enable`.
  - **Connection watchdog** (C8.2, Table C8-2): a WebSocket Ping to the arm holder at least
    every 250 ms; 1 s without any frame drops the arm.
  - **TCI CW onto P2** (C8.2.5; 0021 §36): TCI `KEYER` and CW text go to the radio as DUC
    I&Q with PTT, shaped in AetherSDR's TX chain, not as CWX paddle bits, which cannot play
    straight-key timing and whose 1 s cap would cut a long string (B10.5a). The caps that act
    on level 1 are then: on the TCI side, applied by AetherSDR, the keyed-liveness rule of
    Table C8-1, the `KEYER` key-down cap and the CW text liveness; on the P2 side, applied by
    the radio, the watchdog, the keyed timeout, the DUC underrun check (keep DUC I&Q running
    during key-up, B10.5) and the maximum TX time. The CWX cap does not act. Keep DUC-specific
    byte 5 bit [1] clear while doing this: with it set, the radio's keyer forms the RF and
    drops the DUC I&Q (B10.5b; 0022 §85).
  - **Arm trigger** (C5.5.7; 0021 §30): arm only on an explicit arm action in AetherSDR's own
    user interface or a valid `G7250_TX_ARM`; never on a TCI, CAT or rigctl TX request, and
    never on a key in the user interface (Q-O3).
- **Fixed by part C (0021).** Keep port 50001 (0021 §2) and `protocol:ExpertSDR3,2.1`
  (0021 §4). `device:` SHOULD become `G7250` for a G7250 radio, MAY stay `AetherSDR` (0021 §5).
  Add the `G7250_` extension commands of part C C5, including `G7250_TX_POLICY` (0021 §1).
  Require TX_CHRONO answers while keyed with TCI audio (0021 §7). Accept TCI connections only
  from the local subnet, with the optional allow-list (0021 §10).
- **Added by the rc1 review (0021 §18–§26).** Apply the TCI-side keyed liveness rule before
  forwarding TX to P2: only application frames feed the keyed timeout, a generic client's
  `TUNE`/`KEYER`/CW key-down gets a 1 s window, and P2 PTT is cleared when it expires
  (0021 §18; C8.2.4). Unkey on TX-frequency changes that leave the range or need another
  RF-path state, and refuse front-end changes while keyed (0021 §19). Map the UI and at most
  one TCI client onto the one P2 arm; write `TX_ARM` = 0 only when no holder remains (0021 §24).
  Broadcast `TRX:0,false;`/`TUNE:0,false;` after every radio-side unkey. Never retake the radio
  after a takeover; on a new session rewrite mode class, limits and keyed timeout (0021 §26).
- **Added by the second review (0022 §22–§32).** Treat ownership as lost and stop C&C when
  neither DDC0 nor status packets arrive for more than 200 ms (A2.4; C7.2.1); expect the final
  `TX_ARM_STATE` (`TAKEOVER`) up to three times. Map TCI `STOP` to clearing the P2 run bit
  (C4; C5.5.5). Cap a TCI `KEYER` key-down at 1 s and require application frames at least every
  30 s while CW text is sent (C8.2). Send `G7250_FAULT_CLEAR:<fault_code>;` with the current
  code (C5.10). Derive the part C `inhibits` field from the new `LIMIT_INHIBIT` register
  (B8, 0x0221). Never start an ATU tune while keyed, and declare the mode class before arming;
  a class change while keyed unkeys on P2 (A3.9b, A3.9c).
- **Added by the final rc1 review (0022 §33–§51).** Accept two holders of the one P2 arm, the
  user interface and one TCI client, which joins without a new P2 arm; the holder that set
  `G7250_TX_POLICY:holder` owns it (C5.5.5.1, C5.5.8; 0022 §46). Never start a session on a TCI
  `START` while discovery answers 0x03 without the user's confirmation (C4; 0022 §47). Stop C&C
  at once on a `TAKEOVER` notice and when the radio's streams restart at sequence 0 by
  themselves (A2.4; C7.2.1; 0022 §48). Report `update` and `link` stream reasons and the
  firmware-update state to TCI clients (C2.8, C5.11). Pass `NOT_KEYABLE` for a refused
  `G7250_ATU_TUNE` on (C5.7).
- **Added by the review of the rc2 draft (0022 §52–§61).** Number High Priority packets so
  that numbered mode engages, and start DUC I&Q with the PTT edge (B11.6a, B10.5; 0022 §55–§57).
  Expect the takeover notices at the register destination of General bytes 31–32 (or the
  session source port), not only at the source port (A2.4; 0022 §58). A `RESTART`,
  `FACTORY_RESET` or `SWITCH_LEVEL` reply comes only after the unkey; a `RESTART` may be
  refused with status 59 while RF is present (A1.9; 0022 §59, §61). With CWX, release the
  paddle bits between characters: a paddle bit held for more than 1 s is cut (B10.5a). A
  power limit written to `LIMIT_FWD_POWER` also bounds the mean and the tune power (Table
  A3-3; 0022 §52, §53).
- **Added by the rc3 review (0021 §27–§41).** Apply the total keyed-liveness table (Table
  C8-1): a generic client's microphone `TRX` in any mode and `TRX` in `CW` get the 1 s generic
  keyed window, which only a restatement of the TX request feeds, not a read poll (0021 §27,
  §28). Apply the CW text liveness, not the keyed timeout, while CW text is sent, also for an
  opted-in client; ignore `G7250_ATU_TUNE` from a client that has not opted in; keep a
  client's liveness rule until the unkey when it changes its opt-in while keyed (0021 §29).
  Let every stop request, and `G7250_ATU_TUNE:0,abort;`, abort an ATU tune cycle by writing
  `ATU_TUNE` = 0 (C5.7; 0021 §32). Ignore `KEYER` outside `CW`, clamp arg3 at 1000 ms, and
  measure the key-down cap on the played key-down (0021 §33). Send `STOP;` to TCI clients when
  the P2 session ends without a client `STOP`; after a 1→2 level switch while unarmed report
  `level_change`, not `takeover` (C2.7, C2.9a; 0021 §41). Resend `G7250_FEATURES` when the
  token set changes, at P2 session start and end. When the last TCI client leaves, write the
  P2 `LIMIT_<name>` registers and `KEYED_TIMEOUT` back to the user interface's values
  (C5.10.1 item 8). Refuse `TRX:0,true,tci;` when the client's TX audio block period exceeds
  half the keyed timeout (C6.3.3).
- **Added by the review of rc2 (0022 §62–§73).** Without a PEP detector the radio bounds
  power on a PEP estimate from the crest factor of AetherSDR's DUC I&Q, for every mode class:
  a high-crest-factor voice signal or a signal sent under a wrong class gets less power than
  the class bound alone would give, and `TX_SENSORS` arg4 carries the estimate (A3.4c, Table
  A3-3; part C C4, C8.1; 0022 §62). Resend the General packet with PA enable and the
  DUC-specific packet in every new session, also after a takeover (A3.6a; part C C7.2.1,
  C8.5a; 0022 §64). The KEY input keys only while the DUC-specific packet of this session
  selects hardware CW (A3.1a, B10.9; 0022 §65); in v1.0 the KEY input is not used at all
  (0030), so AetherSDR need not offer hardware CW for the G7250. An ATU tune cycle runs in the TX state of the
  DUC0 frequency and returns to the DDC0 state (G-10; part C C5.7; 0022 §66). The SWR cap and,
  with break-in, SEND, the T/R relay and the RX attenuator during TX are held across element
  gaps shorter than the hang delay; choose the hang delay knowing that an external amplifier
  stays keyed for it (A3.4a, A3.9a item 4; part C C7.6.7; 0022 §67; §73 item 9). Every wait for
  RF gone is bounded by `t_reset_wait` (2 s), after which the radio is in fault
  `RF_NOT_KEYED` (A3.9a item 1a; part C C7.6.6; 0022 §68). While unkeyed the radio keeps only
  the last 25 ms of DUC I&Q (B10.7; 0022 §63). A PTT 0 in a discarded High Priority packet
  unkeys (B11.6a; 0022 §72). A DDC0 change alone during the settle wait of a key does not
  refuse it (A3.9a item 3; 0022 §73 item 10).
- **Added by the rc3 review of part C (0021 §43).** Write to each P2 `LIMIT_<name>` the
  strictest of the user interface's value and the TCI session value, and answer a TCI write
  looser than the user interface's value with `LOOSER_LIMIT` (C5.10.1 item 6). Report a P2
  `RUN_CLEARED` caused by a TCI `STOP` as `STOP` (C5.5.5 row 5). Write `ATU_TUNE` = 0 also
  when the requesting client's keyed liveness expires or its connection is lost (C5.7). Keep
  the TX source fixed while keyed and unkey on `AUDIO_STOP` or an audio rate, block size or
  channel change of a client keyed with TCI audio (C7.5 items 7, 8). Show `TRX`/`TUNE` by TX
  source, `TUNE` `true` during an ATU tune (C7.4.1). Treat the hang delay of a TCI CW
  transmission as keyed: refuse TCI `MODULATION` and front-end changes in it, and a
  user-interface mode change in it unkeys first and discards queued CW text (C7.6.3, C7.6.7;
  A3.4d; 0022 §74). Size TX_CHRONO with `length` = `AUDIO_STREAM_SAMPLES` over all channels
  and `channels` of the client (C6.3.3, C-C25). Check the active level before reporting a
  loss of ownership (C2.7). `RECEIVE_ONLY` from the backend's transmit capability (above).
- **Added by the rc4 draft of part C (0021 §44; 0022 §78, §81).** Keep `tx_enable` `true` while
  keyed: it is the permission of C5.5.6 items 1, 2 and 4–9, not an activity flag (C5.5.6).
  Accept TX audio blocks of another length than TX_CHRONO asked when the length is a multiple
  of the channels and at most 16384 data bytes, and discard others without counting them as
  application frames (C6.3.5, C8.2). Set DUC0 to the effective TX frequency, with split and
  XIT, never as a baseband offset (C7.6.1 item 5; G-08a). With level 2 active (management
  identification I-4) reject a TCI arm with `LEVEL_NOT_ACTIVE`, not `NOT_RUNNING` (C5.5.3).
  When `G7250_TX_POLICY:holder` is set while a non-holder keys, unkey it (`unkey` 1); refuse
  `holder` while disarmed with `NOT_ARMED` (C5.5.8). Send `G7250_FEATURES` with the tokens of
  Table C5-1 only (C5.4). Ignore an arm value of `G7250_TX_ARM` from a client that has not opted in, and CW text
  outside `CW` (C5.2, C4). Count AetherSDR's own user interface as an arm holder for `G7250_LIMIT` and
  `G7250_KEYED_TIMEOUT` (C5.10, C5.10.1 item 3), and write no P2 `KEYED_TIMEOUT` below
  200 ms, twice its 100 ms keep-alive (C5.10). Report `G7250_PROFILE` `active` false at every
  end of its P2 session (C5.3). After a session end resend the General packet (for the arm and
  PA enable) and the DUC-specific packet, and the DDC-specific packet as well after a
  takeover, reset, update, level or address change (G-08b; C7.2.1, C8.5a item 3; B5.4).
  Before it arms after every session end, also without a takeover, write `TX_MODE_CLASS`, the P2
  `LIMIT_<name>` registers as part C C5.10.1 item 6 sets them and `KEYED_TIMEOUT` if used
  (C8.5a item 3; 0021 §45 item 1).
  Discovery byte 21 is now fixed at 1 with scale 122 880 000 Hz, AetherSDR's encoding (G-11;
  0022 §81).
- **Added by rc4 of part C (0021 §45).** Bound buffered TX audio to `TX_STREAM_AUDIO_BUFFERING`
  plus one block, accept blocks of `length` 0, and discard blocks whose `channels`,
  `sample_rate` or `receiver` differs from the client's settings (C6.3.5). Refuse a missing
  part of a command as C4 states and list `iq_384` and `mic` where they apply (C4.1, C5.4).
  Report an ownership loss inferred from 200 ms of silence or a sequence restart as
  `LINK_DOWN` and stream reason `link`, not `TAKEOVER`, and wait for a user restart (C7.2.1).
  Answer several reasons for one TX request in the order `BUSY`, `NOT_ARMED`, `KEYED`,
  `BAD_VALUE`, `NOT_KEYABLE` (C5.12). Send `G7250_TX_STATE` `keyed` per CW element, with
  `source` and `mine` kept in the gaps (C5.5.3).
- **Added by rc4 of part C (0021 §46; 0022 §84–§87).** Implement break-in and the hang delay
  of TCI CW by holding the P2 PTT bit with DUC I&Q at zero until the hang delay has run out
  without a new element; the hang delay is the value written to DUC-specific bytes 11–12,
  clamped to 2000 ms as part B A3.4d; with hang delay 0, PTT per element, which costs the key
  latency and a T/R cycle per element, so prefer a hang delay above 0 (C8.2.5 item 1; 0021
  §47 item 2). Send exact zeros (I = Q = 0) in the gaps and the hang delay: the radio treats
  a run of all-zero DUC samples longer than `t_blank` inside the PTT transmission as no RF
  commanded, so its "RF while not keyed" check runs in every gap and detects RF there within
  `t_blank` + 10 ms (C8.2.5 item 2; A3.9a item 6; 0022 §88 item 1). Non-zero noise in a gap
  hides that gap from the check (the remaining known limit).
  While the user has hardware CW from KEY selected (DUC-specific byte 5 bit [1]), refuse TCI
  CW with `NOT_KEYABLE` and leave the bit set (C8.2.5 item 3). Wait for the P2 write echo
  (B8.4) of `TX_MODE_CLASS` and the `LIMIT_<name>` writes before the PTT edge (C5.10.1 item 6,
  C8.5a item 3). Send `STOP;` at every end of the P2 session other than a client `STOP`, a
  watchdog standby, a link loss and an inferred loss included (C2.9a, C7.2.1). Derive
  `G7250_TX_STATE` `keyed` from the own element timing (C5.5.3). Write `ATU_TUNE` = 0 at the
  policy unkey of a non-holder's tune cycle (C5.7). With level 2 active, answer `MAINTENANCE`
  before `LEVEL_NOT_ACTIVE` where I-3 shows an update or trial boot (C5.5.3). Drop the oldest
  buffered TX audio frames at the buffer bound (C6.3.5). The radio now sends the `TAKEOVER`
  notice from port 7250 to every old owner (A2.4; 0022 §87 item 2), and with CW selected
  and break-in clear drops DUC I&Q under PTT (B10.5b; 0022 §85, §87 item 6).
- **Other keying interfaces** (0021 §37). AetherSDR also has remote keying paths besides TCI
  (CAT, rigctl and its own TCI extensions `aethersdr_ext`, as the rc3 review reports; not
  surveyed in §2). Each of them SHOULD be bound by a keyed-liveness rule equivalent to part C
  C8.2: unkey and clear the P2 PTT bit when the remote client stops restating its TX request
  for 1 s, or stops sending for the keyed timeout where it can be told of one (0021 §37;
  0007 Decision 2). Where AetherSDR does not do this, the known limit is that such a client
  whose TX logic hangs is bounded on the radio only by the maximum TX time, because
  AetherSDR's own High Priority keep-alive feeds the P2 keyed timeout (C8.2.4). None of these
  paths may arm (C5.5.7).

### 3.3 Summary

| Id | Gap | Decision | Size | Milestone |
|----|-----|----------|------|-----------|
| G-01 | Discovery of board type 200 | 0002, 0003 | S | M1 |
| G-02 | Capability record | 0003 | M | M1 |
| G-03 | Register map, one port | 0005 | M | M1 (record), M2 |
| G-04 | No DDC audio | 0017 | S | M1 |
| G-05 | Rates 48–384, refusals | 0004 | M | M1 |
| G-06 | Wideband 4096 × 20/s | 0004 | M | M2 |
| G-07 | Raw telemetry, per-unit constants | 0006 | M | M2 |
| G-08a | DUC I&Q 192 ksps 24-bit | 0004 | L | M4 |
| G-08b | Arm token, mode class | 0007 | M | M4 |
| G-08c | HP refresh, keyed timeout | 0007 | S | M4 |
| G-08d | Takeover, disarm, fault display | 0007 | M | M3–M4 |
| G-08e | Emergency stop | 0007 | S | M4 |
| G-09 | Owner = IP + port | 0007 | S | M1 |
| G-10 | Antenna, filters, ATU | 0005, 0007 | M | M2 |
| G-11 | Frequency encoding, DSP clock (compatible) | B3.8 | — | — |
| G-12 | Management channel client | 0008 | L (tool) / S | M5 |
| G-13 | TCI profile | 0014 | M | after part C |

M1 needs G-01, G-02, G-03 (capability request only), G-04, G-05, G-09: about one M of
new code on top of the reused P2 codec.

---

## 4. Where the G7250 code lives

### Options

- **A. Extend the ANAN backend** with board-200 branches.
- **B. A new backend family `g7250`** in `src/core/backends/g7250/` (`G7250Backend`,
  `G7250Discovery`, register-map client, TX chain), with the board-neutral P2 code moved into
  a shared `src/core/backends/p2/` used by both ANAN and G7250.
- **C. A new backend family `g7250`** with its own copy of the P2 wire code it needs.
- **D. Aether-gate** [AETHER-GATE]: a separate bridge that presents the radio to AetherSDR as
  a FlexRadio, so that AetherSDR speaks the Flex protocol to the gate and the gate translates.
  Considered on ON8ST's request (2026-11-20) and **rejected**; see below.

### Aether-gate (option D)

What it is, from its public README [AETHER-GATE] and AetherSDR's README [AETHER], both read
on 2026-11-20:

- A bridge that "presents _any_ radio" to AetherSDR as a FlexRadio, for radios without native
  AetherSDR support: Icom LAN rigs, the IC-7300 over USB, CAT transceivers through hamlib,
  SoapySDR dongles, and a simulator. GPL-3.0-or-later. Status "working, but very young".
  Originally by G0JKN (forked from `nigelfenton/Aether-gate` into the aethersdr organisation).
- "Receive & control only — no transmit yet"; "It does not key the radio" [AETHER-GATE].
- AetherSDR integrates "radios that earn deep native support directly in-engine" through the
  `IRadioBackend` seam, and names the gate as the route for the rest [AETHER].

Why it is rejected for the product (ON8ST, 2026-11-20):

- **No transmit.** The gate never keys the radio. Every integration must offer full
  functionality including TX (0012), with the TX arm and safety rules of part A.
- **The radio would pose as a Flex.** G7250-P2 is openHPSDR Protocol 2 with board type 200
  (0002, 0016) and its own register map and arm handshake (0005, 0007). Behind a Flex
  emulation, the arm, the limits and the write rejections of the spec have no equivalent, and
  AetherSDR would treat the radio as a Flex.
- **A second hop.** A separate bridge process is what 0013 ruled out ("no separate bridge").

The native route stays: a G7250 backend on AetherSDR's `IRadioBackend` seam, on its P2 code
(0013, 0016; options B and C). That still means AetherSDR's P2 support must be extended with
transmit and with board type 200 / G7250-P2, which Q-O1 and Q-O2 cover.

A gate adapter for the stock radio (stock firmware, not G7250-P2) would be a separate project
outside the product, and is not part of this spec (ON8ST, 2026-11-20).

### Assessment

- AetherSDR states that board type is a picker filter only and "must not decide what the
  backend does" once connected (`P2Protocol.h:118–123`; `AnanDiscovery.h:67–69`). Option A
  would do exactly that in many places: Alex words, speaker stream, Saturn clock, two ADCs,
  droop tables per gateware, TX arm.
- The ANAN code is RX-only "by construction" (`P2Protocol.h:17–20`) and its TX is a planned,
  separate upstream phase (`docs/architecture/anan-p2-backend-design.md:388–406`). G7250 TX
  inside it would collide with that plan.
- Much of the ANAN code is board-neutral and already matches the G7250 (section 3.1):
  discovery, General, DDC-specific rows, DDC I&Q parsing, the session socket and keepalive.
  Copying it (option C) duplicates code that upstream would then maintain twice.
- AetherSDR's governance requires an RFC, opened as a GitHub issue, for architecture changes
  such as new threads (`GOVERNANCE.md:99–142`, list at `:106–117`). A new backend, and even
  more a shared P2 layer, falls under that. That is an outward action and needs a go from
  ON8ST.

### Recommendation

**Option B**, in two steps:

1. A new `g7250` backend family with its own discovery, backend, register-map client and
   G7250 High Priority builder, so that no ANAN behaviour changes.
2. Move the board-neutral P2 parts (discovery, General, DDC-specific rows, DDC I&Q parse,
   socket, keepalive, sender-port demultiplexing) into a shared P2 layer, and generalise the
   HL2 TX chain's output rate so that both P2 backends can use it at 192 kHz.

If upstream does not want step 2, or before a go from ON8ST, option C is the fallback: the
G7250 backend carries its own copy of the minimal P2 code, with attribution as GPL-3.0
requires (0018).

---

## 5. Unit of the P2 DDC sample-rate field (DDC-specific bytes 18–19)

Question from 0004 ("Still open"): does a host write the DDC0 rate in ksps (48, 96, …) or
in another unit?

**Result: ksps.** Both hosts checked write the rate in thousands of samples per second as a
16-bit big-endian value.

- **AetherSDR.** `P2Protocol.cpp:125` writes `rateKsps` with a big-endian 16-bit write into
  the row at `17 + 6·n` plus 1, so bytes 18–19 for DDC0 (`P2Protocol.cpp:123–125`). The
  callers pass values from `kDdc0RatesKsps` = {48, 96, 192, 384, 768, 1536}
  (`P2Protocol.h:70`; also the parameter comment at `P2Protocol.h:151–152`). A unit test
  checks the field: `tests/anan_p2_protocol_test.cpp:158` expects bytes 18–19 = 48, and
  `:165` expects 96 for a 96 ksps request.
- **piHPSDR** (github.com/dl1ycf/pihpsdr, public, commit
  `422bec56419eac45101e4dfdb3a9cd38af280398` of 2026-11-19, file `src/new_protocol.c`). The
  receiver rate is kept in Hz (for example `rx->sample_rate = 48000` in `src/receiver.c:751`,
  `384000` at `:422`), and the DDC-specific packet gets `sample_rate / 1000` as MSB and LSB:
  `src/new_protocol.c:1403–1404` (bytes 18 + 6·ddc and 19 + 6·ddc), `:1439–1440` (DDC0, bytes
  18–19, commented "sample rate MSB/LSB") and `:1443–1444` (DDC1). For PureSignal it writes
  the constant 0 and 192 into bytes 18–19 and 24–25 (`:1417–1418`, `:1421–1422`), commented
  "the sample rate is always 192".

This matches [P2] (DDC-specific bytes 18–19 in ksps, as cited in Table B6-2) and
`ddc_rates` in 0003. The open point in 0004 ("DDC rate units … still to be checked against a
public P2 host implementation") can be closed for bytes 18–19. Appendix C bytes 52–55 are not
used by the G7250 (0003 §1: no Appendix C reply), so they need no check.

piHPSDR was read only for this one field; no code was copied.

---

## 6. Open questions

### For ON8ST

- **Q-O1 Upstream route.** A G7250 backend in AetherSDR needs an RFC issue upstream
  (`GOVERNANCE.md:99–142`). When, if at all, may we open one? Until then the work can only be
  done in a local branch or fork (and a public fork is itself an outward action).
  **Proposed by 0038:** the backend is written in C++/Qt inside AetherSDR by a separate
  product-side session after firmware milestone F2, and offered upstream through the RFC
  (draft in `drafts/rfc-aethersdr-g7250-backend.md`); posting it still needs ON8ST's go.
- **Q-O2 Shared P2 layer.** Propose the shared P2 refactor (section 4, step 2), or keep the
  G7250 backend self-contained (option C) to avoid touching ANAN code?
- **Q-O3 Arm in the user interface.** Should arming be an explicit user action per session
  (a "TX arm" button), or may the backend arm automatically when the user first keys? 0007
  requires the handshake but not who triggers it. An explicit action is the safer default.
  **Answered by 0021 §30:** an explicit action. AetherSDR MUST arm only on an
  explicit arm action in its own user interface or on a valid `G7250_TX_ARM` from a TCI
  client, never on a TX request from TCI, CAT, rigctl or its own user interface (part C
  C5.5.7).
- **Q-O4 Management client.** A separate tool, or a page in AetherSDR (0008 allows both,
  with a go)? **Answered by 0026:** both, in this order. The standalone CLI
  `g7250ctl` is the reference implementation of the spec part A management protocol; a firmware
  page in AetherSDR (install new firmware, return to stock, show version and state) follows on
  the same protocol (spec A1.1a).
- **Q-O5 TCI identity on level 1.** AetherSDR sends `device:AetherSDR` for WSJT-X reasons
  (`TciProtocol.cpp:370–377`). Should the G7250 profile (part C) fix the device name, or
  leave it to the host? **Answered by 0021 §5:** level 2 MUST send `G7250`; level 1
  SHOULD send `G7250` and MAY keep `AetherSDR`.

### For the spec (part B), raised by this survey — answered

All four were decided in 0020 (§13, §14) and are in part B: Q-S1 in B3.2/B5.7, Q-S2 in
B2.4a, Q-S3 in B4.5, Q-S4 in B2.4a. Kept here for the record.


- **Q-S1 Discovery from a second socket of the owner's host.** AetherSDR sends discovery from
  a picker socket on another port of the same IP, every 5 s, during a session
  (`AnanDiscovery.cpp:240–263`). With owner = IP + port (0007), part B should state that such a
  discovery (a) is answered with 0x02 or 0x03 (B3.4), and (b) does not count as the owner's
  `REDISCOVERY` (0007 drop event 12). Proposed: 0x03 and no drop, and the host recognises its
  own session; to be decided in B3.4 and B5.7.
- **Q-S2 Host register port.** P2 General bytes 31–32 name the host's memory-mapped port
  [P2]. AetherSDR uses one ephemeral socket for everything. Part B should allow 0 to mean
  "the source port of the discovery or of the register packet", so the host needs no second
  socket.
- **Q-S3 Capability request before the session.** 0003 answers a request without a session to
  its source IP and port. If the host sends it from the session socket before the first General
  packet, the answer reaches the right socket; part B should confirm that this request is not
  C&C and does not make that port the owner.
- **Q-S4 Demultiplexing.** Part B should state from which local port the G7250 sends register
  packets and wideband packets, so that a host can demultiplex by sender port (G-03, G-06).

### For the HIS

None new. G-07 and G-08 depend on HIS-01, -11, -12 and -18, already open; G-11 no longer
depends on HIS-04, because the encoding is fixed (B3.8; 0022 §81).

---

## Sources

- [AETHER] AetherSDR, github.com/aethersdr/AetherSDR, commit
  `9f7cbebcc976b7b798a33dd5a9fc491548d1bbaa` (2026-11-18), read 2026-11-19: `README.md`;
  `GOVERNANCE.md`; `CMakeLists.txt`; `docs/architecture/anan-p2-backend-design.md`;
  `src/core/backends/IRadioBackend.h`, `RadioCapabilities.h`,
  `LocalRadioDiscoverySource.cpp`; `src/core/backends/anan/` (`P2Protocol.*`, `P2Client.*`,
  `AnanDiscovery.*`, `AnanBackend.*`); `src/core/backends/hl2/` (`Hl2TxDsp.h`,
  `Hl2Backend.cpp`, `MetisClient.h`, `MetisProtocol.h`, `Hl2TelemetryService.cpp`,
  `Hl2EmergencyStop.*`, `Hl2Spectrum.cpp`); `src/core/TciServer.*`, `TciProtocol.cpp`,
  `TciExtensionSet.h`; `src/models/RadioModel.cpp`; `src/gui/MainWindow_Session.cpp`;
  `tests/anan_p2_protocol_test.cpp`. Licence GPL-3.0. `sources.md` lists only the README
  under [AETHER] so far; the source-code reading should be added there.
- piHPSDR, github.com/dl1ycf/pihpsdr, commit `422bec56419eac45101e4dfdb3a9cd38af280398`
  (2026-11-19), `src/new_protocol.c`, `src/receiver.c`, read 2026-11-19 for section 5 only.
  Public; not yet listed in `sources.md`.
- [P2] openHPSDR Ethernet Protocol V4.4: DDC-specific packet bytes 4–22; General packet bytes
  23–39; High Priority packet bytes 4, 329–332, 345, 1400–1443; memory-mapped register
  packets; DUC-specific and DUC I&Q packets; High Priority status bytes 0–59.
- [TCI] Transceiver Control Interface 2.1 (version, `TRX_COUNT`, `PROTOCOL`).
- [G7250-MAN] Xiegu G7250 Operation Manual rev 0.1 (modes, AM power).
- Decisions 0002, 0003, 0004, 0005, 0006, 0007, 0008, 0012, 0013, 0014, 0015, 0016, 0017,
  0018; `spec/network-interface.md` rc2 (A, B); `design-note.md`.
