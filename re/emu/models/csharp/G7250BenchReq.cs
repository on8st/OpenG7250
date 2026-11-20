// Bench-only DMA request source (test scaffolding, not a G7250 device).
// Writing N to offset 0 pulses the output N times; the bench platform
// connects it to a DMAMUX1 request input so the DMA self-test can drive
// requests without depending on any peripheral model. Never part of the
// product or stock platforms.
using Antmicro.Renode.Core;
using Antmicro.Renode.Peripherals.Bus;

namespace Antmicro.Renode.Peripherals.Miscellaneous
{
    public class G7250BenchReq : IDoubleWordPeripheral, IKnownSize
    {
        private uint pulses;

        public G7250BenchReq() { IRQ = new GPIO(); }

        public GPIO IRQ { get; private set; }
        public long Size { get { return 0x10; } }

        public void Reset() { pulses = 0; IRQ.Unset(); }

        public uint ReadDoubleWord(long offset) { return offset == 4 ? pulses : 0; }

        public void WriteDoubleWord(long offset, uint value)
        {
            if(offset != 0) return;
            for(uint i = 0; i < value; i++) { IRQ.Set(true); IRQ.Set(false); pulses++; }
        }
    }
}
