#!/usr/bin/env python3
"""HIS-02 probe: which of the three NCO words in the FPGA 0x01 frame is which.

Boots the stock Main with the fused GPIOE+FPGA model (which logs every 0x01
configuration frame the MCU writes), plays the Control's role on USART2 by
sending SLIP 0x01 settings images (the layout from g7250-base
docs/protocol.md and cmd01_fields.csv, used as data), and changes VFO A, VFO B,
the selected VFO, RIT and split step by step. The frames logged after each step
are then searched for the NCO words expected for the step's frequencies
(word = round(f * 2^32 / 122.88 MHz)).

Output in .private/nco/: renode.log, console.txt, steps.txt, report.md.
Needs the local image-map (refuses with exit 2 otherwise). No decryption.
"""
from __future__ import annotations

import os
import re
import socket
import struct
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import bootcore  # noqa: E402
import m1_run    # noqa: E402

REPO = m1_run.REPO
OUT = os.path.join(REPO, ".private", "nco")
PORT = 3461
F_CLK = 122.88e6

# 0x01 settings image (424 B) offsets, g7250-base cmd01_fields.csv (VERIFIED)
VFO_A, VFO_B = 0x024, 0x04C          # 40-byte records; u64 frequency at +0x00
REC_MODE = 0x18                      # mode code, 2 = USB
RIT_HZ, SEL_VFO, RIT_ON, SPLIT_ON = 0x074, 0x098, 0x0A2, 0x0A4

STEPS = [
    # name, A Hz, B Hz, selected (0=A,1=B), RIT Hz, RIT on, split on
    ("base: A 7.100 MHz, B 14.200 MHz, A selected", 7_100_000, 14_200_000, 0, 0, 0, 0),
    ("change A to 7.123456 MHz", 7_123_456, 14_200_000, 0, 0, 0, 0),
    ("change B to 21.300 MHz (A selected)", 7_123_456, 21_300_000, 0, 0, 0, 0),
    ("select B", 7_123_456, 21_300_000, 1, 0, 0, 0),
    ("select A, RIT +1000 Hz on", 7_123_456, 21_300_000, 0, 1000, 1, 0),
    ("RIT off, split on", 7_123_456, 21_300_000, 0, 1000, 0, 1),
    ("split on, change A to 3.600 MHz", 3_600_000, 21_300_000, 0, 1000, 0, 1),
]


def crc16_arc(data):
    crc = 0
    for b in data:
        crc ^= b
        for _ in range(8):
            crc = (crc >> 1) ^ 0xA001 if crc & 1 else crc >> 1
    return crc


def slip(packet):
    out = bytearray()
    for b in packet:
        if b == 0xC0:
            out += b"\xDB\xDC"
        elif b == 0xDB:
            out += b"\xDB\xDD"
        else:
            out.append(b)
    return bytes(out) + b"\xC0"


def settings_frame(a_hz, b_hz, sel, rit_hz, rit_on, split_on):
    p = bytearray(424)
    p[0] = 0xAA
    for base, f in ((VFO_A, a_hz), (VFO_B, b_hz)):
        p[base:base + 8] = struct.pack("<Q", f)
        p[base + REC_MODE] = 2
    p[RIT_HZ:RIT_HZ + 2] = struct.pack("<h", rit_hz)
    p[SEL_VFO], p[RIT_ON], p[SPLIT_ON] = sel, rit_on, split_on
    pkt = bytes([0x01, 0, 0]) + bytes(p)
    return slip(pkt + struct.pack("<H", crc16_arc(pkt)))


def word(f_hz):
    return round(f_hz * 2**32 / F_CLK) & 0xFFFFFFFF


def build_resc(image, log_path, console_path):
    lines = [ln for ln in m1_run.build_resc(image, log_path, fpga=True).splitlines()
             if "LogAllPeripheralsAccess" not in ln and not ln.startswith("logLevel -1")
             and ln.strip() != "start"]
    lines += [
        "logLevel 3",
        "logLevel 1 sysbus.gpioPortE",
        "sysbus.usart1 CreateFileBackend @%s true" % console_path,
        # USART2 RX through the generic USART2 + DMA models (m1_run; docs/USART-model.md)
        'emulation CreateServerSocketTerminal %d "ctrl" false' % PORT,
        "connector Connect sysbus.usart2 ctrl",
        "start",
    ]
    return "\n".join(lines) + "\n"


def main():
    try:
        import imagemap
        image = imagemap.resolve("main")
    except Exception as e:
        print("nco probe: no runnable Main image.\n  %s" % e)
        sys.exit(2)
    os.makedirs(OUT, exist_ok=True)
    log_path = os.path.join(OUT, "renode.log")
    console = os.path.join(OUT, "console.txt")
    for p in (log_path, console):
        open(p, "w").close()
    resc = os.path.join(OUT, "probe.resc")
    open(resc, "w").write(build_resc(image, log_path, console))
    proc = subprocess.Popen([bootcore.RENODE, "--disable-xwt", "--console",
                             "-e", "path add @%s" % REPO, "-e", "include @%s" % resc],
                            stdin=subprocess.PIPE, stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL, text=True)
    steps_log = []
    try:
        t0 = time.time()
        while "Start RX mode" not in open(console, errors="replace").read():
            if time.time() - t0 > float(os.environ.get("NCO_BOOT_TIMEOUT", "400")):
                print("nco probe: Main did not reach RX mode"); sys.exit(1)
            time.sleep(2)
        print("Main in RX mode after %.0f s host time" % (time.time() - t0))
        time.sleep(5)
        s = socket.create_connection(("127.0.0.1", PORT), timeout=10)
        # The boot continues after "Check complete!" (bias trim, final RX set-up)
        # before the superloop applies 0x01 images. Keep sending the base image
        # until the superloop starts, then start the steps.
        base = settings_frame(*STEPS[0][1:])
        limit = float(os.environ.get("NCO_LOOP_TIMEOUT", "3600"))
        t1 = time.time()

        # "Init Radio Server" is printed right before the superloop starts
        # (after the bias trim); wait for it, then a few seconds more.
        while "Init Radio Server" not in open(console, errors="replace").read():
            if time.time() - t1 > limit:
                print("nco probe: superloop not reached within %.0f s" % limit); break
            s.sendall(base); time.sleep(0.02)
        print("superloop start seen after %.0f s host time" % (time.time() - t1))
        t2 = time.time()
        while time.time() - t2 < 10:
            s.sendall(base); time.sleep(0.02)
        hold = float(os.environ.get("NCO_STEP_HOLD", "20"))
        for i, st in enumerate(STEPS):
            fr = settings_frame(*st[1:])
            steps_log.append((time.strftime("%H:%M:%S"), time.time(), i, st))
            print("step %d: %s" % (i, st[0]))
            t = time.time()
            while time.time() - t < hold:
                s.sendall(fr)
                time.sleep(0.02)
        s.close()
        time.sleep(3)
    finally:
        try:
            proc.stdin.write("pause\nquit\n"); proc.stdin.flush()
        except Exception:
            pass
        try:
            proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            proc.kill()
    report(log_path, steps_log)


def report(log_path, steps_log):
    pat = re.compile(r"^\[(\d\d):(\d\d):(\d\d)\.(\d+)\].*FPGA cmd 0x01 frame (\d+) B: ([0-9A-F]+)")
    frames = []
    day = time.strftime("%Y-%m-%d")
    for ln in open(log_path, errors="replace"):
        m = pat.search(ln)
        if m:
            hh, mm, ss, frac = m.group(1), m.group(2), m.group(3), m.group(4)
            ts = time.mktime(time.strptime("%s %s:%s:%s" % (day, hh, mm, ss),
                                           "%Y-%m-%d %H:%M:%S")) + float("0." + frac)
            frames.append((ts, bytes.fromhex(m.group(6))))
    lines = ["# HIS-02 NCO probe", "", "%d 0x01 frames logged." % len(frames), ""]
    bounds = [s[1] for s in steps_log] + [float("inf")]
    for k, (_, t_start, i, st) in enumerate(steps_log):
        fr = [f for (ts, f) in frames if t_start <= ts < bounds[k + 1]]
        name, a, b, sel, rit, rit_on, split = st
        exp = {"A": a, "B": b, "A+RIT": a + rit, "B+RIT": b + rit}
        lines.append("## step %d: %s (%d frames)" % (i, name, len(fr)))
        uniq = []
        for f in fr:
            if f not in uniq:
                uniq.append(f)
        for f in uniq[-3:]:
            lines.append("- `%s`" % f.hex())
            for off in range(0, len(f) - 3):
                w = struct.unpack(">I", f[off:off + 4])[0]
                hits = [k2 for k2, v in exp.items() if abs(w - word(v)) <= 1]
                if hits:
                    lines.append("  - offset %d: 0x%08X = %s" % (off, w, "/".join(hits)))
        lines.append("")
    rep = os.path.join(OUT, "report.md")
    open(rep, "w").write("\n".join(lines) + "\n")
    open(os.path.join(OUT, "steps.txt"), "w").write(
        "\n".join("%s step %d %s" % (s[0], s[2], s[3][0]) for s in steps_log) + "\n")
    print("report: %s" % rep)


if __name__ == "__main__":
    main()
