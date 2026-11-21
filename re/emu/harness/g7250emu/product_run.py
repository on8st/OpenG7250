#!/usr/bin/env python3
"""Product test-bench runner (decision 0007, docs/PRODUCT-TESTBENCH-PLAN.md).

    product_run.py <image.elf> <tests.json> [--run-id ID] --selftest

Runs one image on the Main platform with the generic models only (no stock
image map, no stock-only shims) for a fixed virtual time and writes
.private/product-runs/<run-id>/:
  result.json   pass/fail per test, plus "not modelled: ..." entries
  console.txt   the image's USART1 output
  usart2.bin    the image's USART2 transmit bytes (interlink)
  renode.log    emulator log (RE side only; never relayed)

Self-test list (--selftest, own programs in firmware-tests/bench/ only; product
runs will use the testbench format g7250-testlist v1, step 3):
  {"run_s": 5,
   "tests": [{"name": "banner", "expect": "regex"},
             {"name": "no_fault", "forbid": "regex"},
             {"name": "self", "bench": true}]}
"expect": the regex must match the console; "forbid": it must not; "bench":
every "TEST <name> PASS|FAIL ..." console line becomes a test, and
"BENCH DONE" must appear. A failing test carries the not-modelled accesses
seen in the run, named by RM0468 peripheral and register offset only.

--selftest also adds the bench-only request source (platforms/g7250_bench.repl);
never for product images.
"""
from __future__ import annotations
import argparse
import datetime
import json
import os
import re
import struct
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
sys.path.insert(0, HERE)
import bootcore          # noqa: E402
import platform_extras   # noqa: E402

P = lambda *a: os.path.join(REPO, *a)   # noqa: E731
OUT_ROOT = P(".private", "product-runs")

# Generic models (hardware behaviour from RM0468, datasheets, the HIS).
GENERIC = [
    ("cs", P("models", "csharp", "G7250Slaves.cs")),
    ("repl", P("platforms", "g7250_main_i2c1_func.repl")),
    ("cs", P("models", "csharp", "G7250Ch394.cs")),
    ("repl", P("platforms", "g7250_main_spi6_ch394.repl")),
    ("cs", P("models", "csharp", "G7250Mdma.cs")),
    ("repl", P("platforms", "g7250_main_mdma.repl")),
    ("cs", P("models", "csharp", "G7250Dma.cs")),
    ("cmd", "sysbus Unregister sysbus.dma1"),
    ("cmd", "sysbus Unregister sysbus.dma2"),
    ("repl", P("platforms", "g7250_main_dma.repl")),
    ("cs", P("models", "csharp", "G7250Usart.cs")),
    ("cmd", "sysbus Unregister sysbus.usart2"),
    ("repl", P("platforms", "g7250_main_usart2.repl")),
    ("cs", P("models", "csharp", "G7250Adc.cs")),
    ("cmd", "sysbus Unregister sysbus.adcM1S2"),
    ("repl", P("platforms", "g7250_main_adc.repl")),
    # PA bias DAC + plant at bus/pin level (his/85), driving the ADC inputs
    ("cs", P("models", "csharp", "G7250PaPlant.cs")),
    ("repl", P("platforms", "g7250_main_pa_plant.repl")),
    ("cmd", "sysbus.spi2.pa Publish"),
    # IWDG1 with the RCC_RSR reset flags (docs/IWDG-model.md)
    ("cs", P("models", "csharp", "G7250Iwdg.cs")),
    ("cmd", "sysbus Unregister sysbus.watchdog"),
    ("repl", P("platforms", "g7250_main_iwdg.repl")),
    # GPIOE with the RF FPGA on its pins (his/80), DRDY at the his/80 rate
    ("cs", P("models", "csharp", "G7250Gpioe.cs")),
    ("cmd", "sysbus Unregister sysbus.gpioPortE"),
    ("repl", P("platforms", "g7250_main_fpga_his.repl")),
]
BENCH = [
    ("cs", P("models", "csharp", "G7250BenchReq.cs")),
    ("repl", P("platforms", "g7250_bench.repl")),
    # USART2 <-> USART3 through a hub, the peer for the USART self-test
    ("cmd", 'emulation CreateUARTHub "benchhub"'),
    ("cmd", "connector Connect sysbus.usart2 benchhub"),
    ("cmd", "connector Connect sysbus.usart3 benchhub"),
    # fixed ADC inputs for the ADC self-test
    ("cmd", "sysbus.adc12 SetChannelVoltage 1 0.5"),
    ("cmd", "sysbus.adc12 SetChannelVoltage 4 1.65"),
    ("cmd", "sysbus.adc12 SetChannelVoltage 5 3.3"),
]

# RM0468 memory map (public), base -> name, for naming not-modelled accesses.
RM0468_MAP = [
    (0x40000000, "TIM2"), (0x40000400, "TIM3"), (0x40000800, "TIM4"), (0x40000C00, "TIM5"),
    (0x40001000, "TIM6"), (0x40001400, "TIM7"), (0x40001800, "TIM12"), (0x40001C00, "TIM13"),
    (0x40002000, "TIM14"), (0x40002400, "LPTIM1"), (0x40003800, "SPI2"), (0x40003C00, "SPI3"),
    (0x40004000, "SPDIFRX"), (0x40004400, "USART2"), (0x40004800, "USART3"), (0x40004C00, "UART4"),
    (0x40005000, "UART5"), (0x40005400, "I2C1"), (0x40005800, "I2C2"), (0x40005C00, "I2C3"),
    (0x40006000, "I2C5"), (0x40006C00, "CEC"), (0x40007400, "DAC1"), (0x40007800, "UART7"),
    (0x40007C00, "UART8"), (0x40008400, "CRS"), (0x40008800, "SWPMI"), (0x40009000, "OPAMP"),
    (0x40009400, "MDIOS"), (0x4000A000, "FDCAN"), (0x4000E000, "TIM23"), (0x4000E400, "TIM24"),
    (0x40010000, "TIM1"), (0x40010400, "TIM8"), (0x40011000, "USART1"), (0x40011400, "USART6"),
    (0x40011800, "UART9"), (0x40011C00, "USART10"), (0x40013000, "SPI1"), (0x40013400, "SPI4"),
    (0x40014000, "TIM15"), (0x40014400, "TIM16"), (0x40014800, "TIM17"), (0x40015000, "SPI5"),
    (0x40015800, "SAI1"), (0x40017400, "DFSDM1"), (0x40020000, "DMA1"), (0x40020400, "DMA2"),
    (0x40020800, "DMAMUX1"), (0x40022000, "ADC1_2"), (0x40024400, "CRC"), (0x40028000, "ETH"),
    (0x40040000, "OTG_HS"), (0x48020000, "DCMI_PSSI"), (0x48021800, "RNG"), (0x48022400, "SDMMC2"),
    (0x48022800, "CORDIC"), (0x48022C00, "FMAC"), (0x51000000, "AXI_GPV"), (0x52000000, "MDMA"),
    (0x52001000, "DMA2D"), (0x52002000, "FLASH_IF"), (0x52003000, "JPEG"), (0x52004000, "FMC"),
    (0x52005000, "OCTOSPI1"), (0x52007000, "SDMMC1"), (0x5200A000, "OCTOSPI2"),
    (0x58000000, "EXTI"), (0x58000400, "SYSCFG"), (0x58000C00, "LPUART1"), (0x58001400, "SPI6"),
    (0x58001C00, "I2C4"), (0x58002400, "LPTIM2"), (0x58002800, "LPTIM3"), (0x58002C00, "LPTIM4"),
    (0x58003000, "LPTIM5"), (0x58003800, "COMP"), (0x58003C00, "VREFBUF"), (0x58004000, "RTC"),
    (0x58004800, "IWDG"), (0x58005400, "SAI4"), (0x58020000, "GPIOA"), (0x58020400, "GPIOB"),
    (0x58020800, "GPIOC"), (0x58020C00, "GPIOD"), (0x58021000, "GPIOE"), (0x58021400, "GPIOF"),
    (0x58021800, "GPIOG"), (0x58021C00, "GPIOH"), (0x58022000, "GPIOJ"), (0x58022400, "GPIOK"),
    (0x58024400, "RCC"), (0x58024800, "PWR"), (0x58025400, "BDMA"), (0x58025800, "DMAMUX2"),
    (0x58026000, "ADC3"), (0x58026400, "HSEM"), (0x5C001000, "DBGMCU"),
]


def periph_name(addr):
    best = None
    for base, name in RM0468_MAP:
        if base <= addr and (best is None or base > best[0]):
            best = (base, name)
    if best and addr - best[0] < 0x400:
        return "%s/offset 0x%03X" % (best[1], addr - best[0])
    return "address 0x%08X" % addr


_NOPERI = re.compile(r"(?:Read|Write)\w* (?:from|to) non existing peripheral at (0x[0-9A-Fa-f]+)")
_TAG = re.compile(r"(?:Read|Write)\w* (?:from|to) (0x[0-9A-Fa-f]+) \(tag: '([^']+)'")
_OURS = re.compile(r"not modelled: ([^ ]+/offset 0x[0-9A-Fa-f]+)")
_UNHANDLED = re.compile(r"\] (\w+): Unhandled (?:read|write) (?:from|to) offset (0x[0-9A-Fa-f]+)")
_RENODE_NAMES = {"usart1": 0x40011000, "usart2": 0x40004400, "usart3": 0x40004800,
                 "spi1": 0x40013000, "spi2": 0x40003800, "spi3": 0x40003C00, "spi6": 0x58001400,
                 "i2c1": 0x40005400, "rcc": 0x58024400, "pwr": 0x58024800, "exti": 0x58000000,
                 "syscfg": 0x58000400, "rtc": 0x58004000, "watchdog": 0x58004800,
                 "adcM1S2": 0x40022000, "adc3": 0x58026000, "dmamux1": 0x40020800}


def not_modelled(log_path):
    seen = []
    for ln in open(log_path, errors="replace"):
        item = None
        m = _NOPERI.search(ln)
        if m:
            item = periph_name(int(m.group(1), 16))
        m = m or None
        if not item:
            t = _TAG.search(ln)
            if t:
                item = periph_name(int(t.group(1), 16))
        if not item:
            o = _OURS.search(ln)
            if o:
                item = o.group(1)
        if not item and "Unhandled bits" not in ln:   # ignored configuration bits are not a missing register
            u = _UNHANDLED.search(ln)
            if u and u.group(1) in _RENODE_NAMES:
                item = periph_name(_RENODE_NAMES[u.group(1)] + int(u.group(2), 16))
        if item and item not in seen:
            seen.append(item)
    return seen


def elf_entry(path):
    """(vector table address, initial SP, reset PC) from the lowest loadable
    segment in flash; plain ELF32 parse, no toolchain needed."""
    d = open(path, "rb").read()
    if d[:4] != b"\x7fELF" or d[4] != 1:
        sys.exit("not an ELF32 file: %s" % path)
    phoff, = struct.unpack_from("<I", d, 0x1C)
    phentsize, phnum = struct.unpack_from("<HH", d, 0x2A)
    segs = []
    for i in range(phnum):
        ptype, off, vaddr, paddr, filesz = struct.unpack_from("<IIIII", d, phoff + i * phentsize)
        if ptype == 1 and filesz >= 8 and 0x08000000 <= paddr < 0x08200000:
            segs.append((paddr, off))
    if not segs:
        sys.exit("no loadable flash segment in %s" % path)
    paddr, off = min(segs)
    sp, pc = struct.unpack_from("<II", d, off)
    return paddr, sp, pc


def build_resc(elf, out, run_s, bench):
    vtor, sp, pc = elf_entry(elf)
    lines = ['mach create "product"',
             'machine LoadPlatformDescription @%s' % P("platforms", "g7250_main.repl")]
    for kind, val in GENERIC + (BENCH if bench else []):
        lines.append({"cs": "i @%s", "repl": "machine LoadPlatformDescription @%s",
                      "str": 'machine LoadPlatformDescriptionFromString "%s"', "cmd": "%s"}[kind] % val)
    lines += platform_extras.all_tag_commands(exclude=("MDMA",))
    lines += [
        'nvic Frequency 275000000',
        'logFile @%s' % os.path.join(out, "renode.log"),
        'logLevel 2',
        'sysbus.usart1 CreateFileBackend @%s true' % os.path.join(out, "console.txt"),
        'sysbus.usart2 CreateFileBackend @%s true' % os.path.join(out, "usart2.bin"),
        'sysbus LoadELF @%s' % elf,
        'cpu VectorTableOffset 0x%08X' % vtor,
        'cpu SP 0x%08X' % sp,
        'cpu PC 0x%08X' % (pc & ~1),
        'emulation RunFor "%s"' % str(datetime.timedelta(seconds=run_s)),
        'quit',
    ]
    return "\n".join(lines) + "\n"


CH394_REPL = P("platforms", "g7250_main_spi6_ch394.repl")


def build_product_resc(elf, out, console, log, ch_port, agent_port):
    """A product boot for a g7250-testlist run: the generic models, the CH394
    with the runner's network bridge and the bench agent; runs until the
    runner quits it."""
    vtor, sp, pc = elf_entry(elf)
    lines = ['mach create "product"',
             'machine LoadPlatformDescription @%s' % P("platforms", "g7250_main.repl")]
    for kind, val in GENERIC:
        if kind == "repl" and val == CH394_REPL:
            rp = os.path.join(out, "ch394.repl")
            open(rp, "w").write("ch394: Antmicro.Renode.Peripherals.SPI.G7250Ch394 @ spi6\n"
                                "    bridgePort: %d\n    IRQ -> gpioPortC@13\n\n"
                                "gpioPortG:\n    15 -> ch394@0\n\ngpioPortB:\n    5 -> ch394@1\n" % ch_port)
            lines.append("machine LoadPlatformDescription @%s" % rp)
            continue
        lines.append({"cs": "i @%s", "repl": "machine LoadPlatformDescription @%s",
                      "str": 'machine LoadPlatformDescriptionFromString "%s"', "cmd": "%s"}[kind] % val)
    ap_ = os.path.join(out, "agent.repl")
    open(ap_, "w").write("agent: Antmicro.Renode.Peripherals.SPI.G7250BenchAgent @ spi1\n    agentPort: %d\n\n"
                         "gpioPortA:\n    11 -> agent@0\n\ngpioPortH:\n    12 -> agent@1\n\n"
                         "gpioPortD:\n    13 -> agent@2\n\ngpioPortG:\n    2 -> agent@3\n    5 -> agent@4\n" % agent_port)
    lines += ['i @%s' % P("models", "csharp", "G7250BenchAgent.cs"),
              'machine LoadPlatformDescription @%s' % ap_]
    lines += platform_extras.all_tag_commands(exclude=("MDMA",))
    lines += [
        'nvic Frequency 275000000',
        'logFile @%s' % log,
        'logLevel 2',
        'sysbus.usart1 CreateFileBackend @%s true' % console,
        'sysbus LoadELF @%s' % elf,
        'cpu VectorTableOffset 0x%08X' % vtor,
        'cpu SP 0x%08X' % sp,
        'cpu PC 0x%08X' % (pc & ~1),
        'start',
    ]
    return "\n".join(lines) + "\n"


def evaluate(tests, console, nm):
    res = []
    for t in tests:
        name = t["name"]
        if t.get("bench"):
            lines = re.findall(r"^TEST (\S+) (PASS|FAIL)(.*)$", console, re.M)
            for n, r, d in lines:
                res.append({"name": "%s.%s" % (name, n), "result": r.lower(), "detail": d.strip()})
            if "BENCH DONE" not in console:
                res.append({"name": "%s.done" % name, "result": "fail", "detail": "no BENCH DONE line"})
        elif "expect" in t:
            ok = re.search(t["expect"], console, re.M) is not None
            res.append({"name": name, "result": "pass" if ok else "fail",
                        "detail": "" if ok else "no console match for %r" % t["expect"]})
        elif "forbid" in t:
            m = re.search(t["forbid"], console, re.M)
            res.append({"name": name, "result": "fail" if m else "pass",
                        "detail": "console matched %r" % m.group(0) if m else ""})
        else:
            res.append({"name": name, "result": "fail", "detail": "unknown test kind"})
    for r in res:
        if r["result"] == "fail" and nm:
            r["not_modelled"] = ["not modelled: %s" % x for x in nm]
    return res


def main():
    ap = argparse.ArgumentParser(description="product test-bench runner")
    ap.add_argument("elf")
    ap.add_argument("tests")
    ap.add_argument("--run-id")
    ap.add_argument("--selftest", action="store_true",
                    help="own self-test program: simple console test list and the bench-only request source")
    ap.add_argument("--host-timeout", type=float, default=600.0)
    ap.add_argument("-o", "--output", help="result file (default: <run dir>/result.json)")
    ap.add_argument("--scope", default="v1.0")
    args = ap.parse_args()
    if not os.path.isfile(bootcore.RENODE):
        sys.exit("Renode not found (install the pinned portable in tools/)")
    spec = json.load(open(args.tests))
    if spec.get("format") == "g7250-testlist":
        import testlist_run
        run_id = args.run_id or datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
        sys.exit(testlist_run.run(sys.modules[__name__], os.path.abspath(args.elf), os.path.abspath(args.tests),
                                  run_id, scope=args.scope, out_path=args.output))
    if not args.selftest:
        sys.exit("unknown test list format (expected g7250-testlist, or --selftest for own programs)")
    run_s = float(spec.get("run_s", 5))
    run_id = args.run_id or datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    out = os.path.join(OUT_ROOT, run_id)
    os.makedirs(out, exist_ok=True)
    for f in ("renode.log", "console.txt", "usart2.bin"):
        open(os.path.join(out, f), "w").close()
    resc = os.path.join(out, "run.resc")
    open(resc, "w").write(build_resc(os.path.abspath(args.elf), out, run_s, args.selftest))
    try:
        p = subprocess.run([bootcore.RENODE, "--disable-xwt", "--console", "-e", "path add @%s" % REPO,
                            "-e", "include @%s" % resc], stdin=subprocess.DEVNULL,
                           capture_output=True, text=True, timeout=args.host_timeout)
        rc = p.returncode
        open(os.path.join(out, "renode.stdout"), "w").write(p.stdout)
    except subprocess.TimeoutExpired:
        rc = "host-timeout"
    console = open(os.path.join(out, "console.txt"), errors="replace").read()
    nm = not_modelled(os.path.join(out, "renode.log"))
    tests = evaluate(spec.get("tests", []), console, nm)
    ok = bool(tests) and all(t["result"] == "pass" for t in tests)
    result = {"run_id": run_id, "image": os.path.basename(args.elf), "run_s": run_s,
              "emulator_exit": rc, "result": "pass" if ok else "fail", "tests": tests,
              "not_modelled": ["not modelled: %s" % x for x in nm],
              "console": "console.txt", "network": ["usart2.bin"]}
    json.dump(result, open(os.path.join(out, "result.json"), "w"), indent=2)
    for t in tests:
        print("  %-4s %s %s" % (t["result"].upper(), t["name"], t.get("detail", "")))
    print("PRODUCT RUN %s: %s (%d tests) -> %s" % (run_id, result["result"].upper(), len(tests),
                                                   os.path.relpath(out, REPO)))
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
