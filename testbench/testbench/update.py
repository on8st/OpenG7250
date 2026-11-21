"""Update client of the test bench: the package header of Annex M Table M-4, a synthetic image,
and the transfer UPDATE_BEGIN, UPDATE_BLOCK, UPDATE_VERIFY, UPDATE_ACTIVATE, UPDATE_ABORT,
UPDATE_STATUS (A1.8; Table M-1). The image is synthetic: the bench never builds or sends a real
firmware image; a radio that accepts it boots it as a trial only on the reference model or an
emulator that models the slots.

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import hashlib
import struct
import zlib

from .mgmt import Mgmt, tlv

PK_MAGIC = 0x4737504B  # "G7PK"
HW_ID = 0x00C8
SECTOR = 128 * 1024
FLAG_FOREIGN, FLAG_DOWNGRADE = 0x01, 0x02


def image(n: int, seed: int = 1) -> bytes:
    """A deterministic synthetic image of n bytes."""
    out, x = bytearray(), seed & 0xFFFFFFFF or 1
    while len(out) < n:
        x = (x * 1103515245 + 12345) & 0xFFFFFFFF
        out += x.to_bytes(4, "big")
    return bytes(out[:n])


def header(img_len: int, sha: bytes, version=(1, 0, 1), slot: int = 0, hw: int = HW_ID, hw_rev: int = 0,
           magic: int = PK_MAGIC, hver: int = 1, component: int = 1) -> bytes:
    """Table M-4, 53 bytes."""
    h = struct.pack(">IBBHH", magic, hver, component, hw, hw_rev)
    h += struct.pack(">BBH", version[0], version[1], version[2])
    h += struct.pack(">I", img_len) + sha[:32].ljust(32, b"\0") + struct.pack(">H", 0) + bytes([slot])
    assert len(h) == 53
    return h


def begin_payload(flags: int, hdr: bytes) -> bytes:
    return bytes([flags]) + hdr


def block_payload(n: int, data: bytes, crc: int | None = None) -> bytes:
    return struct.pack(">II", n, zlib.crc32(data) & 0xFFFFFFFF if crc is None else crc) + data


def layout(t: dict[int, bytes]):
    """IDENTIFY tag 25 (0062): ((first, count) of slot A, of slot B, of the store), or None."""
    v = t.get(25)
    if not v or len(v) != 6:
        return None
    return (v[0], v[1]), (v[2], v[3]), (v[4], v[5])


def version_of(t: dict[int, bytes], tag: int = 4):
    v = t.get(tag, b"")
    return (v[0], v[1], struct.unpack(">H", v[2:4])[0]) if len(v) == 4 else None


class Updater:
    """One update transfer over a management socket that holds the write session."""

    def __init__(self, c: Mgmt, secret: bytes):
        self.c, self.s = c, secret

    def req(self, cmd: str, payload: bytes = b""):
        return self.c.authed(cmd, self.s, payload)

    def identify(self) -> dict[int, bytes]:
        r = self.c.req("IDENTIFY")
        return tlv(r["payload"]) if r and r.get("status") == 0 else {}

    def begin(self, hdr: bytes, flags: int = 0):
        return self.req("UPDATE_BEGIN", begin_payload(flags, hdr))

    def blocks(self, img: bytes, block: int = 512, start: int = 0, bad_crc: int | None = None):
        """Send the blocks from `start`; the status of the first that is not 0, else 0."""
        n = (len(img) + block - 1) // block
        for i in range(start, n):
            data = img[i * block:(i + 1) * block]
            crc = (zlib.crc32(data) ^ 1) & 0xFFFFFFFF if i == bad_crc else None
            r = self.req("UPDATE_BLOCK", block_payload(i, data, crc))
            st = r["status"] if r else None
            if st != 0:
                return st, i
        return 0, n

    def status(self):
        r = self.c.req("UPDATE_STATUS")
        if not r or r.get("status") != 0 or len(r["payload"]) < 10:
            return None
        p = r["payload"]
        return dict(state=p[0], received=struct.unpack(">I", p[1:5])[0], count=struct.unpack(">I", p[5:9])[0], sectors=p[9])

    def full(self, img: bytes, hdr: bytes, flags: int = 0):
        """BEGIN, all blocks, VERIFY; (status, step)."""
        r = self.begin(hdr, flags)
        if not r or r["status"] != 0:
            return (r["status"] if r else None), "UPDATE_BEGIN"
        block = struct.unpack(">H", r["payload"][:2])[0]
        st, _ = self.blocks(img, block)
        if st != 0:
            return st, "UPDATE_BLOCK"
        r = self.req("UPDATE_VERIFY")
        if not r or r["status"] != 0:
            return (r["status"] if r else None), "UPDATE_VERIFY"
        if r["payload"][:32] != hashlib.sha256(img).digest():
            return "hash", "UPDATE_VERIFY"
        return 0, "verified"
