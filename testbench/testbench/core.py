"""Test framework: registry, checks, endpoint, results (testbench/docs/testlist-format.md).

Black box: a test knows the spec and the endpoint only. GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import json
import socket
import time
from dataclasses import dataclass, field
from typing import Callable

REGISTRY: dict[str, Callable] = {}


def test(*ids: str):
    """Register a test function for one or more catalogue ids."""
    def deco(fn):
        for i in ids:
            REGISTRY[i] = fn
        return fn
    return deco


class Skip(Exception):
    pass


class Blocked(Exception):
    pass


@dataclass
class Ctx:
    endpoint: dict
    params: dict = field(default_factory=dict)
    checks: list = field(default_factory=list)
    notes: list = field(default_factory=list)

    @property
    def net(self):
        from .net import Net
        if "_net" not in self.__dict__:
            self.__dict__["_net"] = Net(self.endpoint)
        return self.__dict__["_net"]

    @property
    def radio_ip(self) -> str:
        return self.endpoint["radio"]["ip"]

    @property
    def host_ip(self) -> str:
        return self.endpoint["hosts"][0]["ip"]

    @property
    def kind(self) -> str:
        return self.endpoint.get("kind", "?")

    def check(self, name: str, ok: bool, detail: str = "") -> bool:
        self.checks.append({"name": name, "ok": bool(ok), "detail": detail})
        return ok

    def eq(self, name: str, got, want) -> bool:
        return self.check(name, got == want, f"got {fmt(got)}, want {fmt(want)}")

    def need(self, fixture: str):
        if fixture not in self.endpoint.get("fixtures", []):
            raise Skip(f"fixture:{fixture}")

    def fixture(self, op: str, **args):
        """Call a fixture on the control socket (section 4)."""
        self.need(op)
        addr = self.endpoint["control"]["address"]
        host, port = addr.rsplit(":", 1)
        with socket.create_connection((host, int(port)), timeout=5) as s:
            s.sendall((json.dumps({"op": op, "args": args}) + "\n").encode())
            line = s.makefile().readline()
        rep = json.loads(line)
        if not rep.get("ok"):
            raise Skip(rep.get("error", f"fixture:{op}"))
        return rep.get("value")


def fmt(v):
    if isinstance(v, int) and v > 255:
        return hex(v)
    return repr(v)


def run_one(tid: str, ctx: Ctx) -> dict:
    fn = REGISTRY.get(tid)
    t0 = time.monotonic()
    if fn is not None and "hw.trace" in ctx.endpoint.get("fixtures", []):
        try:  # safe-state checks count from the start of each test (format v1.1 section 4)
            ctx.fixture("hw.trace", action="start")
        except Skip:
            pass
    res: dict = {"id": tid}
    if fn is None:
        res.update(status="skip", reason="not implemented in the test bench yet")
        return res
    try:
        fn(ctx)
        ok = all(c["ok"] for c in ctx.checks) and ctx.checks
        res["status"] = "pass" if ok else "fail"
        if not ctx.checks:
            res["reason"] = "no checks recorded"
    except Skip as e:
        res.update(status="skip", reason=str(e))
    except Blocked as e:
        res.update(status="blocked", reason=str(e))
    except Exception as e:  # noqa: BLE001 - a broken test is an error result
        res.update(status="error", reason=f"{type(e).__name__}: {e}")
    if ctx.notes:
        res["reason"] = "; ".join(filter(None, [res.get("reason", ""), *ctx.notes]))
    res["duration_s"] = round(time.monotonic() - t0, 3)
    res["wall_s"] = res["duration_s"]  # the bench measures wall time; a runner may replace duration_s
    res["checks"] = ctx.checks
    return res
