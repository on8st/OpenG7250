#!/usr/bin/env python3
"""Fixture calls over SLOW_CALL_S are recorded (with a power cycle's parts and slow agent requests), others
not; a call that is not answered yet shows too. No emulator: a fixture server with a stand-in agent."""
import json
import os
import socket
import sys
import threading
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "harness", "g7250emu"))
import benchnet  # noqa: E402

benchnet.SLOW_CALL_S, benchnet.SLOW_ASK_S = 0.4, 0.2
hold = threading.Event()


def emulator(conn):
    for line in conn.makefile("r"):
        time.sleep(0.3 if line.startswith("slow") else 0.0)
        conn.sendall(b"ok\n")


def cycle():
    ag.ask("slow request")
    time.sleep(0.3)
    return {"before_quit_s": 0.0, "quit_s": 0.1, "start_s": 0.5}


class Srv(benchnet.FixtureServer):
    def handle(self, op, args):
        if op == "held":
            hold.wait(10)
            return {"ok": True}
        return super().handle(op, args)


ag = benchnet.Agent(os.environ.get("TMPDIR", "."))
mine = socket.create_connection(("127.0.0.1", ag.port))
threading.Thread(target=emulator, args=(mine,), daemon=True).start()
t0 = time.time()
while ag.conn is None and time.time() - t0 < 5:
    time.sleep(0.01)
srv = Srv(lambda: ag, cycle)


def call(op, wait=5):
    s = socket.create_connection(("127.0.0.1", srv.port), timeout=wait)
    f = s.makefile("rw")
    f.write(json.dumps({"op": op, "args": {}}) + "\n")
    f.flush()
    return json.loads(f.readline())


fails = []


def check(name, ok, detail=""):
    print("  %s %s %s" % ("ok  " if ok else "FAIL", name, "" if ok else detail))
    if not ok:
        fails.append(name)


t1 = time.time()
call("no.such.op")
check("a fast call is not recorded", srv.slow_since(t1) == [], str(srv.slow_calls))
call("power.cycle")
v = srv.slow_since(t1)
check("a slow power cycle is recorded with its parts and the load", len(v) == 1 and v[0]["op"] == "power.cycle" and v[0]["wait_s"] >= 0.5
      and v[0].get("parts", {}).get("start_s") == 0.5 and "load_1min" in v[0], str(v))
check("the slow agent request inside it is named", len(v) == 1 and [a["request"] for a in v[0]["agent_requests_over_1s"]] == ["slow request"], str(v))
t2 = time.time()
threading.Thread(target=call, args=("held", 10), daemon=True).start()
time.sleep(0.8)
w = srv.slow_since(t2)
check("a call not answered yet shows", len(w) == 1 and w[0].get("not_answered") and w[0]["op"] == "held", str(w))
hold.set()
time.sleep(0.3)
w = srv.slow_since(t2)
check("the same call once answered shows once, as answered", len(w) == 1 and not w[0].get("not_answered"), str(w))
check("a call before the test is not in its list", all(x["t_wall"] >= t2 for x in srv.slow_since(t2)))
sys.exit(1 if fails else 0)
