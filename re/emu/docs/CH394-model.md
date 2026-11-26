# CH394 model (Main Ethernet controller)

`models/csharp/G7250Ch394.cs` with `platforms/g7250_main_spi6_ch394.repl`
(stock runs, no network) or a per-boot repl with the bridge port (product
runs, `docs/PRODUCT-RUNNER.md`). Replaces the earlier register-file model in
`G7250Slaves.cs`.

Sources: his/40-ethernet.md (SPI6; CS# PG15, RST# PB5, INT# PC13 active low;
register-compatible with the W5500 per the CH394 datasheet) and the public
W5500 datasheet (register map, socket commands, buffer handling). No
firmware-specific behaviour.

## What it models

- SPI frames `[addr hi][addr lo][control = BSB<<3 | RWB<<2 | OM]` plus data
  with auto-increment, framed by CS#; RST# low resets the chip; MR.RST software
  reset.
- Common registers: GAR, SUBR, SHAR (the chip's own MAC, set at
  construction: 02:00:00:00:72:50), SIPR, IR/IMR, SIR/SIMR, RTR, RCR, PHYCFGR
  (link up, 100 Mbit/s full duplex; `SetLinkSpeed` 0/10/100 changes it; no
  traffic while the link is down), VERSIONR (0x04, the
  W5500 value; the CH394 value is a HYPOTHESIS).
- 8 sockets: Sn_MR, Sn_CR (OPEN, CLOSE, SEND, SEND_MAC, RECV), Sn_IR (write 1
  to clear), Sn_SR, Sn_PORT, Sn_DIPR, Sn_DPORT, buffer sizes, Sn_TX_FSR,
  Sn_TX_RD/WR, Sn_RX_RSR, Sn_RX_RD/WR, Sn_IMR; TX and RX buffers with
  wrap-around.
- UDP: SEND sends TX_RD..TX_WR as one datagram to Sn_DIPR:Sn_DPORT; a
  received datagram is written with the 8-byte header (source IP, source port,
  length) and sets Sn_IR.RECV; Sn_MR.BCASTB blocks broadcasts; a full buffer
  drops the datagram.
- The destination address (2026-11-26): the bridge's frames carry the datagram's
  destination IP (frame type 8). The chip takes a unicast only when that address
  is its SIPR, as in UDP mode on the hardware; any other unicast is dropped before
  the sockets, counted, and logged in net.log as `dropped` with reason
  `not our address` (socket: the UDP socket open on that port, else -1).
  Broadcasts (flags bit 0) are not checked against SIPR. Frames of types 2 and 7
  (no destination) and `net.inject` datagrams are still delivered by port alone.
  Before that date the model delivered every datagram by port alone, so a datagram
  for the radio's former address reached the image as if for the current one.
- MACRAW on socket 0 (2026-11-26, format v1.6 eth.log). OPEN with Sn_MR protocol
  0x04 on socket 0 gives Sn_SR 0x42 (SOCK_MACRAW). SEND passes TX_RD..TX_WR as one
  Ethernet frame to eth.log (`from_radio`) and to the bridge (frame type 10); the
  bridge writes it to the capture and to arp.peer, never onto the host network. A
  frame from the bridge (type 11) keeps its place among the datagrams (the same
  arrival pacing), goes into socket 0's RX buffer behind a 2-byte length, sets
  RECV, and is logged `to_radio`; a frame not taken is logged `dropped` with its
  reason (link down, no socket, BCASTB, MFEN, MMB, MIP6B, no buffer, buffer full).
  UDP sockets keep their datagrams; frames reach only MACRAW.
  - **HYPOTHESIS for the CH394** (taken from the W5500 datasheet, as VERSIONR; for
    the gate to check against the CH394 datasheet):
    1. MACRAW exists on socket 0 only;
    2. Sn_SR reads 0x42 when it is open;
    3. the TX buffer holds the frame from the destination MAC on, without FCS, and
       the chip adds no padding;
    4. a received frame is stored behind a 2-byte big-endian length;
    5. the Sn_MR bits in MACRAW: bit 7 MFEN (only the chip's own MAC, broadcast and
       multicast), bit 6 BCASTB (no broadcast), bit 5 MMB (no multicast while MFEN
       is set), bit 4 MIP6B (no IPv6, ethertype 0x86DD).
  - Emulator assumptions of format v1.6, kept as they are: the RX length is the
    frame alone (`G7250_MACRAW_LEN_HEADER=1` counts the 2 header bytes too, as the
    W5500 datasheet does); the chip does no ARP of its own in UDP mode.
- INT# from (SIR & SIMR) and (IR & IMR).
- The network is the runner's bridge (local TCP link). The address the image
  programs (SIPR, SUBR) is reported on it, so the runner learns the radio's
  address from the image itself.

- Socket buffers: Sn_RXBUF_SIZE (0x1E) and Sn_TXBUF_SIZE (0x1F) per socket in
  whole KiB, 0 to 16, not only powers of two. Size 0 means no buffer: received
  datagrams for that socket are dropped, TX_FS reads 0, SEND sends nothing. A
  buffer byte is addressed by (16-bit pointer or SPI address) mod size, the same
  in the SPI access and in the model's own copies. **Emulator assumption:** how
  the CH394 maps a 16-bit pointer onto a buffer whose size is not a power of two
  is not documented (CH394DS1 V1.2 says only that each socket's buffers can be
  configured freely within the 32 KiB). The sum per direction is not checked
  against 16 KiB.

## Not modelled

TCP and MACRAW on sockets 1 to 7 ("not modelled: CH394/Sn_MR protocol ..."), the
chip's own ARP in UDP mode (an emulator assumption of format v1.6) and ICMP
behaviour, timeouts and retransmission, fragmentation, wake-on-LAN, PPPoE,
timing.

## Tests

`firmware-tests/bench/net_test.c` with the runner self-check
(`docs/PRODUCT-RUNNER.md`): link status, UDP socket open, echo through the
bridge from three hosts and as a broadcast. Since step 1g the stock Main's own driver
reads the chip through SPI6 and the BDMA: chip version, MAC, and its UDP socket
opens.
