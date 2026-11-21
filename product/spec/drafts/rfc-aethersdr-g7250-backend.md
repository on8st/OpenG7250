# Draft — RFC issue for AetherSDR: a G7250 backend (openHPSDR Protocol 2)

- Status: **draft, not posted.** Posting is an outward action and needs a separate go from
  ON8ST (AGENTS.md rule 6; annex Q-O1). Approved on 2026-11-20: writing this draft, with
  option B and option C as fallback (annex §4).
- Target: an issue in github.com/aethersdr/AetherSDR, as its governance asks for architecture
  changes (`GOVERNANCE.md:99–142`, cited in annex §4).
- Sources: `spec/annex-aethersdr.md` (§2–§4, §6), `spec/network-interface.md`,
  `spec/part-c-tci-profile.md`, decisions 0002, 0007, 0013, 0016, 0018, 0021, 0026.
- © 2026 ON8ST, CC BY 4.0 (this draft). Code contributed to AetherSDR would be GPL-3.0 (0018).

Everything below the line is the text that would be posted. It names no source other than the
public spec, and it contains nothing from the reverse-engineering side.

---

## RFC: G7250 backend over openHPSDR Protocol 2

### Summary

We are writing open firmware for the main unit of the Xiegu G7250. Its network interface,
G7250-P2, is openHPSDR Protocol 2 (V4.4) with board type 200 (0xC8), plus a small G7250
register map for the radio's own functions and transmit safety. We would like to add a G7250
backend to AetherSDR, with receive and transmit, and we ask how you would prefer it to be
structured.

The specification is public: <link to `product/spec` on the fork, at tag `spec-rc7` or later>.

### What the radio offers

- **Discovery and streams:** standard P2 discovery (board type 200), one DDC at 48–384 ksps,
  TX IQ at 192 ksps (24-bit), wideband data 4096 samples × 20 frames per second. No DDC audio
  stream to the radio (spec B-part; 0004, 0017).
- **G7250 register map** on UDP 7250, using P2 memory-mapped registers: capability record,
  band and filter state, sensors, limits, and the TX arm (0003, 0005, 0006).
- **TX safety:** transmit needs an explicit arm per session (a token handshake); keying only
  on a 0→1 edge; keyed timeout and maximum TX time; one owner, takeover allowed but it unkeys
  and disarms (0007, spec A3).
- **Management channel** on UDP 7251 for identification, logs and firmware update; separate
  from the radio session. A standalone CLI is its reference implementation; a firmware page in
  AetherSDR could follow later on the same protocol (0008, 0026).
- **TCI 2.1 profile** on TCP 50001 for level 2 (on-board DSP) and for TCI clients (part C;
  0014, 0021).

### Proposed structure

1. **A new `g7250` backend family**, with its own discovery, backend, register-map client and
   G7250 High Priority builder. ANAN behaviour does not change.
2. **Optionally, a shared P2 layer.** Discovery, General, DDC-specific rows, DDC I&Q parsing,
   socket and keepalive, and sender-port demultiplexing are board-neutral and already used by
   the ANAN backend. Moving them into a shared layer would let both P2 backends use one copy.
   The HL2 TX chain's output rate would be generalised so both can use it at 192 kHz.

If you prefer not to touch the ANAN code, we would keep the G7250 backend self-contained,
with its own copy of the minimal P2 code and proper attribution.

### Questions for the maintainers

1. Is a G7250 backend welcome in AetherSDR?
2. Shared P2 layer (step 2), or a self-contained G7250 backend?
3. The ANAN backend is RX-only and its TX is a planned separate phase. Should G7250 TX wait
   for, follow, or stay independent of that plan?
4. Arming: we propose an explicit "TX arm" action in the user interface per session, never
   automatic on the first key press (0021 §30). Does that fit your UI conventions?
5. Would you accept a firmware management page (install, return to stock, version and state)
   later, on the protocol the CLI implements?

### Licence

Contributed code is GPL-3.0, matching AetherSDR.

---

## Notes for ON8ST (not part of the post)

- The link to the spec has to point at a public location. Today the spec is on the fork
  (on8st/OpenG7250, branch `spec/draft`). Check that this is the location you want to show.
- The issue appears under your GitHub account and makes the project public in the AetherSDR
  community.
- Suggested timing (0038): after firmware milestone F2, so the RFC can point at
  running firmware; the backend is then written by a separate product-side session.
- Since this draft: transmit is armable only once calibrated (`NOT_CALIBRATED`, 0035, 0036);
  ATU and antenna selection unavailable until measured (0032); no local KEY (0030). Mention in
  the post if asked about TX.
