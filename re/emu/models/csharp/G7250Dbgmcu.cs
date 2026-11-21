// STM32H72x debug MCU block DBGMCU at 0x5C001000 (clean-room, ST RM0468
// public), see docs/DBGMCU-model.md.
//   - IDCODE (0x000): DEV_ID 0x483 (STM32H72x/H73x, his/10) and a REV_ID; the
//     revision on the radio is not known, so the REV_ID is a parameter
//     (default 0x1001, HYPOTHESIS)
//   - CR (0x004), APB3FZ1 (0x034), APB1LFZ1 (0x03C), APB2FZ1 (0x04C),
//     APB4FZ1 (0x054): read/write storage with the RM0468 writable bits
// Not modelled: the effect of the freeze and low-power debug bits (they act
// only while a debugger halts the core; the emulator has no debugger attached),
// the trace and debug clock enables.
using Antmicro.Renode.Core;
using Antmicro.Renode.Logging;
using Antmicro.Renode.Peripherals.Bus;

namespace Antmicro.Renode.Peripherals.Miscellaneous
{
    public class G7250Dbgmcu : IDoubleWordPeripheral, IKnownSize
    {
        private readonly uint idcode;
        private uint cr, apb3, apb1l, apb2, apb4;

        public G7250Dbgmcu(uint revisionId = 0x1001, uint deviceId = 0x483)
        {
            idcode = (revisionId << 16) | (deviceId & 0xFFF);
            Reset();
        }

        public long Size { get { return 0x1000; } }

        public void Reset() { cr = apb3 = apb1l = apb2 = apb4 = 0; }

        public uint ReadDoubleWord(long offset)
        {
            switch(offset)
            {
                case 0x000: return idcode;
                case 0x004: return cr;
                case 0x034: return apb3;
                case 0x03C: return apb1l;
                case 0x04C: return apb2;
                case 0x054: return apb4;
                default:
                    this.Log(LogLevel.Warning, "not modelled: DBGMCU/offset 0x{0:X3} (read)", offset);
                    return 0;
            }
        }

        public void WriteDoubleWord(long offset, uint value)
        {
            switch(offset)
            {
                case 0x000: return;                                   // IDCODE is read-only
                case 0x004: cr = value & 0x30E00187u; return;         // DBGSLEEP/STOP/STBY_D1, D3 domain bits, D1/D3DBGCKEN, TRGOEN
                case 0x034: apb3 = value & (1u << 6); return;         // WWDG1
                case 0x03C: apb1l = value & 0x00E003FFu; return;      // TIM2..7/12..14, LPTIM1, I2C1..3 (I2C5)
                case 0x04C: apb2 = value & 0x20070003u; return;       // TIM1, TIM8, TIM15..17, HRTIM-class bits
                case 0x054: apb4 = value & 0x0005FC00u; return;       // I2C4, LPTIM2..5, RTC, IWDG1 (bit 18)
                default:
                    this.Log(LogLevel.Warning, "not modelled: DBGMCU/offset 0x{0:X3} (write)", offset);
                    return;
            }
        }
    }
}
