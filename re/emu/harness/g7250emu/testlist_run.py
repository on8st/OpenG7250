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
import re
import os
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import benchnet  # noqa: E402

FORMAT_MINOR = 4

# Not modelled with a known, limited reach: the image's own console or the log
# shows it, and only the tests that judge that function are marked; every other
# result stays as it is, with a run-level note (the coordinator's rule).
MACRAW_LOG = "CH394/Sn_MR protocol 0x04"
MACRAW_TESTS = {"T-A1-69", "T-A1-70", "T-A1-71", "T-A1-72", "T-A1-73", "T-A3-06"}   # T-A1-69: its bound counts the claim; T-A1-76 (the failure path itself) not
MACRAW_NOTE = ("not modelled: MACRAW (CH394 socket 0 with Sn_MODE 0x04 stays closed in the emulator); "
               "the image's link-local claim ran its failure path (address used without probe or announcement)")


# Datagram delivery shift (CH394 arrival pacing): reported per burst of datagrams
# to the radio (a burst ends at 100 ms without one), so a test's judged burst can be
# told apart from the rest; the status is not converted.


def shift_bursts(tdir):
    path = os.path.join(tdir, "net-log.jsonl")
    if not os.path.isfile(path):
        return []
    ents = []
    for ln in open(path):
        try:
            e = json.loads(ln)
        except ValueError:
            continue
        if e.get("dir") in ("to_radio", "dropped"):
            ents.append(e)
    bursts, cur = [], None
    for e in ents:
        if cur is None or e["t_us"] - cur["last"] > 100000 or e.get("boot") != cur["boot"]:
            cur = {"boot": e.get("boot"), "first_t_us": e["t_us"], "last": e["t_us"], "count": 0, "dropped": 0, "max_shift_us": 0}
            bursts.append(cur)
        cur["last"] = e["t_us"]
        cur["count"] += 1
        cur["dropped"] += e["dir"] == "dropped"
        cur["max_shift_us"] = max(cur["max_shift_us"], abs(int(e.get("shift_us", 0))))
    for b in bursts:
        b["last_t_us"] = b.pop("last")
    return bursts


# The bench names the bursts a check judges: "[burst NAME, host wall T0-T1]" (epoch s).
# A test whose judged burst had a datagram shifted by more than its limit is "not
# evidence" (status error), not pass or fail.
SHIFT_LIMIT_US = {"T-B5-20": 200}
_BURST = re.compile(r"\[burst ([^,\]]+), host wall ([0-9.]+)-([0-9.]+)\]")


def judged_bursts(entry, tdir):
    """{name: (count, dropped, max_shift_us)} for the bursts named in the checks."""
    names = {}
    for c in entry.get("checks", []):
        for m in _BURST.finditer("%s %s" % (c.get("name", ""), c.get("detail", ""))):
            names[m.group(1)] = (float(m.group(2)), float(m.group(3)))
    if not names:
        return {}
    path = os.path.join(tdir, "net-log.jsonl")
    ents = []
    if os.path.isfile(path):
        for ln in open(path):
            try:
                e = json.loads(ln)
            except ValueError:
                continue
            if e.get("dir") in ("to_radio", "dropped") and "t_wall" in e:
                ents.append(e)
    out = {}
    for n, (t0, t1) in names.items():
        sel = [e for e in ents if t0 - 0.002 <= e["t_wall"] <= t1 + 0.002]
        out[n] = {"host_wall": [t0, t1], "count": len(sel), "dropped": sum(e["dir"] == "dropped" for e in sel),
                  "max_shift_us": max([abs(int(e.get("shift_us", 0))) for e in sel] or [0])}
    return out


def macraw_seen(log_path):
    try:
        return any(MACRAW_LOG in ln for ln in open(log_path, errors="replace"))
    except OSError:
        return False
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


def testbench_commit(tbdir, repo):
    """The test bench's own commit: a .tb-commit file in the bench directory
    (written when a tree is unpacked without git), else the commit of the git
    work tree the bench directory belongs to, unless that is this repository."""
    if not tbdir:
        return ""
    marker = os.path.join(tbdir, ".tb-commit")
    if os.path.isfile(marker):
        return open(marker).read().strip()[:12]
    top = subprocess.run(["git", "-C", tbdir, "rev-parse", "--show-toplevel"], capture_output=True, text=True).stdout.strip()
    mine = subprocess.run(["git", "-C", repo, "rev-parse", "--show-toplevel"], capture_output=True, text=True).stdout.strip()
    if not top or top == mine:
        return "unknown"
    return subprocess.run(["git", "-C", tbdir, "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip()


class Boot:
    """One emulator process with its bridge, agent and fixture server."""

    def __init__(self, pr, elf, out, boot="standin", pwr_cr3=None, no_address=False, flash_erase_us=0, flash_preload=(),
                 dhcp_on=False):
        self.pr, self.elf, self.out, self.boot = pr, elf, out, boot
        self.pwr_cr3, self.no_address = pwr_cr3, no_address
        self.flash_erase_us, self.flash_preload = flash_erase_us, list(flash_preload)
        os.makedirs(out, exist_ok=True)
        self.bridge = benchnet.Bridge(os.path.join(out, "network.pcapng"))
        self.agent = benchnet.Agent(out)
        self.fixtures = benchnet.FixtureServer(lambda: self.agent, self.power_cycle, lambda: self.bridge)
        self.bridge.now_us = self.agent.now_us
        self.proc = None
        self.boots = 0
        self.log = os.path.join(out, "renode.log")
        self.flash = None
        self.cycle_lock = threading.Lock()
        self.agent.on_cut = lambda: self.power_cycle(cut=True)
        if dhcp_on:
            self.bridge.dhcp.running = True     # the LAN's DHCP server answers from power-on
        self.start()

    def start(self):
        self.boots += 1
        console = os.path.join(self.out, "console.txt" if self.boots == 1 else "console-boot%d.txt" % self.boots)
        open(console, "w").close()
        resc = os.path.join(self.out, "run.resc")
        open(resc, "w").write(self.pr.build_product_resc(self.elf, self.out, console, self.log,
                                                         self.bridge.ch_port, self.agent.port, self.boot,
                                                         flash_image=self.flash,
                                                         pwr_cr3=self.pwr_cr3 if self.boots == 1 else None,
                                                         flash_erase_us=self.flash_erase_us,
                                                         flash_preload=self.flash_preload,
                                                         boot_forces=list(getattr(getattr(self, "fixtures", None), "boot_forces", [])),
                                                         boot_lines=(self.fixtures.boot_lines() if getattr(self, "fixtures", None) else [])))
        env = dict(os.environ)
        # CH394 socket log of this boot (OPEN/CLOSE with port and emulated time)
        env["G7250_SOCKLOG"] = os.path.join(self.out, "sockets.jsonl" if self.boots == 1 else "sockets-boot%d.jsonl" % self.boots)
        self.proc = subprocess.Popen([self.pr.bootcore.RENODE, "--disable-xwt", "--console",
                                      "-e", "path add @%s" % self.pr.REPO, "-e", "include @%s" % resc],
                                     stdin=subprocess.PIPE, stdout=subprocess.DEVNULL,
                                     stderr=subprocess.DEVNULL, text=True, env=env)

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

    def save_netlog(self):
        """Append the CH394's own datagram log of this emulator (always on: every
        datagram at the radio and every drop, emulated time) to net-log.jsonl."""
        path = os.path.join(self.out, "net-log.jsonl")
        nxt = 0
        with open(path, "a") as f:
            for _ in range(200):
                r = self.agent.ask("netlog runner %d" % nxt, timeout=30, internal=True)
                if not r or not r.startswith("ok "):
                    break
                v = json.loads(r[3:])
                for e in v.get("entries", []):
                    e["boot"] = self.boots
                    f.write(json.dumps(e) + "\n")
                if v.get("next", 0) >= v.get("count", 0) or v.get("next", 0) == nxt:
                    break
                nxt = v["next"]
        return path

    def power_cycle(self, cut=False):
        """Power off and on again. Flash keeps its contents (dumped from the
        running emulator and loaded into the new one); RAM does not."""
        with self.cycle_lock:
            self._power_cycle(cut)

    def _power_cycle(self, cut):
        self.agent.cycling = True
        try:
            tracing = self.agent.trace is not None and not self.agent.trace.closed
            seq = 0
            if tracing:
                seq = sum(1 for _ in open(self.agent.trace.name))
            if not cut:
                self.agent.ask("halt", internal=True)          # supply gone: the core stops first
            self.save_netlog()
            try:
                self.fixtures.sock_pull()
                self.fixtures.console_pull()
                self.fixtures.before_power_cycle()
            except Exception:  # noqa: BLE001
                pass
            dump = os.path.join(self.out, "flash-%d.bin" % self.boots)
            if self.agent.ask("flashdump " + dump, timeout=30, internal=True) == "ok":
                self.flash = dump
            self.stop_emulator()
            self.agent.conn = None          # the old emulator's link: wait for the new one
            self.start()
            self.fixtures.sock_new_boot(self.boots)
            self.agent.wait_connected(BOOT_WALL_TIMEOUT)
            if tracing:
                self.agent.append_event({"seq": seq, "t_us": 0, "kind": "reset", "cause": "power"})
                self.agent.ask(("trace start @%d %s" % (seq + 1, " ".join(self.fixtures.trace_pins))).strip(), internal=True)
        finally:
            self.agent.cycling = False

    def wait_ready(self):
        if not self.agent.wait_connected(BOOT_WALL_TIMEOUT):
            return "emulator did not start"
        if self.no_address:
            time.sleep(3.0)
            if self.bridge.radio_ip is None:
                self.bridge.set_radio("0.0.0.0", "255.255.255.0")   # endpoint only; the image has no address
            return None
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
    # the test's params (e.g. package_slot_a/b) reach the bench as --param key=value
    for k, v in (test.get("params") or {}).items():
        if isinstance(v, (str, int, float)) and not isinstance(v, bool):
            cmd += ["--param", "%s=%s" % (k, v)]
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
    if emu is not None and emu < 0:
        emu = None      # a power cycle during the test restarted emulated time: no single duration
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


def run(pr, elf, tests_path, run_id, scope="v1.0", out_path=None, boot="standin", pwr_cr3=None, no_address=False,
        flash_erase_us=0, flash_preload=(), dhcp_on=False):
    spec = json.load(open(tests_path))
    tbdir = testbench_dir(pr.REPO)
    root = os.path.join(pr.OUT_ROOT, run_id)
    os.makedirs(root, exist_ok=True)
    started = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    offered = benchnet.FixtureServer.OFFERED
    defaults = spec.get("defaults", {})
    per_list = spec.get("boot") == "per-list"
    results = []
    run_notes = set()
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
        try:
            bt = shared if per_list and shared else Boot(pr, elf, tdir, boot, pwr_cr3, no_address, flash_erase_us, flash_preload,
                                                         dhcp_on)
        except Exception as ex:  # noqa: BLE001
            results.append({"id": t["id"], "status": "error", "reason": "runner: %s" % ex})
            continue
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
        ep["usart1_timed"] = False                 # console.log: USART1 not timed at the baud rate
        ep["flash_timing"] = dict(bt.fixtures.FLASH_TIMING)   # format v1.8: the erase durations in use and their source
        if os.environ.get("G7250_TIME_SCALE"):
            ep["time_scale"] = float(os.environ["G7250_TIME_SCALE"])   # emulated / wall, for timed runs (the bench stretches its waits)
        ep_path = os.path.join(tdir, "endpoint.json")
        json.dump(ep, open(ep_path, "w"), indent=2)
        wall_cap = 30.0 * float(t.get("timeout_s", 30)) + 120.0
        entries, err, emu, wall = run_bench(tbdir, ep_path, t, bt.agent, wall_cap)
        # keep the emulator running a little after the bench ended, so a late answer to its
        # last request is still in the capture and the logs (G7250_GRACE_S, default 5 s wall)
        try:
            time.sleep(float(os.environ.get("G7250_GRACE_S", "5")))
        except ValueError:
            pass
        nm = pr.not_modelled(bt.log) if os.path.isfile(bt.log) else []
        flash_note = None
        macraw = macraw_seen(bt.log)
        if macraw:
            run_notes.add(MACRAW_NOTE)
        try:
            bt.save_netlog()
        except Exception:  # noqa: BLE001
            pass
        try:                                   # the flash layer's log of every boot (flash.log entries)
            open(os.path.join(tdir, "flash-info.txt"), "w").write(str(bt.agent.ask("ft info", timeout=10, internal=True)) + "\n")
            bt.fixtures.flash_pull()
            with open(os.path.join(tdir, "flash-log.jsonl"), "w") as f:
                for e in bt.fixtures.fl_entries:
                    f.write(json.dumps(e) + "\n")
            div = int(os.environ.get("G7250_ERASE_MIPS_DIV") or 1)
            if div > 1 and any(e.get("op") in ("erase", "bank_erase") for e in bt.fixtures.fl_entries):
                flash_note = ("not modelled: CPU speed during flash work (MIPS/%d): the image's own work during an erase "
                              "(TX path samples, watchdog feeds) ran %d times slower in emulated time; no result timed "
                              "inside an erase counts as evidence" % (div, div))
        except Exception:  # noqa: BLE001
            pass
        if os.environ.get("G7250_DUMP_FLASH_END") == "1":      # diagnostics: the flash as the test left it
            try:
                bt.agent.ask("flashdump " + os.path.join(tdir, "flash-end.bin"), timeout=60, internal=True)
            except Exception:  # noqa: BLE001
                pass
        first_check = None
        try:
            fc = bt.agent.ask("firstcheck", timeout=10, internal=True)
            if fc and fc.startswith("ok ") and fc[3:] != "{}":
                first_check = json.loads(fc[3:])
        except Exception:  # noqa: BLE001
            pass
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
        arts = {"console": os.path.join(rel, "console.txt"), "console_timed": os.path.join(rel, "console-timed.txt"),
                "network": os.path.join(rel, "network.pcapng")}
        if os.path.isfile(os.path.join(tdir, "net-log.jsonl")):
            arts["net_log"] = os.path.join(rel, "net-log.jsonl")
        if bt.agent.traces:
            arts["trace"] = [os.path.join(rel, os.path.basename(x)) for x in bt.agent.traces]
        e["artifacts"] = arts
        jb = judged_bursts(e, tdir)
        if jb:
            e.setdefault("emulator", {})["judged_bursts"] = jb
            lim = SHIFT_LIMIT_US.get(t["id"])
            worst = max(b["max_shift_us"] for b in jb.values())
            if lim is not None and worst > lim and e.get("status") in ("pass", "fail"):
                bad = [n for n, b in jb.items() if b["max_shift_us"] > lim]
                e["reason"] = ("not evidence: datagram delivery shifted by up to %d us in emulated time in the judged burst(s) %s "
                               "(> %d us); bench: %s %s" % (worst, ", ".join(bad), lim, e.get("status"), e.get("reason") or ""))
                e["status"] = "error"
        if first_check:
            e.setdefault("emulator", {})["first_tx_port_read"] = first_check
        bursts = shift_bursts(tdir)
        if bursts:
            em = e.setdefault("emulator", {})
            em["max_delivery_shift_us"] = max(b["max_shift_us"] for b in bursts)
            em["bursts"] = [b for b in bursts if b["count"] >= 3 or b["max_shift_us"] or b["dropped"]]
        if flash_note:
            e.setdefault("emulator", {}).setdefault("not_modelled", []).append(flash_note)
            e["reason"] = flash_note + ("; bench: %s" % e["reason"] if e.get("reason") else "")
        if macraw and t["id"] in MACRAW_TESTS:
            if e.get("status") in ("fail", "error"):
                e["status"] = "error"
                e["reason"] = "not modelled: MACRAW" + (" (bench: %s)" % e["reason"] if e.get("reason") else "")
            else:
                e.setdefault("reason", "emulator coverage: not modelled: MACRAW")
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
        tbcommit = testbench_commit(tbdir, pr.REPO)
    except Exception:  # noqa: BLE001
        commit = tbcommit = ""
    result = {
        "format": "g7250-testresult", "version": 1, "minor": FORMAT_MINOR,
        "testlist": {"sha256": _sha256(tests_path)},
        "subject": {"kind": "emulator", "elf_sha256": _sha256(elf)},
        "runner": {"name": "product_run.py", "version": "1.%d" % FORMAT_MINOR, "emulator_commit": commit},
        "testbench": {"commit": tbcommit},
        "environment": dict({"boot": pr.BOOT_MODES[boot][1]},
                            **({"pwr_cr3_at_power_on": "0x%08X locked" % pwr_cr3} if pwr_cr3 is not None else {}),
                            **({"flash_erase_us": flash_erase_us} if flash_erase_us else {}),
                            **({"dhcp_server_at_power_on": True} if dhcp_on else {}),
                            **({"flash_preload": ["0x%08X" % a for a, _ in flash_preload]} if flash_preload else {})),
        "started": started,
        "finished": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "results": results, "summary": summary,
        **({"notes": sorted(run_notes)} if run_notes else {}),
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
