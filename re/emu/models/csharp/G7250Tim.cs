// General-purpose timer of the STM32H72x (TIM2-5, TIM12-14), clean-room from
// ST RM0468 (public), see docs/CLOCKS-model.md. Replaces Renode 1.17's
// STM32_Timer for these instances: that model counts from a platform frequency
// that the RCC does not change, and its prescaler handling could not be kept
// consistent with a kernel clock that changes at run time.
//   - CR1: CEN, UDIS, URS, OPM, DIR (up only, down not modelled), ARPE; DIER;
//     SR (rc_w0); EGR (UG, CCxG); CNT; PSC (preloaded, takes effect at the next
//     update event as RM0468 says); ARR (preloaded if ARPE); CCR1-4; CCMR1/2,
//     CCER stored (output compare pins not driven)
//   - counting in emulated time at kernel clock / (PSC + 1); 16 or 32 bit
//   - update event at overflow (UIF, interrupt with UIE), compare match on
//     each channel (CCxIF, interrupt with CCxIE), one-pulse mode stops at update
//   - kernel clock: SetKernelClock (the clock model calls it), default 64 MHz
// Not modelled: down/center-aligned counting, slave/trigger modes, input
// capture, PWM outputs on pins, DMA requests, repetition counter, dithering.
using System;
using Antmicro.Renode.Core;
using Antmicro.Renode.Logging;
using Antmicro.Renode.Peripherals.Bus;
using Antmicro.Renode.Time;

namespace Antmicro.Renode.Peripherals.Timers
{
    public class G7250Tim : IDoubleWordPeripheral, IWordPeripheral, IKnownSize
    {
        private const uint CEN = 1, UDIS = 2, URS = 4, OPM = 8, ARPE = 0x80;
        private readonly IMachine machine;
        private readonly object sync = new object();
        private readonly bool wide;
        private readonly string label;
        private readonly LimitTimer tick;
        private ulong kernelHz = 64000000;
        private uint cr1, cr2, smcr, dier, sr, ccmr1, ccmr2, ccer, psc, pscActive, arr, arrActive;
        private readonly uint[] ccr = new uint[4];
        // counter state: cnt0 at time t0 (emulated us * 1e6 resolution via ticks)
        private ulong cntAtStart;
        private ulong startTicks;           // tick count of the LimitTimer clock at the last (re)start
        private bool running;

        public G7250Tim(IMachine machine, bool wide = false, string timName = "TIM")
        {
            this.machine = machine;
            this.wide = wide;
            label = timName;
            IRQ = new GPIO();
            tick = new LimitTimer(machine.ClockSource, kernelHz, this, "tim", 1UL << 62,
                                  direction: Direction.Ascending, enabled: false,
                                  workMode: WorkMode.Periodic, eventEnabled: true);
            tick.LimitReached += Event;
            Reset();
        }

        public GPIO IRQ { get; private set; }
        public long Size { get { return 0x400; } }
        private uint Max { get { return wide ? 0xFFFFFFFFu : 0xFFFFu; } }

        public void Reset()
        {
            lock(sync)
            {
                cr1 = cr2 = smcr = dier = sr = ccmr1 = ccmr2 = ccer = psc = pscActive = 0;
                arr = arrActive = Max;
                Array.Clear(ccr, 0, 4);
                running = false;
                cntAtStart = 0;
                tick.Enabled = false;
            }
            IRQ.Unset();
        }

        // the clock model sets the timer kernel clock
        public void SetKernelClock(long hz)
        {
            lock(sync)
            {
                ulong c = Count();
                kernelHz = (ulong)Math.Max(1, hz);
                Restart(c);
            }
        }

        // ---- counter arithmetic in prescaled ticks ----
        private ulong Elapsed()
        {
            // the LimitTimer counts kernel / (PSC + 1) ticks upwards from 0 since the restart
            return running ? tick.Value : 0;
        }

        private ulong Count()
        {
            if(!running) return cntAtStart;
            ulong period = (ulong)arrActive + 1;
            return (cntAtStart + Elapsed()) % period;
        }

        private void Restart(ulong cnt)
        {
            cntAtStart = cnt;
            running = (cr1 & CEN) != 0;
            tick.Enabled = false;
            if(!running) return;
            tick.Frequency = Math.Max(1UL, kernelHz / (pscActive + 1UL));
            tick.Value = 0;
            tick.Limit = Math.Max(1UL, NextEventTicks(cnt));
            tick.Enabled = true;
        }

        // ticks from count c to the next update or enabled compare match
        private ulong NextEventTicks(ulong c)
        {
            ulong period = (ulong)arrActive + 1;
            ulong best = period - c;                       // to the overflow (update event)
            for(int i = 0; i < 4; i++)
            {
                if((dier & (2u << i)) == 0) continue;
                ulong m = ccr[i];
                if(m > arrActive) continue;
                ulong d = m > c ? m - c : m + period - c;
                if(d > 0 && d < best) best = d;
            }
            return best;
        }

        private void Event()
        {
            bool irq;
            lock(sync)
            {
                ulong period = (ulong)arrActive + 1;
                ulong c = (cntAtStart + tick.Limit) % period;
                bool update = c == 0;
                for(int i = 0; i < 4; i++) if((ulong)ccr[i] == c && c != 0) sr |= 2u << i;
                if(update)
                {
                    if((ccr[0] == 0) || (ccr[1] == 0) || (ccr[2] == 0) || (ccr[3] == 0))
                        for(int i = 0; i < 4; i++) if(ccr[i] == 0) sr |= 2u << i;
                    if((cr1 & UDIS) == 0)
                    {
                        sr |= 1;
                        pscActive = psc;                       // preload transfer at the update event
                        arrActive = (cr1 & ARPE) != 0 ? arr : arrActive;
                    }
                    if((cr1 & OPM) != 0) cr1 &= ~CEN;
                }
                Restart(c);
                irq = (sr & dier & 0x1F) != 0;
            }
            IRQ.Set(irq);
        }

        private void UpdateIrq()
        {
            bool on;
            lock(sync) on = (sr & dier & 0x1F) != 0;
            IRQ.Set(on);
        }

        public ushort ReadWord(long offset) { return (ushort)ReadDoubleWord(offset & ~3L); }
        public void WriteWord(long offset, ushort value) { WriteDoubleWord(offset & ~3L, value); }

        public uint ReadDoubleWord(long offset)
        {
            lock(sync)
            {
                switch(offset)
                {
                    case 0x00: return cr1;
                    case 0x04: return cr2;
                    case 0x08: return smcr;
                    case 0x0C: return dier;
                    case 0x10: return sr;
                    case 0x18: return ccmr1;
                    case 0x1C: return ccmr2;
                    case 0x20: return ccer;
                    case 0x24: return (uint)Count();
                    case 0x28: return psc;
                    case 0x2C: return arr;
                    case 0x34: return ccr[0];
                    case 0x38: return ccr[1];
                    case 0x3C: return ccr[2];
                    case 0x40: return ccr[3];
                    default: return 0;
                }
            }
        }

        public void WriteDoubleWord(long offset, uint value)
        {
            lock(sync)
            {
                ulong c = Count();
                switch(offset)
                {
                    case 0x00:
                        cr1 = value & 0x3FF;
                        if((value & 0x10) != 0) this.Log(LogLevel.Warning, "not modelled: {0}/CR1 DIR (down counting)", label);
                        break;
                    case 0x04: cr2 = value; break;
                    case 0x08: smcr = value; if(value != 0) this.Log(LogLevel.Warning, "not modelled: {0}/SMCR (slave mode)", label); break;
                    case 0x0C: dier = value & 0x5F; break;
                    case 0x10: sr &= value; break;                   // rc_w0
                    case 0x14:
                        if((value & 1) != 0)
                        {
                            // UG: reinitialise the counter and transfer the preloads; an update
                            // event (UIF) unless URS or UDIS
                            pscActive = psc; arrActive = arr; c = 0;
                            if((cr1 & (URS | UDIS)) == 0) sr |= 1;
                        }
                        sr |= value & 0x1E;
                        break;
                    case 0x18: ccmr1 = value; break;
                    case 0x1C: ccmr2 = value; break;
                    case 0x20: ccer = value; break;
                    case 0x24: c = value & Max; break;
                    case 0x28: psc = value & 0xFFFF; break;
                    case 0x2C: arr = value & Max; if((cr1 & ARPE) == 0) arrActive = arr; break;
                    case 0x34: ccr[0] = value & Max; break;
                    case 0x38: ccr[1] = value & Max; break;
                    case 0x3C: ccr[2] = value & Max; break;
                    case 0x40: ccr[3] = value & Max; break;
                }
                Restart(Math.Min(c, (ulong)arrActive));
            }
            UpdateIrq();
        }
    }
}
