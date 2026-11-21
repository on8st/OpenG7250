// DWT (Arm v7-M Architecture Reference Manual, Data Watchpoint and Trace; ST RM0468
// for the Cortex-M7 of the STM32H72x): the cycle counter only. Replaces Renode's DWT,
// whose counter runs at a fixed platform frequency (250 MHz in stm32h7.repl) instead
// of the core clock; firmware that times with CYCCNT (the product image's 5 ms polls,
// its flash timeouts) then ran 3.9 times fast at the 64 MHz reset clock.
//   - DWT_CTRL (0x000): CYCCNTENA (bit 0) starts and stops the counter; NUMCOMP read
//     as 4 (bits 31:28); other bits stored
//   - DWT_CYCCNT (0x004): counts core clock cycles while enabled, in emulated time;
//     a write sets it; wraps at 32 bits
//   - the core clock comes from the clock model (SetKernelClock), 64 MHz at reset
//   - DWT_LAR (0xFB0) writes accepted (the lock is not modelled); comparators, PC
//     sampling, CPI/exception/sleep/LSU/fold counters: stored, not counted
using System;
using System.Collections.Generic;
using Antmicro.Renode.Core;
using Antmicro.Renode.Logging;
using Antmicro.Renode.Peripherals.Bus;
using Antmicro.Renode.Peripherals.Timers;
using Antmicro.Renode.Time;

namespace Antmicro.Renode.Peripherals.Miscellaneous
{
    public class G7250Dwt : IDoubleWordPeripheral, IKnownSize
    {
        private readonly object sync = new object();
        private readonly IMachine machine;
        private readonly Dictionary<long, uint> regs = new Dictionary<long, uint>();
        private long coreHz = 64000000;
        private uint ctrl = 0x40000000;             // NUMCOMP = 4
        private double baseCycles;                  // CYCCNT at baseUs
        private double baseUs;

        public G7250Dwt(IMachine machine)
        {
            this.machine = machine;
        }

        public long Size { get { return 0x1000; } }

        public void Reset()
        {
            lock(sync) { ctrl = 0x40000000; baseCycles = 0; baseUs = NowUs(); regs.Clear(); }
        }

        // the machine's virtual time (a timer of the clock source read from inside the core's
        // register access stalled the emulation at start-up)
        private double NowUs() { return machine.LocalTimeSource.ElapsedVirtualTime.TotalMicroseconds; }

        private double Cycles()
        {
            if((ctrl & 1) == 0) return baseCycles;
            return baseCycles + (NowUs() - baseUs) * coreHz / 1e6;
        }

        // the clock model sets the core clock; the count so far is kept
        public void SetKernelClock(long hz)
        {
            lock(sync) { baseCycles = Cycles(); baseUs = NowUs(); coreHz = Math.Max(1, hz); }
        }

        public uint ReadDoubleWord(long offset)
        {
            lock(sync)
            {
                if(offset == 0x000) return ctrl;
                if(offset == 0x004) return (uint)((ulong)Cycles() & 0xFFFFFFFF);
                uint v;
                return regs.TryGetValue(offset, out v) ? v : 0;
            }
        }

        public void WriteDoubleWord(long offset, uint value)
        {
            lock(sync)
            {
                if(offset == 0x000)
                {
                    baseCycles = Cycles(); baseUs = NowUs();
                    ctrl = (value & 0x0FFFFFFF) | 0x40000000;
                }
                else if(offset == 0x004) { baseCycles = value; baseUs = NowUs(); }
                else regs[offset] = value;
            }
        }
    }
}
