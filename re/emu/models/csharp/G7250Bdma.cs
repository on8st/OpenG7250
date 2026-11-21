// Generic STM32H7 basic DMA (BDMA, D3 domain), clean-room from ST RM0468
// (public), see docs/BDMA-model.md. Step 1g of docs/PRODUCT-TESTBENCH-PLAN.md.
// Replaces Renode 1.17's STM32LDMA at 0x58025400.
//   - 8 channels: ISR (GIF/TCIF/HTIF/TEIF per channel), IFCR, CCR, CNDTR,
//     CPAR, CM0AR, CM1AR at 0x08 + 0x14 * x
//   - peripheral-to-memory (DIR = 0) and memory-to-peripheral (DIR = 1): one
//     item per request edge from DMAMUX2 (input number = channel), edges
//     during a transfer queued; a request pending when the channel is enabled
//     is served; memory-to-memory (MEM2MEM): the whole transfer when enabled
//   - PSIZE and MSIZE separately: an item is read at the source size and
//     written at the destination size (zero-extended or truncated); PINC/MINC
//   - circular mode (CNDTR and addresses reload), double-buffer mode (CT
//     toggles); TCIF, HTIF, TEIF; interrupt per channel to its NVIC line
//   - the BDMA reaches only the D3 domain (SRAM4 0x38000000, backup SRAM
//     0x38800000, D3 peripherals 0x58000000-0x5FFFFFFF): any other address
//     gives TEIF and disables the channel, as on the hardware
// Not modelled: transfer timing, channel priorities, secure/privileged
// attributes.
using System;
using System.Collections.Generic;
using Antmicro.Renode.Core;
using Antmicro.Renode.Logging;
using Antmicro.Renode.Peripherals.Bus;

namespace Antmicro.Renode.Peripherals.DMA
{
    public class G7250Bdma : IDoubleWordPeripheral, IKnownSize, IGPIOReceiver, INumberedGPIOOutput
    {
        private const int Channels = 8;
        private const uint EN = 1u << 0, TCIE = 1u << 1, HTIE = 1u << 2, TEIE = 1u << 3, DIR = 1u << 4,
                           CIRC = 1u << 5, PINC = 1u << 6, MINC = 1u << 7, MEM2MEM = 1u << 14,
                           DBM = 1u << 15, CT = 1u << 16;
        private const uint GIF = 1, TCIF = 2, HTIF = 4, TEIF = 8;

        private readonly IMachine machine;
        private readonly object sync = new object();
        private readonly Dictionary<int, IGPIO> irq = new Dictionary<int, IGPIO>();
        private readonly uint[] ccr = new uint[Channels], ndtr = new uint[Channels], par = new uint[Channels],
                                m0ar = new uint[Channels], m1ar = new uint[Channels], flags = new uint[Channels];
        private readonly uint[] initial = new uint[Channels], cur = new uint[Channels];
        private readonly ulong[] curP = new ulong[Channels], curM = new ulong[Channels];
        private readonly bool[] level = new bool[Channels], busy = new bool[Channels];
        private readonly int[] pending = new int[Channels];

        public G7250Bdma(IMachine machine)
        {
            this.machine = machine;
            for(int i = 0; i < Channels; i++) irq[i] = new GPIO();
            Connections = irq;
            Reset();
        }

        public long Size { get { return 0x400; } }
        public IReadOnlyDictionary<int, IGPIO> Connections { get; private set; }

        public void Reset()
        {
            lock(sync)
            {
                foreach(var a in new[] { ccr, ndtr, par, m0ar, m1ar, flags, initial, cur }) Array.Clear(a, 0, Channels);
                for(int i = 0; i < Channels; i++) { level[i] = busy[i] = false; pending[i] = 0; }
            }
            foreach(var g in irq.Values) g.Unset();
        }

        private static bool D3(ulong a)
        {
            return (a >= 0x38000000 && a < 0x38010000) || (a >= 0x38800000 && a < 0x38801000) || (a >= 0x58000000 && a < 0x60000000);
        }

        public uint ReadDoubleWord(long offset)
        {
            lock(sync)
            {
                if(offset == 0x00)
                {
                    uint v = 0;
                    for(int c = 0; c < Channels; c++)
                    {
                        uint f = flags[c] & (TCIF | HTIF | TEIF);
                        if(f != 0) f |= GIF;
                        v |= f << (4 * c);
                    }
                    return v;
                }
                if(offset == 0x04) return 0;
                int ch, reg;
                if(!Decode(offset, out ch, out reg)) return 0;
                switch(reg)
                {
                    case 0x00: return ccr[ch];
                    case 0x04: return (ccr[ch] & EN) != 0 ? cur[ch] : ndtr[ch];
                    case 0x08: return par[ch];
                    case 0x0C: return m0ar[ch];
                    case 0x10: return m1ar[ch];
                }
                return 0;
            }
        }

        public void WriteDoubleWord(long offset, uint value)
        {
            lock(sync)
            {
                if(offset == 0x04)
                {
                    for(int c = 0; c < Channels; c++)
                    {
                        uint v = (value >> (4 * c)) & 0xF;
                        if((v & GIF) != 0) flags[c] = 0; else flags[c] &= ~v;
                        UpdateIrq(c);
                    }
                    return;
                }
                if(offset == 0x00) return;
                int ch, reg;
                if(!Decode(offset, out ch, out reg))
                {
                    this.Log(LogLevel.Warning, "not modelled: BDMA/offset 0x{0:X3} (write)", offset);
                    return;
                }
                bool en = (ccr[ch] & EN) != 0;
                switch(reg)
                {
                    case 0x00:
                        if(en && (value & EN) != 0) { ccr[ch] = (ccr[ch] & ~0xEu) | (value & 0xEu); UpdateIrq(ch); return; }
                        ccr[ch] = value;
                        if(!en && (value & EN) != 0) Start(ch);
                        else if(en && (value & EN) == 0) ndtr[ch] = cur[ch];
                        UpdateIrq(ch);
                        return;
                    case 0x04: if(!en) ndtr[ch] = value & 0xFFFF; return;
                    case 0x08: if(!en) par[ch] = value; return;
                    case 0x0C: if(!en || ((ccr[ch] & DBM) != 0 && (ccr[ch] & CT) != 0)) m0ar[ch] = value; return;
                    case 0x10: if(!en || ((ccr[ch] & DBM) != 0 && (ccr[ch] & CT) == 0)) m1ar[ch] = value; return;
                }
            }
        }

        private void Start(int ch)
        {
            initial[ch] = cur[ch] = ndtr[ch];
            curP[ch] = par[ch];
            curM[ch] = ((ccr[ch] & DBM) != 0 && (ccr[ch] & CT) != 0) ? m1ar[ch] : m0ar[ch];
            if(initial[ch] == 0) { ccr[ch] &= ~EN; return; }
            if((ccr[ch] & MEM2MEM) != 0) { while((ccr[ch] & EN) != 0 && cur[ch] > 0) Beat(ch); }
            else if(level[ch]) { pending[ch]++; Pump(ch); }
        }

        public void OnGPIO(int number, bool value)
        {
            if(number < 0 || number >= Channels) return;
            lock(sync)
            {
                level[number] = value;
                if(!value) return;
                pending[number]++;
                Pump(number);
            }
        }

        private void Pump(int ch)
        {
            if(busy[ch]) return;
            busy[ch] = true;
            try
            {
                while(pending[ch] > 0)
                {
                    pending[ch]--;
                    if((ccr[ch] & EN) == 0 || (ccr[ch] & MEM2MEM) != 0 || cur[ch] == 0) { pending[ch] = 0; break; }
                    Beat(ch);
                }
            }
            finally { busy[ch] = false; }
        }

        private static ulong Rd(IBusController bus, ulong a, int size)
        {
            return size == 1 ? bus.ReadByte(a) : size == 2 ? bus.ReadWord(a) : (ulong)bus.ReadDoubleWord(a);
        }

        private static void Wr(IBusController bus, ulong a, int size, ulong v)
        {
            if(size == 1) bus.WriteByte(a, (byte)v); else if(size == 2) bus.WriteWord(a, (ushort)v); else bus.WriteDoubleWord(a, (uint)v);
        }

        private void Beat(int ch)
        {
            uint c = ccr[ch];
            int psize = 1 << (int)((c >> 8) & 3), msize = 1 << (int)((c >> 10) & 3);
            bool m2p = (c & DIR) != 0;      // DIR = 1: read from memory (in MEM2MEM: from M0AR... per RM: source CPAR when DIR = 0)
            ulong src = m2p ? curM[ch] : curP[ch], dst = m2p ? curP[ch] : curM[ch];
            int ssize = m2p ? msize : psize, dsize = m2p ? psize : msize;
            var bus = machine.SystemBus;
            if(!D3(src) || !D3(dst) || bus.WhatIsAt(src) == null || bus.WhatIsAt(dst) == null)
            {
                flags[ch] |= TEIF;
                ccr[ch] &= ~EN;
                ndtr[ch] = cur[ch];
                this.Log(LogLevel.Warning, "BDMA channel {0}: transfer error at 0x{1:X8} -> 0x{2:X8} (the BDMA reaches the D3 domain only)", ch, src, dst);
                UpdateIrq(ch);
                return;
            }
            ulong v = Rd(bus, src, ssize);
            if(dsize < 8) v &= dsize == 4 ? 0xFFFFFFFFUL : (1UL << (8 * dsize)) - 1;
            Wr(bus, dst, dsize, v);
            if((c & PINC) != 0) curP[ch] += (ulong)psize;
            if((c & MINC) != 0) curM[ch] += (ulong)msize;
            cur[ch]--;
            uint done = initial[ch] - cur[ch];
            if(initial[ch] >= 2 && done == initial[ch] / 2) flags[ch] |= HTIF;
            if(cur[ch] == 0)
            {
                flags[ch] |= TCIF;
                bool dbm = (c & DBM) != 0;
                if(dbm || (c & CIRC) != 0)
                {
                    if(dbm) ccr[ch] ^= CT;
                    cur[ch] = initial[ch];
                    curP[ch] = par[ch];
                    curM[ch] = (dbm && (ccr[ch] & CT) != 0) ? m1ar[ch] : m0ar[ch];
                }
                else { ccr[ch] &= ~EN; ndtr[ch] = 0; }
            }
            UpdateIrq(ch);
        }

        private void UpdateIrq(int ch)
        {
            uint c = ccr[ch], f = flags[ch];
            bool on = ((f & TCIF) != 0 && (c & TCIE) != 0) || ((f & HTIF) != 0 && (c & HTIE) != 0) || ((f & TEIF) != 0 && (c & TEIE) != 0);
            irq[ch].Set(on);
        }

        private static bool Decode(long offset, out int ch, out int reg)
        {
            ch = reg = -1;
            if(offset < 0x08 || offset >= 0x08 + 0x14 * Channels) return false;
            ch = (int)((offset - 0x08) / 0x14);
            reg = (int)((offset - 0x08) % 0x14);
            return true;
        }
    }
}
