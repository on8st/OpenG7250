"""The v1.3 live-trace flow against a fake fixture server: the bench keeps the file from
hw.trace start, reads it at a checkpoint, and fails a test whose stop reports dropped events.
GPL-3.0-or-later, (c) 2026 ON8ST."""
import json
import socket
import tempfile
import threading
import unittest
from pathlib import Path

from testbench import core
from testbench.t_tx_locked import safe

SNAPSHOT = dict(tr_lines=dict(PF4=1, PF6=0, PC7=0, PH11=0), spi1_a=dict(oe_PA10=1, last_frame=None),
                pa_dac=dict(reset_PB14=0, ch=[None] * 8), atu_latch_writes=dict(PD13=0, PG2=0, PG5=0))


class FakeRunner:
    def __init__(self, trace_lines, dropped=0):
        self.dir = tempfile.TemporaryDirectory()
        self.file = str(Path(self.dir.name) / "trace-1.jsonl")
        Path(self.file).write_text("".join(json.dumps(e) + "\n" for e in trace_lines))
        self.dropped = dropped
        self.srv = socket.create_server(("127.0.0.1", 0))
        self.addr = "127.0.0.1:%d" % self.srv.getsockname()[1]
        threading.Thread(target=self.serve, daemon=True).start()

    def serve(self):
        while True:
            try:
                c, _ = self.srv.accept()
            except OSError:
                return
            with c:
                req = json.loads(c.makefile().readline())
                op, args = req["op"], req.get("args", {})
                if op == "hw.observe":
                    v = SNAPSHOT
                elif op == "hw.trace" and args.get("action") == "start":
                    v = dict(file=self.file)
                elif op == "hw.trace":
                    v = dict(count=1, dropped=self.dropped, file=self.file)
                else:
                    v = None
                c.sendall((json.dumps(dict(ok=True, value=v)) + "\n").encode())

    def endpoint(self):
        return dict(radio=dict(ip="127.0.0.1"), hosts=[dict(ip="127.0.0.1")],
                    control=dict(address=self.addr), fixtures=["hw.observe", "hw.trace"])

    def close(self):
        self.srv.close()
        self.dir.cleanup()


def run(runner):
    core.REGISTRY["T-UNIT-01"] = lambda ctx: safe(ctx, "checkpoint")
    try:
        return core.run_one("T-UNIT-01", core.Ctx(runner.endpoint()))
    finally:
        del core.REGISTRY["T-UNIT-01"]


class FlowTest(unittest.TestCase):
    def test_live_file_read_at_checkpoint(self):
        good = FakeRunner([dict(seq=1, kind="spi", bus="SPI1", cs=None, data="01"),
                           dict(seq=2, kind="gpio", pin="PA11", level=1)])
        try:
            r = run(good)
            self.assertEqual(r["status"], "pass", r)
            self.assertTrue(any("(trace since start)" in c["name"] for c in r["checks"]))
        finally:
            good.close()
        bad = FakeRunner([dict(seq=1, kind="spi", bus="SPI1", cs=None, data="80"),
                          dict(seq=2, kind="gpio", pin="PA11", level=1)])
        try:
            self.assertEqual(run(bad)["status"], "fail")
        finally:
            bad.close()

    def test_dropped_events_fail(self):
        r_ = FakeRunner([], dropped=3)
        try:
            r = run(r_)
            self.assertEqual(r["status"], "fail")
            self.assertTrue(any(c["name"] == "hw.trace: no events dropped" for c in r["checks"]))
        finally:
            r_.close()


if __name__ == "__main__":
    unittest.main()
