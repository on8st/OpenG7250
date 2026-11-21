# 0004 — Streams and sample rates on G7250-P2

- Status: accepted (ON8ST, 2026-11-19); values provisional until HIS-01, HIS-02, HIS-14, HIS-17
- Date: 2026-11-19

## Context

Level 1 is G7250-P2: openHPSDR Protocol 2 with a G7250 board type (0016, 0002). This decision
sets which P2 data streams the G7250 offers, with which parameters, and what happens when a
host asks for something that is not offered. It is the "Streams" item of the spec structure
(0001, spec section 6) and a dependency of milestone M1 (design note, section 6).

### The P2 data streams [P2]

| Stream | Direction, radio port | Packet (bytes) | Parameters set by the host |
|--------|-----------------------|----------------|----------------------------|
| DDC I&Q (RX IQ) | out, DDC0 on 1035 (DDC n on 1035 + n) | DDC Packet: bytes 0–3 sequence number, 4–11 time stamp, 12–13 bits per sample, 14–15 I&Q samples per frame, then the samples; with one DDC at 24 bits: 238 I&Q samples, bytes 16–1443, 1444 bytes in all | DDC Specific packet: byte 7 DDC0 enable, byte 17 ADC for DDC0, bytes 18–19 DDC0 sampling rate ("48/96/192/384/768/1536 ksps"), byte 22 DDC0 sample size ("default 24 bits"); General packet byte 37 bits [0]–[1] (time stamp, VITA-49) and byte 39 (endian and I&Q format: 3-byte, float or double) |
| DUC I&Q (TX IQ) | in, DUC0 on 1029 | DUC I&Q Data packet: bytes 0–3 sequence number, then 240 I&Q samples of 3+3 bytes, bytes 4–1443, 1444 bytes in all | DUC Specific packet: bytes 14–15 DUC0 sampling rate ("For current hardware fixed at 192ksps"), byte 16 DUC0 bits ("fixed at 24 bits per sample") |
| Microphone data | out, on 1026 | Microphone Data packet: bytes 0–3 sequence number, 64 samples of 16 bits, bytes 4–131, 132 bytes in all; "The sample rate is 48ksps" | none (DUC Specific byte 50 selects line in or microphone) |
| Wideband data (raw ADC) | out, ADC0 on 1027 (ADC n on 1027 + n) | Wideband Data packet: bytes 0–3 sequence number, 512 samples of 16 bits, bytes 4–1027, 1028 bytes in all (defaults) | General packet byte 23 enable per ADC, bytes 24–25 samples per packet (default 512), byte 26 sample size (default 16), byte 27 update rate (0–255 ms per frame, default 20 ms), byte 28 packets per frame (default 32) |
| DDC audio | in, on 1028 | DDC Audio packet, 64 L+R samples of 16 bits, 260 bytes | General packet bytes 13–14 |

P2 also says, in NOTE2 of "Detailed operation" [P2]: "For all current openHPSDR
implementations the sample width is set to 24 bits and sampling rate fixed at 192 ksps" for
the DUC. If the hardware has an audio codec, the rate at which microphone packets arrive can
pace the host's DUC I&Q; without one, the host should use a fractional resampler. The DUC FIFO
status in the High Priority Status packet "has been removed" [P2, NOTE2].

The wideband stream is "intended to be used to display very wide spectral and waterfall
displays" and is presented "on a block basis so may not be used for real-time demodulation
purposes" [P2, Wideband Data packet].

The full hardware description in the discovery reply (board type 255, Appendix C) has fields
for exactly these capabilities [P2, Appendix C]: byte 39 number of DDCs, byte 42 wideband
data supported, bytes 43–50 wideband samples per packet, sample size, update rate and packets
per frame (each "fixed or variable"), byte 51 individual DDC sample rates supported, bytes
52–53 minimum and 54–55 maximum DDC sampling rate (ksps), byte 59 DDC audio codec, byte 64
time stamping supported, byte 70 number of DUCs, byte 72 microphone supported, byte 73 line in
supported, byte 79 DUC I&Q sample rate (ksps), byte 80 bits per I/Q sample, bytes 81–82 I&Q
samples per Ethernet packet. The standard discovery reply has byte 20 "Number of DDCs
implemented" [P2, Discovery reply].

### What is already decided

- DDC audio (1028) is not offered and has no socket (0017).
- The G7250 has one receiver according to the start brief (0001); not stated in
  [G7250-MAN], to be confirmed (HIS-02).
- The microphone, speaker, key, USB and Bluetooth are on the controller; the main unit has no
  audio connector [G7250-MAN] (0017). Whether the CTRL cable carries microphone audio to the
  main unit is HIS-17.
- Nothing about the ADC, sample clock or decimated rates is public: "RF direct sampling"
  only [G7250-MAN] (HIS-01).
- The capability block is decided in 0003, written in parallel. This decision only says
  what goes into it.

### The network hardware [CH394]

The Ethernet controller is a WCH CH394Q (ON8ST, own hardware inspection, 2026-11-19;
to be confirmed, HIS-14). From the datasheet [CH394]:

- 10/100 Mbit/s MAC and PHY; 8 sockets on the CH394Q.
- SPI device interface "最高 62MHz" (up to 62 MHz), mode 0 or 3 (section 3). The SPI clock
  limit is 62 MHz at VDDIO = 3.3 V, 55 MHz at 1.8 V and 30 MHz at 1.2 V (table 10-4).
- Every SPI data frame is a 16-bit offset address, an 8-bit control word and N data bytes
  (section 5.2.1). The control word has one read/write bit (table 5-1), so a frame moves data
  in one direction only: transmit and receive share the SPI time.
- Above 45 MHz, a gap tSR of at least 10 ns is needed between data bytes when reading or
  writing the socket buffers; reading a buffer needs tSC ≥ 180 ns between address and first
  data byte; a register operation is followed by a processing time tRI of (0.3·N − 0.3) µs in
  IPv4 mode (table 10-4 and notes).
- 16 KB transmit and 16 KB receive buffer, 2 KB per socket by default, re-assignable in 1 KB
  blocks per socket through Sn_TXBUF_SIZE and Sn_RXBUF_SIZE (0–16 KB each), as long as each
  total stays within 16 KB (sections 6, 7.2.12, 7.2.13).
- In UDP mode the chip prepends an 8-byte information header (source IP 4, port 2, length 2)
  to each received datagram in the socket receive buffer. "UDP 模式下 CH394Q 无法提供流控"
  (no flow control in UDP mode): received data should be read quickly to avoid loss. At most
  1472 bytes of UDP data per packet; longer writes are split by the chip (section 12.2.4).

`analysis/ch394-network-budget.md` gives the payload per RX sample rate and concludes: 384 ksps
or lower realistic, 768 ksps marginal, 1536 ksps impossible. This decision adds the packet
overheads and the TX direction.

## Throughput arithmetic

### Assumptions

- Packet sizes as in [P2]: DDC0 1444 bytes per 238 samples, DUC0 1444 bytes per 240 samples,
  microphone 132 bytes per 64 samples, wideband 1028 bytes per 512 samples, High Priority
  Status 60 bytes (bytes 0–59).
- High Priority Status is sent every 50 ms, and every 1 ms during TX [P2, High Priority Status
  packet].
- On the wire, each UDP datagram adds UDP 8 [RFC 768], IPv4 20 without options [RFC 791],
  Ethernet header 14 and FCS 4, preamble and SFD 8, inter-frame gap 12 [IEEE 802.3]: **66 bytes
  per packet**. (These three standards are not yet listed in `sources.md`.)
- In socket mode the CH394Q builds the UDP, IP and Ethernet headers itself, so only the UDP
  payload crosses SPI [CH394, section 12.2.4]. Per packet the SPI carries in addition: the
  3-byte frame header (address + control word) [CH394, 5.2.1], the 8-byte information header on
  received packets [CH394, 12.2.4], and the register accesses for free size, pointers and the
  SEND/RECV command with its completion poll. Those register accesses, with tRI and tSC
  [CH394, table 10-4], are **estimated at 3 µs per packet**. This is an estimate, to be
  replaced by a measurement on the hardware.
- SPI at 62 MHz with the 10 ns tSR gap: 8/62 MHz + 10 ns = 139 ns per byte, so about
  **57.5 Mbit/s effective** for buffer data, in both directions together.
- In MACRAW mode (HIS-14) the full frames (payload + 42 bytes of headers) would cross SPI and
  the processor would run IP and UDP itself [CH394]. The numbers below assume socket mode.

### Per stream

| Stream | Packets/s | UDP payload (Mbit/s) | On the wire (Mbit/s) | SPI busy at 62 MHz |
|--------|-----------|----------------------|----------------------|--------------------|
| DDC0 48 ksps | 201.7 | 2.33 | 2.44 | 4.1 % |
| DDC0 96 ksps | 403.4 | 4.66 | 4.87 | 8.2 % |
| DDC0 192 ksps | 806.7 | 9.32 | 9.75 | 16.5 % |
| DDC0 384 ksps | 1613.4 | 18.64 | 19.49 | 32.9 % |
| DDC0 768 ksps | 3226.9 | 37.28 | 38.98 | 65.9 % |
| DDC0 1536 ksps | 6453.8 | 74.55 | 77.96 | 131.8 % |
| DUC0 192 ksps, 24-bit (in) | 800.0 | 9.24 | 9.66 | 16.4 % |
| Microphone 48 ksps, 16-bit | 750.0 | 0.79 | 1.19 | 1.6 % |
| Wideband, P2 defaults (32 × 512 samples every 20 ms) | 1600.0 | 13.16 | 14.00 | 23.4 % |
| Wideband, reduced (1 packet every 100 ms) | 10.0 | 0.08 | 0.09 | 0.1 % |
| Wideband, chosen (8 × 512 samples every 50 ms) | 160.0 | 1.32 | 1.40 | 2.3 % |
| High Priority Status, TX (1 ms) | 1000.0 | 0.48 | 1.01 | 1.2 % |
| High Priority Status, RX (50 ms) | 20.0 | 0.01 | 0.02 | < 0.1 % |

Packets/s = sample rate / samples per packet. Payload = packets/s × packet bytes × 8. Wire =
packets/s × (packet bytes + 66) × 8. SPI busy = packets/s × ((packet bytes + 3 [+ 8 if
received]) × 139 ns + 3 µs).

### RX and TX together

Worst case is transmitting: DDC0 is assumed to keep running (nothing in [P2] stops the DDCs
during TX, and P2 hosts may keep showing the receive spectrum),
DUC0 runs at 192 ksps, and High Priority Status goes to 1 ms. Host-to-radio control packets
(General, DDC/DUC Specific, High Priority, register map) are sent on change [P2] and are left
out; they are small next to the IQ streams.

| Scenario | UDP payload (Mbit/s) | Wire, radio→host (Mbit/s) | Wire, host→radio (Mbit/s) | SPI busy at 62 MHz |
|----------|----------------------|---------------------------|---------------------------|--------------------|
| S1: RX 192 + TX + status | 19.0 | 10.8 | 9.7 | 34 % |
| S2: RX 384 + TX + status | 28.4 | 20.5 | 9.7 | 51 % |
| S3: S2 + microphone | 29.2 | 21.7 | 9.7 | 52 % |
| S3W: S3 + wideband as chosen (4096 samples, 20/s) | 30.5 | 23.1 | 9.7 | 54 % |
| S4: S3 + wideband at P2 defaults | 42.3 | 35.7 | 9.7 | 76 % |
| S5: RX 768 + TX + status | 47.0 | 40.0 | 9.7 | 84 % |
| S6: RX 768, no TX | 37.3 | 39.0 | — | 66 % |
| S7: RX 1536, no TX | 74.6 | 78.0 | — | 132 % |

The 100 Mbit/s link is full duplex [CH394] and is not the limit in any scenario up to 768 ksps.
SPI is. 100 % SPI busy is not usable in practice: the same SPI also carries interrupt
handling, socket status polling, discovery, the register map (0005) and the management
channel (0008), and the processor must drain a received packet before the next one overwrites
the buffer (no flow control in UDP mode [CH394, 12.2.4]). This decision uses a **design target
of at most 60 % SPI busy in the worst case**, leaving 40 % for those and for the error in the
3 µs estimate.

### Sensitivity to the SPI clock (HIS-14)

Whether the processor drives SPI at 62 MHz is not known (HIS-14). Same method, below 45 MHz
without the tSR gap [CH394, table 10-4]:

| SPI clock | S1 (RX 192 + TX) | S2 (RX 384 + TX) | S5 (RX 768 + TX) |
|-----------|------------------|------------------|------------------|
| 62 MHz | 34 % | 51 % | 84 % |
| 45 MHz | 43 % | 64 % | 106 % |
| 31 MHz | 63 % | 93 % | 154 % |

So 384 ksps with TX meets the 60 % target only with SPI close to the maximum. At about 45 MHz
it is just above the target; at about 31 MHz even 192 ksps is.

## Options considered

### 1. RX IQ: highest DDC0 sample rate

The set always starts at 48 ksps and goes up in the P2 steps [P2, DDC Specific bytes 18–19].
P2 can only declare a minimum and a maximum rate (Appendix C bytes 52–55), so the offered set
has to be a contiguous run of P2 rates; that rules out sets with gaps.

#### 1A. 48, 96 and 192 ksps

- For: the lowest load: 34 % SPI busy with TX at 62 MHz (S1), so it stays within the 60 %
  target down to about 35 MHz SPI. The most tolerant of an unfavourable HIS-14 answer.
- For: the same rate as the DUC (192 ksps [P2, NOTE2]); one packet rate in each direction of
  roughly 800 packets/s, which keeps the processor's interrupt load predictable (HIS-18).
- For: 192 kHz of spectrum covers a whole HF band segment for most uses.
- Against: leaves half the SPI budget unused when the SPI does run at 62 MHz.
- Against: narrower than what P2 hosts usually offer; a panadapter of 192 kHz cannot show,
  for example, the whole 14.000–14.350 MHz band [G7250-MAN, transmit ranges] at once.
- Against: if HIS-01 shows the ADC and decimator could do more, the network would be the only
  reason to refuse it, and at 62 MHz SPI it is not a reason.

#### 1B. 48 to 384 ksps (chosen, provisionally)

- For: 51 % SPI busy with TX at 62 MHz (S2), 52 % with the microphone (S3): within the 60 %
  target with the stated margin. Matches the conclusion of `analysis/ch394-network-budget.md`.
- For: 384 kHz covers 160, 40, 30, 20, 17 and 12 m whole in one view [G7250-MAN, transmit
  ranges] (80 m is 400 kHz, 15 m 450 kHz, 10 m 1.7 MHz), which is what users expect of an SDR
  panadapter.
- For: about 1600 RX packets/s, one every 620 µs; with the receive buffers below that is
  manageable for a socket chip without flow control.
- Against: depends on SPI running close to 62 MHz (HIS-14). At 45 MHz it is 64 %, at 31 MHz
  93 %. If HIS-14 gives a low clock, the maximum drops to 192 (option 1A).
- Against: depends on the ADC clock and decimation chain producing 384 ksps exactly (HIS-01).
  P2 rates are 48 ksps × 2^n [P2]; an ADC clock that is not a matching multiple would need a
  fractional resampler in the firmware, at a compute cost (HIS-18).
- Against: leaves no SPI room for wideband data at P2 defaults (S4, 76 %).

#### 1C. 48 to 768 ksps

- For: the widest view the hardware might give; 768 kHz shows 80 m and 15 m whole
  [G7250-MAN, transmit ranges].
- For: receive only, 66 % at 62 MHz (S6): only just above the target.
- Against: with TX it is 84 % (S5), above the target and with almost nothing left for control,
  registers and management. A burst on the SPI during TX would drop IQ packets in one of the
  two directions; a lost DUC packet is a gap in the transmitted signal on a 100 W PA.
- Against: a variant "768 only while receiving" would mean the radio changes or refuses a
  rate when PTT goes on. P2 has no way to tell the host that the effective RX rate changed
  (the DDC packet carries bits per sample and samples per frame, not the rate, bytes 12–15
  [P2]), and a host would not expect it.
- Against: the processor must move 37 Mbit/s of RX data through SPI at about 3200 packets/s,
  every 310 µs; unknown whether that fits (HIS-18).

#### 1D. All P2 rates up to 1536 ksps

- For: full P2 range, like other P2 hardware (Appendix A gives 48 to 1,536 ksps for the openHPSDR boards, for example Hermes [P2]).
- Against: impossible. 1536 ksps alone is 132 % of the SPI at 62 MHz (S7), and 78 Mbit/s on
  the wire is close to the 100 Mbit/s link [CH394]. It would be declared but never work.

### 2. RX IQ: format and number of DDCs

#### 2A. One DDC, 24-bit big-endian 3-byte format only (chosen)

- For: the default of P2: "Big-Endian and I&Q data in 3 Byte format" when General byte 39 is
  zero, and "Current hardware FPGA code supports 24 bits per sample" [P2, General packet byte
  39; DDC Packet]. Every P2 host supports it.
- For: one DDC fits the socket budget: one socket for port 1035; G7250-P2 then uses 7 of the 8
  CH394Q sockets, one left for the management channel (0017, `analysis/ch394-network-budget.md`).
- Against: if HIS-02 shows a second receiver, it cannot be offered without a ninth socket:
  DDC1 needs port 1036 [P2, General bytes 17–18], and the only free socket is meant for the
  management channel (0008). A second DDC at 384 ksps would also take the SPI load with TX to
  84 %.

#### 2B. Also float or double I&Q (DFC formats) or little-endian

- For: P2 allows it since V2.9 for hosts doing all DSP in software [P2, "Direct Fourier
  Conversion"; General byte 39 bits [1], [3], [4]].
- Against: float is 4 bytes and double 8 bytes per I or Q instead of 3, so fewer samples per
  packet and up to 2.7 times the SPI load for the same rate. It does not fit the budget.
- Against: no G7250 host needs it; AetherSDR's P2 path works with an existing P2 radio
  [AETHER], which uses the default format.

#### 2C. Time stamps / VITA-49

- Against: they need a 1 PPS or GPS time base [P2, DDC Packet "Time stamp"] that the G7250
  does not have according to [G7250-MAN]. Not offered: Appendix C byte 64 = 0, and the
  time-stamp bytes 4–11 of each DDC packet are sent as zero (spec part B choice).

### 3. TX IQ (DUC0)

#### 3A. DUC0 at 192 ksps, 24 bits, 240 samples per packet (chosen)

- For: what P2 calls current practice: "sample width is set to 24 bits and sampling rate fixed
  at 192 ksps" [P2, NOTE2; DUC Specific bytes 14–16]. Every P2 host that transmits sends this.
- For: 16 % SPI busy at 62 MHz; fits in every scenario.
- Against: P2 "enables a variable DUC I&Q sample width and sample rate" [P2, NOTE2]; a host
  might ask for another rate. That case is handled under option group 6.

#### 3B. A lower DUC rate (for example 48 ksps) to save SPI time

- For: SSB, CW and AM need far less than 192 kHz.
- Against: no P2 host is known to send anything other than 192 ksps; P2 itself says "For
  current hardware fixed at 192ksps" [P2, DUC Specific bytes 14–15]. AetherSDR would have to
  change too. The saving (12 % SPI) is not needed at 384 ksps RX.

### 4. Microphone data (out on 1026)

The microphone is on the controller [G7250-MAN]. The main unit can only send microphone data
if the controller passes the audio over the CTRL cable (HIS-17). The microphone stream goes
out on port 1026, which already has a socket for DUC Specific packets [P2], so it costs no
socket, only 1.6 % SPI.

#### 4A. Not offered

- For: simplest; nothing depends on HIS-17.
- For: on level 1 the DSP runs on the host (0011), so a host such as AetherSDR (0013) can take
  transmit audio from a microphone on the computer. Whether AetherSDR does so for P2 is not
  stated in its README [AETHER] and is to be checked.
- Against: the H-22 microphone and its PTT [G7250-MAN] on the controller would be unusable for
  phone TX through the host.

#### 4B. Offered if HIS-17 shows a microphone path; not offered otherwise (chosen)

48 ksps, 16 bits, 64 samples per packet, exactly as [P2, Microphone Data packet].

- For: uses the hardware the user already has (the H-22 on the controller) when it is there.
- For: standard P2, no extension; declared by Appendix C byte 72 [P2].
- For: when present, its packet rate gives the host a pacing clock for DUC I&Q, as P2
  describes [P2, NOTE2].
- Against: the decision stays open until HIS-17. The M4 exit criterion (design note) cannot
  be fixed before then.
- Against: the microphone audio comes over the CTRL link; whether its sample clock is locked
  to the DUC clock is also part of HIS-17.

#### 4C. Always send microphone packets; silence if there is no microphone path

- For: gives every host the pacing clock of [P2, NOTE2], independent of HIS-17.
- Against: the radio would send a stream that pretends a microphone exists; Appendix C byte
  72 would have to say "supported" for a stream that carries only zeros. That is the kind of
  "reads as standard but is not" that 0005 avoids.
- Against: P2 has dropped the DUC FIFO status because host developers said pacing help "is
  not necessary" [P2, NOTE2]; hosts resample on their side.

### 5. Wideband / raw ADC data (out on 1027)

Wideband data is a block of raw ADC samples [P2, Wideband Data packet]. It shares the port
1027 socket with the High Priority packets from the host [P2], so it costs no socket. It needs
access to raw ADC samples in a block of 16 k samples (P2 defaults: 32 packets × 512 samples
[P2, General byte 28]), which depends on the ADC and its connection (HIS-01) and on memory
(HIS-18).

#### 5A. Not offered in the first version; revisit after HIS-01 and HIS-18

- For: M1 (discovery, capability block, RX IQ) does not need it.
- For: no SPI cost; keeps all headroom for RX IQ at 384 ksps with TX.
- For: P2 lets the radio say "not supported" (Appendix C byte 42 = 0) [P2], so a host knows.
- Against: no band-wide spectrum overview from the radio until it is added; the host can only
  show the DDC0 bandwidth.

#### 5B. Offer with P2 defaults (512 × 16-bit, 32 packets per frame, 20 ms)

- For: what P2 hosts expect if they use wideband at all.
- Against: 13.2 Mbit/s payload, 23 % SPI; with RX 384 and TX that is 76 % (S4), above the
  target. It would push RX down to 192 ksps.
- Against: needs 32 KB of raw samples per frame in processor RAM (HIS-18), the same as the
  whole CH394Q buffer [CH394].

#### 5C. Offer with reduced fixed parameters (chosen, for milestone M2)

For example one or two packets of 512 samples per frame and a frame every 100 ms, declared as
fixed values in Appendix C bytes 45–50 [P2]. P2 already has a precedent: "For ANAN-10E this is
fixed at 1024 by 16 bit samples per frame" [P2, General byte 28].

- For: under 0.2 % SPI; fits in every scenario.
- For: standard P2 mechanism, no extension.
- Against: still needs raw ADC access (HIS-01). Not known whether AetherSDR uses wideband data
  at all [AETHER, README does not say].
- Against: a host that ignores the "fixed" declarations and sets other values would get the
  fixed ones; that needs the same reject-or-report rule as option group 6.
- This is the preferred route if HIS-01 and HIS-18 show that raw ADC blocks are available.

### 6. Requests for a rate or stream that is not offered

P2 has no negative acknowledgement for settings [P2]; the host sets, the radio follows. The DDC
packet does not carry the sample rate (bytes 12–15 carry bits per sample and samples per frame
only [P2]), so a host cannot see from the data which rate it got.

#### 6A. Clamp silently to the nearest offered rate

- For: the host always gets IQ.
- Against: the host interprets the IQ at the rate it asked for: wrong frequency scale, wrong
  filter widths, wrong demodulation. Nothing in the data shows it.
- Against: for the DUC, IQ played at another rate than the host meant is a wrong transmitted
  signal on a 100 W PA.

#### 6B. Clamp and report the effective rate in a G7250 register (0005)

- For: a G7250-aware host can see and correct it.
- Against: a generic P2 host does not read the G7250 register map (0015), so for that host it
  is option 6A.

#### 6C. Ignore the packet and keep the previous setting

- Against: the host believes the new setting is active; same problem as 6A.

#### 6D. Refuse: the affected stream does not run until the request is valid; report the reason in the register map (chosen)

- For: no data is better than mislabelled data, especially for TX.
- For: the host learns the offered set beforehand from the capability block (0003) and, if
  0003 uses the P2 full description, from the standard P2 fields (Appendix C bytes 39, 42,
  51–55, 64, 70, 72, 79–82 [P2]). A correct host never hits the refusal.
- For: a G7250-aware host can read why a stream does not run.
- Against: a generic host that asks for 768 ksps gets no IQ and no standard explanation. That
  host can still read the maximum rate from the full description, if 0003 uses it.

## Decision

Accepted by ON8ST, 2026-11-19. The values remain provisional, pending HIS-01, HIS-02, HIS-14
and HIS-17.

1. **RX IQ.** One DDC, DDC0 on port 1035 (HIS-02). Sample rates 48, 96, 192 and 384 ksps
   (option 1B; maximum depends on HIS-01 and HIS-14). 24-bit I and Q, big-endian, 3-byte
   format, 238 samples per packet, 1444-byte packets as in [P2, DDC Packet]. No float, double
   or little-endian formats, no time stamps or VITA-49 (2A, 2C). DDC0 takes its data from
   ADC0 only (HIS-01, HIS-03).
2. **TX IQ.** DUC0 on port 1029 at 192 ksps, 24 bits, 240 samples per packet, 1444-byte
   packets, as in [P2, DUC I&Q Data packet; NOTE2] (3A). TX itself still needs the arm (0015).
3. **Microphone.** Offered as standard P2 microphone data (48 ksps, 16-bit, 64 samples per
   packet, out on 1026) only if HIS-17 shows that the controller's microphone reaches the main
   unit; otherwise not offered (4B). No silent microphone stream just for timing (4C rejected;
   ON8ST, 2026-11-19).
4. **Wideband.** Planned for milestone M2 with small fixed parameters (5C; ON8ST,
   2026-11-19). Not offered in M1. Proposed fixed parameters for ADC0, out on port 1027:
   **4096 samples per frame, 20 frames per second** (ON8ST, 2026-11-19): 512 samples per
   packet, 16-bit samples, 8 packets per frame, one frame every 50 ms (General bytes 24–28
   [P2]: 512, 16, 50 ms, 8). That is 160 packets/s, 1028 bytes each, 1.32 Mbit/s payload and
   about 2.3 % SPI busy at 62 MHz (method of "Per stream": 160 × ((1028 + 3) × 139 ns + 3 µs)
   = 23.4 ms per second). With RX 384, TX and microphone (S3W) the total is about 54 %, within
   the 60 % target. The frequency resolution of a 4096-point FFT is the ADC rate / 4096
   (HIS-01); at an ADC rate of 100 MSPS that would be about 24 kHz per bin. Earlier proposal
   in the draft: 1 × 512 samples every 100 ms (0.15 % SPI), replaced by ON8ST's choice. The values are announced as fixed in the capability
   block (0003) and, if 0003 uses the P2 full description, in Appendix C byte 42 = 1 and
   bytes 43–50 with each parameter marked "fixed" [P2, Appendix C], as P2 already does for
   ANAN-10E [P2, General byte 28]. Depends on HIS-01 (raw ADC access) and HIS-18 (memory and
   processor load); if the HIS shows that raw ADC blocks cannot be delivered, this point is
   revisited.
5. **DDC audio.** Not offered (0017).
6. **Design target.** Worst-case SPI load (RX at the maximum rate, TX, status at 1 ms, all
   offered streams) stays at or below 60 % of the SPI time at the actual SPI clock. The
   maximum RX rate is set by this target once HIS-14 gives the clock: 384 ksps only if the SPI
   clock is about 50 MHz or more, 192 ksps maximum below that (see "Sensitivity"; ON8ST,
   2026-11-19).
7. **Requests not offered** (option 6D):
   - DDC Specific packet with a DDC0 rate outside the offered set, or a sample size other than
     24 (or 0 = default): DDC0 does not run (it stops if running) until a valid DDC Specific
     packet arrives. Enables for DDC1–DDC79 are ignored.
   - General byte 39 asking for a format other than big-endian 3-byte, or byte 37 asking for
     time stamps or VITA-49: the request is not followed, and DDC0 does not run while it
     stands.
   - DUC Specific packet with a DUC0 rate other than 192 or bits other than 24: DUC I&Q is not
     accepted, and transmit is refused (the PA stays off) until a valid DUC Specific packet
     arrives.
   - Wideband (General bytes 23–28): before M2, the enable is not followed and no wideband
     packets are sent. From M2, an enable for ADC0 is followed only while bytes 24–28 are
     0 (default) or equal to the fixed values of point 4; any other value means wideband
     does not run until the request is valid. Enables for other ADCs are not followed.
   - In each case the reason is reported in a G7250 status register (0005). Requests are
     never clamped to an offered value (6A, 6B are rejected; ON8ST, 2026-11-19).
   - **768 ksps and above are not offered**, not even receive-only (option 1C; ON8ST,
     2026-11-19).
8. **How the host learns what is offered:** the capability block (0003). If 0003 uses the P2
   full description, the values above go into Appendix C bytes 39 (1), 42 (0 in M1, 1 from
   M2), 43–50 (from M2: the fixed wideband values of point 4), 51, 52–55 (48, 384), 59 (0, per
   0017), 64 (0), 70 (1), 72 and 73 (per HIS-17), 79 (192), 80 (24), 81–82 (240) [P2]; the
   standard discovery reply byte 20 is 1 [P2].
9. **10 Mbit/s link** (ON8ST, 2026-11-19). On a 10 Mbit/s link the radio refuses the session:
   it answers discovery, but starts no stream (no DDC0, DUC0, microphone or wideband), and
   reports the reason ("link speed below 100 Mbit/s") in the status register (0005) and in
   the capability block (0003). Reason: DUC0 alone is 9.7 Mbit/s on the wire ("Per stream"),
   so TX cannot work, and a partial RX-only mode is not offered.

## Consequences

- Spec part B, streams section: one table per stream with port, packet layout, sample rates,
  sizes and the refusal rule above, each with a [P2] reference. The annex "Differences from
  standard P2" lists the refusals in point 7 (P2 itself has no refusal mechanism).
- The G7250 register map (0005) gets a read-only stream status register: per stream running or
  not, and the reason when refused (for example "DDC0 rate not offered").
- The capability block (0003) carries: number of DDCs, the offered DDC rates (minimum and
  maximum), IQ format, DUC rate and bits, microphone yes/no, wideband yes/no (no in M1, yes
  from M2) with its fixed parameters (512 samples per packet, 16 bits, 8 packets per frame, 50 ms), DDC
  audio no, and the reason when a session is refused (10 Mbit/s link).
- **Wideband in M2.** Milestone M2 gains wideband data at the fixed parameters of Decision
  point 4; its exit criterion includes a host receiving a complete 4096-sample frame (8 packets of
  1028 bytes) every 50 ms while DDC0 at the maximum rate and DUC0 run, with the SPI load
  measured. The raw ADC block is 4096 × 16 bits = 8 KB of processor RAM (HIS-18), against
  32 KB at P2 defaults (5B). If HIS-01 or HIS-18 shows that raw ADC blocks cannot be delivered, wideband is
  revisited and M2 is re-planned.
- **CH394Q buffer split.** With the default 2 KB per socket [CH394, 7.2.12–7.2.13], a socket
  holds one 1444-byte P2 packet (plus 8 bytes of header on receive). With no flow control in
  UDP mode [CH394, 12.2.4], a second DUC packet arriving before the first is read would be
  lost. The firmware therefore redistributes the 16 + 16 KB. Indicative split, to be fixed in
  spec part B or the firmware design (HIS-14, socket mode). **Fixed by 0053** in spec part B,
  Table B2-2, which differs from this table:

  | Socket | Port | RX buffer | TX buffer |
  |--------|------|-----------|-----------|
  | 1 | 1024 | 2 KB | 1 KB |
  | 2 | 1025 (DDC Specific in, status out) | 2 KB | 1 KB |
  | 3 | 1026 (DUC Specific in, microphone out) | 1 KB | 1 KB |
  | 4 | 1027 (High Priority in, wideband out) | 2 KB | 0 KB in M1 (2 KB from M2, see below) |
  | 5 | 1029 (DUC I&Q in) | 6 KB (4 packets) | 0 KB |
  | 6 | 1035 (DDC0 I&Q out) | 0 KB | 8 KB (5 packets) |
  | 7 | register map (0005) | 2 KB | 2 KB |
  | 8 | management channel (0008) | 1 KB | 3 KB |
  | | total | 16 KB | 16 KB |

  The 1 KB block granularity [CH394, 7.2.12] means each buffer holds a whole number of
  kilobytes. In MACRAW mode (HIS-14) this table does not apply. From M2, a 1028-byte
  wideband packet does not fit in 1 KB, so socket 4 needs 2 KB of transmit buffer; the 2 KB
  is taken from socket 6 (8 → 6 KB, 4 DDC0 packets) or socket 8 (3 → 1 KB), to be fixed in
  spec part B. A wideband frame is 8 packets (8 KB); the processor keeps the frame in its own
  RAM and hands packets to the CH394Q as buffer space frees, so the frame does not have to
  fit in the socket buffer.
- **Link speed.** On a 10 Mbit/s link the session is refused (Decision point 9): the radio
  still answers discovery, so the host can find it and read why, but no stream starts. This
  replaces the earlier suggestion to refuse only TX and limit RX to 48 ksps. Spec part B
  states how the firmware reads the negotiated link speed from the CH394Q (HIS-14) and the
  exact reason code (0005).
- **M1 exit criterion** ("receives IQ at the supported rates", design note section 6) is
  tested at 48, 96, 192 and 384 ksps, with and without simultaneous TX IQ, against the
  reference model in g7250-testbench, and on the radio with an SPI load measurement that
  replaces the 3 µs estimate.
- **AetherSDR annex.** The G7250 backend must ask only for offered rates, read them from the
  capability block, and send DUC I&Q at 192 ksps 24-bit. Whether AetherSDR's P2 path sends
  wideband enable or other formats is to be checked against its public source.
- **Revisit** when the HIS answers: HIS-01 (rates the hardware can make; wideband), HIS-02
  (more DDCs; then also sockets), HIS-14 (SPI clock and socket or MACRAW mode; maximum rate),
  HIS-17 (microphone), HIS-18 (processor load at 1600 + 800 packets/s; wideband memory). The
  decision is accepted; the values change only through these HIS answers.

## Resolved (ON8ST, 2026-11-19)

- Design target of at most 60 % SPI busy in the worst case: accepted. 384 ksps is offered
  only if the SPI clock is about 50 MHz or more; otherwise the maximum is 192 ksps.
- Requests that are not offered: refused, with the reason reported in a G7250 register
  (option 6D). No clamping (6A, 6B rejected).
- 768 ksps: not offered, not even receive-only (option 1C rejected).
- Microphone: no silent microphone stream just for timing (option 4C rejected).
- Wideband: changed from "not in the first version" to planned for milestone M2 with small
  fixed parameters (option 5C, under 0.2 % SPI), announced in the capability block (0003);
  revisited if HIS-01 or HIS-18 shows it is impossible.
- 10 Mbit/s link: the radio refuses the session; it answers discovery, starts no streams and
  reports the reason in the status register and capability block (0003).

### Still open

These were not answered on 2026-11-19 and remain open:

- A second receiver (HIS-02): if one exists, does DDC1 take the last CH394Q socket from the
  management channel (0008), or does the firmware move to MACRAW mode (HIS-14)?
- Sources: RFC 768, RFC 791 and IEEE 802.3 (per-packet overhead of 66 bytes) are still to be
  added to `sources.md`.
- DDC rate units: **closed 2026-11-19.** DDC Specific bytes 18–19 carry the rate in ksps,
  as two public P2 hosts write it: AetherSDR `P2Protocol.cpp:123–125` [AETHER-SRC] and
  piHPSDR `new_protocol.c:1403–1404` [PIHPSDR]. Appendix C bytes 52–55 are not used (0003).

## Sources

- [P2] openHPSDR Ethernet Protocol V4.4, github.com/TAPR/OpenHPSDR-Firmware,
  `Protocol 2/Documentation/`: "Detailed operation" NOTE2 (DUC I&Q width, rate and pacing) and
  the "Direct Fourier Conversion" paragraph; General packet bytes 13–14, 17–28, 37, 39;
  DDC Specific packet bytes 4–22; DUC Specific packet bytes 14–16 and 50; DDC Audio packet;
  DUC I&Q Data packet (bytes 0–1443); High Priority Status packet (bytes 0–59, update rates);
  Microphone Data packet (bytes 0–131); Wideband Data packet (bytes 0–1027); DDC Packet
  (bytes 0–1443); Discovery reply byte 20; Appendix A (Hermes example); Appendix C bytes 39–82.
- [CH394] WCH CH394 manual 1, V1.2: section 3 (features: 8 sockets, SPI up to 62 MHz,
  32 KB RAM), 5.2 (SPI data frame, control word), 6 and 7.2.12–7.2.13 (16 + 16 KB buffers,
  1 KB blocks), 10.4.1 table 10-4 (SPI timing: fSCK, tSC, tSR, tRI), 12.2.4 (UDP mode: 1472-byte
  limit, 8-byte receive header, no flow control).
- [G7250-MAN] Xiegu G7250 Operation Manual rev 0.1: construction, controller interfaces
  (microphone, speaker), main-unit interfaces, transmit ranges, "RF direct sampling".
- [AETHER] AetherSDR README, read 2026-11-19.
- [RFC 768] User Datagram Protocol; [RFC 791] Internet Protocol; [IEEE 802.3] Ethernet frame,
  preamble and inter-frame gap. Public standards, not yet listed in `sources.md`.
- `analysis/ch394-network-budget.md`, `analysis/p1-vs-p2.md`, `open-questions-his.md`
  (HIS-01, HIS-02, HIS-03, HIS-14, HIS-17, HIS-18).
- Decisions 0001, 0002, 0003 (in preparation), 0005, 0007 and 0008 (reserved), 0013, 0015,
  0016, 0017.
- Start brief, ON8ST, 2026-11-19 (one receiver).
- ON8ST, own hardware inspection, 2026-11-19 (CH394Q).
- ON8ST, decision of 2026-11-19 (acceptance; wideband in M2; 10 Mbit/s session refusal).
