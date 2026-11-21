// Clock tree of the STM32H72x (clean-room, ST RM0468 RCC chapter), see
// docs/CLOCKS-model.md. Renode 1.17's STM32 timers count from a fixed input
// frequency given in the platform (250 MHz) and SysTick from a fixed NVIC
// frequency; neither follows the RCC. This model follows the RCC registers the
// firmware writes and sets, as the hardware derives them:
//   - sys_ck from CFGR.SW: HSI (64 MHz / HSIDIV), CSI (4 MHz), HSE (hseHz), PLL1_P
//     (source from PLLCKSELR, DIVM1, DIVN1, DIVP1, FRACN1 when PLL1FRACEN)
//   - the core clock (D1CPRE) for SysTick (NVIC frequency) and the CPU's
//     instruction rate (PerformanceInMips = core MHz, one instruction a cycle)
//   - hclk (HPRE), the APB clocks (D2PPRE1, D2PPRE2) and the timer kernel clocks
//     with the TIMPRE rule: TIMPRE = 0: the APB clock if its prescaler is 1, else
//     twice it; TIMPRE = 1: hclk if the prescaler is 1, 2 or 4, else four times
//     the APB clock; applied to TIM2-7, 12-14 (APB1) and TIM1, 8, 15-17 (APB2)
// The RCC model itself (Renode's) keeps the register file and the ready flags;
// this model only reads the written values through the hook of the watchdog
// model (G7250Iwdg), which owns the RCC hooks, and recomputes on every write.
// Not modelled: clock switching delays, the HSI/CSI calibration, PLL2/PLL3
// outputs, the per-peripheral kernel clock selections beyond the timers, LSE/LSI.
using System;
using System.Collections.Generic;
using Antmicro.Renode.Core;
using Antmicro.Renode.Logging;
using Antmicro.Renode.Peripherals;
using Antmicro.Renode.Peripherals.Bus;

namespace Antmicro.Renode.Peripherals.Miscellaneous
{
    public class G7250Clocks : IDoubleWordPeripheral, IKnownSize
    {
        private readonly IMachine machine;
        private readonly long hseHz;
        private readonly bool setCpuRate;
        private readonly Dictionary<long, uint> reg = new Dictionary<long, uint>();
        private static readonly string[] Apb1Timers = { "timer2", "timer3", "timer4", "timer5", "timer6", "timer7", "timer12", "timer13", "timer14" };
        private static readonly string[] Apb2Timers = { "timer1", "timer8", "timer15", "timer16", "timer17" };

        public G7250Clocks(IMachine machine, long hseHz = 25000000, bool setCpuRate = true)
        {
            this.machine = machine;
            this.hseHz = hseHz;
            this.setCpuRate = setCpuRate;
            ResetValues();
            Apply();
        }

        public long Size { get { return 0x10; } }
        public uint ReadDoubleWord(long offset) { return offset == 0 ? (uint)(CoreHz / 1000) : offset == 4 ? (uint)(Apb1TimerHz / 1000) : 0u; }
        public void WriteDoubleWord(long offset, uint value) { }
        public void Reset() { ResetValues(); Apply(); }

        public long SysHz { get; private set; }
        public long CoreHz { get; private set; }
        public long HclkHz { get; private set; }
        public long Apb1TimerHz { get; private set; }
        public long Apb2TimerHz { get; private set; }

        private void ResetValues()
        {
            reg[0x00] = 0x00000025;     // CR: HSION, HSIRDY, HSIDIV /1
            reg[0x10] = 0;              // CFGR: SW = HSI, TIMPRE = 0
            reg[0x18] = 0;              // D1CFGR
            reg[0x1C] = 0;              // D2CFGR
            reg[0x28] = 0x02020200;     // PLLCKSELR reset value
            reg[0x2C] = 0x01FF0000;     // PLLCFGR reset value
            reg[0x30] = 0x01010280;     // PLL1DIVR reset value
            reg[0x34] = 0;              // PLL1FRACR
        }

        // called by the RCC write hook (G7250Iwdg) for every RCC write
        public void RccWrite(long offset, uint value)
        {
            if(!reg.ContainsKey(offset)) return;
            reg[offset] = value;
            Apply();
        }

        private static int CoreDiv(uint v)   // D1CPRE / HPRE encoding
        {
            v &= 0xF;
            if(v < 8) return 1;
            int[] d = { 2, 4, 8, 16, 64, 128, 256, 512 };
            return d[v - 8];
        }

        private static int ApbDiv(uint v)    // D2PPREx encoding
        {
            v &= 7;
            return v < 4 ? 1 : 1 << (int)(v - 3);
        }

        private long Pll1P()
        {
            uint sel = reg[0x28];
            long src;
            switch(sel & 3) { case 0: src = HsiHz(); break; case 1: src = 4000000; break; case 2: src = hseHz; break; default: return 0; }
            uint m = (sel >> 4) & 0x3F;
            if(m == 0) return 0;
            uint div = reg[0x30];
            double n = (div & 0x1FF) + 1;
            if((reg[0x2C] & 1) != 0) n += ((reg[0x34] >> 3) & 0x1FFF) / 8192.0;   // PLL1FRACEN
            double vco = src / (double)m * n;
            uint p = ((div >> 9) & 0x7F) + 1;
            return (long)(vco / p);
        }

        private long HsiHz() { return 64000000L >> (int)((reg[0x00] >> 3) & 3); }

        private void Apply()
        {
            uint cfgr = reg[0x10], d1 = reg[0x18], d2 = reg[0x1C];
            switch(cfgr & 7)
            {
                case 0: SysHz = HsiHz(); break;
                case 1: SysHz = 4000000; break;
                case 2: SysHz = hseHz; break;
                case 3: SysHz = Pll1P(); break;
                default: SysHz = HsiHz(); break;
            }
            if(SysHz <= 0) SysHz = HsiHz();
            CoreHz = SysHz / CoreDiv(d1 >> 8);
            HclkHz = CoreHz / CoreDiv(d1);
            int p1 = ApbDiv(d2 >> 4), p2 = ApbDiv(d2 >> 8);
            bool timpre = (cfgr & (1u << 15)) != 0;
            Apb1TimerHz = TimerHz(HclkHz, p1, timpre);
            Apb2TimerHz = TimerHz(HclkHz, p2, timpre);
            Set("nvic", CoreHz);
            foreach(var t in Apb1Timers) Set("sysbus." + t, Apb1TimerHz);
            foreach(var t in Apb2Timers) Set("sysbus." + t, Apb2TimerHz);
            foreach(var cpu in setCpuRate ? machine.SystemBus.GetCPUs() : new Antmicro.Renode.Peripherals.CPU.ICPU[0])
            {
                var pm = cpu.GetType().GetProperty("PerformanceInMips");
                if(pm != null) pm.SetValue(cpu, Convert.ChangeType(Math.Max(1L, CoreHz / 1000000), pm.PropertyType));
            }
            this.Log(LogLevel.Info, "clocks: sys {0} Hz, core {1} Hz, hclk {2} Hz, APB1 timers {3} Hz, APB2 timers {4} Hz",
                     SysHz, CoreHz, HclkHz, Apb1TimerHz, Apb2TimerHz);
        }

        private static long TimerHz(long hclk, int apbDiv, bool timpre)
        {
            long pclk = hclk / apbDiv;
            if(!timpre) return apbDiv == 1 ? pclk : 2 * pclk;
            return apbDiv <= 4 ? hclk : 4 * pclk;
        }

        private void Set(string name, long hz)
        {
            IPeripheral p;
            if(!machine.TryGetByName(name, out p)) return;
            var f = p.GetType().GetProperty("Frequency");
            if(f == null || !f.CanWrite) return;
            try { f.SetValue(p, Convert.ChangeType(hz, f.PropertyType)); }
            catch(Exception e) { this.Log(LogLevel.Warning, "clocks: cannot set {0}: {1}", name, e.Message); }
        }
    }
}
