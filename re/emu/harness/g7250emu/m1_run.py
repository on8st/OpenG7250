#!/usr/bin/env python3
"""M1 boot runner for re/emu: stock Main boot to the main loop.

Builds the Main Renode script (platform + I2C1 slave shims + SysTick clock +
observe logging + the decrypted image at its verified reset vector) and hands it
to bootcore.execute, which runs it and classifies the result (poll-hang /
idle-hang / running). See docs/M1-status.md.

Image policy (SECURITY.md): the decrypted image is resolved only through the
local image-map (imagemap.resolve), hash-verified against images/manifest.yaml.
This runner NEVER decrypts and refuses an unverified/absent image (exit 2).
"""
from __future__ import annotations

import argparse
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
REPL = os.path.join(REPO, "platforms", "g7250_main.repl")
REPL_I2C1_MOCK = os.path.join(REPO, "platforms", "g7250_main_i2c1_mock.repl")
REPL_I2C1_FUNC = os.path.join(REPO, "platforms", "g7250_main_i2c1_func.repl")
REPL_SPI6_CH394 = os.path.join(REPO, "platforms", "g7250_main_spi6_ch394.repl")
CS_CH394 = os.path.join(REPO, "models", "csharp", "G7250Ch394.cs")
CS_SPI = os.path.join(REPO, "models", "csharp", "G7250Spi.cs")
CS_BDMA = os.path.join(REPO, "models", "csharp", "G7250Bdma.cs")
REPL_SPI6_DMA = os.path.join(REPO, "platforms", "g7250_main_spi6_dma.repl")
REPL_FPGA = os.path.join(REPO, "platforms", "g7250_main_fpga.repl")
CS_GPIOE = os.path.join(REPO, "models", "csharp", "G7250Gpioe.cs")
CS_SLAVES = os.path.join(REPO, "models", "csharp", "G7250Slaves.cs")
CS_MDMA = os.path.join(REPO, "models", "csharp", "G7250Mdma.cs")
REPL_MDMA = os.path.join(REPO, "platforms", "g7250_main_mdma.repl")
CS_PA = os.path.join(REPO, "models", "csharp", "G7250PaPlant.cs")
CS_DMA = os.path.join(REPO, "models", "csharp", "G7250Dma.cs")
REPL_DMA = os.path.join(REPO, "platforms", "g7250_main_dma.repl")
CS_TIM = os.path.join(REPO, "models", "csharp", "G7250Tim.cs")
REPL_TIM = os.path.join(REPO, "platforms", "g7250_main_tim.repl")
CS_CLOCKS = os.path.join(REPO, "models", "csharp", "G7250Clocks.cs")
CS_PWR = os.path.join(REPO, "models", "csharp", "G7250Pwr.cs")
CS_IWDG = os.path.join(REPO, "models", "csharp", "G7250Iwdg.cs")
REPL_IWDG = os.path.join(REPO, "platforms", "g7250_main_iwdg_stock.repl")
CS_ADC = os.path.join(REPO, "models", "csharp", "G7250Adc.cs")
REPL_ADC = os.path.join(REPO, "platforms", "g7250_main_adc.repl")
CS_USART = os.path.join(REPO, "models", "csharp", "G7250Usart.cs")
REPL_USART2 = os.path.join(REPO, "platforms", "g7250_main_usart2.repl")
REPL_PA = os.path.join(REPO, "platforms", "g7250_main_pa_plant.repl")
OUT = os.path.join(REPO, ".private", "m1")

sys.path.insert(0, HERE)
import bootcore   # noqa: E402
import observe    # noqa: E402
import platform_extras  # noqa: E402

# Verified Main reset facts (main_unit/docs/startup.md, memory_map.md):
LOAD_BASE = 0x08020000      # app vector table / load base
INIT_MSP = 0x24050000       # initial stack pointer (top of 320 KiB AXI SRAM)
RESET_PC = 0x08024554       # reset handler
CORE_HZ = 275000000         # HCLK (flow.md); SysTick input clock for the HAL tick


def build_resc(image_path, log_path, fpga=False):
    lines = [
        'mach create "main"',
        'machine LoadPlatformDescription @%s' % REPL,
        # compile the clean-room functional slaves (EEPROM) into Renode, then
        # load the I2C1 overlay that uses the functional EEPROM at 0x50 plus
        # ACKing dummies for the rest, so the device bring-up completes at full
        # speed and settings load from the EEPROM (docs/M1-status.md).
        'i @%s' % CS_SLAVES,
        'machine LoadPlatformDescription @%s' % REPL_I2C1_FUNC,
        # functional CH394 Ethernet on SPI6 (CS# = PG15) so the forced default
        # IP is programmed into the chip (A1.12 confirmed chip-side).
        # SPI6 with DMA through DMAMUX2 and the BDMA (step 1g; docs/SPI-model.md)
        'i @%s' % CS_SPI,
        'i @%s' % CS_BDMA,
        'sysbus Unregister sysbus.spi6',
        'sysbus Unregister sysbus.bdma',
        'machine LoadPlatformDescription @%s' % REPL_SPI6_DMA,
        'i @%s' % CS_CH394,
        'machine LoadPlatformDescription @%s' % REPL_SPI6_CH394,
        # SysTick and the timers follow the RCC through the clock model (below)
        # polled software-request MDMA (docs/MDMA-model.md): the DSP's MDMA
        # copies/fills must complete, or every poll runs into its timeout.
        'i @%s' % CS_MDMA,
        'machine LoadPlatformDescription @%s' % REPL_MDMA,
        # generic DMA1/DMA2 with circular mode (docs/DMA-model.md) in place of
        # Renode's STM32DMA; dmamux1 is re-wired to the new streams.
        'i @%s' % CS_DMA,
        'sysbus Unregister sysbus.dma1',
        'sysbus Unregister sysbus.dma2',
        'machine LoadPlatformDescription @%s' % REPL_DMA,
        # generic USART2 (docs/USART-model.md): RX and TX through the DMA
        'i @%s' % CS_USART,
        'sysbus Unregister sysbus.usart2',
        'machine LoadPlatformDescription @%s' % REPL_USART2,
        # generic ADC1 (docs/ADC-model.md); the PA plant drives its inputs
        'i @%s' % CS_ADC,
        'sysbus Unregister sysbus.adcM1S2',
        'machine LoadPlatformDescription @%s' % REPL_ADC,
        # generic PWR: write-once supply configuration, ACTVOSRDY (docs/PWR-model.md)
        'i @%s' % CS_PWR,
        'sysbus Unregister sysbus.pwr',
        'machine LoadPlatformDescriptionFromString "pwr: Miscellaneous.G7250Pwr @ sysbus <0x58024800, +0x400>"',
        # own general-purpose timers TIM2-5, TIM12-14 (docs/CLOCKS-model.md)
        'i @%s' % CS_TIM,
        'sysbus Unregister sysbus.timer2', 'sysbus Unregister sysbus.timer3', 'sysbus Unregister sysbus.timer4',
        'sysbus Unregister sysbus.timer5', 'sysbus Unregister sysbus.timer12', 'sysbus Unregister sysbus.timer13',
        'sysbus Unregister sysbus.timer14',
        'machine LoadPlatformDescription @%s' % REPL_TIM,
        # clock tree from the RCC (docs/CLOCKS-model.md): timer kernel clocks, SysTick, CPU rate
        'i @%s' % CS_CLOCKS,
        # stock runs keep Renode's instruction rate (host speed for the logged boot)
        'machine LoadPlatformDescriptionFromString "clocks: Miscellaneous.G7250Clocks @ sysbus <0x5FFF0100, +0x10> { setCpuRate: false }"',
        # generic IWDG1 with the RCC_RSR reset flags (docs/IWDG-model.md)
        'i @%s' % CS_IWDG,
        'sysbus Unregister sysbus.watchdog',
        'machine LoadPlatformDescription @%s' % REPL_IWDG,
        # PA bias plant on SPI2 feeding ADC1 Idd/Vext/NTC (docs/PA-plant-model.md),
        # so the boot-time bias trim converges and the boot reaches the superloop.
        'i @%s' % CS_PA,
        'machine LoadPlatformDescription @%s' % REPL_PA,
        'sysbus.spi2.pa Publish',
    ]
    if fpga:
        # fused GPIOE+FPGA port (M3/D2): compile it, drop the base gpioPortE,
        # and swap in the port that answers the bit-banged FPGA bus internally.
        lines += [
            'i @%s' % CS_GPIOE,
            'sysbus Unregister sysbus.gpioPortE',
            'machine LoadPlatformDescription @%s' % REPL_FPGA,
        ]
    # H72x-only + unmodelled (DAC1/MDMA) blocks: tag so a touch is named/logged
    lines += platform_extras.all_tag_commands(exclude=("MDMA",))
    lines += observe.strict_logging_commands("machine", log_file=log_path)
    # the stock supply routine polls PWR_CR3 until its 1 s timeout (a flag it
    # never sees, on the hardware too; docs/PWR-model.md): log nothing for that
    # poll, or the fully logged boot needs hours of host time for 1 s
    lines += ["sysbus LogPeripheralAccess sysbus.pwr false"]
    lines += [
        'sysbus LoadBinary @%s 0x%08X' % (image_path, LOAD_BASE),
        'cpu VectorTableOffset 0x%08X' % LOAD_BASE,
        'cpu SP 0x%08X' % INIT_MSP,
        'cpu PC 0x%08X' % RESET_PC,
        'start',
    ]
    return "\n".join(lines) + "\n"


def run_boot(image_path, timeout=60.0, cadence=0.5, fpga=False):
    os.makedirs(OUT, exist_ok=True)
    log_path = os.path.join(OUT, "m1_renode.log")
    report_path = os.path.join(OUT, "m1_observe_report.md")
    verdict, n = bootcore.execute(build_resc(image_path, log_path, fpga=fpga), log_path,
                                  report_path, timeout=timeout, cadence=cadence)
    return report_path, verdict, n


def main():
    ap = argparse.ArgumentParser(description="M1 stock Main boot runner")
    ap.add_argument("--timeout", type=float, default=60.0)
    ap.add_argument("--cadence", type=float, default=0.5)
    ap.add_argument("--fpga", action="store_true", help="use the fused GPIOE+FPGA port (M3/D2; slower)")
    args = ap.parse_args()

    if not os.path.isfile(bootcore.RENODE):
        sys.exit("Renode not found at %s (install the pinned portable in tools/)" % bootcore.RENODE)

    try:
        import imagemap
        image = imagemap.resolve("main")
    except Exception as e:
        print("M1 boot: no runnable Main image yet.\n  %s\n" % e)
        print("Supply the decrypted Main image locally, outside any repo, then set")
        print("  image_map = /abs/path   in local.config (git-ignored),")
        print("and fill images/manifest.yaml image_sha256 for 'main'. See SECURITY.md.")
        sys.exit(2)

    print("M1 boot: image %s" % image)
    report, verdict, n = run_boot(image, timeout=args.timeout, cadence=args.cadence, fpga=args.fpga)
    print("MMIO accesses: %d   verdict: %s" % (n, verdict.kind if verdict else "?"))
    if verdict and verdict.detail:
        print("  %s" % verdict.detail)
    print("report: %s" % report)
    print("console: %s" % os.path.join(OUT, "main_console.txt"))
    open(os.path.join(OUT, "main_console.txt"), "w").write(
        bootcore.console_from_log(os.path.join(OUT, "m1_renode.log")))


if __name__ == "__main__":
    main()
