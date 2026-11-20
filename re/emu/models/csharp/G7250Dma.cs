// Generic STM32H7 DMA controller (DMA1/DMA2), clean-room from ST RM0468 (public).
// See docs/DMA-model.md. Replaces Renode 1.17's STM32DMA, which keeps no
// circular mode. Modelled as the hardware behaves, independent of any firmware:
//   - 8 streams; LISR/HISR, LIFCR/HIFCR; per stream CR, NDTR, PAR, M0AR, M1AR, FCR
//   - peripheral-to-memory, memory-to-peripheral (one data item per request
//     edge from DMAMUX, input number = stream) and memory-to-memory (whole
//     transfer when the stream is enabled)
//   - PSIZE/MSIZE, PINC/MINC, PINCOS; NDTR counts PSIZE items
//   - circular mode (NDTR and pointers reload), double-buffer mode (CT toggles)
//   - TCIF, HTIF, TEIF (unmapped address), interrupt per stream (TCIE/HTIE/TEIE/
//     DMEIE/FEIE) on GPIO output 0..7
//   - registers that are read-only while EN = 1 ignore writes, as in RM0468
// Not modelled: transfer timing, FIFO thresholds and burst beats (FIFO mode is
// done as direct beats; packing gives the same memory contents), priorities,
// PFCTRL (peripheral flow control), FIFO/direct-mode error conditions.
using System;
using System.Collections.Generic;
using Antmicro.Renode.Core;
using Antmicro.Renode.Logging;
using Antmicro.Renode.Peripherals.Bus;

namespace Antmicro.Renode.Peripherals.DMA
{
    public class G7250Dma : IDoubleWordPeripheral, IKnownSize, IGPIOReceiver, INumberedGPIOOutput
    {
        private const int Streams = 8;
        // CR bits
        private const uint EN = 1u << 0, DMEIE = 1u << 1, TEIE = 1u << 2, HTIE = 1u << 3, TCIE = 1u << 4,
                           CIRC = 1u << 8, PINC = 1u << 9, MINC = 1u << 10, PINCOS = 1u << 15,
                           DBM = 1u << 18, CT = 1u << 19;
        // stream flags (in stream-local bit positions)
        private const uint FEIF = 1u << 0, DMEIF = 1u << 2, TEIF = 1u << 3, HTIF = 1u << 4, TCIF = 1u << 5;
        private static readonly int[] FlagShift = { 0, 6, 16, 22 };

        private readonly IMachine machine;
        private readonly string name;
        private readonly object sync = new object();
        private readonly Stream[] s = new Stream[Streams];
        private readonly Dictionary<int, IGPIO> irq = new Dictionary<int, IGPIO>();
        private readonly bool[] level = new bool[Streams], busy = new bool[Streams];
        private readonly int[] pending = new int[Streams];

        private class Stream
        {
            public uint Cr, Ndtr, Par, M0ar, M1ar, Fcr, Flags;
            public uint Initial, CurNdtr;
            public ulong CurP, CurM;
            public ulong Beats;
        }

        public G7250Dma(IMachine machine, string controllerName = "DMA")
        {
            this.machine = machine;
            this.name = controllerName;
            for(int i = 0; i < Streams; i++) { s[i] = new Stream(); irq[i] = new GPIO(); }
            Connections = irq;
            Reset();
        }

        public long Size { get { return 0x400; } }
        public IReadOnlyDictionary<int, IGPIO> Connections { get; private set; }

        public void Reset()
        {
            lock(sync)
            {
                foreach(var x in s)
                {
                    x.Cr = x.Ndtr = x.Par = x.M0ar = x.M1ar = x.Flags = 0;
                    x.Fcr = 0x21; x.Initial = x.CurNdtr = 0; x.CurP = x.CurM = 0; x.Beats = 0;
                }
                foreach(var g in irq.Values) g.Unset();
                for(int i = 0; i < Streams; i++) { level[i] = busy[i] = false; pending[i] = 0; }
            }
        }

        private static uint PSize(uint cr) { return 1u << (int)((cr >> 11) & 3); }
        private static uint Dir(uint cr) { return (cr >> 6) & 3; }

        public uint ReadDoubleWord(long offset)
        {
            lock(sync)
            {
                if(offset == 0x00 || offset == 0x04)
                {
                    int b = offset == 0x00 ? 0 : 4;
                    uint v = 0;
                    for(int i = 0; i < 4; i++) v |= (s[b + i].Flags & 0x3D) << FlagShift[i];
                    return v;
                }
                if(offset == 0x08 || offset == 0x0C) return 0;
                int st, reg;
                if(!Decode(offset, out st, out reg)) { NotModelled(offset, "read"); return 0; }
                var x = s[st];
                switch(reg)
                {
                    case 0x00: return x.Cr;
                    case 0x04: return (x.Cr & EN) != 0 ? x.CurNdtr : x.Ndtr;
                    case 0x08: return x.Par;
                    case 0x0C: return x.M0ar;
                    case 0x10: return x.M1ar;
                    case 0x14: return x.Fcr;
                }
                return 0;
            }
        }

        public void WriteDoubleWord(long offset, uint value)
        {
            lock(sync)
            {
                if(offset == 0x08 || offset == 0x0C)
                {
                    int b = offset == 0x08 ? 0 : 4;
                    for(int i = 0; i < 4; i++) s[b + i].Flags &= ~((value >> FlagShift[i]) & 0x3D);
                    for(int i = 0; i < 4; i++) UpdateIrq(b + i);
                    return;
                }
                if(offset == 0x00 || offset == 0x04) return;   // ISRs are read-only
                int st, reg;
                if(!Decode(offset, out st, out reg)) { NotModelled(offset, "write"); return; }
                var x = s[st];
                bool en = (x.Cr & EN) != 0;
                switch(reg)
                {
                    case 0x00: WriteCr(st, value); return;
                    case 0x04: if(!en) x.Ndtr = value & 0xFFFF; return;
                    case 0x08: if(!en) x.Par = value; return;
                    // in double-buffer mode the memory address not in use may change while enabled
                    case 0x0C: if(!en || ((x.Cr & DBM) != 0 && (x.Cr & CT) != 0)) x.M0ar = value; return;
                    case 0x10: if(!en || ((x.Cr & DBM) != 0 && (x.Cr & CT) == 0)) x.M1ar = value; return;
                    case 0x14: x.Fcr = (x.Fcr & 0x38) | (value & 0x87); return;
                }
            }
        }

        private void WriteCr(int st, uint value)
        {
            var x = s[st];
            bool was = (x.Cr & EN) != 0, now = (value & EN) != 0;
            if(was && now)
            {
                // while enabled the configuration bits are protected; only the
                // interrupt enables (bits 1..4) take a new value
                x.Cr = (x.Cr & ~0x1Eu) | (value & 0x1Eu);
                UpdateIrq(st);
                return;
            }
            x.Cr = value;
            if(!was && now) Start(st);
            else if(was && !now) { x.Ndtr = x.CurNdtr; }   // stopped by software: NDTR shows what is left
            UpdateIrq(st);
        }

        private void Start(int st)
        {
            var x = s[st];
            x.Initial = x.Ndtr;
            x.CurNdtr = x.Ndtr;
            x.CurP = x.Par;
            x.CurM = ((x.Cr & DBM) != 0 && (x.Cr & CT) != 0) ? x.M1ar : x.M0ar;
            this.Log(LogLevel.Debug, "{0} stream {1} enabled: CR=0x{2:X8} NDTR={3} PAR=0x{4:X8} MAR=0x{5:X8}",
                     name, st, x.Cr, x.Ndtr, x.Par, x.CurM);
            if(x.Initial == 0) { Finish(st, false); return; }
            if(Dir(x.Cr) == 2)
            {
                // memory-to-memory: the whole transfer at once (source PAR, destination M0AR)
                while((x.Cr & EN) != 0 && x.CurNdtr > 0) Beat(st);
            }
            else if(level[st])
            {
                // the request line is already high (e.g. a transmitter with TXE
                // set): serve it now, as the hardware would
                pending[st]++;
                Pump(st);
            }
        }

        // a request edge from DMAMUX on input "number" = stream number. Edges that
        // arrive while this stream is already serving one (a peripheral that
        // raises its next request from inside the transfer, such as a USART
        // transmitter) are queued, not served recursively.
        public void OnGPIO(int number, bool value)
        {
            if(number < 0 || number >= Streams) return;
            lock(sync)
            {
                level[number] = value;
                if(!value) return;
                pending[number]++;
                Pump(number);
            }
        }

        private void Pump(int st)
        {
            if(busy[st]) return;
            busy[st] = true;
            try
            {
                while(pending[st] > 0)
                {
                    pending[st]--;
                    var x = s[st];
                    if((x.Cr & EN) == 0 || Dir(x.Cr) == 2 || x.CurNdtr == 0) { pending[st] = 0; break; }
                    Beat(st);
                }
            }
            finally { busy[st] = false; }
        }

        private void Beat(int st)
        {
            var x = s[st];
            uint size = PSize(x.Cr);
            uint dir = Dir(x.Cr);
            ulong src = dir == 1 ? x.CurM : x.CurP;
            ulong dst = dir == 1 ? x.CurP : x.CurM;
            var bus = machine.SystemBus;
            if(bus.WhatIsAt(src) == null || bus.WhatIsAt(dst) == null)
            {
                x.Flags |= TEIF;
                x.Cr &= ~EN;
                x.Ndtr = x.CurNdtr;
                this.Log(LogLevel.Warning, "{0} stream {1}: transfer error at 0x{2:X8} -> 0x{3:X8}", name, st, src, dst);
                UpdateIrq(st);
                return;
            }
            switch(size)
            {
                case 1: bus.WriteByte(dst, bus.ReadByte(src)); break;
                case 2: bus.WriteWord(dst, bus.ReadWord(src)); break;
                default: bus.WriteDoubleWord(dst, bus.ReadDoubleWord(src)); break;
            }
            x.Beats++;
            if((x.Cr & PINC) != 0) x.CurP += (x.Cr & PINCOS) != 0 ? 4u : size;
            if((x.Cr & MINC) != 0) x.CurM += size;
            x.CurNdtr--;
            uint done = x.Initial - x.CurNdtr;
            if(x.Initial >= 2 && done == x.Initial / 2) { x.Flags |= HTIF; UpdateIrq(st); }
            if(x.CurNdtr == 0) Finish(st, true);
        }

        private void Finish(int st, bool complete)
        {
            var x = s[st];
            if(complete) x.Flags |= TCIF;
            bool dbm = (x.Cr & DBM) != 0;
            if(complete && (dbm || (x.Cr & CIRC) != 0) && x.Initial > 0)
            {
                if(dbm) x.Cr ^= CT;
                x.CurNdtr = x.Initial;
                x.CurP = x.Par;
                x.CurM = (dbm && (x.Cr & CT) != 0) ? x.M1ar : x.M0ar;
            }
            else
            {
                x.Cr &= ~EN;
                x.Ndtr = 0;
            }
            UpdateIrq(st);
        }

        private void UpdateIrq(int st)
        {
            var x = s[st];
            uint cr = x.Cr;
            bool on = ((x.Flags & TCIF) != 0 && (cr & TCIE) != 0)
                   || ((x.Flags & HTIF) != 0 && (cr & HTIE) != 0)
                   || ((x.Flags & TEIF) != 0 && (cr & TEIE) != 0)
                   || ((x.Flags & DMEIF) != 0 && (cr & DMEIE) != 0)
                   || ((x.Flags & FEIF) != 0 && (x.Fcr & 0x80) != 0);
            irq[st].Set(on);
        }

        private static bool Decode(long offset, out int st, out int reg)
        {
            st = reg = -1;
            if(offset < 0x10 || offset >= 0x10 + 0x18 * Streams) return false;
            st = (int)((offset - 0x10) / 0x18);
            reg = (int)((offset - 0x10) % 0x18);
            return true;
        }

        private void NotModelled(long offset, string what)
        {
            this.Log(LogLevel.Warning, "not modelled: {0}/offset 0x{1:X3} ({2})", name, offset, what);
        }
    }
}
