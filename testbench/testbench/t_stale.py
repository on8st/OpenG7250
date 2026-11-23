"""T-A1-92: the old secret in flash and the start check (A1.10 "The old secret in flash"; 0069 and
its addendum). Emulator: flash.log, and where offered flash.dump, flash.ecc, flash.fill, power.cut.

The bench knows no layout of the store. It looks for the 32 bytes of secrets it set itself, takes
the commit word of a compaction as the last word that compaction programs, and makes the states of
the addendum with the radio's own requests, with power cuts and with flash.fill (foreign data, an
erased sector). Where the radio does not keep the secret as 32 bytes in a row, a search proves
nothing: the bench first asks that it finds the secret in force, and says so when it does not.

flash.fill {address, len, data?}: these bytes of flash become `data` (hex) repeated over
`len`, or erased without `data`; ECC consistent, a 32-byte word of only FF is an erased one; no
flash.log entry. The cases that empty the store put both sectors back as they were at the start of
the test (flash.dump, flash.fill), so the boot record and the configuration are kept.

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import os

from .core import Ctx, PartSkip, RadioClock, test
from .mgmt import Mgmt, identify
from .p2 import wait_reachable
from .t_flash import (LOST_LINE, REPLY_S, SECTOR, compaction_writes, cut_at_word, cut_entry, double_ecc, flog, flog_start, in_range, is_erase,
                      mg, need, plain, planned_cuts_seen, set_plain, set_tx_time, store_range, tx_time, words)
from .t_store import BUILT_IN_TX_MS, address, change_secret, first_secret, has, keep_secret, sectors, where
from .t_update import secret, updating
from .update import Updater

STALE = "store: stale secret erased"
UNREC = "store: non-live sector erased (unrecognised data)"
WORD, PAGE = 32, 32768


def store_erases(entries: list[dict], secs: list[int]) -> list[dict]:
    return [e for e in entries if is_erase(e) and any(in_range(e, x, x + SECTOR) for x in secs)]


def programs(entries: list[dict], secs: list[int]) -> list[dict]:
    return [e for e in entries if e.get("op") == "program" and any(in_range(e, x, x + SECTOR) for x in secs)]


def which(secs: list[int], entries: list[dict]) -> list[int]:
    """The sectors of these entries, in order."""
    return [next(x for x in secs if in_range(e, x, x + SECTOR)) for e in entries]


def commit_word(entries: list[dict], secs: list[int]) -> int | None:
    """The last word a compaction programmed."""
    p = programs(entries, secs)
    return address(p[-1]) + int(p[-1].get("len", WORD)) - WORD if p else None


def program_words(entries: list[dict], secs: list[int]) -> list[int]:
    return [address(e) + k * WORD for e in programs(entries, secs) for k in range(words(e))]


def dump(ctx, sector: int) -> bytes:
    out = b""
    while len(out) < SECTOR:
        out += bytes.fromhex(ctx.fixture("flash.dump", address=sector + len(out), len=min(PAGE, SECTOR - len(out)))["hex"])
    return out


def erased(data: bytes) -> bool:
    return data.count(0xFF) == len(data)


def fill(ctx, sector: int, data: bytes | None = None, at: int = 0, length: int = SECTOR):
    ctx.fixture("flash.fill", address=sector + at, len=length, **({} if data is None else dict(data=data.hex())))


def ident(ctx) -> dict[int, bytes]:
    with Mgmt(ctx.net) as c:
        c.clock = RadioClock(ctx)
        return identify(ctx, c)


def accepts(ctx, s: bytes) -> int | None:
    """The status of a request authenticated with this secret."""
    with Mgmt(ctx.net) as c:
        c.clock = RadioClock(ctx)
        r = Updater(c, s).req("END_WRITE_SESSION")
    return r["status"] if r else None


def run_texts(ctx) -> list[str]:
    """The log texts of the running run."""
    from .t_log import entries, last_start
    log = entries(ctx)
    k = last_start(log)
    return [] if k is None else [e[2] for e in log[k:]]


def socket_open_us(ctx) -> float | None:
    """When the management socket last opened (sock.log), on the clock of flash.log."""
    if not has(ctx, "sock.log"):
        return None
    out: list = []
    while True:
        r = ctx.fixture("sock.log", action="read", **{"from": len(out)}) or {}
        out += r.get("entries", [])
        if len(out) >= r.get("count", 0):
            break
    t = [e["t_us"] for e in out if e.get("action") == "OPEN" and e.get("port") == 7251]
    return t[-1] if t else None


class State:
    """What the cases share: the sectors, the secret in force, the planned cuts not seen."""

    def __init__(self, ctx, original: bytes):
        self.ctx, self.cur, self.missing, self.wiped = ctx, original, [], False
        with mg(ctx, original) as u:
            self.secs = sectors(u)
        self.dump = has(ctx, "flash.dump")
        self.known = self.dump and len(where(ctx, self.secs, original)) >= 1
        self.words = 0  # words of the last compaction

    def other(self, x: int) -> int:
        return next(y for y in self.secs if y != x)

    def use(self, s: bytes):
        self.cur = s
        keep_secret(self.ctx, s)

    def change(self, label: str):
        """One CHANGE_SECRET; (old, new, the live sector, the flash.log entries), or None: not done."""
        ctx, old, new = self.ctx, self.cur, os.urandom(32)
        with mg(ctx, old) as u:
            flog_start(ctx)
            r = change_secret(ctx, u, new)
            if not ctx.eq(f"{label}: CHANGE_SECRET 0", r["status"] if r else None, 0):
                return None
            self.cur = new
            entries = flog(ctx)
            er = store_erases(entries, self.secs)
            ctx.check(f"{label}: one erase, of the sector that becomes live, and the reply within 15 s",
                      len(er) == 1 and u.c.reply_s <= REPLY_S, f"{len(er)} store erases, reply after {u.c.reply_s:.2f} s of radio time")
        if not er:
            return None
        live = which(self.secs, er)[0]
        self.words = sum(words(e) for e in programs(entries, self.secs))
        if self.known:
            ctx.eq(f"{label}: the new secret only in the live sector", where(ctx, self.secs, new), [live])
            ctx.eq(f"{label}: the old secret only in the other, stale sector", where(ctx, self.secs, old), [self.other(live)])
        return old, new, live, entries

    def restart(self, how: str) -> list[dict]:
        """A RESTART, or a power cycle; the flash.log entries from then on."""
        ctx = self.ctx
        flog_start(ctx)
        if how == "RESTART":
            with updating(ctx, self.cur):
                pass
        else:
            ctx.need("power.cycle")
            ctx.fixture("power.cycle")
            wait_reachable(ctx, "a power cycle")
        return flog(ctx)

    def lines(self, label: str, want: tuple, texts: list[str] | None = None, also: tuple = ()):
        """This start logged these of the two lines of the start check, and the other not unless in
        `also` (where both rules apply to the sector, the text does not say which line alone)."""
        texts = run_texts(self.ctx) if texts is None else texts
        got = tuple(x for x in (STALE, UNREC) if any(x in t for t in texts))
        self.ctx.check(f"{label}: the start check's lines in this run: {want or 'none'}", bool(texts) and set(want) <= set(got) <= {*want, *also},
                       f"got {got or 'none'}, the run's last entries {texts[-4:]}")

    def gone(self, label: str, sector: int, *secrets: bytes):
        """The sector is erased and no sector holds these secrets (a dump)."""
        if self.dump:
            self.ctx.check(f"{label}: sector {sector:#x} is fully erased", erased(dump(self.ctx, sector)), "it holds data")
        if self.known:
            for s in secrets:
                self.ctx.eq(f"{label}: no sector holds the secret", where(self.ctx, self.secs, s), [])

    def left(self, label: str, entries: list[dict], sector: int, before: bytes | None, skip: int | None = None):
        """The sector was neither erased nor programmed by that start, and its dump is unchanged
        (`skip`: the word whose ECC the bench broke)."""
        ops = [e for e in store_erases(entries, self.secs) + programs(entries, self.secs) if in_range(e, sector, sector + SECTOR)]
        self.ctx.check(f"{label}: sector {sector:#x} not erased and not programmed", not ops, f"{ops[:2]}")
        if before is not None:
            now = dump(self.ctx, sector)
            diff = [k for k in range(0, SECTOR, WORD) if now[k:k + WORD] != before[k:k + WORD] and sector + k != skip]
            self.ctx.check(f"{label}: the dump of sector {sector:#x} is unchanged", not diff, f"{len(diff)} words differ, first at {diff[:1]}")


@test("T-A1-92", whole=("flash.ecc", "flash.dump", "flash.fill", "power.cut", "sock.log", "power.cycle"))
def t_a1_92(ctx: Ctx):
    """A1.10 "The old secret in flash", B2.3 "Flash work" (0069 and its addendum). Emulator."""
    need(ctx, "flash.log")
    original = secret(ctx)
    st = State(ctx, original)
    if not st.dump:
        ctx.part("what the sectors hold not judged: no flash.dump; judged on flash.log and the log")
    elif not st.known:
        ctx.part("where the secrets are not judged: the secret in force is not found as 32 bytes in a row")
    snapshot = {x: dump(ctx, x) for x in st.secs} if st.dump and has(ctx, "flash.fill") and has(ctx, "power.cycle") else None
    slot = ident(ctx).get(23, b"")[:1]
    if has(ctx, "sock.log"):
        ctx.fixture("sock.log", action="start")
    cases = [stale_at_start, compaction, unrecognised_word, cut_erase, cut_compaction, fall_back, older_generation, stricter_record,
             newer_than_record, lost_sector, foreign, config_only]
    broken = ""
    try:
        for case in cases:
            lost = [f for f in NEEDS.get(case, ()) if not has(ctx, f)]
            if lost:
                ctx.part(f"{case.__name__}: not run, no {', '.join(lost)}")
            elif case in WIPES and (snapshot is None or slot != b"\x01"):
                # the boot record is in the store too: without it the boot stage's choice is not this run's
                ctx.part(f"{case.__name__}: not run, " + ("no way to put the store back (flash.dump, flash.fill, power.cycle)"
                                                        if snapshot is None else f"the radio does not run from slot A (tag 23 {slot!r})"))
            elif broken and case not in (foreign, config_only):  # these two start from a store of their own
                ctx.part(f"{case.__name__}: not run after {broken}")
            else:
                try:
                    if case(ctx, st) is False:  # a request or state the later cases build on did not come
                        broken = case.__name__
                except PartSkip as e:
                    ctx.part(f"{case.__name__}: {e}")
                    if ctx.no_pass:  # an ECC mark not in force may still come: the later cases would meet it
                        broken = case.__name__
    finally:
        put_back(ctx, st, original, snapshot)
    ctx.notes.append("the FACTORY_RESET whose second erase fails is judged in T-A1-39; 'other store writes at a start compact when the "
                     "live sector is full' is not judged (only the firmware knows its store's fill level)")
    planned_cuts_seen(st.missing)


def put_back(ctx, st: State, original: bytes, snapshot: dict | None):
    """The radio with the store, or at least the secret, of the start of the test."""
    if st.wiped and snapshot is not None:
        for x, data in snapshot.items():
            for k in range(0, SECTOR, PAGE):
                fill(ctx, x, data[k:k + PAGE], at=k, length=PAGE)
        ctx.fixture("power.cycle")
        wait_reachable(ctx, "the store put back")
        st.use(original)
        ctx.eq("the store of the start put back: the secret of the start authenticates", accepts(ctx, original), 0)
    elif st.cur != original:
        with mg(ctx, st.cur) as u:
            r = change_secret(ctx, u, original)
            ctx.eq("the secret of the start put back", r["status"] if r else None, 0)


# ---------------- the cases ----------------
def stale_at_start(ctx, st: State):
    """After CHANGE_SECRET the stale sector is erased at the next start, before the socket opens; the
    start after that finds nothing to erase."""
    for how in ("RESTART",) + (("a power cycle",) if has(ctx, "power.cycle") else ()):
        got = st.change(how)
        if got is None:
            return False
        old, new, live, _ = got
        stale = st.other(live)
        er = store_erases(st.restart(how), st.secs)
        ctx.eq(f"{how}: the stale sector erased once, the live one not", which(st.secs, er), [stale])
        op = socket_open_us(ctx)
        if er and op is not None:
            ctx.check(f"{how}: before the management socket opens (sock.log)", er[0].get("t_end_us", 0) <= op,
                      f"erase ended at {er[0].get('t_end_us')} us, socket opened at {op} us")
        elif er:
            ctx.part(f"{how}: 'before the management socket opens' not judged: no sock.log entry")
        st.lines(how, (STALE,))
        st.gone(how, stale, old)
        ctx.eq(f"{how}, a second start: nothing erased (the non-live sector is erased)", which(st.secs, store_erases(st.restart(how), st.secs)), [])
        st.lines(f"{how}, a second start", ())


def compaction(ctx, st: State):
    """A SET_CONFIG that needs a compaction before any restart erases the stale sector then; a
    non-live sector with only the live secret is left at a start."""
    got = st.change("compaction")
    if got is None:
        return False
    old, new, live, _ = got
    with mg(ctx, st.cur) as u:
        lo, hi = store_range(u)
        flog_start(ctx)
        res = compaction_writes(ctx, u, 230_000, lo, hi)
        if not ctx.check("compaction: a SET_CONFIG needed one", res is not None, ctx.__dict__.get("_compaction", "")):
            return None
        worst = ctx.__dict__.get("_reply", (99.0, 9))
        ctx.check("compaction: each reply within 15 s, one erase per request", worst[0] <= REPLY_S and worst[1] <= 1, f"{worst}")
        ctx.eq("compaction: it erased the stale sector", which(st.secs, store_erases(flog(ctx), st.secs))[:1], [st.other(live)])
        if st.known:
            ctx.eq("compaction: neither sector holds the old secret", where(ctx, st.secs, old), [])
    er = store_erases(st.restart("RESTART"), st.secs)
    ctx.eq("a non-live sector holding only the live secret is not erased at start", len(er), 0)
    st.lines("a non-live sector holding only the live secret", ())


def clean_start(ctx, st: State, label: str):
    """A CHANGE_SECRET and a RESTART: the non-live sector is erased; (live, non-live), or None."""
    got = st.change(label)
    if got is None:
        return None
    er = store_erases(st.restart("RESTART"), st.secs)
    if not ctx.eq(f"{label}: the stale sector erased at the start", which(st.secs, er), [st.other(got[2])]):
        return None
    return got[2], st.other(got[2])


def unrecognised_word(ctx, st: State):
    """One word the store cannot place in the non-live sector, else erased: the sector is erased at
    the next start, with the line of the addendum, and the store goes on."""
    for at in (0, SECTOR // 2 + 32, SECTOR - WORD):
        label = f"a foreign word at offset {at:#x} of the non-live sector"
        got = clean_start(ctx, st, label)
        if got is None:
            return False
        live, idle = got
        fill(ctx, idle, os.urandom(WORD), at=at, length=WORD)
        er = store_erases(st.restart("RESTART"), st.secs)
        ctx.eq(f"{label}: that sector erased once at the start, the live one not", which(st.secs, er), [idle])
        st.lines(label, (UNREC,))
        st.gone(label, idle)
        ctx.eq(f"{label}: the secret in force still authenticates", accepts(ctx, st.cur), 0)


CUT_FROM_START = ("the erase cuts inside a record are placed as if the emulator's erase ran from the start of the sector "
                  "(an assumption, used only to choose the fraction): the verdict is what flash.log and the dump show after "
                  "the next start")


def cut_offset(data: bytes, s: bytes, words_in: int) -> int | None:
    """Where an erase that runs from the start of the sector is cut so that a part of the record of
    secret `s` is left: at the word with its first byte (`words_in` 0), or that many words further on.
    None: not in the dump, at offset 0 (nothing erased), or nothing left there."""
    pos = data.rfind(s)
    at = pos // WORD * WORD + words_in * WORD
    return at if pos >= 0 and 0 < at < len(data) and not erased(data[at:at + WORD]) else None


def cut_erase(ctx, st: State):
    """The stale-sector erase of a start, cut by a power loss: completed at the next start. Cuts inside
    the old secret's record (the store fills only the first part of a sector, so that the large
    fractions leave nothing), and large fractions."""
    torn = 0
    for frac in ("at the first word of the old secret", "one word further on", 0.1, 0.5, 0.9):
        label = f"the stale-sector erase cut {frac}" if isinstance(frac, str) else f"the stale-sector erase cut at {frac}"
        if isinstance(frac, str) and not st.known:
            ctx.part(f"{label}: not run, the dump does not show where the old secret lies")
            continue
        got = st.change(label)
        if got is None:
            return False
        old, new, live, _ = got
        stale = st.other(live)
        inside = isinstance(frac, str)
        if inside:
            if CUT_FROM_START not in ctx.notes:
                ctx.notes.append(CUT_FROM_START)
            data = dump(ctx, stale)
            at = cut_offset(data, old, 0 if "first" in frac else 1)
            if at is None:
                ctx.part(f"{label}: not run, no such word in the dump (the secret at offset {data.rfind(old)})")
                continue
            frac = at / SECTOR
        ctx.fixture("power.cut", during_erase=frac)
        entries = st.restart("RESTART")
        e = cut_entry(entries, "erase")
        if e is None or not in_range(e, stale, stale + SECTOR):
            st.missing.append(f"{label} (cut entries {[x for x in entries if x.get('result') == 'cut'][:2]})")
            continue
        done = e.get("fraction_done", frac)
        if abs(done - frac) * SECTOR >= WORD:
            ctx.part(f"{label}: the cut came at {done * SECTOR:.0f} bytes, {frac * SECTOR:.0f} planned")
        er = store_erases(entries[entries.index(e) + 1:], st.secs)
        texts = run_texts(ctx)
        rest = any(x in t for t in texts for x in (STALE, UNREC))
        if not er and not rest:
            # nothing left, or only whole records with no secret but the live one, which a start leaves:
            # the bench cannot tell records apart, so without an erase only the old secret is looked for
            if st.known:
                data = dump(ctx, stale)
                left = any(old[i:i + 8] in data for i in range(len(old) - 7))
                ctx.check(f"{label}, not erased again: no part of the old secret is left (8 bytes in a row)",
                          not left, "a part of the old secret is there")
                if inside and not left:
                    ctx.part(f"{label}: the cut left no part of the old secret, not shown here")
            else:
                ctx.part(f"{label}: not erased again, what the cut left not judged (flash.dump)")
            continue
        ctx.eq(f"{label}: completed at the next start, one erase", which(st.secs, er), [stale])
        # a part of a record is left: the addendum's line; the record still whole: the stale line
        ctx.check(f"{label}: logged, one of the start check's lines", rest, f"{texts[-4:]}")
        torn += any(UNREC in t for t in texts)
        st.gone(label, stale, old)
        ctx.eq(f"{label}: the new secret stays in force", accepts(ctx, new), 0)
    if not torn:
        ctx.part(f"the line '{UNREC}' after a cut erase: not shown, no cut left a part of a record")


def cut_compaction(ctx, st: State):
    """A compaction cut by a power loss leaves part of the new secret's generation in the non-live
    sector: erased at the next start (0069 addendum point 2)."""
    for k in (1, max(2, st.words // 2)):
        label = f"CHANGE_SECRET cut at word {k} of its compaction"
        old, new = st.cur, os.urandom(32)
        with mg(ctx, old) as u:
            flog_start(ctx)
            ctx.fixture("power.cut", after_flash_words=k)
            change_secret(ctx, u, new, window=3.0)  # no reply is expected
        wait_reachable(ctx, label)
        entries = flog(ctx)
        if not cut_at_word(entries, k):
            st.missing.append(f"{label} (cut entries {[x for x in entries if x.get('result') == 'cut'][:2]})")
            return False  # whatever secret is in force now, the planned state is not there
        st.use(old)
        torn = which(st.secs, [cut_entry(entries, "program")])[0]
        er = store_erases(entries[entries.index(cut_entry(entries, "program")) + 1:], st.secs)
        ctx.eq(f"{label}: the torn sector erased once at the next start, the live one not", which(st.secs, er), [torn])
        texts = run_texts(ctx)
        if k == 1:  # nothing but a torn word
            st.lines(label, (UNREC,), texts)
        else:  # a whole record of the new secret may be there as well as the torn word
            ctx.check(f"{label}: logged, one of the start check's lines", any(x in t for t in texts for x in (STALE, UNREC)), f"{texts[-4:]}")
        st.gone(label, torn, new)
        ctx.eq(f"{label}: the old secret is in force, the new one gets 33", (accepts(ctx, old), accepts(ctx, new)), (0, 33))


def fall_back(ctx, st: State):
    """The new generation's commit word unreadable: the older generation is live and kept, the failed
    newer one is erased at that start."""
    got = st.change("fall-back")
    if got is None:
        return False
    old, new, live, entries = got
    word = commit_word(entries, st.secs)
    if not ctx.check("fall-back: the write of the new generation seen in flash.log", word is not None, "no program entry"):
        return None
    double_ecc(ctx, word)
    er = store_erases(st.restart("a power cycle"), st.secs)
    st.use(old)
    kept = st.other(live)
    ctx.eq("fall-back: the sector with the failed newer generation erased once, the older one kept", which(st.secs, er), [live])
    texts = run_texts(ctx)
    st.lines("fall-back", (STALE,), texts, also=(UNREC,))
    ctx.check("fall-back: the fall-back itself logged (text not fixed: another entry on the store)",
              any(("store" in t or "fall" in t) and STALE not in t and UNREC not in t for t in texts), f"{texts[-5:]}")
    ctx.eq("fall-back: the old secret authenticates, the new one gets 33", (accepts(ctx, old), accepts(ctx, new)), (0, 33))
    st.gone("fall-back", live, new)
    if st.known:
        ctx.eq("fall-back: the old secret is in the kept sector", where(ctx, st.secs, old), [kept])


def lost_sector(ctx, st: State):
    """The only committed generation in sector 6, its commit word unreadable, sector 7 erased: no
    start check erases sector 6, although the starts write into sector 7; after a SET_CONFIG the next
    start erases it (the accepted consequence)."""
    six, seven = st.secs[0], st.secs[1]
    label = "lost sector 6"
    got = None
    for _ in range(2):
        got = st.change(label)
        if got is None:
            return False
        if got[2] == six:
            break
    if not ctx.eq(f"{label}: a generation committed in sector 6", got[2], six):
        return False
    word = commit_word(got[3], st.secs)
    er = store_erases(st.restart("RESTART"), st.secs)  # erases the stale sector 7
    if not ctx.eq(f"{label}: sector 7 erased at the start before", which(st.secs, er), [seven]) or word is None:
        return False
    st.wiped = True  # from here the store is not the one of the start
    before = dump(ctx, six) if st.dump else None
    double_ecc(ctx, word)
    for n in ("the first start", "the second start"):
        entries = st.restart("a power cycle")
        st.left(f"{label}, {n}", entries, six, before, skip=word)
        st.lines(f"{label}, {n}", ())
        # the boot counter of B10.2 item 5 goes into sector 7; without it the case shows less
        if not any(in_range(e, seven, seven + SECTOR) for e in programs(entries, st.secs)):
            ctx.part(f"{label}, {n}: no program of sector 7 seen: 'although that start writes its boot counter' not shown")
    t = ident(ctx)
    if (t.get(11), t.get(12, b"")[:1]) != (b"\x00", b"\x01"):
        ctx.part(f"{label}: 'after a SET_CONFIG the next start erases sector 6' not run: no open first-secret window (tags 11, 12: "
                 f"{t.get(11)!r}, {t.get(12)!r})")
        return False
    new = os.urandom(32)
    if not ctx.eq(f"{label}: SET_FIRST_SECRET in the window", first_secret(ctx, new), 0):
        return False
    st.use(new)
    with mg(ctx, new) as u:
        flog_start(ctx)
        r = set_plain(u, 210_000)  # no C-3 item (T-A1-92: a SET_CONFIG with no C-3 item)
        ctx.eq(f"{label}: a SET_CONFIG answered 0", r["status"] if r else None, 0)
        ops = flog(ctx)
        ctx.check(f"{label}: the SET_CONFIG itself erased nothing in sector 6", six not in which(st.secs, store_erases(ops, st.secs)), f"{ops[:2]}")
    er = store_erases(st.restart("RESTART"), st.secs)
    ctx.eq(f"{label}, after a SET_CONFIG: the next start erases sector 6 once and nothing else", which(st.secs, er), [six])
    texts = run_texts(ctx)
    ctx.check(f"{label}, after a SET_CONFIG: logged, one of the start check's lines", any(x in t for t in texts for x in (STALE, UNREC)),
              f"{texts[-4:]}")
    st.gone(f"{label}, after a SET_CONFIG", six, got[1])
    with mg(ctx, new) as u:
        ctx.eq(f"{label}: the SET_CONFIG is kept", plain(u), 210_000)


def foreign(ctx, st: State):
    """Both sectors filled with data that is not this firmware's: the start check erases nothing, the
    first store write works, and the start after it erases the other sector's data."""
    label = "foreign data in both sectors"
    st.wiped = True
    data = {x: os.urandom(4096) for x in st.secs}
    for x in st.secs:
        fill(ctx, x, data[x])
    full = {x: data[x] * (SECTOR // 4096) for x in st.secs}
    entries = st.restart("a power cycle")
    st.lines(label, ())
    # a write of this start (the boot counter) may be the first store write: it erases the sector it
    # writes into; anything else erased would be the start check
    er, wrote = which(st.secs, store_erases(entries, st.secs)), which(st.secs, programs(entries, st.secs))
    ctx.check(f"{label}: nothing erased but a sector that start then writes into", len(er) <= 1 and all(x in wrote for x in er),
              f"erased {[hex(x) for x in er]}, programmed {[hex(x) for x in wrote]}")
    rest = [x for x in st.secs if x not in er]
    for x in rest:
        st.left(label, entries, x, full[x] if st.dump else None)
    t = ident(ctx)
    ctx.eq(f"{label}: Table A1-4 values: no secret, the window open (tags 11, 12)", (t.get(11), t.get(12, b"")[:1]), (b"\x00", b"\x01"))
    with Mgmt(ctx.net) as c:
        ms = tx_time(Updater(c, b""))
    ctx.eq(f"{label}: C-3 takes the built-in values (TX time {BUILT_IN_TX_MS} ms; 0072)", ms, BUILT_IN_TX_MS)
    ctx.check(f"{label}: no '{LOST_LINE}' line", not any(LOST_LINE in x for x in run_texts(ctx)), f"{run_texts(ctx)[-3:]}")
    new = os.urandom(32)
    flog_start(ctx)
    if not ctx.eq(f"{label}: the first store write works (SET_FIRST_SECRET 0)", first_secret(ctx, new), 0):
        return False
    st.use(new)
    er2 = which(st.secs, store_erases(flog(ctx), st.secs))
    ctx.check(f"{label}: that request erased at most the sector it writes into", len(er2) <= 1, f"{[hex(x) for x in er2]}")
    rest = [x for x in rest if x not in er2]
    if len(rest) != 1:
        ctx.part(f"{label}: 'erased once a complete generation is live' not judged: sectors with foreign data left: {len(rest)}")
        return None
    er = which(st.secs, store_erases(st.restart("RESTART"), st.secs))
    ctx.eq(f"{label}, with a first generation that holds the secret: the next start erases the other sector", er, rest)
    st.lines(f"{label}, with a first generation", (UNREC,))
    st.gone(f"{label}, with a first generation", rest[0])
    ctx.eq(f"{label}: the secret is kept", accepts(ctx, new), 0)


def older_generation(ctx, st: State):
    """0072 point I: with G live and committed and the older generation G-1 in the other sector, G-1's
    commit word given a double ECC error: C-3 is G's, TX is not closed, and the start check erases
    G-1's sector as unrecognised data. Each generation is a C-3 write (always a compaction); a
    commit word is the last word its compaction programmed (its place is the firmware's choice)."""
    label = "G-1's commit word unreadable, G live"
    with mg(ctx, st.cur) as u:
        tx0 = tx_time(u)
        if tx0 is None:
            ctx.part(f"{label}: not run, GET_CONFIG gives no TX time")
            return None
        made = []
        for ms in (tx0 - 20, tx0 - 40):  # G-1, then G (tighter)
            flog_start(ctx)
            r = set_tx_time(u, ms)
            entries = flog(ctx)
            if not ctx.eq(f"{label}: the C-3 write of {ms}: 0", r["status"] if r else None, 0):
                return False
            made.append((commit_word(entries, st.secs), which(st.secs, store_erases(entries, st.secs))))
    (w1, er1), (w2, er2) = made
    if w1 is None or w2 is None or len(er1) != 1 or len(er2) != 1 or er1 == er2:
        ctx.part(f"{label}: not run, the two C-3 writes are not two compactions into the two sectors "
                 f"(erases {[hex(x) for x in er1]}, {[hex(x) for x in er2]})")
        return None
    older = st.other(er2[0])  # G was written into the sector erased for it; G-1 is in the other
    double_ecc(ctx, w1)
    er = which(st.secs, store_erases(st.restart("a power cycle"), st.secs))
    with mg(ctx, st.cur) as u:
        ms = tx_time(u)
        ctx.eq(f"{label}: C-3 is G's (TX time {tx0 - 40})", ms, tx0 - 40)
        ctx.check(f"{label}: TX not closed: no '{LOST_LINE}' line", not any(LOST_LINE in x for x in run_texts(ctx)), f"{run_texts(ctx)[-3:]}")
        ctx.eq(f"{label}: the start check erases G-1's sector once, and nothing else", er, [older])
        st.lines(label, (UNREC,))
        set_tx_time(u, tx0)
    ctx.part(f"{label}: the same with G-1's number unreadable in both places: not run, its places are the firmware's choice")


def strict_setup(ctx, st: State, label: str, delta: int = 100):
    """A C-3 write that tightens the TX time by `delta` commits N in the other sector; N's commit word
    (the last word its compaction programmed) is given a double ECC error, so the next start falls back
    with N a failed generation that counts (0072). (the stricter value, N's sector) or None."""
    with mg(ctx, st.cur) as u:
        tx = tx_time(u)
        flog_start(ctx)
        r = set_tx_time(u, tx - delta) if tx is not None else None
        entries = flog(ctx)
    if not ctx.eq(f"{label}: the tightening C-3 write: 0", r["status"] if r else None, 0):
        return None
    w, er = commit_word(entries, st.secs), which(st.secs, store_erases(entries, st.secs))
    if w is None or len(er) != 1:
        ctx.part(f"{label}: not run, the C-3 write is not one compaction (erases {[hex(x) for x in er]})")
        return None
    double_ecc(ctx, w)
    return tx - delta, er[0]


def strict_kept(ctx, st: State, label: str, strict: int, starts: int = 2):
    """This start and the next have the stricter value, and no 'C-3 lost' line."""
    for i in range(starts):
        if i:
            st.restart("a power cycle")
        with mg(ctx, st.cur) as u:
            ms = tx_time(u)
        ctx.eq(f"{label}: {'the start after' if i else 'the next start'}: the stricter TX time {strict}", ms, strict)
        ctx.check(f"{label}: no '{LOST_LINE}' line", not any(LOST_LINE in x for x in run_texts(ctx)), f"{run_texts(ctx)[-3:]}")


def before_erase(entries: list[dict], secs: list[int], sector: int) -> tuple[list[int], int]:
    """The words the start programmed outside `sector` before erasing it (the C-3 state record among
    them), and the number of erases of `sector`."""
    er = [e for e in store_erases(entries, secs) if in_range(e, sector, sector + SECTOR)]
    head = entries[:entries.index(er[0])] if er else entries
    return [w for w in program_words(head, secs) if not sector <= w < sector + SECTOR], len(er)


def stricter_record(ctx, st: State):
    """T-A1-92, 0072 "Keeping the judgment": a fall-back whose failed generation N holds a C-3 record
    stricter than the live one, with a power loss at each point: while the C-3 state record is
    programmed, during the start check's erase, after it before the next compaction, during that
    compaction, after it. Every following start has the stricter value."""
    label = "a fall-back with a stricter C-3 record"
    with mg(ctx, st.cur) as u:
        tx0 = tx_time(u)
    got = strict_setup(ctx, st, label)
    if got is None:
        return None
    strict, n_sec = got
    pre, n_er = before_erase(st.restart("a power cycle"), st.secs, n_sec)
    ctx.eq(f"{label}: the start erases N's sector once", n_er, 1)
    # the boot counter is among them, so this shows only where the cuts go; the verdict is the stricter value
    ctx.check(f"{label}: words programmed into the live sector before that erase (where the state record must be)", bool(pre), "none")
    strict_kept(ctx, st, label, strict)
    points = [(f"cut at word {k} of the start's writes before the erase", dict(after_flash_words=k)) for k in range(1, len(pre) + 1)]
    points += [("cut during the start check's erase", dict(during_erase=0.5)), ("after the erase, before the next compaction", None),
               ("cut during the next compaction", "compaction-cut"), ("after the next compaction", "compaction")]
    for name, how in points:
        lab = f"{label}, {name}"
        got = strict_setup(ctx, st, lab)
        if got is None:
            return False
        strict, n_sec = got
        if isinstance(how, dict):
            ctx.fixture("power.cut", **how)
        entries = st.restart("a power cycle")
        if isinstance(how, dict) and cut_entry(entries, "program" if "after_flash_words" in how else "erase") is None:
            st.missing.append(f"{lab} (cut entries {[x for x in entries if x.get('result') == 'cut'][:2]})")
            continue
        if isinstance(how, str):
            with mg(ctx, st.cur) as u:
                lo, hi = store_range(u)
                flog_start(ctx)
                if how == "compaction-cut":
                    ctx.fixture("power.cut", during_erase=0.5)
                compaction_writes(ctx, u, 0, lo, hi)
            if how == "compaction-cut":
                wait_reachable(ctx, lab)
                if cut_entry(flog(ctx), "erase") is None:
                    st.missing.append(f"{lab} (no cut erase entry)")
                    continue
        strict_kept(ctx, st, lab, strict)
    ctx.part(f"{label}, with the failed generation's C-3 record unreadable (TX closed at every point): not run, the place "
             "of the C-3 record is the firmware's choice")
    with mg(ctx, st.cur) as u:
        if tx0 is not None:
            set_tx_time(u, tx0)


def newer_than_record(ctx, st: State):
    """T-A1-92, 0072 point I: after a fall-back L holds a C-3 state record; a tightening SET_CONFIG commits
    N in the other sector; N's commit word given a double ECC error. The next start takes N's stricter
    value, not L's state record, and writes a new state record into L before N's sector is erased; a
    power loss at each point of that keeps the stricter value."""
    label = "a state record in L, then N committed with its commit word unreadable"
    with mg(ctx, st.cur) as u:
        tx0 = tx_time(u)
    for name, how in [("", None), ("cut at word 1 of the start's writes before the erase", dict(after_flash_words=1)),
                      ("cut during the start check's erase", dict(during_erase=0.5))]:
        lab = f"{label}{', ' + name if name else ''}"
        got = strict_setup(ctx, st, f"{lab} (the first fall-back)")
        if got is None:
            return None
        st.restart("a power cycle")  # L now holds a state record
        got = strict_setup(ctx, st, lab)
        if got is None:
            return False
        strict, n_sec = got
        if how:
            ctx.fixture("power.cut", **how)
        entries = st.restart("a power cycle")
        if how and cut_entry(entries, "program" if "after_flash_words" in how else "erase") is None:
            st.missing.append(f"{lab} (cut entries {[x for x in entries if x.get('result') == 'cut'][:2]})")
            continue
        if not how:
            pre, n_er = before_erase(entries, st.secs, n_sec)
            ctx.eq(f"{lab}: N's sector erased once", n_er, 1)
            ctx.check(f"{lab}: words programmed into L before that erase (where the new state record must be)", bool(pre), "none")
        strict_kept(ctx, st, lab, strict)
    with mg(ctx, st.cur) as u:
        if tx0 is not None:
            set_tx_time(u, tx0)


def config_only(ctx, st: State):
    """After a fall-back to a first generation that holds only configuration, the failed newer
    generation with its secret is erased. The first generation is made without a compaction (empty
    store, SET_FIRST_SECRET, SET_CONFIG); its secret is then made unreadable (double ECC errors in
    every word SET_FIRST_SECRET wrote: an item with no readable record, 0065 point 5)."""
    label = "fall-back to a first generation with only configuration"
    st.wiped = True
    for x in st.secs:
        fill(ctx, x)
    st.restart("a power cycle")
    first = os.urandom(32)
    flog_start(ctx)
    if not ctx.eq(f"{label}: SET_FIRST_SECRET in an empty store", first_secret(ctx, first), 0):
        return False
    st.use(first)
    entries = flog(ctx)
    secret_words, erases = program_words(entries, st.secs), store_erases(entries, st.secs)
    with mg(ctx, first) as u:
        flog_start(ctx)
        r = set_plain(u, 205_000)
        if not ctx.eq(f"{label}: SET_CONFIG 0", r["status"] if r else None, 0):
            return False
        entries = flog(ctx)
    home = {*which(st.secs, programs(entries, st.secs)), *(x for x in st.secs if any(x <= w < x + SECTOR for w in secret_words))}
    if len(home) != 1 or store_erases(entries, st.secs) or [x for x in which(st.secs, erases) if x not in home] or not secret_words:
        ctx.part(f"{label}: not run, the writes did not stay in one first generation (sectors {[hex(x) for x in home]})")
        return None
    got = st.change(label)
    if got is None:
        return False
    old, new, live, entries = got
    word, kept = commit_word(entries, st.secs), st.other(live)
    if word is None or kept not in home:
        ctx.part(f"{label}: not run, the compaction did not leave the first generation in the other sector")
        return None
    for w in [*secret_words, word]:
        double_ecc(ctx, w)
    er = which(st.secs, store_erases(st.restart("a power cycle"), st.secs))
    t = ident(ctx)
    with Mgmt(ctx.net) as c:
        ms = plain(Updater(c, b""))
    if (t.get(11), ms) != (b"\x00", 205_000):
        ctx.part(f"{label}: not judged, the state is not there (tag 11 {t.get(11)!r}, allow-list number {ms}): the live generation must hold the "
                 "configuration and no secret")
        return None
    ctx.eq(f"{label}: the failed newer generation erased once, the first generation kept", er, [live])
    st.lines(label, (STALE,), also=(UNREC,))
    st.gone(label, live, new)


NEEDS = {unrecognised_word: ("flash.fill",), cut_erase: ("power.cut",), cut_compaction: ("power.cut",),
         fall_back: ("flash.ecc", "power.cycle"), older_generation: ("flash.ecc", "power.cycle"),
         stricter_record: ("flash.ecc", "power.cycle", "power.cut"), newer_than_record: ("flash.ecc", "power.cycle", "power.cut"), lost_sector: ("flash.ecc", "power.cycle"),
         foreign: ("flash.fill", "power.cycle"), config_only: ("flash.fill", "flash.ecc", "power.cycle")}
WIPES = (lost_sector, foreign, config_only)
