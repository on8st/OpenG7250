"""Wait for a masked erase of the image: its end is read from flash.log, not guessed from a host time.
A masked erase of 8 s emulated takes 60 to 100 s of host time and more on a loaded host."""
import os
import time

ERASE_WAIT_S = 600.0        # host time for the erase's end
ENTRY_WAIT_S = 60.0         # host time until the erase must have its flash.log entry
ANSWER_WAIT_S = 60.0        # host time for the image's answer after the erase has ended
SLOW_ERASE_S = 120.0        # longer than this in host time: a line in the result


def erases(entries):
    return [x for x in entries if x.get("op") in ("erase", "bank_erase")]


def wait_erase(send, recv, log, erase_wait=None, entry_wait=None, answer_wait=None, clock=time.time, load=lambda: os.getloadavg()[0]):
    """send() the command; recv(seconds) -> answer or None; log() -> flash.log entries.
    -> dict: ok, why (when not ok), answer, entry, host_s, load_start, load_max, load_end."""
    ew, nw, aw = erase_wait or ERASE_WAIT_S, entry_wait or ENTRY_WAIT_S, answer_wait or ANSWER_WAIT_S
    n0, t0, l0 = len(erases(log())), clock(), load()
    r = {"ok": False, "why": "", "answer": None, "entry": None, "host_s": None, "load_start": l0, "load_max": l0, "load_end": l0}
    send()
    ended = None
    while True:
        a = recv(1.0 if r["answer"] is None else 0.1)
        if a is not None:
            r["answer"] = a
        new = erases(log())[n0:]
        l1 = load()
        r["load_max"], r["load_end"], r["host_s"] = max(r["load_max"], l1), l1, clock() - t0
        if len(new) > 1:
            r["why"] = "%d erases since the command" % len(new)
            return r
        if new:
            r["entry"] = new[0]
            if new[0].get("result") != "running":
                ended = ended or clock()
        if ended is not None and r["answer"] is not None:
            r["ok"] = True
            return r
        if ended is not None and clock() - ended > aw:
            r["why"] = "the erase ended (%s) but the image did not answer within %.0f s" % (r["entry"].get("result"), aw)
            return r
        if not new and clock() - t0 > nw:
            r["why"] = "no erase entry in flash.log within %.0f s" % nw
            return r
        if ended is None and clock() - t0 > ew:
            r["why"] = "the erase did not end within %.0f s of host time" % ew
            return r


def line(kind, r):
    """The line for the result of an erase over SLOW_ERASE_S, or None."""
    if not r["host_s"] or r["host_s"] <= SLOW_ERASE_S:
        return None
    return "erase over %.0f s of host time: %s took %.1f s (1-minute load %.1f at the start, %.1f highest, %.1f at the end)%s" % (
        SLOW_ERASE_S, kind, r["host_s"], r["load_start"], r["load_max"], r["load_end"], "" if r["ok"] else ", " + r["why"])
