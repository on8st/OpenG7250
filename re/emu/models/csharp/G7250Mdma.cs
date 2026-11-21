// Generic STM32H7 MDMA (clean-room, ST RM0468 public register map), see
// docs/MDMA-model.md. Step 1d of docs/PRODUCT-TESTBENCH-PLAN.md. Modelled as
// the hardware behaves:
//   - 16 channels at 0x40 + 0x40 * x: CISR, CIFCR, CESR, CCR, CTCR, CBNDTR, CSAR,
//     CDAR, CBRUR, CLAR, CTBR, CMAR, CMDR; GISR0 at 0x00
//   - requests: software (CTCR.SWRM, a CCR write with SWRQ while EN) or a
//     hardware trigger on GPIO input TSEL (CTBR.TSEL)
//   - each request transfers one unit of the trigger mode CTCR.TRGM: buffer
//     (TLEN + 1 bytes), block (BNDT bytes), repeated block (BRC + 1 blocks with
//     the CBRUR address updates, BRSUM/BRDUM) or the whole channel transfer
//     including linked-list nodes (CLAR)
//   - source/destination increment, decrement or fixed with SINCOS/DINCOS;
//     source and destination element sizes may differ (packing through a byte
//     stream gives the same memory contents)
//   - flags TCIF (buffer), BTIF (block), BRTIF (repeated block), CTCIF (channel),
//     TEIF with CESR on an unmapped address; EN cleared at channel completion;
//     one interrupt line (GPIO 0, MDMA IRQ 122) from the channel enables
//   - linked list: at the end of a node with CLAR != 0 the next node (CTCR,
//     CBNDTR, CSAR, CDAR, CBRUR, CLAR, CTBR, -, CMAR, CMDR) is loaded from memory
// Not modelled: transfer timing (every unit is instantaneous in virtual time),
// bursts, bus selection (CTBR.SBUS/DBUS), priorities, mask writes
// (CMAR/CMDR), byte/half-word/word endianness exchange (BEX/HEX/WEX), PAM
// padding/alignment modes beyond equal-size or byte packing, buffer-wise
// hardware request handshakes (BWM), secure/privileged attributes.
using System;
using System.Collections.Generic;
using Antmicro.Renode.Core;
using Antmicro.Renode.Logging;
using Antmicro.Renode.Peripherals;
using Antmicro.Renode.Peripherals.Bus;

namespace Antmicro.Renode.Peripherals.DMA
{
    public class G7250Mdma : IDoubleWordPeripheral, IKnownSize, IGPIOReceiver, INumberedGPIOOutput
    {
        private const int Channels = 16;
        private const int ISR = 0x00, IFCR = 0x04, ESR = 0x08, CR = 0x0C, TCR = 0x10,
                          BNDTR = 0x14, SAR = 0x18, DAR = 0x1C, BRUR = 0x20, LAR = 0x24,
                          TBR = 0x28, MAR = 0x30, MDR = 0x34;
        private const uint TEIF = 1u << 0, CTCIF = 1u << 1, BRTIF = 1u << 2, BTIF = 1u << 3, TCIF = 1u << 4;
        private const uint EN = 1u << 0, SWRQ = 1u << 16;

        private readonly uint[,] regs = new uint[Channels, 16];
        private readonly uint[] blockLen = new uint[Channels];   // BNDT at block start, for reloads
        private readonly IMachine machine;
        private readonly object sync = new object();
        private readonly Dictionary<int, IGPIO> outputs = new Dictionary<int, IGPIO>();
        private ulong units;

        public G7250Mdma(IMachine machine)
        {
            this.machine = machine;
            outputs[0] = new GPIO();
            Connections = outputs;
            Reset();
        }

        public long Size { get { return 0x1000; } }
        public IReadOnlyDictionary<int, IGPIO> Connections { get; private set; }

        public void Reset()
        {
            lock(sync)
            {
                Array.Clear(regs, 0, regs.Length);
                Array.Clear(blockLen, 0, blockLen.Length);
                units = 0;
            }
            outputs[0].Unset();
        }

        private uint R(int ch, int reg) { return regs[ch, reg / 4]; }
        private void W(int ch, int reg, uint v) { regs[ch, reg / 4] = v; }

        private static bool Decode(long offset, out int ch, out int reg)
        {
            ch = reg = -1;
            if(offset < 0x40) return false;
            ch = (int)((offset - 0x40) / 0x40);
            reg = (int)((offset - 0x40) % 0x40);
            return ch < Channels;
        }

        public uint ReadDoubleWord(long offset)
        {
            lock(sync)
            {
                if(offset == 0x00)
                {
                    uint g = 0;
                    for(int c = 0; c < Channels; c++) if((R(c, ISR) & 0x1F) != 0) g |= 1u << c;
                    return g;
                }
                int ch, reg;
                if(!Decode(offset, out ch, out reg)) return 0;
                if(reg == IFCR) return 0;
                return R(ch, reg);
            }
        }

        public void WriteDoubleWord(long offset, uint value)
        {
            lock(sync)
            {
                int ch, reg;
                if(!Decode(offset, out ch, out reg))
                {
                    this.Log(LogLevel.Warning, "not modelled: MDMA/offset 0x{0:X3} (write)", offset);
                    return;
                }
                switch(reg)
                {
                    case ISR: case ESR: return;                               // read only
                    case IFCR:
                        W(ch, ISR, R(ch, ISR) & ~(value & 0x1F));
                        if((value & TEIF) != 0) W(ch, ESR, 0);
                        break;
                    case CR:
                    {
                        bool was = (R(ch, CR) & EN) != 0;
                        W(ch, CR, value & ~SWRQ);                             // SWRQ is write-only
                        if(!was && (value & EN) != 0) blockLen[ch] = R(ch, BNDTR) & 0x1FFFF;
                        if((value & EN) != 0 && (value & SWRQ) != 0) Request(ch);
                        break;
                    }
                    default:
                        // configuration registers are writable while the channel is disabled
                        if((R(ch, CR) & EN) == 0) W(ch, reg, value);
                        break;
                }
            }
            UpdateIrq();
        }

        // hardware trigger: input number = TSEL
        public void OnGPIO(int number, bool value)
        {
            if(!value) return;
            lock(sync)
            {
                for(int c = 0; c < Channels; c++)
                {
                    uint tcr = R(c, TCR);
                    if((R(c, CR) & EN) != 0 && (tcr & (1u << 30)) == 0 && (R(c, TBR) & 0x3F) == (uint)number)
                        Request(c);
                }
            }
            UpdateIrq();
        }

        // one request: transfer one unit of the trigger mode
        private void Request(int ch)
        {
            uint trgm = (R(ch, TCR) >> 28) & 3;
            for(int guard = 0; guard < 65536; guard++)
            {
                if((R(ch, CR) & EN) == 0) return;
                uint tlen = ((R(ch, TCR) >> 18) & 0x7F) + 1;
                uint left = R(ch, BNDTR) & 0x1FFFF;
                if(left == 0) left = blockLen[ch];
                uint n = trgm == 0 ? Math.Min(tlen, left) : left;
                if(!Move(ch, n)) return;
                left -= n;
                W(ch, BNDTR, (R(ch, BNDTR) & ~0x1FFFFu) | left);
                W(ch, ISR, R(ch, ISR) | TCIF);
                units++;
                if(left > 0)
                {
                    if(trgm == 0) return;   // buffer mode: next buffer on the next request
                    continue;
                }
                // end of a block
                W(ch, ISR, R(ch, ISR) | BTIF);
                uint brc = (R(ch, BNDTR) >> 20) & 0xFFF;
                if(brc > 0)
                {
                    uint bndtr = R(ch, BNDTR), brur = R(ch, BRUR);
                    long suv = brur & 0xFFFF, duv = (brur >> 16) & 0xFFFF;
                    if((bndtr & (1u << 18)) != 0) suv = -suv;
                    if((bndtr & (1u << 19)) != 0) duv = -duv;
                    W(ch, SAR, (uint)(R(ch, SAR) + suv));
                    W(ch, DAR, (uint)(R(ch, DAR) + duv));
                    W(ch, BNDTR, (bndtr & 0x000C0000) | ((brc - 1) << 20) | blockLen[ch]);
                    if(trgm <= 1) return;   // buffer/block mode: next block on the next request
                    continue;
                }
                // end of the repeated block = end of this node
                W(ch, ISR, R(ch, ISR) | BRTIF);
                uint lar = R(ch, LAR);
                if(lar != 0)
                {
                    if(!LoadNode(ch, lar)) return;
                    trgm = (R(ch, TCR) >> 28) & 3;
                    if(((R(ch, TCR) >> 28) & 3) != 3) return;   // the new node waits for its own request
                    continue;
                }
                W(ch, ISR, R(ch, ISR) | CTCIF);
                W(ch, CR, R(ch, CR) & ~EN);
                if((units & (units - 1)) == 0)
                    this.Log(LogLevel.Debug, "MDMA ch{0}: channel transfer complete (unit #{1})", ch, units);
                return;
            }
        }

        private bool LoadNode(int ch, uint addr)
        {
            var bus = machine.SystemBus;
            if(bus.WhatIsAt(addr) == null) { Error(ch, addr); return false; }
            W(ch, TCR, bus.ReadDoubleWord(addr + 0x00));
            W(ch, BNDTR, bus.ReadDoubleWord(addr + 0x04));
            W(ch, SAR, bus.ReadDoubleWord(addr + 0x08));
            W(ch, DAR, bus.ReadDoubleWord(addr + 0x0C));
            W(ch, BRUR, bus.ReadDoubleWord(addr + 0x10));
            W(ch, LAR, bus.ReadDoubleWord(addr + 0x14));
            W(ch, TBR, bus.ReadDoubleWord(addr + 0x18));
            W(ch, MAR, bus.ReadDoubleWord(addr + 0x20));
            W(ch, MDR, bus.ReadDoubleWord(addr + 0x24));
            blockLen[ch] = R(ch, BNDTR) & 0x1FFFF;
            return true;
        }

        private void Error(int ch, ulong addr)
        {
            W(ch, ISR, R(ch, ISR) | TEIF);
            W(ch, ESR, (uint)(addr & 0x7F) | (1u << 11));   // TEA low bits, TED: destination/source not distinguished
            W(ch, CR, R(ch, CR) & ~EN);
            this.Log(LogLevel.Warning, "MDMA ch{0}: transfer error at 0x{1:X8}", ch, addr);
        }

        // move n bytes from CSAR to CDAR with the element sizes and increments of CTCR
        private bool Move(int ch, uint n)
        {
            var bus = machine.SystemBus;
            uint tcr = R(ch, TCR);
            int sinc = (int)(tcr & 3), dinc = (int)((tcr >> 2) & 3);
            int ssize = 1 << (int)((tcr >> 4) & 3), dsize = 1 << (int)((tcr >> 6) & 3);
            int sincos = 1 << (int)((tcr >> 8) & 3), dincos = 1 << (int)((tcr >> 10) & 3);
            long sstep = sinc == 2 ? sincos : sinc == 3 ? -sincos : 0;
            long dstep = dinc == 2 ? dincos : dinc == 3 ? -dincos : 0;
            ulong src = R(ch, SAR), dst = R(ch, DAR);
            if(bus.WhatIsAt(src) == null) { Error(ch, src); return false; }
            if(bus.WhatIsAt(dst) == null) { Error(ch, dst); return false; }
            var stream = new byte[n];
            int k = 0;
            while(k < n)
            {
                ulong v = Read(bus, src, ssize);
                for(int b = 0; b < ssize && k < n; b++) stream[k++] = (byte)(v >> (8 * b));
                src = (ulong)((long)src + sstep);
            }
            k = 0;
            while(k < n)
            {
                ulong v = 0;
                for(int b = 0; b < dsize && k < n; b++) v |= (ulong)stream[k++] << (8 * b);
                Write(bus, dst, dsize, v);
                dst = (ulong)((long)dst + dstep);
            }
            W(ch, SAR, (uint)src);
            W(ch, DAR, (uint)dst);
            return true;
        }

        private static ulong Read(IBusController bus, ulong a, int size)
        {
            switch(size)
            {
                case 1: return bus.ReadByte(a);
                case 2: return bus.ReadWord(a);
                case 4: return bus.ReadDoubleWord(a);
                default: return bus.ReadDoubleWord(a) | ((ulong)bus.ReadDoubleWord(a + 4) << 32);
            }
        }

        private static void Write(IBusController bus, ulong a, int size, ulong v)
        {
            switch(size)
            {
                case 1: bus.WriteByte(a, (byte)v); break;
                case 2: bus.WriteWord(a, (ushort)v); break;
                case 4: bus.WriteDoubleWord(a, (uint)v); break;
                default: bus.WriteDoubleWord(a, (uint)v); bus.WriteDoubleWord(a + 4, (uint)(v >> 32)); break;
            }
        }

        private void UpdateIrq()
        {
            bool on = false;
            lock(sync)
            {
                for(int c = 0; c < Channels && !on; c++)
                {
                    uint f = R(c, ISR), cr = R(c, CR);
                    // CCR: TEIE bit1 -> TEIF, CTCIE bit2 -> CTCIF, BRTIE bit3 -> BRTIF, BTIE bit4 -> BTIF, TCIE bit5 -> TCIF
                    on = ((f & 0x1F) & ((cr >> 1) & 0x1F)) != 0;
                }
            }
            outputs[0].Set(on);
        }
    }
}
