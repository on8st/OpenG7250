// Generic STM32H7 SPI (master), clean-room from ST RM0468 (public), see
// docs/SPI-model.md. Step 1g of docs/PRODUCT-TESTBENCH-PLAN.md. Replaces Renode
// 1.17's STM32H7_SPI where DMA matters (it raises no TX DMA request and SPI6 had
// no request wiring at all).
//   - CR1 (SPE, CSTART, SSI), CR2 (TSIZE), CFG1 (DSIZE, FTHLV, RXDMAEN,
//     TXDMAEN, MBR), CFG2 (COMM, MASTER, LSBFRST, CPOL/CPHA, SSM), IER, SR, IFCR,
//     TXDR, RXDR, CRCPOLY, UDRDR, I2SCFGR (stored)
//   - master transfers: each frame written to TXDR (8/16/32-bit access packs
//     1..4 frames of DSIZE+1 bits) goes to the attached SPI device byte by byte
//     (MSB first, or LSB first with LSBFRST); the reply goes to the RX FIFO
//     (full duplex); simplex transmit discards it; simplex receive sends 0x00
//     frames on demand (TSIZE > 0: TSIZE frames after CSTART)
//   - TSIZE > 0: EOT and TXC after TSIZE frames, CSTART cleared, the device's
//     FinishTransmission called; TSIZE = 0: endless
//   - SR: RXP when the RX FIFO holds at least FTHLV+1 frames, TXP, DXP, EOT,
//     TXTF, OVR (RX FIFO over its depth), TXC, RXPLVL, RXWNE, CTSIZE
//   - DMA: RxDma request edge per received frame while CFG1.RXDMAEN; TxDma
//     request while TXDMAEN and frames remain (after CSTART and after each TXDR
//     write)
//   - interrupt line from IER
// Not modelled: slave mode, timing and baud rate, CRC calculation, underrun
// (slave), mode fault, TI mode, I2S, suspend (CSUSP/MASRX), the FIFO size
// differences between instances beyond fifoDepth, SS pin management.
using System;
using System.Collections.Generic;
using Antmicro.Renode.Core;
using Antmicro.Renode.Core.Structure;
using Antmicro.Renode.Logging;
using Antmicro.Renode.Peripherals.Bus;

namespace Antmicro.Renode.Peripherals.SPI
{
    public class G7250Spi : NullRegistrationPointPeripheralContainer<ISPIPeripheral>,
                            IDoubleWordPeripheral, IWordPeripheral, IBytePeripheral, IKnownSize
    {
        private const uint SPE = 1u << 0, CSTART = 1u << 9;
        private const uint RXDMAEN = 1u << 14, TXDMAEN = 1u << 15;
        private const uint RXP = 1u << 0, TXP = 1u << 1, DXP = 1u << 2, EOT = 1u << 3, TXTF = 1u << 4,
                           OVR = 1u << 6, TXC = 1u << 12;

        private readonly object sync = new object();
        private readonly Queue<uint> rx = new Queue<uint>();
        private readonly string label;
        private readonly int fifoDepth;
        private uint cr1, cr2, cfg1, cfg2, ier, flags, crcpoly = 0x107, udrdr, i2scfgr;
        private uint done;   // frames transferred since CSTART

        public G7250Spi(IMachine machine, string spiName = "SPI", int fifoDepth = 16) : base(machine)
        {
            label = spiName;
            this.fifoDepth = fifoDepth;
            IRQ = new GPIO(); RxDma = new GPIO(); TxDma = new GPIO();
            Reset();
        }

        public GPIO IRQ { get; private set; }
        public GPIO RxDma { get; private set; }
        public GPIO TxDma { get; private set; }
        public long Size { get { return 0x400; } }

        public override void Reset()
        {
            lock(sync)
            {
                cr1 = cr2 = cfg2 = ier = flags = udrdr = i2scfgr = 0;
                cfg1 = 0x00070007;      // reset value: DSIZE 8 bit
                crcpoly = 0x107; done = 0;
                rx.Clear();
            }
            IRQ.Unset(); RxDma.Unset(); TxDma.Unset();
        }

        private int Bits() { return (int)(cfg1 & 0x1F) + 1; }
        private int FrameBytes() { return (Bits() + 7) / 8; }
        private uint Comm() { return (cfg2 >> 17) & 3; }   // 0 full duplex, 1 simplex TX, 2 simplex RX, 3 half duplex
        private uint Tsize() { return cr2 & 0xFFFF; }
        private bool Running() { return (cr1 & SPE) != 0 && (cr1 & CSTART) != 0; }
        private bool Remaining() { return Tsize() == 0 || done < Tsize(); }

        private uint Frame(uint value)
        {
            int bits = Bits(), n = FrameBytes();
            bool lsb = ((cfg2 >> 23) & 1) != 0;
            uint mask = bits >= 32 ? 0xFFFFFFFFu : (1u << bits) - 1;
            value &= mask;
            uint reply = 0;
            var dev = RegisteredPeripheral;
            for(int i = 0; i < n; i++)
            {
                int shift = lsb ? 8 * i : 8 * (n - 1 - i);
                byte outb = (byte)(value >> shift);
                byte inb = dev != null ? dev.Transmit(outb) : (byte)0;
                reply |= (uint)inb << shift;
            }
            return reply & mask;
        }

        // one frame on the bus; returns true if the transfer ended (TSIZE reached)
        private bool Shift(uint value)
        {
            uint reply = Frame(value);
            if(Comm() != 1)
            {
                if(rx.Count >= fifoDepth) flags |= OVR;
                else rx.Enqueue(reply);
            }
            done++;
            if(Tsize() > 0 && done >= Tsize())
            {
                flags |= EOT | TXTF | TXC;
                cr1 &= ~CSTART;
                return true;
            }
            return false;
        }

        private void EndOfTransfer()
        {
            var dev = RegisteredPeripheral;
            if(dev != null) dev.FinishTransmission();
        }

        private uint Status()
        {
            uint v = flags & (EOT | TXTF | OVR | TXC | (1u << 5) | (1u << 7) | (1u << 8) | (1u << 9) | (1u << 10) | (1u << 11));
            int thr = (int)((cfg1 >> 5) & 0xF) + 1;
            if(rx.Count >= thr || (rx.Count > 0 && (flags & EOT) != 0)) v |= RXP;
            if((cr1 & SPE) != 0 && Remaining()) v |= TXP;
            if((v & RXP) != 0 && (v & TXP) != 0) v |= DXP;
            if(Tsize() == 0 || !Running()) v |= TXC;
            int lvl = Math.Min(rx.Count, 3);
            v |= (uint)lvl << 13;
            if(rx.Count > 0 && FrameBytes() <= 2 && rx.Count * FrameBytes() < 4) v |= 1u << 15;   // RXWNE: less than a word left
            uint ct = Tsize() > done ? Tsize() - done : 0;
            v |= (ct & 0xFFFF) << 16;
            return v;
        }

        // ---- register access ----
        public uint ReadDoubleWord(long offset) { return Read(offset, 4); }
        public ushort ReadWord(long offset) { return (ushort)Read(offset, 2); }
        public byte ReadByte(long offset) { return (byte)Read(offset, 1); }
        public void WriteDoubleWord(long offset, uint value) { Write(offset, value, 4); }
        public void WriteWord(long offset, ushort value) { Write(offset, value, 2); }
        public void WriteByte(long offset, byte value) { Write(offset, value, 1); }

        private uint Read(long offset, int width)
        {
            bool rxAgain = false, ended = false;
            uint v = 0;
            lock(sync)
            {
                switch(offset & ~3L)
                {
                    case 0x00: v = cr1; break;
                    case 0x04: v = cr2; break;
                    case 0x08: v = cfg1; break;
                    case 0x0C: v = cfg2; break;
                    case 0x10: v = ier; break;
                    case 0x14: v = Status(); break;
                    case 0x30:
                    {
                        int per = Math.Max(1, width / FrameBytes());   // frames packed into this access
                        int fb = FrameBytes();
                        for(int i = 0; i < per; i++)
                        {
                            if(rx.Count == 0 && Comm() == 2 && Running() && Remaining())
                                ended |= Shift(0);                     // simplex receive: clock in on demand
                            if(rx.Count == 0) break;
                            v |= rx.Dequeue() << (8 * fb * i);
                        }
                        if(rx.Count == 0 && Comm() == 2 && (cfg1 & RXDMAEN) != 0 && Running() && Remaining())
                            ended |= Shift(0);                         // simplex receive by DMA: next frame
                        rxAgain = rx.Count > 0 && (cfg1 & RXDMAEN) != 0;
                        break;
                    }
                    case 0x40: v = crcpoly; break;
                    case 0x4C: v = udrdr; break;
                    case 0x50: v = i2scfgr; break;
                    default: v = 0; break;
                }
            }
            if(ended) EndOfTransfer();
            if((offset & ~3L) == 0x30) { if(rxAgain) Edge(RxDma); else RxDma.Unset(); }
            UpdateIrq();
            return v;
        }

        private void Write(long offset, uint value, int width)
        {
            bool started = false, ended = false, txReq = false, rxReq = false;
            lock(sync)
            {
                switch(offset & ~3L)
                {
                    case 0x00:
                    {
                        bool wasOn = (cr1 & SPE) != 0;
                        if((value & SPE) == 0 && wasOn) { rx.Clear(); done = 0; }
                        bool start = (value & CSTART) != 0 && (cr1 & CSTART) == 0 && (value & SPE) != 0;
                        cr1 = value;
                        if(start) { done = 0; flags &= ~(EOT | TXTF | TXC); started = true; }
                        break;
                    }
                    case 0x04: if((cr1 & SPE) == 0) cr2 = value & 0xFFFFFFFF; break;
                    case 0x08: cfg1 = value; break;
                    case 0x0C: if((cr1 & SPE) == 0) cfg2 = value; break;
                    case 0x10: ier = value & 0x7FF; break;
                    case 0x18: flags &= ~(value & 0xFF8); break;      // IFCR: EOTC .. SUSPC
                    case 0x20:
                    {
                        if((cr1 & SPE) == 0) break;
                        int fb = FrameBytes();
                        int per = Math.Max(1, width / fb);
                        for(int i = 0; i < per && Remaining(); i++)
                            ended |= Shift((value >> (8 * fb * i)) & (fb >= 4 ? 0xFFFFFFFFu : ((1u << (8 * fb)) - 1)));
                        break;
                    }
                    case 0x40: crcpoly = value; break;
                    case 0x4C: udrdr = value; break;
                    case 0x50: i2scfgr = value; break;
                    default:
                        this.Log(LogLevel.Warning, "not modelled: {0}/offset 0x{1:X3} (write)", label, offset);
                        break;
                }
                bool txOn = (cfg1 & TXDMAEN) != 0 && Running() && Remaining() && Comm() != 2;
                txReq = txOn && ((offset & ~3L) == 0x20 || started || (offset & ~3L) == 0x08);
                rxReq = (cfg1 & RXDMAEN) != 0 && rx.Count > 0;
                if(Comm() == 2 && started && (cfg1 & RXDMAEN) != 0 && Tsize() > 0)
                {
                    // simplex receive with DMA: the master clocks in the first frame
                    ended |= Shift(0);
                    rxReq = true;
                }
                if((cfg1 & TXDMAEN) == 0 || !txOn) txReq = false;
            }
            if(ended) EndOfTransfer();
            bool txIdle;
            lock(sync) txIdle = !((cfg1 & TXDMAEN) != 0 && Running() && Remaining() && Comm() != 2);
            if(txIdle) TxDma.Unset();               // no TX request while the SPI cannot take data
            else if(txReq) Edge(TxDma);
            bool rxEmpty;
            lock(sync) rxEmpty = rx.Count == 0 || (cfg1 & RXDMAEN) == 0;
            if(rxReq && !rxEmpty) Edge(RxDma); else if(rxEmpty) RxDma.Unset();
            UpdateIrq();
        }

        private static void Edge(GPIO g) { g.Unset(); g.Set(); }

        private void UpdateIrq()
        {
            bool on;
            lock(sync) on = (Status() & ier & 0x7FF) != 0;
            IRQ.Set(on);
        }
    }
}
