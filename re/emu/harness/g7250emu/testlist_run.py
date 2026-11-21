#!/usr/bin/env python3
"""Product runs against a g7250-testlist v1.2 (docs/PRODUCT-RUNNER.md).

Called by product_run.py for test lists with "format": "g7250-testlist". Per
test (or once per list with "boot": "per-list") it boots the image on the Main
platform with the generic models, the network bridge and the bench agent,
writes endpoint.json, runs the test bench as a subprocess

    python3 -m testbench run --endpoint <endpoint.json> --test <id> --json

(cwd = the test-bench directory, local.config `testbench_dir` or env
G7250_TESTBENCH), and writes a g7250-testresult v1.2 file. Run directory:
.private/product-runs/<run-id>/ with per test console, network.pcapng and
trace-<n>.jsonl. Nothing from the emulator's internals goes into the result
apart from "not modelled: <peripheral>/<register>".
"""
from __future__ import annotations
import datetime
import hashlib
import json
import os
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import benchnet  # noqa: E402

FORMAT_MINOR = 3
BOOT_WALL_TIMEOUT = 180.0     # host seconds for the image to bring up its address


def _sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def testbench_dir(repo):
    d = os.environ.get("G7250_TESTBENCH")
    if not d:
        cfg = os.path.join(repo, "local.config")
        if os.path.isfile(cfg):
            for ln in open(cfg):
                k, _, v = ln.partition("=")
                if k.strip() == "testbench_dir":
                    d = v.split("#")[0].strip()
    return d


class Boot:
    """One emulator process with its bridge, agent and fixture server."""

    def __init__(self, pr, elf, out, boot="standin"):
        self.pr, self.elf, self.out, self.boot = pr, elf, out, boot
        os.makedirs(out, exist_ok=True)
        self.bridge = benchnet.Bridge(os.path.join(out, "network.pcapng"))
        self.agent = benchnet.Agent(out)
        self.fixtures = benchnet.FixtureServer(lambda: self.agent, self.power_cycle)
        self.proc = None
        self.boots = 0
        self.log = os.path.join(out, "renode.log")
        self.flash = None
        self.cycle_lock = threading.Lock()
        self.agent.on_cut = lambda: self.power_cycle(cut=True)
        self.start()

    def start(self):
        self.boots += 1
        console = os.path.join(self.out, "console.txt" if self.boots == 1 else "console-boot%d.txt" % self.boots)
        open(console, "w").close()
        resc = os.path.join(self.out, "run.resc")
        open(resc, "w").write(self.pr.build_product_resc(self.elf, self.out, console, self.log,
                                                         self.bridge.ch_port, self.agent.port, self.boot,
                                                         flash_image=self.flash))
        self.proc = subprocess.Popen([self.pr.bootcore.RENODE, "--disable-xwt", "--console",
                                      "-e", "path add @%s" % self.pr.REPO, "-e", "include @%s" % resc],
                                     stdin=subprocess.PIPE, stdout=subprocess.DEVNULL,
                                     stderr=subprocess.DEVNULL, text=True)

    def stop_emulator(self):
        if self.proc and self.proc.poll() is None:
            try:
                self.proc.stdin.write("quit\n")
                self.proc.stdin.flush()
                self.proc.wait(timeout=15)
            except Exception:  # noqa: BLE001
                self.proc.kill()
        self.agent.conn = None
        self.bridge.ch = None

    def power_cycle(self, cut=False):
        """Power off and on again. Flash keeps its contents (dumped from the
        running emulator and loaded into the new one); RAM does not."""
        with self.cycle_lock:
            self._power_cycle(cut)

    def _power_cycle(self, cut):
        tracing = self.agent.trace is not None and not self.agent.trace.closed
        seq = 0
        if tracing:
            seq = sum(1 for _ in open(self.agent.trace.name))
        if not cut:
            self.agent.ask("halt")          # supply gone: the core stops first
        dump = os.path.join(self.out, "flash-%d.bin" % self.boots)
        if self.agent.ask("flashdump " + dump, timeout=30) == "ok":
            self.flash = dump
        self.stop_emulator()
        self.start()
        self.agent.wait_connected(BOOT_WALL_TIMEOUT)
        if tracing:
            self.agent.append_event({"seq": seq, "t_us": 0, "kind": "reset", "cause": "power"})
            self.agent.ask(("trace start @%d %s" % (seq + 1, " ".join(self.fixtures.trace_pins))).strip())

    def wait_ready(self):
        if not self.agent.wait_connected(BOOT_WALL_TIMEOUT):
            return "emulator did not start"
        t0 = time.time()
        while self.bridge.radio_ip is None and time.time() - t0 < BOOT_WALL_TIMEOUT:
            if self.proc.poll() is not None:
                return "emulator exited during boot"
            if time.time() - t0 > 2.0 and self.agent.ask("halted") == "halted 1":
                return "no image started (boot: %s)" % self.pr.BOOT_MODES[self.boot][1]
            time.sleep(0.1)
        if self.bridge.radio_ip is None:
            return "the image configured no IP address on the Ethernet controller within %.0f s" % BOOT_WALL_TIMEOUT
        return None

    def close(self):
        self.stop_emulator()
        self.fixtures.close()
        self.agent.close()
        self.bridge.close()


def run_bench(tbdir, endpoint_path, test, agent, wall_cap):
    """Run one test in the bench; returns (entries, error, emulated_s, wall_s)."""
    t_emu0 = agent.now_us()
    t0 = time.time()
    cmd = [sys.executable, "-m", "testbench", "run", "--endpoint", endpoint_path, "--test", test["id"], "--json"]
    p = subprocess.Popen(cmd, cwd=tbdir, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    out = []
    threading.Thread(target=lambda: out.append(p.stdout.read()), daemon=True).start()
    limit_us = int(float(test.get("timeout_s", 30)) * 1e6)
    err = None
    while p.poll() is None:
        time.sleep(0.25)
        now = agent.now_us()
        if t_emu0 is not None and now is not None and now - t_emu0 > limit_us:
            p.kill(); err = "timeout: %.1f s emulated" % (limit_us / 1e6); break
        if time.time() - t0 > wall_cap:
            p.kill(); err = "timeout: %.0f s wall" % wall_cap; break
    p.wait()
    time.sleep(0.1)
    t_emu1 = agent.now_us()
    wall = time.time() - t0
    emu = (t_emu1 - t_emu0) / 1e6 if t_emu0 is not None and t_emu1 is not None else None
    entries = []
    for ln in "".join(out).splitlines():
        ln = ln.strip()
        if ln.startswith("{"):
            try:
                entries.append(json.loads(ln))
            except ValueError:
                pass
    if err is None and p.returncode != 0:
        tail = (p.stderr.read() or "").strip().splitlines()[-1:] if p.stderr else []
        err = "test bench exit %d%s" % (p.returncode, (": " + tail[0]) if tail else "")
    return entries, err, emu, wall


def run(pr, elf, tests_path, run_id, scope="v1.0", out_path=None, boot="standin"):
    spec = json.load(open(tests_path))
    tbdir = testbench_dir(pr.REPO)
    root = os.path.join(pr.OUT_ROOT, run_id)
    os.makedirs(root, exist_ok=True)
    started = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    offered = benchnet.FixtureServer.OFFERED
    defaults = spec.get("defaults", {})
    per_list = spec.get("boot") == "per-list"
    results = []
    shared = None

    def skip(t, reason):
        results.append({"id": t["id"], "status": "skip", "reason": reason})

    for t in spec.get("tests", []):
        t = dict(defaults, **t)
        if t.get("target") == "host":
            skip(t, "target:host"); continue
        if t.get("target") == "needs-radio":
            skip(t, "target:needs-radio"); continue
        if t.get("scope", "v1.0") != scope:
            skip(t, "scope:%s" % t.get("scope")); continue
        missing = [n for n in t.get("needs", []) if n not in offered]
        if missing:
            skip(t, "fixture:%s" % missing[0]); continue
        if not tbdir or not os.path.isdir(tbdir):
            results.append({"id": t["id"], "status": "error", "reason": "runner: no test-bench directory (testbench_dir)"})
            continue
        tdir = os.path.join(root, t["id"]) if not per_list else os.path.join(root, "list")
        bt = shared if per_list and shared else Boot(pr, elf, tdir, boot)
        if per_list:
            shared = bt
        why = bt.wait_ready()
        if why:
            results.append({"id": t["id"], "status": "error", "reason": why})
            if not per_list:
                bt.close()
            continue
        try:
            ep = bt.bridge.endpoint(bt.fixtures.port, offered)
        except Exception as ex:  # noqa: BLE001
            results.append({"id": t["id"], "status": "error", "reason": "runner: %s" % ex})
            if not per_list:
                bt.close()
            continue
        ep["minor"] = FORMAT_MINOR
        ep_path = os.path.join(tdir, "endpoint.json")
        json.dump(ep, open(ep_path, "w"), indent=2)
        wall_cap = 30.0 * float(t.get("timeout_s", 30)) + 120.0
        entries, err, emu, wall = run_bench(tbdir, ep_path, t, bt.agent, wall_cap)
        nm = pr.not_modelled(bt.log) if os.path.isfile(bt.log) else []
        if not per_list:
            bt.close()
        got = {e.get("id"): e for e in entries}
        e = got.get(t["id"])
        if e is None:
            e = {"id": t["id"], "status": "error", "reason": err or "test bench reported no result"}
        e.setdefault("reqs", t.get("reqs", []))
        if emu is not None:
            e["duration_s"] = round(emu, 3)
        e["wall_s"] = round(wall, 3)
        rel = os.path.relpath(tdir, root)
        arts = {"console": os.path.join(rel, "console.txt"), "network": os.path.join(rel, "network.pcapng")}
        if bt.agent.traces:
            arts["trace"] = [os.path.join(rel, os.path.basename(x)) for x in bt.agent.traces]
        e["artifacts"] = arts
        if nm and e.get("status") in ("fail", "error"):
            e["status"] = "error"
            e["reason"] = "not modelled: " + ", ".join(nm)
        elif nm and e.get("status") == "pass":
            e.setdefault("reason", "emulator coverage: not modelled: " + ", ".join(nm))
        results.append(e)
    if shared:
        shared.close()

    summary = {k: 0 for k in ("pass", "fail", "skip", "blocked", "error", "not_run")}
    for r in results:
        summary[r.get("status", "error")] = summary.get(r.get("status", "error"), 0) + 1
    try:
        commit = subprocess.run(["git", "-C", pr.REPO, "rev-parse", "--short", "HEAD"],
                                capture_output=True, text=True).stdout.strip()
        tbcommit = subprocess.run(["git", "-C", tbdir, "rev-parse", "--short", "HEAD"],
                                  capture_output=True, text=True).stdout.strip() if tbdir else ""
    except Exception:  # noqa: BLE001
        commit = tbcommit = ""
    result = {
        "format": "g7250-testresult", "version": 1, "minor": FORMAT_MINOR,
        "testlist": {"sha256": _sha256(tests_path)},
        "subject": {"kind": "emulator", "elf_sha256": _sha256(elf)},
        "runner": {"name": "product_run.py", "version": "1.%d" % FORMAT_MINOR, "emulator_commit": commit},
        "testbench": {"commit": tbcommit},
        "environment": {"boot": pr.BOOT_MODES[boot][1]},
        "started": started,
        "finished": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "results": results, "summary": summary,
    }
    out_path = out_path or os.path.join(root, "result.json")
    json.dump(result, open(out_path, "w"), indent=2)
    for r in results:
        print("  %-7s %s %s" % (r.get("status", "?").upper(), r["id"], r.get("reason", "")))
    print("boot: %s" % pr.BOOT_MODES[boot][1])
    print("PRODUCT RUN %s: %s -> %s" % (run_id, " ".join("%s=%d" % kv for kv in summary.items()),
                                        os.path.relpath(out_path, pr.REPO)))
    if summary["error"]:
        return 2
    return 1 if summary["fail"] else 0
