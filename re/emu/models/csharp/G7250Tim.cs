// General-purpose timer of the STM32H72x (TIM2-5, TIM12-14), clean-room from
// ST RM0468 (public), see docs/CLOCKS-model.md. Replaces Renode 1.17's
// STM32_Timer for these instances: that model counts from a platform frequency
// that the RCC does not change, and its prescaler handling could not be kept
// consistent with a kernel clock that changes at run time.
//   - CR1: CEN, UDIS, URS, OPM, DIR (up only, down not modelled), ARPE; DIER;
//     SR (rc_w0); EGR (UG, CCxG); CNT; PSC (preloaded, takes effect at the next
//     update event as RM0468 says); ARR (preloaded if ARPE); CCR1-4; CCMR1/2,
//     CCER stored (output compare pins not driven)
//   - counting in emulated time at kernel clock / (PSC + 1); 16 or 32 bit. The count runs on
//     the kernel clock without a pause: a register write moves the next event and does not
//     restart the prescaler (an earlier version dropped the fraction of a tick at each write,
//     76 to 194 ppm slow with an image that moves a compare register often)
//   - update event at overflow (UIF, interrupt with UIE), compare match on
//     each channel (CCxIF, interrupt with CCxIE), one-pulse mode stops at update
//   - kernel clock: SetKernelClock (the clock model calls it), default 64 MHz
// Not modelled: down/center-aligned counting, slave/trigger modes, input
// capture, PWM outputs on pins, DMA requests, repetition counter, dithering.
using System;
using Antmicro.Renode.Core;
using Antmicro.Renode.Logging;
using Antmicro.Renode.Peripherals.Bus;
using Antmicro.Renode.Peripherals.CPU;
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
        // counter state: the count and the prescaler's position (kernel clocks) at tick.Value 0
        private ulong cntAtStart, subAtStart;
        private bool running;

        public G7250Tim(IMachine machine, bool wide = false, string timName = "TIM")
        {
            this.machine = machine;
            this.wide = wide;
            label = timName;
            diag = Environment.GetEnvironmentVariable("G7250_TIM_DIAG") == timName;
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
                if(diag && ringN > 0) this.Log(LogLevel.Warning, "TIMDIAG {0} RESET at us {1}: period {2} taken {3} merged {4} backward {5} parity went wrong {6} worst flag wait us {7} register writes {8}", label, NowUs(), diagPeriod, diagTaken, diagMerged, diagBack, diagParity, diagWorstUs, diagWrites);
                if(diag && ringN > 0) { DumpRing("RESET", 6000); ringN = 0; diagPeriod = 0; diagLast = 0; diagLastUs = 0; diagSrRead = 0; }
                cr1 = cr2 = smcr = dier = sr = ccmr1 = ccmr2 = ccer = psc = pscActive = 0;
                arr = arrActive = Max;
                Array.Clear(ccr, 0, 4);
                running = false;
                cntAtStart = subAtStart = 0;
                tick.Enabled = false;
            }
            IRQ.Unset();
        }

        // the clock model sets the timer kernel clock
        public void SetKernelClock(long hz)
        {
            lock(sync)
            {
                ulong sub;
                ulong c = Position(out sub);
                kernelHz = (ulong)Math.Max(1, hz);
                Restart(c, sub);
            }
        }

        // ---- counter arithmetic: tick.Value counts kernel clocks since the last event or start ----
        private ulong Position(out ulong sub)
        {
            ulong v = running ? tick.Value : 0;      // first: reading it can bring the time up to date
            ulong k = subAtStart + v, n = pscActive + 1UL;
            sub = k % n;
            return (cntAtStart + k / n) % ((ulong)arrActive + 1);
        }

        private ulong Count()
        {
            ulong sub;
            return Position(out sub);
        }

        // ---- opt-in diagnosis (G7250_TIM_DIAG=<TIMn>): a ring of events and register accesses and
        // the time a 16-bit tick-less driver builds (one period per UIF and per CC1IF it clears,
        // the counter corrected by the parity of the period). A backward step of that time, a flag
        // set while still pending and the ring go to the log; the ring again at a reset.
        private readonly bool diag;
        private readonly string[] ring = new string[6000];
        private long ringN;
        private uint diagSrRead;
        private ulong diagPeriod, diagLast, diagLastUs;
        private int diagEarly, diagOvf, diagPending, diagDumps, diagMerged, diagTaken, diagWrites, diagBack, diagParity, diagLines;
        private ulong diagEvUs, diagWorstUs;
        private bool diagBad;
        private ulong NowUs() { return (ulong)machine.ElapsedVirtualTime.TimeElapsed.TotalMicroseconds; }
        private void Ring(string what)
        {
            ICPU cpu; string pc = machine.SystemBus.TryGetCurrentCPU(out cpu) ? " pc 0x" + cpu.PC.RawValue.ToString("X8") : "";
            ring[ringN++ % ring.Length] = NowUs() + " " + what + pc;
        }
        private void DumpRing(string why, int last)
        {
            if(diagDumps++ > 40) return;
            this.Log(LogLevel.Warning, "TIMRING {0} {1} period {2} last {3}", label, why, diagPeriod, diagLast);
            for(long i = Math.Max(Math.Max(0, ringN - ring.Length), ringN - last); i < ringN; i++)
                this.Log(LogLevel.Warning, "TIMRING {0}", ring[i % ring.Length]);
        }
        private void DiagCount(uint c)
        {
            ulong t = (diagPeriod << 15) + (c ^ ((diagPeriod & 1) << 15));
            Ring("CNT " + c + " time " + t + " base " + cntAtStart + " sr 0x" + sr.ToString("X"));
            // parity of the period against the half the counter is in, with no period flag pending
            if((sr & 3u) != 0) diagPending++;      // a read between a boundary and its interrupt
            if((sr & 3u) == 0)
            {
                bool bad = (diagPeriod & 1) != (c >= 0x8000 ? 1u : 0u);
                if(bad != diagBad && diagLines++ < 400)
                {
                    diagBad = bad; if(bad) diagParity++;
                    this.Log(LogLevel.Warning, "TIMDIAG {0} parity {1}: period {2} cnt {3} at us {4} merged {5} taken {6}", label, bad ? "WRONG" : "right", diagPeriod, c, NowUs(), diagMerged, diagTaken);
                }
            }
            if(t < diagLast)
            {
                diagBack++;
                if(diagLines++ < 400)
                    this.Log(LogLevel.Warning, "TIMDIAG {0} BACKWARD {1} -> {2} ({3}) at us {4}, last read at us {5}, sr 0x{6:X}, period {7}, cnt {8}", label, diagLast, t, diagLast - t, NowUs(), diagLastUs, sr, diagPeriod, c);
                if(diagBack <= 4) DumpRing("BACKWARD", 60);
            }
            else if(t - diagLast > 30000 && NowUs() - diagLastUs < 20000 && diagLines++ < 400)
                this.Log(LogLevel.Warning, "TIMDIAG {0} FORWARD {1} -> {2} at us {3}, last read at us {4}, sr 0x{5:X}, period {6}, cnt {7}", label, diagLast, t, NowUs(), diagLastUs, sr, diagPeriod, c);
            diagLast = t; diagLastUs = NowUs();
        }

        // count and prescaler position set (start, stop, UG, CNT or ARR below the count, new clock)
        private void Restart(ulong cnt, ulong sub)
        {
            cntAtStart = cnt; subAtStart = sub;
            running = (cr1 & CEN) != 0;
            tick.Enabled = false;
            if(!running) return;
            tick.Frequency = Math.Max(1UL, kernelHz);
            tick.Value = 0;
            tick.Limit = ClocksToEvent(cnt, sub);
            tick.Enabled = true;
        }

        // the next event moved, the count untouched
        private void Reschedule()
        {
            if(!running) return;
            ulong sub, c = Position(out sub);
            ulong v = tick.Value;
            tick.Limit = v + ClocksToEvent(c, sub);
        }

        private ulong ClocksToEvent(ulong c, ulong sub)
        {
            ulong n = pscActive + 1UL;
            return Math.Max(1UL, NextEventTicks(c) * n - Math.Min(sub, n - 1));
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
                ulong c = (cntAtStart + (subAtStart + tick.Limit) / (pscActive + 1UL)) % period;
                bool update = c == 0;
                if(diag && c == 0 && diagOvf++ % 128 == 0)
                    this.Log(LogLevel.Warning, "TIMDIAG {0} OVF {1} at ns {2} writes {3} reads with a flag pending {4} backward {5} early entries {6}", label, diagOvf - 1, (ulong)(machine.ElapsedVirtualTime.TimeElapsed.TotalMilliseconds * 1000000.0), diagWrites, diagPending, diagBack, diagEarly);
                if(diag && (c == 0 || c == 0x8000))
                {
                    diagEvUs = diagEvUs == 0 ? NowUs() : diagEvUs;
                    uint f = c == 0 ? 1u : 2u;
                    if((sr & f) != 0) { diagMerged++; if(diagLines++ < 400) this.Log(LogLevel.Warning, "TIMDIAG {0} flag 0x{1:X} set while still pending (event lost to the driver) at us {2}, merged {3}", label, f, NowUs(), diagMerged); }
                }
                if(diag) Ring("EVENT c " + c + " limit " + tick.Limit + " value " + tick.Value + " sr 0x" + sr.ToString("X"));
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
                if((cr1 & CEN) == 0) Restart(c, 0);
                else { cntAtStart = c; subAtStart = 0; tick.Limit = ClocksToEvent(c, 0); }
                if(diag) Ring("EVENT done sr 0x" + sr.ToString("X"));
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
            if(!diag) return Read(offset);
            lock(sync)
            {
                uint v = Read(offset);
                if(offset == 0x24) DiagCount(v);
                else Ring("R 0x" + offset.ToString("X2") + " = 0x" + v.ToString("X"));
                if(offset == 0x10)
                {
                    diagSrRead = v;
                    // one period flag pending, not merged, and the counter still in the half before its event
                    uint f = v & 3u;
                    uint cn = (uint)Count();
                    if((f == 1u && cn >= 0x8000 || f == 2u && cn < 0x8000) && diagEvUs != 0 && NowUs() - diagEvUs < 32000)
                    {
                        diagEarly++;
                        this.Log(LogLevel.Warning, "TIMDIAG {0} EARLY entry: SR 0x{1:X} with CNT {2} at us {3}", label, v, cn, NowUs());
                    }
                }
                return v;
            }
        }
        private uint Read(long offset)
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
                ulong sub;
                ulong c = Position(out sub);
                bool hard = false;                        // the count or the prescaler itself is set
                uint srBefore = sr;
                diagWrites++;
                if(diag && offset == 0x00 && (value & CEN) != 0 && (cr1 & CEN) == 0) { diagPeriod = 0; diagLast = 0; diagLastUs = NowUs(); diagMerged = diagTaken = 0; diagWrites = 0; diagEvUs = 0; }
                if(diag) Ring("W 0x" + offset.ToString("X2") + " = 0x" + value.ToString("X") + " cnt " + c + " sr 0x" + sr.ToString("X"));
                switch(offset)
                {
                    case 0x00:
                        hard = ((cr1 ^ value) & CEN) != 0;
                        cr1 = value & 0x3FF;
                        if((value & 0x10) != 0) this.Log(LogLevel.Warning, "not modelled: {0}/CR1 DIR (down counting)", label);
                        break;
                    case 0x04: cr2 = value; break;
                    case 0x08: smcr = value; if(value != 0) this.Log(LogLevel.Warning, "not modelled: {0}/SMCR (slave mode)", label); break;
                    case 0x0C: dier = value & 0x5F; break;
                    case 0x10:
                        sr &= value;                                  // rc_w0
                        if(diag)
                        {
                            uint gone = srBefore & ~sr;
                            if((gone & ~diagSrRead & 3u) != 0) DumpRing("LOST flag 0x" + (gone & ~diagSrRead).ToString("X"), 100);
                            if((gone & 1u) != 0) { diagPeriod++; diagTaken++; }
                            if((gone & 2u) != 0) { diagPeriod++; diagTaken++; }
                            if((gone & 3u) != 0)
                            {
                                ulong late = NowUs() - diagEvUs; diagEvUs = 0;
                                if(late > diagWorstUs) diagWorstUs = late;
                                if(late > 200 && diagLines++ < 400)
                                    this.Log(LogLevel.Warning, "TIMDIAG {0} period flag 0x{1:X} cleared {2} us after its first event, at us {3}, period now {4}, cnt {5}", label, gone & 3u, late, NowUs(), diagPeriod, c);
                            }
                        }
                        break;
                    case 0x14:
                        if((value & 1) != 0)
                        {
                            // UG: reinitialise the counter and transfer the preloads; an update
                            // event (UIF) unless URS or UDIS
                            pscActive = psc; arrActive = arr; c = 0; sub = 0; hard = true;
                            if((cr1 & (URS | UDIS)) == 0) sr |= 1;
                        }
                        sr |= value & 0x1E;
                        break;
                    case 0x18: ccmr1 = value; break;
                    case 0x1C: ccmr2 = value; break;
                    case 0x20: ccer = value; break;
                    case 0x24: c = value & Max; hard = true; break;
                    case 0x28: psc = value & 0xFFFF; break;
                    case 0x2C: arr = value & Max; if((cr1 & ARPE) == 0) arrActive = arr; break;
                    case 0x34: ccr[0] = value & Max; break;
                    case 0x38: ccr[1] = value & Max; break;
                    case 0x3C: ccr[2] = value & Max; break;
                    case 0x40: ccr[3] = value & Max; break;
                }
                if(c > arrActive) { c = arrActive; hard = true; }
                if(hard) Restart(c, sub);
                else Reschedule();
            }
            UpdateIrq();
        }
    }
}
