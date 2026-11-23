"""The installer tests T-A1-54, -55, -58, -61 (t_installer) against a fake radio with the streaming
installer of A1.8a item 3: each passes on a correct one and fails on a broken one. No network.

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
import hashlib
import os
import struct
import unittest
import zlib
from contextlib import contextmanager
from unittest import mock

from testbench import t_installer, t_update
from testbench.core import Ctx
from testbench.p2 import PORT_DDC
from testbench.update import SECTOR

AREA, BLOCK = 7 * SECTOR, 512


class Radio:
    def __init__(self, broken="", gate=True):
        self.broken, self.gate = broken, gate
        self.vendor()

    def vendor(self):
        self.app, self.secret, self.log, self.inst, self.session, self.slot = "own", None, [], None, False, 0
        self.config = {1: b"\x00", 7: struct.pack(">I", 300_000), 11: b""}

    def say(self, text):
        self.log.append((len(self.log), 0, text))

    # ---- management
    def answer(self, cmd, p, s):
        if self.app in ("stock", "nothing"):
            return None
        if cmd == "IDENTIFY":
            tags = {10: b"\x03" if self.inst else b"\x00", 11: b"\x01" if self.secret else b"\x00", 23: bytes([self.slot])}
            return dict(status=0, payload=b"".join(bytes([k, len(v)]) + v for k, v in tags.items()))
        if cmd == "UPDATE_STATUS":
            st = 0 if not self.inst else (4 if self.inst["sectors"] else 3)
            n = self.inst["count"] if self.inst else 0
            return dict(status=0, payload=bytes([st]) + struct.pack(">II", n, n) + bytes([self.inst["sectors"] if self.inst else 0]))
        key = self.inst["key"] if self.inst else self.secret
        if s != key:
            return dict(status=33, payload=b"")
        st, out = self.command(cmd, p)
        return dict(status=st, payload=out)

    def command(self, cmd, p):
        i = self.inst
        if cmd == "UPDATE_BEGIN":
            flags, h = p[0], p[1:]
            hw, n, sha = struct.unpack(">H", h[6:8])[0], struct.unpack(">I", h[14:18])[0], h[18:50]
            if hw != 0x00C8 and self.broken != "hw-unchecked":  # Table A1-3: 51, 56, 63, 61
                return 51, b""
            if self.broken == "61-first" and n > AREA:
                return 61, b""
            if not self.gate:
                return 56, b""
            if n > AREA:
                return 61, b""
            count = (n + BLOCK - 1) // BLOCK
            self.inst = dict(sha=sha, n=n, count=count, data={}, sectors=0, verified=False, key=self.secret)
            self.app = "before-erase"
            if self.broken != "no-session-end":
                self.session = False
            if self.broken != "no-log":
                self.say(f"installer: foreign image by 127.0.0.1, {n} bytes")
            if self.broken == "secret-at-handover":
                self.secret = None
            return 0, struct.pack(">HI", BLOCK, count) + b"\x01"
        if cmd == "UPDATE_BLOCK":
            k, crc, data = struct.unpack(">I", p[:4])[0], struct.unpack(">I", p[4:8])[0], p[8:]
            if zlib.crc32(data) & 0xFFFFFFFF != crc and self.broken != "crc-written":
                return 53, b""
            i["data"][k] = data
            per = SECTOR // BLOCK
            if all(j in i["data"] for j in range((k // per) * per, min(i["count"], (k // per + 1) * per))):
                i["sectors"], self.secret, self.app = max(i["sectors"], k // per + 1), None, "writing"
            return 0, struct.pack(">II", k, len(i["data"]))
        if cmd == "UPDATE_VERIFY":
            img = b"".join(i["data"].get(j, b"") for j in range(i["count"]))
            i["verified"] = len(i["data"]) == i["count"] and hashlib.sha256(img).digest() == i["sha"]
            return (0, i["sha"]) if i["verified"] else (55, b"")
        if cmd == "UPDATE_ACTIVATE":
            if p[:1] != b"\x01":
                return 17, b""
            if not i["verified"]:
                return 55, b""
            self.app, self.inst = "stock", None
            return 0, b"\x00" if self.broken == "not-permanent" else b"\x01"
        if cmd == "UPDATE_ABORT":
            if i and i["sectors"]:
                return 60, b""
            self.inst, self.app = None, "own"
            self.say("restart: installer-abandoned")
            return 0, b"\x00"
        return 0, b""


class U:
    def __init__(self, r, s):
        self.r, self.s, self.c = r, s, r

    def req(self, cmd, payload=b"", window=None):
        return self.r.answer(cmd, payload, self.s)

    def begin(self, hdr, flags=0):
        return self.req("UPDATE_BEGIN", bytes([flags]) + hdr)

    def identify(self):
        r = self.r.answer("IDENTIFY", b"", self.s)
        if not r:
            return {}
        p, out, i = r["payload"], {}, 0
        while i < len(p):
            out[p[i]] = p[i + 2:i + 2 + p[i + 1]]
            i += 2 + p[i + 1]
        return out

    def status(self):
        r = self.r.answer("UPDATE_STATUS", b"", self.s)
        if not r:
            return None
        p = r["payload"]
        return dict(state=p[0], received=struct.unpack(">I", p[1:5])[0], count=struct.unpack(">I", p[5:9])[0], sectors=p[9])


class FakeHost:
    radio = None

    def __init__(self, net):
        self.r = FakeHost.radio

    def __enter__(self):
        return self

    def __exit__(self, *a):
        pass

    def start(self, **k):
        if self.r.app == "own":
            self.r.session = True

    def recv(self, timeout=0.5, from_port=None, until=None):
        return []

    def recv_alive(self, timeout, from_port=None, every=0.1):
        return [(PORT_DDC, b"", 0.0)] if self.r.session else []


class FakeCtx(Ctx):
    def __init__(self, r):
        fx = ["installer.state", "vendor.install", "time.advance"]
        super().__init__(dict(radio=dict(ip="127.0.0.1"), fixtures=fx, model=dict(f1_gate="open" if r.gate else "closed")))
        self.r = r

    @property
    def host_ip(self):
        return "127.0.0.1"

    def fixture(self, op, /, _timeout=30, **a):
        self.need(op)
        r = self.r
        if op == "installer.state":
            written = sum(len(d) for d in r.inst["data"].values()) if r.inst and r.inst["sectors"] else 0
            return dict(state=r.app, sectors=r.inst["sectors"] if r.inst else 0, bytes_written=written,
                        secret_stored=r.secret is not None)
        if op == "vendor.install":
            r.vendor()
        return None


def run(test_fn, broken="", gate=True):
    r = Radio(broken, gate)
    FakeHost.radio = r
    ctx = FakeCtx(r)

    def secret(ctx_):
        if r.secret is None:
            r.secret = os.urandom(32)
        return r.secret

    @contextmanager
    def updating(ctx_, s, restart_at_end=True):
        yield U(r, s)

    @contextmanager
    def mgmt(net):
        yield r

    def activate_new(ctx_, s, seed, net_down=False):
        if r.broken == "own-update-fails":
            return None
        r.slot = 1
        return 0, 1, (1, 0, 0), (1, 0, 1)
    patches = dict(secret=secret, updating=updating, get_config=lambda c: dict(r.config), wait_reachable=lambda *a, **k: 0.0,
                   last_log_seq=lambda u: len(r.log) - 1, logs_since=lambda u, n0: [e[2] for e in r.log if e[0] > n0],
                   Host=FakeHost, Mgmt=mgmt, req_retransmit=lambda c, cmd, p=b"": r.answer(cmd, p, None), advance=lambda *a, **k: None)
    with mock.patch.multiple(t_installer, **patches), mock.patch.object(t_update, "activate_new", activate_new):
        test_fn(ctx)
    return [c["name"] for c in ctx.checks if not c["ok"]], ctx


class Installer(unittest.TestCase):
    def test_correct_radio(self):
        for fn in (t_installer.t_a1_54, t_installer.t_a1_55, t_installer.t_a1_58, t_installer.t_a1_61, t_installer.foreign_59):
            bad, ctx = run(fn)
            self.assertEqual(bad, [], fn.__name__)
            self.assertGreater(len(ctx.checks), 3, fn.__name__)
        bad, ctx = run(t_installer.t_a1_61, gate=False)
        self.assertEqual(bad, [])
        self.assertTrue(any("56 (before 61)" in c["name"] for c in ctx.checks))

    def test_broken_radios(self):
        for fn, broken, gate, seen in (
                (t_installer.t_a1_55, "hw-unchecked", True, "another hardware id: 51"),
                (t_installer.t_a1_61, "61-first", False, "56 (before 61)"),
                (t_installer.t_a1_54, "secret-at-handover", True, "the secret still in storage"),
                (t_installer.t_a1_54, "no-log", True, "the hand-over logged"),
                (t_installer.t_a1_54, "no-session-end", True, "the session ended"),
                (t_installer.t_a1_55, "not-permanent", True, "permanent (1)"),
                (t_installer.t_a1_55, "crc-written", True, "a block with a bad CRC: 53"),
                (t_installer.t_a1_58, "own-update-fails", True, "this firmware's image"),
                (t_installer.foreign_59, "hw-unchecked", True, "foreign: an image for other hardware: 51"),
                (t_installer.foreign_59, "crc-written", True, "foreign: a block with a wrong CRC-32: 53")):
            bad, _ = run(fn, broken, gate)
            self.assertTrue(any(seen in b for b in bad), (fn.__name__, broken, bad[:4]))


if __name__ == "__main__":
    unittest.main()
