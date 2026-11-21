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


def test(*ids: str, trace_pins: list | None = None, trace_since: str = "start"):
    """Register a test function for one or more catalogue ids. `trace_pins`: extra pins the
    hw.trace of this test watches; `trace_since`: "start" (judge pins from the trace start
    snapshot on) or "boot" (from the boot event + 20 us on, A3.6; 0045)."""
    def deco(fn):
        fn.trace_pins = trace_pins
        fn.trace_since = trace_since
        for i in ids:
            REGISTRY[i] = fn
        return fn
    return deco


class Skip(Exception):
    pass


class Blocked(Exception):
    pass


# Operations of a fixture group whose name is not the group's own (testlist-format section 4)
FIXTURE_GROUP = {op: "dhcp.server" for op in ("dhcp.start", "dhcp.stop", "dhcp.offer", "dhcp.nak",
                                              "dhcp.expire", "dhcp.log")}
FIXTURE_GROUP["time.advance"] = "time.scale"

NOT_READY_RETRY_S = 60
NOT_READY_POLL_S = 0.2


class RunnerFault(Exception):
    """The runner or its control socket failed (no answer, broken connection): the result is
    "error" with the runner named as the cause, not a verdict on the firmware."""


class Missing(Exception):
    """An expected reply did not come: the test fails (it is a verdict on the endpoint, not a
    bench error). Raised by Ctx.got() and by the helpers that read replies."""


@dataclass
class Ctx:
    endpoint: dict
    params: dict = field(default_factory=dict)
    checks: list = field(default_factory=list)
    notes: list = field(default_factory=list)
    trace_file: str | None = None  # v1.3: the live trace file of this test, from hw.trace start
    trace_pins: list | None = None  # extra pins a test watches (hw.trace start {pins})
    trace_since: str = "start"  # from when pin levels are judged: "start" or "boot" (+20 us)
    trace_start_t_us: int | None = None  # emulated time of the trace start, if the runner gives it

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

    def check(self, name: str, ok, detail: str = "") -> bool:
        ok = bool(ok)
        if not detail:
            detail = "ok" if ok else "condition not met"
        self.checks.append({"name": name, "ok": ok, "detail": detail})
        return ok

    def at_least(self, name: str, n: int, k: int = 1, what: str = "packets") -> bool:
        return self.check(name, n >= k, f"{n} {what}")

    def all_of(self, name: str, items: list, pred) -> bool:
        """Every item satisfies pred; the detail names the first that does not."""
        bad = [i for i in items if not pred(i)]
        return self.check(name, not bad and bool(items),
                          "nothing received" if not items else
                          f"all {len(items)} ok" if not bad else f"{len(bad)} of {len(items)} not, first: {bad[0]!r}"[:300])

    def got(self, value, what: str, observed: str = "no reply"):
        """The reply a test needs; without it the test fails here with `what` and the
        observation (format v1.3 section 5: a missing reply is a fail, not an error)."""
        if value is None or value == [] or value == {}:
            raise Missing(f"{what}: {observed}")
        return value

    def eq(self, name: str, got, want) -> bool:
        return self.check(name, got == want, f"got {fmt(got)}, want {fmt(want)}")

    def need(self, fixture: str):
        """A fixture is offered when `fixtures` lists it by its group name (section 4 table) or
        by the operation name; either form satisfies a need for the other."""
        have = set(self.endpoint.get("fixtures", []))
        group = FIXTURE_GROUP.get(fixture, fixture)
        ops = {op for op, g in FIXTURE_GROUP.items() if g == group}
        if fixture in have or group in have or ops & have:
            return
        raise Skip(f"fixture:{fixture}")

    def fixture(self, op: str, **args):
        """Call a fixture on the control socket (section 4)."""
        self.need(op)
        addr = self.endpoint["control"]["address"]
        host, port = addr.rsplit(":", 1)
        # emulated time can run slower than wall time: allow 30 s per fixture call. During a
        # power cycle the runner answers "emulator not ready" at once (v1.4): retry for 60 s.
        deadline = time.monotonic() + NOT_READY_RETRY_S
        while True:
            try:
                with socket.create_connection((host, int(port)), timeout=30) as s:
                    s.sendall((json.dumps({"op": op, "args": args}) + "\n").encode())
                    line = s.makefile().readline()
                rep = json.loads(line)
            except (OSError, ValueError) as e:  # socket.timeout is an OSError
                raise RunnerFault(f"runner: fixture {op}: {type(e).__name__}: {e}") from None
            if rep.get("ok") or "not ready" not in str(rep.get("error", "")):
                break
            if time.monotonic() >= deadline:
                raise RunnerFault(f"runner: fixture {op}: still {rep.get('error')!r} after {NOT_READY_RETRY_S} s")
            time.sleep(NOT_READY_POLL_S)
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
    res: dict = {"id": tid}
    if fn is None:
        res.update(status="skip", reason="not implemented in the test bench yet")
        return res
    ctx.trace_pins = getattr(fn, "trace_pins", None)
    ctx.trace_since = getattr(fn, "trace_since", "start")
    try:
        if "hw.trace" in ctx.endpoint.get("fixtures", []):
            # safe-state checks count from the start of each test (format v1.1 section 4)
            try:
                rep = ctx.fixture("hw.trace", action="start", **({"pins": ctx.trace_pins} if ctx.trace_pins else {}))
                ctx.trace_file = (rep or {}).get("file") if isinstance(rep, dict) else None
                ctx.trace_start_t_us = (rep or {}).get("t_us") if isinstance(rep, dict) else None
            except Skip:
                pass
        flash = "flash.log" in ctx.endpoint.get("fixtures", []) and not getattr(fn, "flash_log_own", False)
        if flash:  # v1.8: run-wide flash rules from the start of each test
            try:
                ctx.fixture("flash.log", action="start")
            except Skip:
                flash = False
        fn(ctx)
        if flash:
            from .flash import check_flash
            entries: list = []
            for _ in range(1000):
                r = ctx.fixture("flash.log", action="read", **{"from": len(entries)}) or {}
                entries += r.get("entries", [])
                if len(entries) >= r.get("count", 0):
                    if r.get("dropped"):
                        ctx.check("flash.log: no entries dropped", False, f"dropped {r.get('dropped')}")
                    break
            for name, ok_, detail in check_flash(entries):
                ctx.check(name, ok_, detail)
        ok = all(c["ok"] for c in ctx.checks) and ctx.checks
        res["status"] = "pass" if ok else "fail"
        if not ctx.checks:
            res["reason"] = "no checks recorded"
        else:
            bad = [c for c in ctx.checks if not c["ok"]]
            if bad:  # the first failed check with what was observed
                res["reason"] = f"{bad[0]['name']}: {bad[0]['detail']}" + (f" (+{len(bad) - 1} more)" if len(bad) > 1 else "")
    except RunnerFault as e:
        res.update(status="error", reason=str(e))
    except Missing as e:
        ctx.checks.append({"name": str(e).split(":")[0], "ok": False, "detail": str(e)})
        res.update(status="fail", reason=str(e))
    except Skip as e:
        res.update(status="skip", reason=str(e))
    except Blocked as e:
        res.update(status="blocked", reason=str(e))
    except Exception as e:  # noqa: BLE001 - a broken test is an error result
        res.update(status="error", reason=f"{type(e).__name__}: {e}")
    if ctx.trace_file:
        try:  # v1.3: dropped events make the trace checks incomplete
            rep = ctx.fixture("hw.trace", action="stop") or {}
            if rep.get("dropped"):
                ctx.checks.append({"name": "hw.trace: no events dropped", "ok": False, "detail": str(rep)})
                if res.get("status") == "pass":
                    res["status"] = "fail"
        except Skip:
            pass
        except RunnerFault as e:
            ctx.notes.append(f"{e} (at the end of the test)")
            if res.get("status") == "pass":
                res["status"] = "error"
    if ctx.notes:
        res["reason"] = "; ".join(filter(None, [res.get("reason", ""), *ctx.notes]))
    res["duration_s"] = round(time.monotonic() - t0, 3)
    res["wall_s"] = res["duration_s"]  # the bench measures wall time; a runner may replace duration_s
    res["checks"] = ctx.checks
    return res
