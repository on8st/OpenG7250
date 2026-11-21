# 0059 — The unasked capability record may come in up to three parts

- Status: accepted (ON8ST, 2026-11-21, relayed by the openg7250 session); answers a firmware
  question, 2026-11-21
- Date: 2026-11-21
- Builds on: 0003 §2, 0022 §49, 0051, 0054.

## Decision

The firmware's reading is accepted and written into B4.4:

1. The `CAP_REQUEST` answer (item 1) stays one packet.
2. The unasked record (item 2) may be sent in up to three packets, so that no single send blocks
   the reading of port 1027 (0054): same register destination, ascending address order,
   increasing numbers of the session's port 7250 counter (a push sent between parts takes the
   numbers in between, so gaps are allowed), all within 50 ms of the first,
   together every record register exactly once.
3. The record is constant within a session, so the parts never disagree. A host must not assume
   one packet; between parts it may see other pushes; a host that needs one packet uses
   `CAP_REQUEST`.
4. Test T-B4-19; AetherSDR annex G-02 note.

## Sources

- Firmware via openg7250, 2026-11-21; decisions 0003, 0022, 0051, 0054.
