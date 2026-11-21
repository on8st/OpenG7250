// Generic STM32H72x power controller PWR at 0x58024800 (clean-room, ST RM0468
// public, PWR chapter), see docs/PWR-model.md. Replaces Renode 1.17's Python
// stub, which read PWR_CR3 as a constant and always reported ACTVOSRDY.
//
//   - CR1 (0x00, reset 0xF000C000; DBP and the other bits stored), CSR1 (0x04),
//     CR2 (0x08), CR3 (0x0C), CPUCR (0x10), D3CR (0x18), WKUPCR/FR/EPR (0x20-0x28)
//   - CR3 reset value 0x00000006 (SMPSEN | LDOEN: the SMPS supplies the LDO)
//   - supply configuration write-once: the first write after power-on sets
//     SMPSLEVEL, SMPSEXTHP, SMPSEN, LDOEN and BYPASS; later writes leave them
//     unchanged until the next power-on (a system reset, e.g. by the watchdog,
//     does not unlock them). The other CR3 bits (USB regulator) stay writable.
//   - CSR1.ACTVOSRDY: set while the active supply configuration matches the
//     board wiring; a configuration that does not match leaves it clear for
//     good (the MCU would not reach a valid core voltage). The board wiring is
//     taken from his/10: the SMPS supplies the LDO (MEDIUM), so a configuration
//     with SMPSEN, LDOEN and not BYPASS is valid; LDO alone, SMPS direct and
//     bypass are not (HYPOTHESIS, tied to his/10).
//   - CSR1.ACTVOS follows D3CR.VOS; D3CR.VOSRDY is set while ACTVOSRDY is
//   - CR3.USB33RDY follows USB33DEN; CR3.SMPSEXTRDY is set while SMPSEXTHP is
//     set in a valid configuration (the SMPS external supply is ready)
// Not modelled: voltage levels and timing, PVD/AVD thresholds, low-power modes
// (Stop/Standby), wake-up pins, the backup domain regulator.
using Antmicro.Renode.Core;
using Antmicro.Renode.Logging;
using Antmicro.Renode.Peripherals.Bus;

namespace Antmicro.Renode.Peripherals.Miscellaneous
{
    public class G7250Pwr : IDoubleWordPeripheral, IKnownSize
    {
        private const uint SupplyMask = 0x3Fu;           // SMPSLEVEL(5:4) SMPSEXTHP(3) SMPSEN(2) LDOEN(1) BYPASS(0)
        private const uint CR3Reset = 0x00000006u;
        private const uint ACTVOSRDY = 1u << 13, VOSRDY = 1u << 13;
        private const uint USB33DEN = 1u << 24, USB33RDY = 1u << 26, SMPSEXTRDY = 1u << 16;

        private readonly object sync = new object();
        private uint cr1, cr2, cr3, cpucr, d3cr, wkupcr, wkupepr;
        private bool supplyLocked, supplyValid = true;

        public G7250Pwr()
        {
            // power-on: the supply configuration is unlocked and at its reset value
            cr3 = CR3Reset;
            supplyLocked = false;
            supplyValid = true;
            Reset();
        }

        public long Size { get { return 0x400; } }
        public bool SupplyLocked { get { return supplyLocked; } }
        public bool SupplyValid { get { return supplyValid; } }

        // a system reset (watchdog, software): the supply configuration and its
        // lock survive; only a power-on (a new emulator boot) clears them
        public void Reset()
        {
            lock(sync)
            {
                cr1 = 0xF000C000u;
                cr2 = 0; cpucr = 0; wkupcr = 0; wkupepr = 0;
                d3cr = 1u << 14;                           // VOS reset value (scale 3)
                cr3 = (cr3 & SupplyMask);                  // keep the supply bits, clear the rest
            }
        }

        private static bool Valid(uint supply)
        {
            bool smps = (supply & 4u) != 0, ldo = (supply & 2u) != 0, bypass = (supply & 1u) != 0;
            return smps && ldo && !bypass;
        }

        public uint ReadDoubleWord(long offset)
        {
            lock(sync)
            {
                switch(offset)
                {
                    case 0x00: return cr1;
                    case 0x04:
                    {
                        uint v = 0;
                        if(supplyValid) v |= ACTVOSRDY;
                        v |= ((d3cr >> 14) & 3u) << 14;    // ACTVOS = VOS
                        return v;
                    }
                    case 0x08: return cr2;
                    case 0x0C:
                    {
                        uint v = cr3 | ((cr3 & USB33DEN) != 0 ? USB33RDY : 0u);
                        // SMPSEXTRDY: the SMPS external supply is ready (SMPSEXTHP in a valid configuration)
                        if(supplyValid && (cr3 & (1u << 3)) != 0) v |= SMPSEXTRDY;
                        return v;
                    }
                    case 0x10: return cpucr;
                    case 0x18: return d3cr | (supplyValid ? VOSRDY : 0u);
                    case 0x20: return wkupcr;
                    case 0x24: return 0;
                    case 0x28: return wkupepr;
                    default: return 0;
                }
            }
        }

        public void WriteDoubleWord(long offset, uint value)
        {
            lock(sync)
            {
                switch(offset)
                {
                    case 0x00: cr1 = value; break;
                    case 0x08: cr2 = value; break;
                    case 0x0C:
                    {
                        uint supply = cr3 & SupplyMask;
                        if(!supplyLocked)
                        {
                            supply = value & SupplyMask;
                            supplyLocked = true;
                            supplyValid = Valid(supply);
                            if(!supplyValid)
                                this.Log(LogLevel.Warning, "PWR: supply configuration 0x{0:X2} does not match the board wiring; ACTVOSRDY stays clear", supply);
                        }
                        else if((value & SupplyMask) != supply)
                            this.Log(LogLevel.Info, "PWR: supply configuration is write-once; write ignored until power-on");
                        cr3 = supply | (value & ~SupplyMask & ~USB33RDY & ~SMPSEXTRDY);
                        break;
                    }
                    case 0x10: cpucr = value; break;
                    case 0x18: d3cr = value & ~VOSRDY; break;
                    case 0x20: wkupcr = value; break;
                    case 0x28: wkupepr = value; break;
                }
            }
        }
    }
}
