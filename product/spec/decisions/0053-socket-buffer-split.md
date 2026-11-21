# 0053 — Normative socket buffer split (Table B2-2), sizes powers of two

- Status: accepted (ON8ST, 2026-11-21, relayed by the openg7250 session); from a firmware point
  after emulator test T-B10-L13, final split agreed with the firmware (097b08c), 2026-11-21
- Date: 2026-11-21
- Builds on: 0004 ("CH394Q buffer split", indicative), 0008, 0017, 0031, 0036; [CH394] sections
  7.2.12–7.2.13, 12.2.4.

## Context

0004 gave an indicative split. On the emulator a host sent run = 1 and run = 0 back to back on
port 1027; the default 2 KB receive buffer held one 1444-byte packet and, with no flow control
in UDP mode, the stop was lost. Port 1027 also transmits wideband data from M2, part of r1.0
(0036), so its transmit buffer cannot be 0. The CH394's socket pointers are 16-bit and wrap at
65536; how a buffer of a size other than a power of two handles that wrap is not documented.

## Decision

1. Table B2-2 in spec B2.3 is normative, in KB for ports 1024, 1025, 1026, 1027, 1029, 1035,
   7250, 7251: receive 1, 2, 1, 4, 4, 0, 2, 2 (16); transmit 1, 1, 0, 2, 0, 8, 2, 2 (16).
2. Every size is 0 or a power of two.
3. Port 1025 holding one DDC-specific packet is acceptable: hosts send it once per change, and a
   second sent at once may have to be resent. Port 1029 holds two DUC0 packets; the firmware reads
   it at least every 2.5 ms at 192 ksps.
4. Test T-B5-20 (back-to-back High Priority packets).

## Sources

- Firmware via openg7250, 2026-11-21 (097b08c); [CH394]; decisions 0004, 0008, 0017, 0031, 0036.
