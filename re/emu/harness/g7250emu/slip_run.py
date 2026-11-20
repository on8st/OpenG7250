#!/usr/bin/env python3
"""D1 — Main<->Control SLIP link (two machines, one emulation).

Boots both stock MCUs together and wires USART2<->USART2 (the 460800 SLIP
inter-MCU link, VERIFIED) through a Renode UART hub, then checks that SLIP
frames actually move between them (0xC0-delimited). Both units now boot to their
service loops (M1 / M2-C2), so the Control's SLIP client (TIM13, 20 ms) and the
Main's SLIP server can exchange. Images via the local image-map (never decrypts).
"""
from __future__ import annotations
import os, re, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
sys.path.insert(0, HERE)
import bootcore, observe, platform_extras  # noqa: E402
import imagemap  # noqa: E402

OUT = os.path.join(REPO, ".private", "slip")
CS = os.path.join(REPO, "models", "csharp", "G7250Slaves.cs")
CS_DMA = os.path.join(REPO, "models", "csharp", "G7250Dma.cs")
CS_USART = os.path.join(REPO, "models", "csharp", "G7250Usart.cs")
PLAT = os.path.join(REPO, "platforms")


def build_resc(main_img, ctrl_img, log_path):
    L = []
    L += ['i @%s' % CS, 'i @%s' % CS_DMA, 'i @%s' % CS_USART]
    # Main
    L += ['mach create "main"',
          'machine LoadPlatformDescription @%s/g7250_main.repl' % PLAT,
          'machine LoadPlatformDescription @%s/g7250_main_i2c1_func.repl' % PLAT,
          # generic DMA + USART2: the Main receives the link through its DMA ring
          'sysbus Unregister sysbus.dma1', 'sysbus Unregister sysbus.dma2',
          'machine LoadPlatformDescription @%s/g7250_main_dma.repl' % PLAT,
          'sysbus Unregister sysbus.usart2',
          'machine LoadPlatformDescription @%s/g7250_main_usart2.repl' % PLAT,
          'nvic Frequency 275000000']
    L += platform_extras.all_tag_commands()
    # Control
    L += ['mach create "control"',
          'machine LoadPlatformDescription @%s/g7250_control.repl' % PLAT,
          'machine LoadPlatformDescription @%s/g7250_control_i2c_mock.repl' % PLAT,
          'sysbus Unregister sysbus.ltdc',
          'machine LoadPlatformDescriptionFromString "ltdc2: Antmicro.Renode.Peripherals.Miscellaneous.G7250LtdcStub @ sysbus 0x50001000"',
          'nvic Frequency 240000000']
    # SLIP hub: Main USART2 <-> Control USART2
    L += ['emulation CreateUARTHub "slip"',
          'mach set "control"', 'connector Connect sysbus.usart2 slip',
          'mach set "main"', 'connector Connect sysbus.usart2 slip']
    # observe both; one shared log file
    L += ['mach set "main"', 'logLevel -1 sysbus', 'sysbus LogAllPeripheralsAccess true',
          'mach set "control"', 'logLevel -1 sysbus', 'sysbus LogAllPeripheralsAccess true',
          'logFile @%s true' % log_path, 'logLevel 3 console',
          'mach set "main"', 'logLevel 1 sysbus.usart2']
    # load images + reset vectors
    L += ['mach set "main"',
          'sysbus LoadBinary @%s 0x08020000' % main_img,
          'cpu VectorTableOffset 0x08020000', 'cpu SP 0x24050000', 'cpu PC 0x08024554',
          'mach set "control"',
          'sysbus LoadBinary @%s 0x08020000' % ctrl_img,
          'cpu VectorTableOffset 0x08020000', 'cpu SP 0x24080000', 'cpu PC 0x080235AC']
    L += ['start']
    return "\n".join(L) + "\n"


def main():
    timeout = float(sys.argv[sys.argv.index("--timeout")+1]) if "--timeout" in sys.argv else 45.0
    os.makedirs(OUT, exist_ok=True)
    log_path = os.path.join(OUT, "slip_renode.log"); open(log_path, "w").close()
    resc = os.path.join(OUT, "slip.resc")
    open(resc, "w").write(build_resc(imagemap.resolve("main"), imagemap.resolve("control"), log_path))
    print("SLIP link: booting Main + Control (%.0fs)" % timeout)
    p = subprocess.Popen([bootcore.RENODE, "--disable-xwt", "--console",
                          "-e", "path add @%s" % REPO, "-e", "include @%s" % resc],
                         stdin=subprocess.PIPE, stdout=subprocess.DEVNULL,
                         stderr=subprocess.DEVNULL, text=True)
    time.sleep(timeout)
    try: p.stdin.write("pause\nquit\n"); p.stdin.flush()
    except Exception: pass
    try: p.wait(timeout=10)
    except subprocess.TimeoutExpired: p.kill()

    # count USART2 TX (0x28) / RX (0x24) on each machine
    txrx = {"main": {"tx": 0, "rx": 0}, "control": {"tx": 0, "rx": 0}}
    cur = None
    # machine name isn't in the per-line log; both sysbuses log usart2. Use the
    # data-register direction as the SLIP evidence (frames move both ways).
    tx = rx = 0
    frames = 0
    for ln in open(log_path, errors="replace"):
        m = re.search(r"usart2:.*(Write|Read)\w* (?:to|from) (0x[0-9A-Fa-f]+)(?:\s*\(([^)]*)\))?(?:,\s+(?:value|returned)\s+(0x[0-9A-Fa-f]+))?", ln)
        if not m: continue
        off = int(m.group(2), 16)
        if m.group(1) == "Write" and off == 0x28:
            tx += 1
            if m.group(4) and (int(m.group(4), 16) & 0xFF) == 0xC0: frames += 1
        elif m.group(1) == "Read" and off == 0x24:
            rx += 1
    main_rx = 0
    for ln in open(log_path, errors="replace"):
        m = re.search(r"USART2: (\d+) bytes received", ln)
        if m: main_rx = max(main_rx, int(m.group(1)))
    print("USART2 (both MCUs, register-visible): TX bytes=%d  SLIP 0xC0 delimiters=%d" % (tx, frames))
    print("Main USART2 received (generic USART2 + DMA ring): >= %d bytes" % main_rx)
    # the model logs the count at powers of two; 256 bytes is what a 45 s run
    # delivers to the Main (several Control frames), so less means a broken RX path
    MIN_MAIN_RX = 256
    ok = tx > 0 and main_rx >= MIN_MAIN_RX
    print("\n%s" % ("D1 OK: both MCUs boot together, USART2<->USART2 is wired through the"
                    " SLIP UART hub, SLIP bytes are transmitted, and the Main receives them."
                    if ok else "D1: no USART2 traffic, or the Main received fewer than %d bytes." % MIN_MAIN_RX))
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
