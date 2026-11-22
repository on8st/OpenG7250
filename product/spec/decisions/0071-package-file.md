# 0071 — The package file

- Status: proposed (2026-11-22); answers question Q6 of the g7250ctl session (relayed by the
  openg7250 session, 2026-11-22)
- Date: 2026-11-22
- Builds on: 0008 (U-2, U-4), 0028, 0043 (one package per slot), 0063 (header codes), 0067.

## Context

Table M-4 defines the package header as the payload of `UPDATE_BEGIN`; no file format is
defined. The firmware build already writes files as the 53-byte Table M-4 header followed by
the image, and the test bench reads that. Hosts (g7250ctl, AetherSDR) need one format to
read.

## Decision

1. **Layout.** A package file is the 53 bytes of Table M-4 (bytes 0–52, big-endian as Annex
   M), immediately followed by the image, and nothing else: its length is 53 + the image
   length of bytes 14–17. Signature length (bytes 50–51) is 0 in this version and no
   signature bytes follow (U-4). The SHA-256 in bytes 18–49 is over the image bytes only.
2. **Name.** `g7250-<major>.<minor>.<build>-slot-<a|b>.g7pk` for an image of this firmware,
   version and slot as in bytes 10–13 and 52 (lower-case slot letter). A release ships one
   file per slot (0043). The name is a convention; a host reads the header, not the name.
   **Telling builds apart** (firmware remark, 2026-11-22): every build that is released (handed
   to anyone outside development, the bench and the first radio included) MUST carry a
   version not used by any earlier released build, raising at least the build field (bytes
   12–13), so that name, `IDENTIFY` and the log tell released builds apart. Development builds
   MAY repeat a version; they are told apart by the SHA-256 of the image (header bytes 18–49,
   and the checksum list the build writes), never by the name.
3. **Writer.** The firmware build writes the package files (its packaging tool is the
   firmware's choice). A foreign image (A1.8a) has no package file from the build: the host
   builds its header from the owner-supplied image and SHA-256 at install time; `g7250ctl`
   MAY save it as a package file in the same layout (slot byte 0, version 0).
4. **Host checks before `UPDATE_BEGIN`.** A host MUST refuse a file whose length is not
   53 + bytes 14–17, whose magic or header version is not Table M-4's, whose signature length
   is not 0, or whose image does not hash to bytes 18–49, before it sends anything. The radio
   still checks everything itself (0063, 0067).
5. **Sending it.** The host sends bytes 0–52 as the `UPDATE_BEGIN` payload after the flags
   byte, and the image as the blocks (block *n* = image bytes 512 *n* … 512 *n* + 511).

## Alternatives considered

- A container with a manifest (several components, signatures): not needed while a package
  carries one image (0028) and no signature (U-4); the header version allows it later.

## Consequences

- Annex M: "Package file" after Table M-4; test T-A1-93 (host checks) and the bench reading.

## Sources

- Decisions 0008, 0028, 0043, 0063, 0067; g7250ctl session question Q6.
