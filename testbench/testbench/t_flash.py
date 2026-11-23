"""Flash work and the store (B2.3 "Flash work", A1.6 "Store writes", A1.8 "Torn boot-record
writes"; 0064, 0065): T-B5-22, T-A1-83, T-A1-85, T-A1-86, T-A1-87. Emulator only, with its flash
controller model (format v1.8): flash.log, flash.timing, flash.fault, power.cut, flash.ecc.

The item written is C-3 maximum continuous TX time (Table M-3 tag 7), set to increasing values
at or below its built-in value: a write that tightens or returns towards the built-in value is
allowed (A1.6), and the values order the writes, so "never an older value" can be judged. The
test puts the value back at the end. The store sectors come from IDENTIFY tag 25 (0062).

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import struct
import time

from .core import Ctx, NotShown, PartSkip, RunnerFault, Skip, test
from .mgmt import config_tlv, tlv
from .p2 import Host, value, wait_reachable
from .update import SECTOR, Updater, image, layout

TAG_TX_TIME = 7
FAULT_FLAGS, FAULT_CODE = 0x0212, 0x0211
WDT_RESET_AT_START, INTERNAL_ERROR = 17, 19
FLASH_BASE = 0x0800_0000
REPLY_S = 15.0  # t_flash_reply (0069)
PROGRAM_MAX_US = 580  # one flash word, the datasheet maximum 0069 uses, where the endpoint gives none


def need(ctx, *fx):
    for f in fx:
        ctx.need(f)


ECC_LIMIT = ("limit of the emulator: a double ECC error shows only in the flash flags, the bus fault the chip gives on the "
             "read is not delivered; a pass does not cover 'without a fault or reset'")


def double_ecc(ctx, address: int):
    """A double ECC error at `address`; the result then says what the emulator cannot show. A mark
    the emulator says is not in force is not judged: PartSkip, and no pass for the test."""
    r = ctx.fixture("flash.ecc", _timeout=63, address=address, type="double")  # in force within 3 s, or it says not
    if isinstance(r, dict) and r.get("in_force") is False:
        ctx.no_pass = True
        raise PartSkip(f"not judged: the double ECC error at {address:#x} was not in force (flash.ecc in_force false)")
    if ECC_LIMIT not in ctx.notes:
        ctx.notes.append(ECC_LIMIT)


def flog_start(ctx):
    ctx.fixture("flash.log", action="start")


def flog(ctx, first: int = 0) -> list[dict]:
    """The flash.log entries from number `first` on; dropped entries are a failed check."""
    out: list = []
    for _ in range(1000):
        r = ctx.fixture("flash.log", action="read", **{"from": first + len(out)}) or {}
        out += r.get("entries", [])
        if first + len(out) >= r.get("count", 0):
            if r.get("dropped", 0):
                ctx.check("flash.log: no entries dropped", False, f"dropped {r.get('dropped', 0)}")
            return out
    return out


def store_range(u: Updater):
    lay = layout(u.identify())
    if not lay:
        raise Skip("IDENTIFY has no flash layout (tag 25)")
    first, n = lay[2]
    return FLASH_BASE + first * SECTOR, FLASH_BASE + (first + n) * SECTOR


def in_range(e: dict, lo: int, hi: int) -> bool:
    a = e.get("address")
    a = int(a, 0) if isinstance(a, str) else a
    return a is not None and lo <= a < hi


# The item of an ordinary, appended store write: the TCI allow-list (C-7, tag 11) with one entry
# 10.x.y.z/32 that carries a number. It holds no C-3 item, and a C-3 write always compacts (0072),
# so the TX time (tag 7) no longer serves for store mechanics.
TAG_PLAIN = 11


def plain_entry(v: int) -> bytes:
    return bytes([10]) + (v & 0xFFFFFF).to_bytes(3, "big") + bytes([32])


def plain_raw(u: Updater) -> bytes | None:
    """The allow-list as GET_CONFIG gives it (to put it back), or None."""
    from .mgmt import req_retransmit
    r = req_retransmit(u.c, "GET_CONFIG") if isinstance(u, Updater) else u.c.req("GET_CONFIG")
    if not r or r.get("status") != 0:
        return None
    return tlv(r["payload"]).get(TAG_PLAIN)


def plain(u: Updater) -> int | None:
    """The number of the plain item, or None (not one entry of this form)."""
    v = plain_raw(u)
    return int.from_bytes(v[1:4], "big") if v and len(v) == 5 and v[0] == 10 and v[4] == 32 else None


def set_plain(u: Updater, v: int, window: float | None = None):
    return u.req("SET_CONFIG", config_tlv((TAG_PLAIN, plain_entry(v))), window=window)


def put_plain(u: Updater, raw: bytes | None):
    """Put the allow-list back as it was."""
    if raw is not None:
        u.req("SET_CONFIG", config_tlv((TAG_PLAIN, raw)))


def tx_time(u: Updater) -> int | None:
    from .mgmt import req_retransmit
    r = req_retransmit(u.c, "GET_CONFIG") if isinstance(u, Updater) else u.c.req("GET_CONFIG")
    if not r or r.get("status") != 0:
        return None
    v = tlv(r["payload"]).get(TAG_TX_TIME)
    return struct.unpack(">I", v)[0] if v and len(v) == 4 else None


def set_tx_time(u: Updater, ms: int, window: float | None = None):
    return u.req("SET_CONFIG", config_tlv((TAG_TX_TIME, struct.pack(">I", ms))), window=window)


def fault_flags(ctx) -> int | None:
    with Host(ctx.net) as h:
        for _ in range(5):  # an emulator may run slower than the wall clock
            v = value(h.write([(FAULT_FLAGS, 0)], timeout=0.3), FAULT_FLAGS)
            if v is not None:
                return v
    return None


def secret(ctx) -> bytes:
    from .t_mgmt import secret_of
    return secret_of(ctx)


def mg(ctx, s):
    from .t_update import updating
    return updating(ctx, s, restart_at_end=False)


def compaction_writes(ctx, u: Updater, start_ms: int, lo: int, hi: int, limit: int = 5000, window: float | None = None):
    """SET_CONFIG with increasing values until flash.log shows an erase of a store sector (a
    compaction); (last value written, number of writes) or None, with the reason in
    ctx._compaction. A write that gets no reply, or another status, still looks at flash.log: the
    erase may have run while the bench waited. ctx._reply: the slowest reply (radio time) and the
    most erases seen for one write (flash.log is read after a slow reply and every 20 writes)."""
    v, seen, progs, worst, most = start_ms, 0, 0, 0.0, 0
    clock = getattr(u.c, "clock", None)
    clock = clock if clock is not None and clock.emu else None
    for n in range(limit):
        v += 1
        w0, t0 = time.monotonic(), clock.now() if clock is not None else 0.0
        r = set_plain(u, v, window)
        st = r["status"] if r else None
        wall = time.monotonic() - w0
        worst = max(worst, clock.now() - t0 if clock is not None else wall)
        if st == 0 and wall < 0.4 and n % 20 != 19:
            continue  # answered at once: no sector erase in this one
        new = flog(ctx, seen)
        seen += len(new)
        progs += len([e for e in new if e.get("op") == "program" and in_range(e, lo, hi)])
        er = [e for e in new if e.get("op") == "erase" and in_range(e, lo, hi)]
        most = max(most, len(er))
        ctx.__dict__["_last_ok"] = v if st == 0 else v - 1
        ctx.__dict__["_reply"] = (worst, most)
        ctx.__dict__["_compaction"] = (f"{n + 1} writes, {progs} store programs and {len(er)} store erases in flash.log; the last write "
                                       f"({v}) answered {st}" + (f": {u.c.why}" if st is None and getattr(u.c, "why", "") else ""))
        if er:
            return v, n + 1
        if st != 0:
            return None
    return None


def near_compaction(ctx, u: Updater, lo: int, hi: int, start_ms: int, left: int = 0):
    """SET_CONFIG writes until the sector being written has room for `left` more of them: the one
    after those is then expected to need a compaction (a store that appends; the test that uses
    it asks flash.log for the erase). The last value written, or None with the reason in ctx._near."""
    def one(v):
        n0 = len(flog(ctx))
        r = set_plain(u, v)
        progs = [e for e in flog(ctx, n0) if e.get("op") == "program" and in_range(e, lo, hi)]
        return (r["status"] if r else None), progs
    flog_start(ctx)
    v = start_ms
    for _ in range(4):  # two would do; a compaction may fall in one of the first
        v += 1
        st, progs = one(v)
        if st != 0 or not progs:
            ctx.__dict__["_near"] = f"a SET_CONFIG answered {st}, {len(progs)} store programs in flash.log"
            return None
        size = sum(int(e.get("len", 32)) for e in progs)
        a = progs[-1]["address"]
        end = (int(a, 0) if isinstance(a, str) else a) + int(progs[-1].get("len", 32))
        room = lo + ((end - lo + SECTOR - 1) // SECTOR) * SECTOR - end
        n = room // size - left
        if n <= 0:
            return v
        if size < 4096:  # an appended write, not a compaction
            break
    for k in range(n):
        v += 1
        r = set_plain(u, v)
        if not r or r["status"] != 0:
            ctx.__dict__["_near"] = f"write {k + 1} of {n} answered {r['status'] if r else None}"
            return None
    ctx.__dict__["_near"] = f"{n} writes of {size} bytes, room for {left} more"
    return v


def is_erase(e: dict) -> bool:
    return e.get("op") == "erase"


def running(e: dict) -> bool:
    """An erase that has not ended: flash.log lists it with result "running" and a predicted end."""
    return is_erase(e) and e.get("result") == "running"


@test("T-B5-22")
def t_b5_22(ctx: Ctx):
    """B2.3 "during flash work" (0065, 0069): every erase stretched to t_erase_max and every word
    program at its maximum."""
    if "flash.delay" in ctx.endpoint.get("fixtures", []):  # the reference model: the host rule of 0069
        from .t_store import host_rule_on_model
        return host_rule_on_model(ctx)
    need(ctx, "flash.log", "flash.timing")
    s = secret(ctx)
    ft = dict(ctx.endpoint.get("flash_timing", {}))
    t_max = ft.get("t_erase_max_us")
    if not t_max:
        raise Skip("the endpoint gives no flash_timing.t_erase_max_us")
    p_max = ft.get("t_program_max_us") or PROGRAM_MAX_US
    back = stretch(ctx, t_max, p_max)
    try:
        flash_work(ctx, s, t_max)
    finally:
        back()


def stretch(ctx, erase_us: int, program_us: int | None = None):
    """flash.timing at these values; the function that puts it back (null: the datasheet model, or
    the values the endpoint gave)."""
    ft = dict(ctx.endpoint.get("flash_timing", {}))
    r = ctx.fixture("flash.timing", erase_us=erase_us) or {}
    note = f"erase stretched to {r.get('erase_us', erase_us)} us ({r.get('source', ft.get('source', 'source not named'))})"
    programs = False
    if program_us is not None:
        try:
            ctx.fixture("flash.timing", program_us=program_us)
            programs, note = True, note + f", word program to {program_us} us"
        except Skip as e:
            note += f"; word program not stretched: {e}"
    ctx.notes.append(note)

    def back():
        ctx.fixture("flash.timing", erase_us=ft.get("erase_us"))
        if programs:
            ctx.fixture("flash.timing", program_us=ft.get("program_us"))
    return back


def uptime_sample(ctx, u: Updater):
    """(emulated time before, uptime, emulated time after) one IDENTIFY, in ms; or None."""
    t = lambda: (ctx.fixture("hw.observe") or {}).get("t_us")
    a, up, b = t(), u.identify().get(16, b""), t()
    return (a / 1000, struct.unpack(">I", up)[0], b / 1000) if len(up) == 4 and None not in (a, b) else None


UPTIME_RES_MS = 1.0  # tag 16 counts whole ms
CLOCK_PPM = 50  # "plus the tolerance of the clock it is counted from" (0073): a crystal


def uptime_behind(a, b) -> tuple[float, float]:
    """(least, most) ms by which uptime fell behind emulated time between two uptime_sample()s."""
    up = b[1] - a[1]
    return b[0] - a[2] - up - UPTIME_RES_MS, b[2] - a[0] - up + UPTIME_RES_MS


def judge_uptime(ctx, label: str, a, b, n: int):
    """0073: after n requests with flash work uptime is behind by no more than n ms. Judged on the
    least the two samples leave open, never stricter than the spec."""
    if a is None or b is None:
        ctx.part(f"{label}: uptime over flash work not judged: no hw.observe t_us, or no IDENTIFY")
        return
    least, most = uptime_behind(a, b)
    allowed = n + (b[2] - a[0]) * CLOCK_PPM / 1e6
    ctx.check(f"{label}: uptime behind the emulator's time by no more than {n} ms after {n} requests with flash work (0073)",
              least <= allowed, f"behind by {least:.1f} to {most:.1f} ms over {(b[2] - a[0]) / 1000:.1f} s")


def flash_work(ctx, s: bytes, t_max: int):
    from .t_tx_locked import safe
    from .t_update import facts, newer, pkg, restart, rx_works
    with mg(ctx, s) as u:
        lo, hi = store_range(u)
        old = plain_raw(u)
        flog_start(ctx)
        up0 = (u.identify().get(16) or b"\0\0\0\0")
        obs = "hw.observe" in ctx.endpoint.get("fixtures", [])
        s0 = uptime_sample(ctx, u) if obs else None
        with Host(ctx.net) as h:
            h.start()
            h.recv(0.3)
            res = compaction_writes(ctx, u, 200_000, lo, hi)
            ctx.check("a SET_CONFIG compaction (an erase of a store sector) happened", res is not None,
                      ctx.__dict__.get("_compaction", ""))
            safe(ctx, "after the store compaction")
            h.stop()
        s1 = uptime_sample(ctx, u) if obs else None
        if res is not None:
            judge_uptime(ctx, "the SET_CONFIG writes and their compaction", s0, s1, res[1])
        worst = ctx.__dict__.get("_reply", (0.0, 0))
        ctx.check("each SET_CONFIG: the reply within 15 s of the request, at most one sector erase (0069)",
                  worst[0] <= REPLY_S and worst[1] <= 1, f"slowest reply {worst[0]:.2f} s of radio time, most erases {worst[1]}")
        run, other, ver, lay = facts(ctx, u)
        img = image(2148, seed=522)
        # the update request by request: its reply time and its erases
        u.c.timed, rows, n = True, [], len(flog(ctx))
        from .update import block_payload
        st, step = None, "UPDATE_BEGIN"
        for k in range(-1, (len(img) + 511) // 512):
            step = "UPDATE_BEGIN" if k < 0 else f"UPDATE_BLOCK {k}"
            r = u.begin(pkg(img, newer(ver), other)) if k < 0 else u.req("UPDATE_BLOCK", block_payload(k, img[512 * k:512 * k + 512]))
            st = r["status"] if r else None
            new = flog(ctx, n)
            n += len(new)
            rows.append((step, st, round(getattr(u.c, "reply_s", 0.0), 2), len([e for e in new if is_erase(e)])))
            if st == 64:  # room made in the store (its one erase): the block again as a new request
                r = u.req("UPDATE_BLOCK", block_payload(k, img[512 * k:512 * k + 512]))
                st = r["status"] if r else None
                new = flog(ctx, n)
                n += len(new)
                rows.append((step + " again", st, round(getattr(u.c, "reply_s", 0.0), 2), len([e for e in new if is_erase(e)])))
            if st != 0:
                break
        u.c.timed = False
        judge_uptime(ctx, "the update's requests", s1, uptime_sample(ctx, u) if obs else None, len(rows))
        ctx.check("an update (its slot erases) transferred", st == 0, u.said(step, st))
        ctx.check("each update request: the reply within 15 s, at most one sector erase (0069)",
                  all(t <= REPLY_S and e <= 1 for _, _, t, e in rows), f"{rows}")
        ctx.eq("each block counted once although retransmissions arrive during the work (UPDATE_STATUS received)",
               (u.status() or {}).get("received"), (len(img) + 511) // 512)
        u.req("UPDATE_ABORT")
        safe(ctx, "after the update's erases")
        entries = flog(ctx)
        erases = [e for e in entries if is_erase(e)]
        ctx.check("every erase took the stretched time", bool(erases) and all(
            (e.get("t_end_us", 0) - e.get("t_us", 0)) >= t_max * 0.99 for e in erases), f"{len(erases)} erases")
        from .p2 import need_record
        with Host(ctx.net) as h2:
            gate_open = bool(need_record(ctx, h2)[0x0C] & 1)  # streams bit 0: DDC0 offered, the bus gate open (0042)
        open_ = [e for e in entries if e.get("op") in ("erase", "program") and e.get("fpga_bus") != "idle"]
        if not gate_open:
            ctx.check("bus gate closed: no MCU drive on PE0-PE10 for the whole run, fpga_bus idle at every erase and "
                      "program (B2.3; c602628)", not open_,
                      f"{len(open_)} operations with the bus driven, first {open_[0]}" if open_ else "all idle")
        else:
            ctx.check("bus gate open: no FPGA bus cycle open across an erase or program (STB, CMD low)", not open_,
                      f"{len(open_)} operations with a cycle open, first {open_[0]}" if open_ else "all idle")
            ctx.notes.append("gate open: 'STB and CMD driven low at every erase start until it ends' needs the trace of "
                             "PE8/PE9 across the erase; judged from fpga_bus only")
        ff = fault_flags(ctx)
        ctx.check("no watchdog reset (FAULT_FLAGS bit 17 clear, WDT_RESET_AT_START)", ff is not None and not ff >> WDT_RESET_AT_START & 1,
                  f"FAULT_FLAGS {ff}")
        up1 = u.identify().get(16) or b"\0\0\0\0"
        ctx.check("no reset at all: uptime went on", struct.unpack(">I", up1)[0] > struct.unpack(">I", up0)[0],
                  f"uptime {struct.unpack('>I', up0)[0]} -> {struct.unpack('>I', up1)[0]} ms")
        if old is not None:
            put_plain(u, old)
    with mg(ctx, s) as u:
        restart(ctx, u)
        ctx.check("after it the management channel goes on", bool(u.identify()), f"no IDENTIFY: {u.c.why}")
    ctx.check("after it RX goes on (a session gets status packets)", rx_works(ctx), "no status packet")
    with Host(ctx.net) as h:
        ctx.check("after it discovery goes on", any(h.discover(0.5) for _ in range(4)), "no discovery reply")
    ctx.notes.append("an installer sector is not run (streaming installer, A1.8a: phase 2)")
    if "gpio.force" in ctx.endpoint.get("fixtures", []):
        erase_force_cases(ctx, s, t_max)
    else:
        ctx.notes.append("TX path check across an erase (f00162e) not run: no gpio.force")


def post_until_erase(ctx, u: Updater, hdr: bytes, img: bytes):
    """Send UPDATE_BEGIN, then the blocks, each without waiting for its reply, until flash.log
    shows an erase (a reply comes only after the erase its request caused, 0069, so waiting for it
    would miss the erase); (the erase entry, the request still unanswered or None)."""
    from .update import begin_payload, block_payload
    cmds = [("UPDATE_BEGIN", begin_payload(0, hdr))]
    cmds += [("UPDATE_BLOCK", block_payload(k, img[512 * k:512 * k + 512])) for k in range((len(img) + 511) // 512)]
    for cmd, payload in cmds:
        data = u.post(cmd, payload)
        if data is None:
            return None, None
        w0, reply = time.monotonic(), None
        while reply is None and time.monotonic() - w0 < 60:
            er = [e for e in flog(ctx) if running(e)]
            if er:
                return er[0], data
            got = u.c.listen(data, 0.05)
            reply = got[0] if got else None
        if reply is None:
            reply = u.collect(data, cmd)
        er = [e for e in flog(ctx) if is_erase(e)]
        if er:
            return er[0], None  # already over: the trace check says so
        if not reply or reply["status"] != 0:
            return None, None
    return None, None


def erase_force_cases(ctx, s: bytes, t_max: int):
    """0066 addendum (f00162e): a Table A3-6 pin forced during a stretched erase gives
    TX_PATH_NOT_BLOCKED no later than 20 ms after the erase ends; forced and released within the
    erase (5 ms, under the 10 ms of the check) gives no fault. The erase comes from an update; the
    force is placed once flash.log shows the erase started and checked against its t_us..t_end_us
    by the hw.trace force event of this case; the fault time is the console line naming the pin
    (console.log), on the same clock."""
    from .t_a3 import console_line_t_us, fault16, faults, wait_radio
    from .trace import load
    from .t_update import facts, newer, pkg, restart

    def forces() -> list[dict]:
        return [e for e in load(ctx.trace_file) if e.get("kind") == "force" and e.get("pin") == "PF4"] if ctx.trace_file else []

    for case, kw in (("held", dict(level=0, target="idr")),
                     ("a 5 ms pulse", dict(level=0, target="idr", duration_us=5000))):
        with mg(ctx, s) as u:
            run, other, ver, lay = facts(ctx, u)
            flog_start(ctx)
            before = len(forces())
            img = image(2148, seed=523)
            er, data = post_until_erase(ctx, u, pkg(img, newer(ver), other), img)
            if not ctx.check(f"{case}: an erase started (flash.log)", er is not None, f"no erase in flash.log {u.c.why}"):
                u.req("UPDATE_ABORT")
                restart(ctx, u)
                continue
            ctx.fixture("gpio.force", pin="PF4", **kw)
            try:
                if data is not None:
                    u.collect(data, "the request that caused the erase")  # its reply comes after the erase
                wait_radio(ctx, 50)
                er = [e for e in flog(ctx) if is_erase(e)]
                start, end = er[0].get("t_us", 0), er[0].get("t_end_us", 0)
                mine = forces()[before:]
                inside = None
                if mine:
                    inside = ctx.check(f"{case}: the force fell inside the erase (hw.trace force event)",
                                       start <= mine[0]["t_us"] <= end, f"force at {mine[0]['t_us']} us, erase {start}-{end} us")
                else:
                    ctx.notes.append(f"{case}: the force time not checked against the erase: no hw.trace force event")
                code, flags = faults(ctx)
                if case == "held":
                    ctx.check("held: TX_PATH_NOT_BLOCKED raised", fault16(code, flags), f"FAULT_CODE {code}, FAULT_FLAGS {flags}")
                    t = console_line_t_us(ctx, "PF4", after_us=start)
                    if t is None:
                        ctx.notes.append("held: the fault time not judged: no console.log, or no line naming PF4 since the erase began")
                    else:
                        ctx.check("held: no later than 20 ms after the erase ends (console line vs t_end_us, emulated)",
                                  t - end <= 20_000, f"fault line at {t} us, erase end {end} us")
                elif inside is False:
                    ctx.notes.append("a 5 ms pulse: not judged, it fell outside the erase")
                else:
                    ctx.check("a 5 ms force within the erase gives no fault", not fault16(code, flags),
                              f"FAULT_CODE {code}, FAULT_FLAGS {flags}")
            finally:
                ctx.fixture("gpio.force", pin="PF4", level=None, target="idr")
            u.req("UPDATE_ABORT")
            restart(ctx, u)


def stream_reason10(ctx) -> bool:
    """A run request is answered with STREAM_STATUS reason 10 (update in progress)."""
    from .p2 import PORT_REG, STREAM_STATUS, parse_regs
    with Host(ctx.net) as h:
        for _ in range(20):
            h.keepalive()
            for _, d, _ in h.recv(0.2, PORT_REG):
                v = value(parse_regs(d), STREAM_STATUS)
                if v is not None and v & 0x7F == 10:
                    h.running = False
                    return True
        h.running = False
    return False


def after_write_failure(ctx, s: bytes):
    """A1.8 "Write failure" (0074): the erase that fails is the one of the second sector, 60 s after the
    last block answered 0, so that a 55 that restarted the 10 minutes would show."""
    from .t_update import advance, expect_restart_at, facts, newer, pkg, restart, slot_bytes
    from .update import block_payload
    if "time.advance" not in ctx.endpoint.get("fixtures", []):
        ctx.part("the 10 minutes after a write failure not run: no time.advance")
    with mg(ctx, s) as u:
        run, other, ver, lay = facts(ctx, u)
        if slot_bytes(lay, other) < 2 * SECTOR:
            ctx.part("after a write failure: the slot has one sector only, no block of a transfer begins another")
            return
        img = image(SECTOR + 1024, seed=853)
        r = u.begin(pkg(img, newer(ver), other))
        if not ctx.eq("after a write failure: UPDATE_BEGIN accepted", r["status"] if r else None, 0):
            restart(ctx, u)
            return
        st, _ = u.blocks(img[:SECTOR])
        if not ctx.eq("after a write failure: the first sector's blocks", st, 0):
            restart(ctx, u)
            return
        clock = u.c.clock
        t0 = clock.now()
        timed = "time.advance" in ctx.endpoint.get("fixtures", [])
        if timed:
            advance(ctx, 60)
        ctx.fixture("flash.fault", op="erase", flag="PGSERR")
        k = SECTOR // 512
        r = u.req("UPDATE_BLOCK", block_payload(k, img[512 * k:512 * k + 512]))
        st = r["status"] if r else None
        if st == 0:
            ctx.part("after a write failure: the block that begins the second sector was answered 0, its erase came earlier")
            restart(ctx, u)
            return
        ctx.eq("an UPDATE_BLOCK whose erase ends with an error flag: 55", st, 55)
        ctx.eq("after it UPDATE_STATUS reports idle", (u.status() or {}).get("state"), 0)
        ctx.check("after it a run request gets reason 10 (update in progress)", stream_reason10(ctx), "no STREAM_STATUS with reason 10")
        if timed:
            expect_restart_at(ctx, u, "the last block answered 0, then the 55", 600, clock.now() - t0)
        else:
            restart(ctx, u)
    with mg(ctx, s) as u:
        run, other, ver, lay = facts(ctx, u)
        ctx.fixture("flash.fault", op="erase", flag="PGSERR")
        img = image(2148, seed=854)
        st, step = u.full(img, pkg(img, newer(ver), other))
        if ctx.eq("a second write failure: 55", st, 55):
            r = u.begin(pkg(img, newer(ver), other))
            ctx.eq("after the 55 a new UPDATE_BEGIN is accepted", r["status"] if r else None, 0)
        restart(ctx, u)


@test("T-A1-85", whole=True)
def t_a1_85(ctx: Ctx):
    """B2.3 "flash work that fails" (0065)."""
    need(ctx, "flash.fault")
    s = secret(ctx)
    from .t_update import facts, newer, pkg, restart, rx_works
    with mg(ctx, s) as u:
        old = plain_raw(u)
        ctx.fixture("flash.fault", op="program", flag="PGSERR")
        r = set_plain(u, 200_001)
        ctx.eq("a store write whose flash write ends with an error flag: SET_CONFIG 55", r["status"] if r else None, 55)
        ctx.eq("the configuration read back is the old one", plain_raw(u), old)
        ff = fault_flags(ctx)
        ctx.check("INTERNAL_ERROR (class 2) raised: FAULT_FLAGS bit 19", ff is not None and ff >> INTERNAL_ERROR & 1, f"FAULT_FLAGS {ff}")
        ctx.check("RX goes on", rx_works(ctx), "no status packet")
        restart(ctx, u)
    with mg(ctx, s) as u:
        run, other, ver, lay = facts(ctx, u)
        ctx.fixture("flash.fault", op="erase", flag="PGSERR")
        img = image(2148, seed=851)
        st, step = u.full(img, pkg(img, newer(ver), other))
        ctx.eq("an update whose erase ends with an error flag: a write failure, 55 before UPDATE_VERIFY", (st, step != "UPDATE_VERIFY"), (55, True))
        ff = fault_flags(ctx)
        ctx.check("INTERNAL_ERROR raised", ff is not None and ff >> INTERNAL_ERROR & 1, f"FAULT_FLAGS {ff}")
        ctx.eq("the running image unchanged", u.identify().get(23, b"")[:1], bytes([run]))
        restart(ctx, u)
    after_write_failure(ctx, s)
    with mg(ctx, s) as u:
        run, other, ver, lay = facts(ctx, u)
        old = plain_raw(u)
        ctx.fixture("flash.fault", op="erase", never_end=True)
        img = image(2148, seed=852)
        u.c.timed = True
        r = u.begin(pkg(img, newer(ver), other))
        st, step = (r["status"] if r else None), "UPDATE_BEGIN"
        k = 0
        from .update import block_payload
        while st == 0 and 512 * k < len(img):
            step = f"UPDATE_BLOCK {k}"
            r = u.req("UPDATE_BLOCK", block_payload(k, img[512 * k:512 * k + 512]))
            st, k = (r["status"] if r else None), k + 1
        dt, u.c.timed = getattr(u.c, "reply_s", None), False
        wait_reachable(ctx, "an erase that does not end")
        t = u.identify()
        ff = fault_flags(ctx)
        reset = ff is not None and ff >> WDT_RESET_AT_START & 1
        if reset:
            ctx.check("the watchdog reset the processor: WDT_RESET_AT_START reported", True, f"FAULT_FLAGS {ff}")
            ctx.eq("the store as before (configuration)", plain_raw(u), old)
            ctx.eq("the boot record as before (running slot)", t.get(23, b"")[:1], bytes([run]))
        else:
            ctx.eq("no watchdog reset: the update gets 55 as a write failure", st, 55)
            ctx.check("the 55 reply within 15 s of the request (radio time; 0069)", st == 55 and dt is not None and dt <= REPLY_S,
                      f"{u.said(step, st)} after {dt if dt is None else round(dt, 2)} s")
            ctx.check("the erase that does not end counts as a failure (INTERNAL_ERROR)",
                      ff is not None and ff >> INTERNAL_ERROR & 1, f"FAULT_FLAGS {ff}")
        restart(ctx, u)
    ctx.notes.append("'each is logged': the log text is not specified; judged in the log by a new entry only where the radio was not reset")


def words(e: dict) -> int:
    return int(e.get("len", 32) or 32) // 32


def cut_entry(entries: list[dict], op: str) -> dict | None:
    """The entry of a planned power cut: the one operation of this reading that it ended."""
    cuts = [e for e in entries if e.get("result") == "cut"]
    return cuts[0] if len(cuts) == 1 and cuts[0].get("op") == op else None


def cut_at_word(entries: list[dict], k: int) -> bool:
    """A program was cut in the k-th word since the reading began: k - 1 words complete (k when the
    torn word is counted), in earlier entries and in the cut one."""
    e = cut_entry(entries, "program")
    if e is None:
        return False
    before = sum(words(x) for x in entries[:entries.index(e)] if x.get("op") == "program" and x.get("result") == "ok")
    w = e.get("words_written")
    return w is not None and (before + w in (k - 1, k) or w in (k - 1, k))


def planned_cuts_seen(missing: list[str]):
    """A planned cut without its own flash.log entry is an error of the run, never a pass."""
    if missing:
        raise RunnerFault("power.cut: no flash.log entry of the planned cut: " + "; ".join(missing))


@test("T-A1-86", whole=True)
def t_a1_86(ctx: Ctx):
    """A1.6 "Store writes" (0065): a power loss after each word of a store write, and during the
    erase of the other store sector in a compaction. Each cut is judged by its own flash.log entry."""
    need(ctx, "flash.log", "power.cut")
    s = secret(ctx)
    with mg(ctx, s) as u:
        old = plain_raw(u)
    v = 210_000
    accepted = v
    with mg(ctx, s) as u:
        r = set_plain(u, v)
        if not ctx.eq("a first write answered 0", r["status"] if r else None, 0):
            return
    rows, missing, armed, whole = [], [], False, False
    for k in range(1, 65):
        v += 1
        with mg(ctx, s) as u:
            flog_start(ctx)
            ctx.fixture("power.cut", after_flash_words=k)  # tears the k-th word after the call (v1.8)
            r = set_plain(u, v, window=3.0)  # the power is cut: no reply is expected
            answered0 = r is not None and r["status"] == 0
        wait_reachable(ctx, f"power cut after word {k}")
        with mg(ctx, s) as u:
            got = plain(u)
            entries = flog(ctx)
        cut = any(e.get("result") == "cut" for e in entries)
        rows.append((k, answered0, cut, got))
        ok = got in (accepted, v) and (not answered0 or got == v)
        if not ok:
            ctx.check(f"after a cut at word {k}: old ({accepted}) or new ({v}) value, a write answered 0 kept", False,
                      f"read {got}, answered 0: {answered0}, cut seen: {cut}")
        if got in (accepted, v):
            accepted = got
        if not cut:
            # the write ended before word k: every word of it has been cut once, if k > 1
            n = sum(words(e) for e in entries if e.get("op") == "program")
            whole = k > 1 and n < k and answered0
            if not whole:
                missing.append(f"word {k} ({n} words programmed, answered 0: {answered0})")
            armed = True
            break
        if not cut_at_word(entries, k):
            missing.append(f"word {k} (cut entry {[e for e in entries if e.get('result') == 'cut']})")
    ctx.check("every word of a store write cut once, each by its own flash.log entry", whole and not missing,
              f"{len(rows) - 1} words; missing {missing}")
    ctx.check("each item old or new, never a mix or older, a write answered 0 kept, the value never goes back",
              all(g is not None and g >= 210_000 for _, _, _, g in rows), str(rows[-5:]))
    for n in range(8):  # the cut that did not fire is still armed: let it fire in a write of its own
        if not armed:
            break
        v += 1
        with mg(ctx, s) as u:
            flog_start(ctx)
            set_plain(u, v, window=3.0)
        wait_reachable(ctx, "the cut left armed")
        with mg(ctx, s) as u:
            armed = not any(e.get("result") == "cut" for e in flog(ctx))
            got = plain(u)
            ctx.check("after the cut left armed: old or new value", got in (accepted, v), f"read {got}, {accepted} or {v}")
            accepted = got if got in (accepted, v) else accepted
    if armed:
        ctx.notes.append("the last word cut did not fire in 8 more writes: taken as no longer armed")
    for frac in (0.1, 0.5, 0.9):
        with mg(ctx, s) as u:
            lo, hi = store_range(u)
            flog_start(ctx)
            ctx.fixture("power.cut", during_erase=frac)
            res = compaction_writes(ctx, u, accepted, lo, hi)
            last = ctx.__dict__.get("_last_ok", accepted)
        wait_reachable(ctx, f"power cut during a compaction erase at {frac}")
        with mg(ctx, s) as u:
            got = plain(u)
            e = cut_entry(flog(ctx), "erase")
            done = (e or {}).get("fraction_done")
            if e is None or done is None or abs(done - frac) > 0.02 or not in_range(e, lo, hi):
                missing.append(f"erase at {frac} (entry {e}; {ctx.__dict__.get('_compaction', '')})")
                continue
            ctx.check(f"a cut during the erase of the other store sector at {frac}: the last write answered 0, or the one cut, is read",
                      got in (last, last + 1), f"read {got}, last answered {last}")
            if got:
                accepted = got
    with mg(ctx, s) as u:
        if old is not None:
            put_plain(u, old)
    planned_cuts_seen(missing)


@test("T-A1-87")
def t_a1_87(ctx: Ctx):
    """A1.6 "Store writes" (0065): a double ECC error in a store record and in the newest commit
    word. Not covered on the emulator: ECC_LIMIT."""
    need(ctx, "flash.log", "flash.ecc", "power.cycle")
    s = secret(ctx)
    from .t_update import restart
    with mg(ctx, s) as u:
        old = plain_raw(u)
        lo, hi = store_range(u)

    def newest_write(base: int):
        """Write base, then base + 1 bracketed in flash.log (v1.8: the program entries between
        a read before the command and one after its answer); (first word, commit word)."""
        with mg(ctx, s) as u:
            set_plain(u, base)
            flog_start(ctx)
            set_plain(u, base + 1)
            progs = [e for e in flog(ctx) if e.get("op") == "program" and in_range(e, lo, hi)]
        if not progs:
            return None
        addr = lambda e: int(e["address"], 0) if isinstance(e["address"], str) else e["address"]
        # a program entry covers a run of back-to-back 32-byte words: the commit word is the
        # last word of the last run
        return addr(progs[0]), addr(progs[-1]) + int(progs[-1].get("len", 32)) - 32

    for i, where in enumerate(("a word of the newest record", "the newest commit word")):
        base = 220_000 + 10 * i
        w = newest_write(base)
        if not ctx.check(f"{where}: the store write of {base + 1} seen in flash.log", w is not None, "no program entry"):
            continue
        addr = w[i]
        try:
            double_ecc(ctx, addr)
        except PartSkip as e:  # the mark may still come: nothing after it in this run is judged
            ctx.part(f"{where}: {e}")
            break
        ctx.fixture("power.cycle")
        wait_reachable(ctx, f"ECC error in {where}")
        with mg(ctx, s) as u:
            got = plain(u)
            ctx.eq(f"{where} unreadable: the store opens and uses the older record or generation", got, base)
            ff = fault_flags(ctx)
            ctx.check(f"{where}: FAULT_FLAGS 0 (no bus fault is delivered by the emulator, so not 'without a fault or reset')",
                      ff == 0, f"FAULT_FLAGS {ff}")
            ecc = [e for e in flog(ctx) if e.get("op") == "ecc"]
            ctx.check(f"{where}: the ECC error was seen by the radio", bool(ecc), f"{len(ecc)} ecc entries")
            flog_start(ctx)
            set_plain(u, base + 5)
            skipped = all((int(e["address"], 0) if isinstance(e["address"], str) else e["address"]) != addr
                          for e in flog(ctx) if e.get("op") == "program" and "address" in e)
            ctx.check(f"{where}: later writes skip the word", skipped, f"address {addr:#x}")
    with mg(ctx, s) as u:
        if old is not None:
            put_plain(u, old)
    ctx.notes.append("'logged once per start' and 'an item with no readable record takes its Table A1-4 value' not judged yet")


@test("T-A1-83", whole=True)
def t_a1_83(ctx: Ctx):
    """A1.8 "Torn boot-record writes" (0064). Each cut is judged by its own flash.log entry."""
    need(ctx, "power.cut", "time.advance", "flash.log")
    s = secret(ctx)
    from .t_a3 import boot_t_us
    from .t_update import advance, facts, trial_package
    if not ctx.trace_file:
        raise Skip("needs hw.trace (the boot event tells UPDATE_ACTIVATE's words from the boot stage's)")
    # a whole activation first: how many words it programs before its reset; the boot stage's
    # trial-attempt write starts with the next one
    flog_start(ctx)
    n0 = 0

    def programs() -> list[dict]:
        return [e for e in flog(ctx) if e.get("op") == "program"]
    with mg(ctx, s) as u:
        run, other, ver, lay = facts(ctx, u)
        img, hdr, _ = trial_package(ctx, other, ver, 830)
        st, step = u.full(img, hdr)
        if not ctx.check("a first update transferred", (st, step) == (0, "verified"), u.said(step, st)):
            return
        n0 = sum(words(e) for e in programs())
        r = u.req("UPDATE_ACTIVATE", b"\x00")
        if not ctx.eq("the first update activated", r["status"] if r else None, 0):
            return
    time.sleep(0.3)
    wait_reachable(ctx, "UPDATE_ACTIVATE")
    reset, es = boot_t_us(ctx), programs()
    act = sum(words(e) for e in es if e.get("t_us", 0) < reset) - n0
    boot = [e for e in es if e.get("t_us", 0) >= reset]
    if not boot or act < 0:
        raise NotShown(f"neither cut made: the boot stage's trial-attempt write is not seen in flash.log ({act} words at the "
                       f"activation, {len(boot)} programs since the start at {reset} us)")
    ctx.notes.append(f"the activation programs {act} words before its reset; the boot stage's first is word {act + 1}")
    advance(ctx, 11)
    missing: list[str] = []
    # a power loss during the trial-attempt write
    with mg(ctx, s) as u:
        if not ctx.eq("the first update confirmed", u.identify().get(10), b"\x00"):
            return
        run, other, ver, lay = facts(ctx, u)
        img, hdr, _ = trial_package(ctx, other, ver, 831)
        st, step = u.full(img, hdr)
        if not ctx.check("the second update transferred", (st, step) == (0, "verified"), u.said(step, st)):
            return
        flog_start(ctx)
        ctx.fixture("power.cut", after_flash_words=act + 1)
        u.req("UPDATE_ACTIVATE", b"\x00", window=3.0)
    wait_reachable(ctx, "power loss in the trial-attempt write")
    if not cut_at_word(flog(ctx), act + 1):
        missing.append(f"trial-attempt write, word {act + 1}")
    else:
        with mg(ctx, s) as u:
            t = u.identify()
            slot = t.get(23, b"")[:1]
            ctx.check("one bootable image starts: the trial image as a trial, or the previous image",
                      (slot == bytes([other]) and t.get(10) == b"\x01") or slot == bytes([run]), f"slot {slot!r}, tag 10 {t.get(10)!r}")
            ctx.check("the store is readable", plain_raw(u) is not None, "GET_CONFIG unanswered")
    advance(ctx, 11)
    # a power loss during the self-confirmation write
    with mg(ctx, s) as u:
        run, other, ver, lay = facts(ctx, u)
        img, hdr, _ = trial_package(ctx, other, ver, 832)
        st, step = u.full(img, hdr)
        if not ctx.check("the third update transferred", (st, step) == (0, "verified"), u.said(step, st)):
            return
        r = u.req("UPDATE_ACTIVATE", b"\x00")
        if not ctx.eq("the third update activated", r["status"] if r else None, 0):
            return
    time.sleep(0.3)
    wait_reachable(ctx, "UPDATE_ACTIVATE")
    with mg(ctx, s) as u:
        t = u.identify()
        if t.get(10) != b"\x01":
            planned_cuts_seen(missing)
            raise PartSkip(f"the torn confirmation: the trial was no longer unconfirmed when reached (tag 10 {t.get(10)!r})")
        flog_start(ctx)
        ctx.fixture("power.cut", after_flash_words=1)
        advance(ctx, 11)  # the confirmation write comes after 10 s
    wait_reachable(ctx, "power loss in the confirmation write")
    if not cut_at_word(flog(ctx), 1):
        missing.append("confirmation write, word 1")
    else:
        with mg(ctx, s) as u:
            t = u.identify()
            ctx.eq("a torn confirmation: the previous image starts (rolled back, tag 10 = 2)", (t.get(23, b"")[:1], t.get(10)),
                   (bytes([run]), b"\x02"))
            ctx.check("the store is readable", plain_raw(u) is not None, "GET_CONFIG unanswered")
            from .t_update import rollback_logged
            rollback_logged(ctx, u, run)
    planned_cuts_seen(missing)
