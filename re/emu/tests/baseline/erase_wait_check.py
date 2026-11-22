#!/usr/bin/env python3
"""The stand-in's wait for a masked erase (fakebench/testbench/erasewait.py): it ends by flash.log, and an erase
that never ends, has no entry or gets no answer fails. No emulator: a clock and a log made here."""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "fakebench"))
from testbench.erasewait import line, wait_erase  # noqa: E402

fails = []


def check(name, ok, detail=""):
    print("  %s %s %s" % ("ok  " if ok else "FAIL", name, "" if ok else detail))
    if not ok:
        fails.append(name)


class World:
    """Host clock: every receive costs its timeout. ends/answers: host seconds after the command, None: never."""
    def __init__(self, starts, ends, answers, n_before=1, extra=0):
        self.t, self.sent, self.starts, self.ends, self.answers = 0.0, None, starts, ends, answers
        self.before, self.extra = [{"op": "erase", "result": "ok"}] * n_before, extra

    def send(self):
        self.sent = self.t

    def recv(self, s):
        self.t += s
        return b"FC" if self.sent is not None and self.answers is not None and self.t - self.sent >= self.answers else None

    def log(self):
        e = list(self.before) + [{"op": "program", "result": "ok"}]
        if self.sent is not None and self.starts is not None and self.t - self.sent >= self.starts:
            done = self.ends is not None and self.t - self.sent >= self.ends
            e += [{"op": "erase", "result": "ok" if done else "running"}] * (1 + self.extra)
        return e

    def run(self):
        return wait_erase(self.send, self.recv, self.log, clock=lambda: self.t, load=lambda: 5.0)


r = World(0, 80, 80).run()
check("an erase of 80 s: ok, no line", r["ok"] and line("x", r) is None, str(r))
r = World(0, 300, 300).run()
check("an erase of 300 s: ok, and a line with the load", r["ok"] and "300" in str(line("8 s", r)) and "load 5.0" in line("8 s", r), str(r))
r = World(0, None, None).run()
check("an erase that never ends: not ok, after 600 s", not r["ok"] and "did not end" in r["why"] and 600 < r["host_s"] < 605, str(r))
check("its line says why", "did not end" in (line("8 s", r) or ""))
r = World(0, None, 10).run()
check("an answer while the entry still runs is not an end", not r["ok"] and "did not end" in r["why"], str(r))
r = World(None, None, 10).run()
check("no entry in flash.log: not ok, also with an answer", not r["ok"] and "no erase entry" in r["why"], str(r))
r = World(None, None, None).run()
check("no entry and no answer: not ok", not r["ok"] and "no erase entry" in r["why"], str(r))
r = World(0, 10, None).run()
check("ended but not answered: not ok", not r["ok"] and "did not answer" in r["why"], str(r))
r = World(0, 10, 10, extra=1).run()
check("two erases since the command: not ok", not r["ok"] and "2 erases" in r["why"], str(r))
r = World(0, 10, 10, n_before=4).run()
check("erases from before the command are not taken", r["ok"], str(r))
sys.exit(1 if fails else 0)
