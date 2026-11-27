# Sources

Public sources used in this repository, with the short keys used for citations
(AGENTS.md rules 2 and 3). Only public documents are listed here. Nothing from the
reverse-engineering side may be added (AGENTS.md rule 1).

For each source: what it is, where it is, which version was read, when, and what it is used
for. "Read" means the document itself was read for this repository; "not yet read" means
the source has been found but not checked.

## Protocols

### [P2] openHPSDR Ethernet Protocol, V4.4

- What: the specification of openHPSDR Protocol 2. The base of level 1, G7250-P2 (0016).
- Where: github.com/TAPR/OpenHPSDR-Firmware, `Protocol 2/Documentation/openHPSDR Ethernet Protocol v4.4.pdf`
  (also as `.docx`; older versions 2.9–4.3 in the same folder).
- Version: V4.4. Revision history up to 7 Mar 2026 (G8NJJ); document © March 2019, Phil
  Harman VK6PH. Last change to the documentation folder: 2026-03-24.
- Read: 2026-11-19.
- Used for: 0002, 0005, 0015, 0016, `analysis/p1-vs-p2.md`.
- Terms: no licence statement in the document. The repository has a LICENSE file; its
  terms for the documentation have not been checked yet.
- Note: Appendix B (XML hardware description) is "To be written". The default port for
  memory-mapped registers is given as "xxxx", so it is not defined.

### [P1-USB] HPSDR – USB Data Protocol, V1.60

- What: the frame and C&C format of openHPSDR Protocol 1.
- Where: github.com/TAPR/OpenHPSDR-SVN, `Documentation/USB_protocol_V1.60.doc` (V1.58 and
  V1.59 in the same folder).
- Version: V1.60, 23 Jan 2019, VK6PH. Repository unchanged since 2019-05-11.
- Read: 2026-11-19.
- Used for: `analysis/p1-vs-p2.md`; reference only, since P1 is out of scope (0016).

### [P1-METIS] Metis – How it works, V1.33

- What: Protocol 1 over Ethernet (discovery, start/stop, UDP framing, firmware update).
- Where: github.com/TAPR/OpenHPSDR-Firmware, `Protocol 1/Documentation/Metis- How it works_V1.33.pdf`.
- Version: V1.33, 28 Feb 2015, VK6PH.
- Read: 2026-11-19.
- Used for: `analysis/p1-vs-p2.md`; reference only (0016).

### [HL2] Hermes-Lite 2 wiki, "Protocol"

- What: the Hermes-Lite 2 extensions to Protocol 1 (extended discovery reply, C&C map,
  watchdog).
- Where: github.com/softerhardware/Hermes-Lite2/wiki/Protocol.
- Version: wiki page, no version number.
- Read: 2026-11-19, through a summarising fetch, not line by line. To be read in full if it
  is used for anything normative.
- Used for: `analysis/p1-vs-p2.md`; reference only (0016).

### [TCI] Transceiver Control Interface, protocol 2.1

- What: the TCI protocol of Expert Electronics ("Expert Group" LLC, Taganrog). WebSocket over
  TCP; ASCII commands of the form `NAME:arg1,arg2;`; binary stream blocks with a 64-byte
  header for IQ, RX audio, TX audio, TX_CHRONO and line-out. The transceiver is the server and
  synchronises all clients. Base of the TCI profile in spec part C (0011, 0014).
- Where: github.com/ExpertSDR3/TCI, `TCI Protocol 2.1.pdf` (Russian version
  `TCI Protocol_RU 2.1.pdf`). Repository last changed 2026-10-22.
- Version: 2.1, document dated September 2026, 50 pages. TCI 1.8 clients stay compatible with
  2.1 (section 3.4).
- Read: 2026-11-19 (licence, version history, sections 3.3–3.5, initialization commands; the
  command reference only in part).
- Terms: the document says "All information contained in this document is for open public
  use. It is provided for the purpose of technical information for 3rd party vendors or
  individuals, who wish to integrate their software/hardware with Expert Electronics'
  products." It then carries an MIT-style permission notice (Copyright (c) 2023 "Expert
  Group" LLC). The repository README adds that TCI lets "manufacturers of transceivers and
  receivers ... switch to the TCI interface while maintaining the device model designation"
  and that "Modern transceivers and software must communicate using one protocol - the TCI
  protocol". No clause forbids a third-party transceiver from implementing TCI, and the
  README invites it; the MIT notice is kept with any copied portions of the document.
- Relevant for G7250: identification by `DEVICE:` and `PROTOCOL:<program>,<version>;`
  (example `PROTOCOL:ExpertSDR3,2.1;`); `TRX_COUNT`, `CHANNELS_COUNT` (renamed from `CHANNEL_COUNT`; the document's version table and its section 2.2 disagree on whether that was in 2.0 or 2.1),
  `VFO_LIMITS`, `MODULATIONS_LIST`, `READY` after initialization; `TRX`, `TUNE`, `DRIVE`,
  `TUNE_DRIVE`, `TX_ENABLE`; `IQ_START`, `IQ_SAMPLERATE`, `AUDIO_SAMPLERATE`; `HW_SENSORS`,
  `RX_SENSORS`. The document does not state a default TCP port.
- Note: AetherSDR's TCI server announces protocol 2.1 [AETHER], the same version. The server
  "will synchronize the parameters of all connected clients" (section 3.3): each client is a
  separate TCP connection, which matters for the CH394Q socket budget.

## Datasheets

### [STM32H72x] STMicroelectronics DS13311, STM32H725xE/G and STM32H735xG datasheet, Rev 5

- What: datasheet of the MCU of the main unit ([HIS] HIS 10). Used for `t_erase_max` (B2.3
  "Flash work"; 0065).
- Where: st.com.
- Version: Rev 5, November 2023.
- Section used: 6.3.13 "Memory characteristics", Table 48 "Flash memory programming"
  (maximum values guaranteed by characterization). Sector (128 KiB) erase time tERASE max:
  4 s at parallelism x8, 3.6 s at x16, 2.2 s at x32 (no x64 row); word (266 bits) programming
  time tprog max: 580 µs at x8 (after 10 000 erase operations), 360 µs at x16, 260 µs at x32,
  200 µs at x64; mass erase max 26 s at x8.
- Read: the values were taken from the datasheet on the product side and relayed by the
  openg7250 session with ON8ST's go, 2026-11-21. The spec session has not read the PDF itself.
- Also used (0081): the MCU has a single flash bank of 1 MiB, and an instruction fetch or read
  from the bank waits while it is erased or programmed (DS13311 and the reference manual
  RM0468). Relayed by the openg7250 session, 2026-11-24, from public ST documents; not read by
  the spec session, no section cited. RM0468 gets its own entry only once read (Annex P, F3).

### [CH394] WCH CH394 manual 1 (以太网协议栈芯片 CH394 手册 1), version 1.2

- What: datasheet of the WCH CH394 Ethernet protocol-stack chip (10/100 MAC+PHY, built-in
  TCP/IP, 8 sockets on CH394Q / 4 on CH394L, 32 KB RAM, SPI up to 62 MHz).
- Where: wch.cn; copy read from atta.szlcsc.com,
  `/upload/public/pdf/source/20251203/9EC0C6F644C6D345895781592F6A1E96.pdf`.
- Version: 1.2. Chinese, 45 pages; mainly the CH394Q. The CH394L manual (CH394DS2) has not been
  read.
- Read: 2026-11-19.
- Used for: `analysis/ch394-network-budget.md`.

## Host software

### [AETHER] AetherSDR README

- What: the open-source host application that is the integration route for level 1 (0013).
- Where: github.com/aethersdr/AetherSDR, `README.md`.
- Version: as of 2026-11-19; latest release v26.11.3 (2026-11-16).
- Read: 2026-11-19.
- Used for: 0016, `analysis/p1-vs-p2.md`.
- Facts used: FlexRadio (SmartSDR) is the primary target. Hermes-Lite 2 is supported over
  Protocol 1, including TX. ANAN-G2 over Protocol 2 is experimental and receive-only
  ("Transmit is a future phase"). Backends use an `IRadioBackend` seam. TCI server
  announcing protocol 2.1. Licence GPL-3.0.
- Read again 2026-11-20 for annex §4 option D: radios that "earn deep native support" are
  integrated "directly in-engine" through `IRadioBackend`; Aether-gate covers radios without
  native support; "Receive + control today (no transmit yet)" for the gate.

### [AETHER-GATE] Aether-gate README

- What: a bridge that presents other radios to AetherSDR as a FlexRadio.
- Where: github.com/aethersdr/Aether-gate, `README.md` (forked from
  github.com/nigelfenton/Aether-gate).
- Read: 2026-11-20 (also read by the openg7250 session on the same day, public page).
- Used for: annex AetherSDR §4, option D (rejected).
- Facts used: targets Icom LAN rigs, IC-7300 over USB, hamlib CAT rigs, SoapySDR dongles and a
  simulator; "Receive & control only — no transmit yet", "It does not key the radio";
  GPL-3.0-or-later; "working, but very young".

### [AETHER-SRC] AetherSDR source code

- What: the public source of AetherSDR (GPL-3.0), read-only, for the annex "Changes needed in
  AetherSDR" (`spec/annex-aethersdr.md`).
- Where: github.com/aethersdr/AetherSDR, shallow clone of commit 9f7cbeb (2026-11-18), kept in
  the session scratchpad, not in this repository. Cited by `path:line`; no code copied.
- Read: 2026-11-19.
- Facts used: `anan` backend (P2, receive-only), Hermes-Lite 2 backend (P1 TX path), TCI server
  on port 50001 announcing `ExpertSDR3,2.1`; DDC sample-rate field written in ksps
  (`src/core/backends/anan/P2Protocol.cpp:123–125`).

### [PIHPSDR] piHPSDR source code

- What: a public openHPSDR host (P1 and P2), used only to confirm the unit of the P2 DDC
  sample-rate field.
- Where: github.com/dl1ycf/pihpsdr, commit 422bec5 (2026-11-19). `src/new_protocol.c:1403–1404`
  writes `sample_rate / 1000` (ksps) into DDC-specific bytes 18–19.
- Read: 2026-11-19.

## The radio

### [G7250] Public G7250 product information

- **[HIS] G7250 hardware-interface specification.** `his/` in the OpenG7250 umbrella repository, cleared by gate review 2 on 2026-11-20; read at commit `2a38b7f` (content `89f5b5d`) and, after gate reviews 3 and 4, at `d0a39b0`, `b95f519`, `9d10e72`, `d199d33`, `6e2208a`, `5292ba8`, `0677c2d` and `72591aa` in a read-only worktree without `re/`. Cited by section, e.g. "HIS 15". The only channel for hardware facts into the product side (AGENTS.md rule 2).
- **[G7250-UPD] Radioddity, "Extended manual for Xiegu G7250 v1.0.1".** File `2026-11-09_Xiegu_G7250_update_V1.0.1.pdf` in the release package `2026-11-09_Xiegu_G7250_V1.0.1.zip`, radioddity.com/pages/xiegu-download. Public vendor update guide; identified by [HIS] HIS 15 (`d0a39b0`). Not yet read by the product side; cited only for the existence of the vendor update path through the controller.
- **[G7250-MAN] Xiegu G7250 Operation Manual, English.** Primary source. The file name says
  "rev0.1"; the title page says "Rev.1.01".
  radioddity.s3.amazonaws.com/Xiegu_G7250_Operation_Manual_EN_rev0.1_20261008.pdf
  (file dated 2026-10-08; back cover "Edition Jul 2026", XDT-B00; © WeiSheng Communication).
  49 pages. Read 2026-11-19 (link from ON8ST). The manual states that specifications may
  change with software upgrades.
- **[G7250-RDY] Radioddity product page.** radioddity.com/products/xiegu-g7250. Not read:
  HTTP 429 on 2026-11-19 (three attempts).
- **[G7250-RIGPIX] RigPix database entry.** rigpix.com/xiegu/g7250.htm, last modified
  2026-08-31. Read 2026-11-19. Secondary; many fields still "?".
- **[G7250-PUDX] PileupDX announcement.** pileupdx.com, SM0MDG, 22 Jul 2026, updated 29 Jul
  2026. Read 2026-11-19. Secondary; pre-release, "Specifications are subject to change".
- **[G7250-TOFF] YouTube channel "Temporarily Offline".** A video in which the Ethernet
  interface is tested and documented (ON8ST, 2026-11-19). Not yet identified.
- **"G7250 HDSDR Network Reception Tutorial".** Referred to by [G7250-MAN] (NETWORK
  Settings). Not found by web search on 2026-11-19. Not needed for the spec: the stock
  network protocol is out of scope (design note §7).

Facts. [G7250-MAN] wins where sources differ; differences are noted.

| Fact | Value | Source |
|------|-------|--------|
| Construction | main unit and controller (head) as separate units, joined by a dedicated cable on the CTRL interfaces (15-pin cable L2001) | [G7250-MAN] |
| Receive range | 0.5–30 MHz and 50.00–54.00 MHz | [G7250-MAN] |
| Transmit ranges | 1.8–2.0, 3.5–3.9, 7.0–7.2, 10.1–10.15, 14.0–14.35, 18.068–18.168, 21.0–21.45, 24.89–24.99, 28.0–29.7, 50.00–54.00 MHz (set per region) | [G7250-MAN] |
| Modes | CW, AM, SSB, FM, WFM | [G7250-MAN] |
| TX power | SSB/CW/NFM 100 W ±1 dB, AM 25 W ±1 dB (13.8 V, max). No separate 50 MHz value in the manual; [G7250-PUDX] and [G7250-RIGPIX] give 90 W on 50 MHz | [G7250-MAN] |
| Receiver | RF direct sampling; MDS −138 dBm (CW, 500 Hz) | [G7250-MAN] |
| Frequency stability | ±1.5 ppm 10 minutes after power-on; 1 ppm/hour at 25 °C | [G7250-MAN] |
| Antennas | ANT1 and ANT2 on the main unit, 50 Ω, selectable (menu "ANT SELECT"). Connector type not stated; [G7250-RIGPIX] says SO-239 | [G7250-MAN] |
| ATU | internal automatic tuner, 20–150 Ω, initial tuning ≤ 10 s, memory recall ≤ 0.1 s, tuning data stored per band | [G7250-MAN] |
| External tuner | TUNER interface on the main unit: +13.8 V out, tuner return, tuner trigger | [G7250-MAN] |
| Other main-unit interfaces | DC power, ground, ALC (RCA, ALC output), SEND (RCA, transmit control), CTRL, LAN | [G7250-MAN] |
| Network | LAN wired network port on the main unit. Stock firmware: HDSDR on a PC can receive the digital baseband signal over the network and "perform demodulation and control" | [G7250-MAN] |
| Ethernet controller | WCH CH394Q (8 sockets, SPI) | ON8ST, own hardware inspection, 2026-11-19; to be confirmed by the HIS |
| Controller interfaces | microphone (H-22, with PTT), SPK (external passive speaker 4–8 Ω), KEY (Morse key), USB (to PC, "control and audio transmission", CH342 driver), TF card slot | [G7250-MAN] |
| Audio output | 2 W into 4 Ω (≤ 10 % THD), 4–16 Ω | [G7250-MAN] |
| Bluetooth | two-way audio and control channel with a phone (device "XG-BT-G7250") | [G7250-MAN] |
| Supply | 13.8 V DC ±15 %, negative ground; RX ≤ 1.2 A, TX ≤ 23 A. [G7250-PUDX] and [G7250-RIGPIX] give 9–15 V | [G7250-MAN] |
| Operating temperature | 0 to +50 °C (storage/use warning: −10 to +50 °C) | [G7250-MAN] |

Not stated in [G7250-MAN]: the number of receivers, ADC type, sample rates, Ethernet speed.
"One receiver" (0001) comes from the start brief and is still to be confirmed (HIS).

## Standards

General standards cited in the decisions and the spec. Public; cited by number, not quoted.

| Key | Standard | Used for |
|-----|----------|----------|
| [RFC768] | RFC 768, User Datagram Protocol | UDP header overhead (0004), transport (spec B2) |
| [RFC791] | RFC 791, Internet Protocol | IPv4 header overhead (0004), transport (spec B2) |
| [IEEE802.3] | IEEE 802.3, Ethernet | frame overhead and 10/100 Mbit/s links (0004); CRC-32 (0008) |
| [RFC6455] | RFC 6455, The WebSocket Protocol | TCI transport [TCI] (spec part C) |
| [RFC2119] [RFC8174] | Key words for requirement levels | conformance language (spec B1) |
| [FIPS180-4] | FIPS 180-4, Secure Hash Standard | SHA-256 image integrity (0008) |
| [RFC2104] | RFC 2104, HMAC | challenge-response on the management channel (0008) |
| [RFC1812] | RFC 1812, Requirements for IP Version 4 Routers | limited broadcast is not forwarded by routers (0022 §70; spec B3.9) |
| [ITU-M1677] | ITU-R Recommendation M.1677-1, International Morse code (2009) | element and space ratios: dash 3 dots, 1 unit between elements, 3 between characters (0046; spec B10.5a); the unit 1200 / *S* ms follows from the 50-unit word PARIS used for words per minute (common practice, stated in 0046) |
| [RFC3927] | RFC 3927, Dynamic Configuration of IPv4 Link-Local Addresses | fallback address while no DHCP lease is held (0035; spec A1.6 C-1, A1.11) |
| [RFC2131] | RFC 2131, Dynamic Host Configuration Protocol | address changes and lease loss during a session (0022 §54; spec A1.11a); retransmission back-off of the DHCP retries (0041; spec A1.11); section 3.1 item 5, the 10 s minimum before DHCP again after a DHCPDECLINE (0083; spec A1.11) |
| [RFC1982] | RFC 1982, Serial Number Arithmetic | ordering of High Priority sequence numbers (0022 §35; spec B11.6a) |

## Discussion with ON8ST

Decisions taken in discussion with ON8ST are cited as "Discussion with ON8ST, <date>" and
recorded in `decisions/`. The start brief is cited as "Start brief, ON8ST, 2026-11-19".
