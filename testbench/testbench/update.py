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


# The reference model's image record (model only: 0067 leaves the record's place and layout to
# the firmware, so a synthetic image passes the image check only on the reference model, whose
# record is its own; documented in product/refmodel/src/radio.rs, image_check).
MODEL_LAYOUT = {1: (2, 2), 2: (4, 2)}


def model_image(n: int, slot: int, version, seed: int = 1, hw: int = HW_ID, kind: int = 1, rslot: int | None = None,
                rlen: int | None = None, sp: int = 0x2001FF00, entry: int | None = None, rversion=None) -> bytes:
    """A synthetic image of n bytes with the reference model's image record at offset 0: "G7RM",
    hardware, kind (1 application, 2 boot stage), slot, version, recorded length, stack pointer,
    entry point, CRC-32 over the recorded length with the CRC field taken as 0."""
    first, count = MODEL_LAYOUT[slot]
    base = 0x08000000 + first * SECTOR
    rlen = n if rlen is None else rlen
    rv = version if rversion is None else rversion
    rec = (b"G7RM" + struct.pack(">HBB", hw, kind, slot if rslot is None else rslot) + bytes([rv[0], rv[1]])
           + struct.pack(">HIII", rv[2], rlen, sp, base + 0x201 if entry is None else entry) + b"\0\0\0\0")
    d = bytearray(rec + image(n - len(rec), seed=seed))
    d[24:28] = struct.pack(">I", zlib.crc32(bytes(d[:rlen])))
    return bytes(d)


def header(img_len: int, sha: bytes, version=(1, 0, 1), slot: int = 0, hw: int = HW_ID, hw_rev: int = 0,
           magic: int = PK_MAGIC, hver: int = 1, component: int = 1) -> bytes:
    """Table M-4, 53 bytes."""
    h = struct.pack(">IBBHH", magic, hver, component, hw, hw_rev)
    h += struct.pack(">BBH", version[0], version[1], version[2])
    h += struct.pack(">I", img_len) + sha[:32].ljust(32, b"\0") + struct.pack(">H", 0) + bytes([slot])
    assert len(h) == 53
    return h


def parse_package(data: bytes):
    """A Table M-4 package file as opaque bytes: the 53-byte header, then the image of the
    header's length. Returns (header bytes, image bytes, fields) or raises ValueError. The bench
    reads only the header fields it knows and sends both parts unchanged."""
    if len(data) < 53:
        raise ValueError("shorter than the 53-byte header")
    h = data[:53]
    magic, hver, comp, hw, rev = struct.unpack(">IBBHH", h[:10])
    version = (h[10], h[11], struct.unpack(">H", h[12:14])[0])
    length = struct.unpack(">I", h[14:18])[0]
    if magic != PK_MAGIC or hver != 1:
        raise ValueError(f"not a version-1 G7PK header (magic {magic:#x}, version {hver})")
    if len(data) != 53 + length:  # Annex M "Package file" (0071): the header, the image, nothing else
        raise ValueError(f"file of {len(data)} bytes, 53 + the header's image length is {53 + length}")
    if h[50:52] != b"\0\0":
        raise ValueError(f"signature length {h[50:52].hex()}, not 0")
    img = data[53:]
    if hashlib.sha256(img).digest() != h[18:50]:
        raise ValueError("the image does not hash to header bytes 18-49")
    return h, img, dict(component=comp, hw=hw, hw_rev=rev, version=version, length=length, sha=h[18:50], slot=h[52])


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
        # a host keeps under the reply cap of A1.10 S4 (20 per second per source); every
        # authenticated command is two datagrams (GET_CHALLENGE and the request)
        c.pace = 18

    def req(self, cmd: str, payload: bytes = b"", window: float | None = None):
        """Every update command as a host sends it: retransmitted per A1.4 (a DHCP borrow of up
        to 2 s closes the management socket, A1.11), for 16 s where it may do flash work (0069);
        `window` overrides that (a power cut: no reply is expected)."""
        from .mgmt import authed_retransmit
        return authed_retransmit(self.c, cmd, self.s, payload, window)

    def post(self, cmd: str, payload: bytes = b"") -> bytes | None:
        """Send a signed request once without waiting for its reply; the datagram, for collect()."""
        from .mgmt import signed
        data = signed(self.c, cmd, self.s, payload)
        if data is not None:
            self.c.post(data)
        return data

    def collect(self, data: bytes, what: str = "request"):
        """The reply to a posted request (retransmitted for 16 s as a repeat, A1.4)."""
        from .mgmt import FLASH_WINDOW_S, send_retransmit
        return send_retransmit(self.c, data, FLASH_WINDOW_S, what)

    def said(self, step, st) -> str:
        """What a step answered, with the reason when there was no reply."""
        return f"{step} answered {st}" + (f": {self.c.why}" if st is None and getattr(self.c, "why", "") else "")

    def identify(self) -> dict[int, bytes]:
        """IDENTIFY TLVs; {} when the radio gave no reply to the request and its retransmissions
        (self.no_identify is then True), or a status other than 0."""
        from .mgmt import req_retransmit
        r = req_retransmit(self.c, "IDENTIFY")
        self.no_identify = r is None
        return tlv(r["payload"]) if r and r.get("status") == 0 else {}

    def begin(self, hdr: bytes, flags: int = 0):
        return self.req("UPDATE_BEGIN", begin_payload(flags, hdr))

    def blocks(self, img: bytes, block: int = 512, start: int = 0, bad_crc: int | None = None):
        """Send the blocks from `start`; the status of the first that is not 0, else 0."""
        n = (len(img) + block - 1) // block
        for i in range(start, n):
            data = img[i * block:(i + 1) * block]
            crc = (zlib.crc32(data) ^ 1) & 0xFFFFFFFF if i == bad_crc else None
            for _ in range(4):
                r = self.req("UPDATE_BLOCK", block_payload(i, data, crc))
                st = r["status"] if r else None
                if st != 64:  # 64: room made in the store, nothing taken: again as a new request (0067 addendum)
                    break
                self.room_made = getattr(self, "room_made", 0) + 1
            if st != 0:
                return st, i
        return 0, n

    def status(self):
        from .mgmt import req_retransmit
        r = req_retransmit(self.c, "UPDATE_STATUS")
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
