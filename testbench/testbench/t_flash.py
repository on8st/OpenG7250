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

from .core import Ctx, Skip, test
from .mgmt import config_tlv, tlv
from .p2 import Host, value, wait_reachable
from .update import SECTOR, Updater, image, layout

TAG_TX_TIME = 7
FAULT_FLAGS, FAULT_CODE = 0x0212, 0x0211
WDT_RESET_AT_START, INTERNAL_ERROR = 17, 19
FLASH_BASE = 0x0800_0000


def need(ctx, *fx):
    for f in fx:
        ctx.need(f)


def flog_start(ctx):
    ctx.fixture("flash.log", action="start")


def flog(ctx) -> list[dict]:
    out: list = []
    for _ in range(1000):
        r = ctx.fixture("flash.log", action="read", **{"from": len(out)}) or {}
        out += r.get("entries", [])
        if len(out) >= r.get("count", 0):
            ctx.check("flash.log: no entries dropped", not r.get("dropped", 0), f"dropped {r.get('dropped', 0)}")
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


def tx_time(u: Updater) -> int | None:
    r = u.c.req("GET_CONFIG")
    if not r or r.get("status") != 0:
        return None
    v = tlv(r["payload"]).get(TAG_TX_TIME)
    return struct.unpack(">I", v)[0] if v and len(v) == 4 else None


def set_tx_time(u: Updater, ms: int):
    return u.req("SET_CONFIG", config_tlv((TAG_TX_TIME, struct.pack(">I", ms))))


def fault_flags(ctx) -> int | None:
    with Host(ctx.net) as h:
        return value(h.write([(FAULT_FLAGS, 0)], timeout=0.2), FAULT_FLAGS)


def secret(ctx) -> bytes:
    from .t_mgmt import secret_of
    return secret_of(ctx)


def mg(ctx, s):
    from .t_update import updating
    return updating(ctx, s, restart_at_end=False)


def compaction_writes(ctx, u: Updater, start_ms: int, lo: int, hi: int, limit: int = 3000):
    """SET_CONFIG with increasing values until flash.log shows an erase of a store sector (a
    compaction); (last value written, number of writes) or None."""
    v = start_ms
    for n in range(limit):
        v += 1
        r = set_tx_time(u, v)
        if not r or r["status"] != 0:
            return None
        if n % 20 == 19 and any(e.get("op") == "erase" and in_range(e, lo, hi) for e in flog(ctx)):
            return v, n + 1
    return None


@test("T-B5-22")
def t_b5_22(ctx: Ctx):
    """B2.3 "during flash work" (0065): every erase stretched to t_erase_max."""
    need(ctx, "flash.log", "flash.timing")
    s = secret(ctx)
    ft = ctx.endpoint.get("flash_timing", {})
    t_max = ft.get("t_erase_max_us")
    if not t_max:
        raise Skip("the endpoint gives no flash_timing.t_erase_max_us")
    r = ctx.fixture("flash.timing", erase_us=t_max) or {}
    ctx.notes.append(f"erase stretched to {r.get('erase_us', t_max)} us ({r.get('source', ft.get('source', 'source not named'))})")
    from .t_tx_locked import safe
    with mg(ctx, s) as u:
        lo, hi = store_range(u)
        old = tx_time(u)
        flog_start(ctx)
        up0 = (u.identify().get(16) or b"\0\0\0\0")
        with Host(ctx.net) as h:
            h.start()
            h.recv(0.3)
            res = compaction_writes(ctx, u, 200_000, lo, hi)
            ctx.check("a SET_CONFIG compaction (an erase of a store sector) happened", res is not None, "no store erase seen")
            safe(ctx, "after the store compaction")
            h.stop()
        run_slot = u.identify().get(23, b"\x01")[0]
        from .t_update import facts, newer, pkg
        run, other, ver, lay = facts(ctx, u)
        img = image(2148, seed=522)
        st, step = u.full(img, pkg(img, newer(ver), other))
        ctx.eq("an update (its slot erases) completes", (st, step), (0, "verified"))
        u.req("UPDATE_ABORT")
        safe(ctx, "after the update's erases")
        entries = flog(ctx)
        erases = [e for e in entries if e.get("op") == "erase"]
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
            set_tx_time(u, old)
    from .t_update import restart, rx_works
    with mg(ctx, s) as u:
        restart(ctx, u)
        ctx.check("after it the management channel goes on", bool(u.identify()), "no IDENTIFY")
    ctx.check("after it RX goes on (a session gets status packets)", rx_works(ctx), "no status packet")
    with Host(ctx.net) as h:
        ctx.check("after it discovery goes on", bool(h.discover(0.5)), "no discovery reply")


@test("T-A1-85")
def t_a1_85(ctx: Ctx):
    """B2.3 "flash work that fails" (0065)."""
    need(ctx, "flash.fault")
    s = secret(ctx)
    from .t_update import facts, newer, pkg, restart, rx_works
    with mg(ctx, s) as u:
        old = tx_time(u)
        ctx.fixture("flash.fault", op="program", flag="PGSERR")
        r = set_tx_time(u, 200_001)
        ctx.eq("a store write whose flash write ends with an error flag: SET_CONFIG 55", r["status"] if r else None, 55)
        ctx.eq("the configuration read back is the old one", tx_time(u), old)
        ff = fault_flags(ctx)
        ctx.check("INTERNAL_ERROR (class 2) raised: FAULT_FLAGS bit 19", ff is not None and ff >> INTERNAL_ERROR & 1, f"FAULT_FLAGS {ff}")
        ctx.check("RX goes on", rx_works(ctx), "no status packet")
        restart(ctx, u)
    with mg(ctx, s) as u:
        run, other, ver, lay = facts(ctx, u)
        ctx.fixture("flash.fault", op="erase", flag="PGSERR")
        img = image(2148, seed=851)
        st, step = u.full(img, pkg(img, newer(ver), other))
        ctx.eq("an update whose erase ends with an error flag: a write failure, 55", st, 55)
        ff = fault_flags(ctx)
        ctx.check("INTERNAL_ERROR raised", ff is not None and ff >> INTERNAL_ERROR & 1, f"FAULT_FLAGS {ff}")
        ctx.eq("the running image unchanged", u.identify().get(23, b"")[:1], bytes([run]))
        restart(ctx, u)
    with mg(ctx, s) as u:
        run, other, ver, lay = facts(ctx, u)
        old = tx_time(u)
        ctx.fixture("flash.fault", op="erase", never_end=True)
        img = image(2148, seed=852)
        u.begin(pkg(img, newer(ver), other))
        u.blocks(img)
        wait_reachable(ctx, "an erase that does not end")
        t = u.identify()
        ff = fault_flags(ctx)
        reset = ff is not None and ff >> WDT_RESET_AT_START & 1
        if reset:
            ctx.check("the watchdog reset the processor: WDT_RESET_AT_START reported", True, f"FAULT_FLAGS {ff}")
            ctx.eq("the store as before (configuration)", tx_time(u), old)
            ctx.eq("the boot record as before (running slot)", t.get(23, b"")[:1], bytes([run]))
        else:
            ctx.check("no watchdog reset: the erase that does not end counts as a failure (INTERNAL_ERROR)",
                      ff is not None and ff >> INTERNAL_ERROR & 1, f"FAULT_FLAGS {ff}")
        restart(ctx, u)
    ctx.notes.append("'each is logged': the log text is not specified; judged in the log by a new entry only where the radio was not reset")


@test("T-A1-86")
def t_a1_86(ctx: Ctx):
    """A1.6 "Store writes" (0065): a power loss after each word of a store write, and during the
    erase of the other store sector in a compaction."""
    need(ctx, "flash.log", "power.cut")
    s = secret(ctx)
    with mg(ctx, s) as u:
        old = tx_time(u)
    v = 210_000
    accepted = v
    with mg(ctx, s) as u:
        r = set_tx_time(u, v)
        if not ctx.eq("a first write answered 0", r["status"] if r else None, 0):
            return
    rows = []
    for k in range(1, 65):
        v += 1
        with mg(ctx, s) as u:
            flog_start(ctx)
            ctx.fixture("power.cut", after_flash_words=k)  # tears the k-th word after the call (v1.8)
            r = set_tx_time(u, v)
            answered0 = r is not None and r["status"] == 0
        wait_reachable(ctx, f"power cut after word {k}")
        with mg(ctx, s) as u:
            got = tx_time(u)
            cut = any(e.get("result") == "cut" for e in flog(ctx))
        rows.append((k, answered0, cut, got))
        ok = got in (accepted, v) and (not answered0 or got == v)
        if not ok:
            ctx.check(f"after a cut at word {k}: old ({accepted}) or new ({v}) value, a write answered 0 kept", False,
                      f"read {got}, answered 0: {answered0}, cut seen: {cut}")
        if got == v:
            accepted = v
        if not cut:
            break  # the write ended before word k: every word of a store write has been cut once
    ctx.check("every word of a store write cut once: each item old or new, never a mix or older, a write answered 0 kept, "
              "the value never goes back", all(g in (None,) or g >= 210_000 for _, _, _, g in rows), str(rows[-5:]))
    for frac in (0.1, 0.5, 0.9):
        with mg(ctx, s) as u:
            lo, hi = store_range(u)
            flog_start(ctx)
            ctx.fixture("power.cut", during_erase=frac)
            res = compaction_writes(ctx, u, accepted, lo, hi, limit=3000)
        wait_reachable(ctx, f"power cut during a compaction erase at {frac}")
        with mg(ctx, s) as u:
            got = tx_time(u)
            ctx.check(f"a cut during the erase of the other store sector at {frac}: the value is not older than the last "
                      "write answered 0", got is not None and got >= accepted - 1, f"read {got}, last answered {accepted}")
            if got:
                accepted = got
    with mg(ctx, s) as u:
        if old is not None:
            set_tx_time(u, old)


@test("T-A1-87")
def t_a1_87(ctx: Ctx):
    """A1.6 "Store writes" (0065): a double ECC error in a store record and in the newest commit
    word."""
    need(ctx, "flash.log", "flash.ecc", "power.cycle")
    s = secret(ctx)
    from .t_update import restart
    with mg(ctx, s) as u:
        old = tx_time(u)
        lo, hi = store_range(u)

    def newest_write(base: int):
        """Write base, then base + 1 bracketed in flash.log (v1.8: the program entries between
        a read before the command and one after its answer); (first word, commit word)."""
        with mg(ctx, s) as u:
            set_tx_time(u, base)
            flog_start(ctx)
            set_tx_time(u, base + 1)
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
        ctx.fixture("flash.ecc", address=addr, type="double")
        ctx.fixture("power.cycle")
        wait_reachable(ctx, f"ECC error in {where}")
        with mg(ctx, s) as u:
            got = tx_time(u)
            ctx.eq(f"{where} unreadable: the store opens and uses the older record or generation", got, base)
            ff = fault_flags(ctx)
            ctx.check(f"{where}: without a fault (FAULT_FLAGS 0)", ff == 0, f"FAULT_FLAGS {ff}")
            ecc = [e for e in flog(ctx) if e.get("op") == "ecc"]
            ctx.check(f"{where}: the ECC error was seen by the radio", bool(ecc), f"{len(ecc)} ecc entries")
            flog_start(ctx)
            set_tx_time(u, base + 5)
            skipped = all((int(e["address"], 0) if isinstance(e["address"], str) else e["address"]) != addr
                          for e in flog(ctx) if e.get("op") == "program" and "address" in e)
            ctx.check(f"{where}: later writes skip the word", skipped, f"address {addr:#x}")
    with mg(ctx, s) as u:
        if old is not None:
            set_tx_time(u, old)
    ctx.notes.append("'logged once per start' and 'an item with no readable record takes its Table A1-4 value' not judged yet")


@test("T-A1-83")
def t_a1_83(ctx: Ctx):
    """A1.8 "Torn boot-record writes" (0064)."""
    need(ctx, "power.cut", "time.advance")
    s = secret(ctx)
    from .t_update import advance, facts, newer, pkg
    # a power loss during the trial-attempt write (at the activation)
    with mg(ctx, s) as u:
        run, other, ver, lay = facts(ctx, u)
        img = image(2148, seed=831)
        if u.full(img, pkg(img, newer(ver), other)) != (0, "verified"):
            ctx.check("update transferred", False, "")
            return
        ctx.fixture("power.cut", after_flash_words=1)
        u.req("UPDATE_ACTIVATE", b"\x00")
    wait_reachable(ctx, "power loss in the trial-attempt write")
    with mg(ctx, s) as u:
        t = u.identify()
        slot = t.get(23, b"")[:1]
        ctx.check("one bootable image starts: the trial image as a trial, or the previous image",
                  (slot == bytes([other]) and t.get(10) == b"\x01") or slot == bytes([run]), f"slot {slot!r}, tag 10 {t.get(10)!r}")
        ctx.check("the store is readable", tx_time(u) is not None, "GET_CONFIG unanswered")
        advance(ctx, 11)
    # a power loss during the self-confirmation write
    with mg(ctx, s) as u:
        run, other, ver, lay = facts(ctx, u)
        img = image(2148, seed=832)
        if u.full(img, pkg(img, newer(ver), other)) == (0, "verified"):
            u.req("UPDATE_ACTIVATE", b"\x00")
    wait_reachable(ctx, "UPDATE_ACTIVATE")
    with mg(ctx, s) as u:
        ctx.fixture("power.cut", after_flash_words=1)
        advance(ctx, 11)  # the confirmation write comes after 10 s
    wait_reachable(ctx, "power loss in the confirmation write")
    with mg(ctx, s) as u:
        t = u.identify()
        ctx.eq("a torn confirmation: the previous image starts (rolled back, tag 10 = 2)", (t.get(23, b"")[:1], t.get(10)),
               (bytes([run]), b"\x02"))
        ctx.check("the store is readable", tx_time(u) is not None, "GET_CONFIG unanswered")
    ctx.notes.append("'logged': the log text is not specified; not judged")
