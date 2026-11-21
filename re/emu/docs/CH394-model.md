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

TCP and MACRAW ("not modelled: CH394/Sn_MR protocol ..."), ARP and ICMP
behaviour, timeouts and retransmission, fragmentation, wake-on-LAN, PPPoE,
timing.

## Tests

`firmware-tests/bench/net_test.c` with the runner self-check
(`docs/PRODUCT-RUNNER.md`): link status, UDP socket open, echo through the
bridge from three hosts and as a broadcast. Since step 1g the stock Main's own driver
reads the chip through SPI6 and the BDMA: chip version, MAC, and its UDP socket
opens.
