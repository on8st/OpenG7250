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

        // ---- timing (Timed = true; SPI6 in product runs): every data frame takes
        // bits / f_SCK of emulated time, f_SCK = kernel clock / 2^(MBR+1) (CFG1 MBR);
        // a received frame is visible (RXP, RXDR, an RX DMA request) only from its
        // completion, EOT/TXTF/TXC only from the last frame's completion, and TXP only
        // while the TX FIFO has room. Time comes from a free-running timer of the
        // emulated clock, so it advances inside a quantum. ----
        private readonly IMachine machine;
        private readonly Antmicro.Renode.Peripherals.Timers.LimitTimer clockRef;   // 1 GHz, free running
        private readonly Antmicro.Renode.Peripherals.Timers.LimitTimer wake;       // next pending completion
        private long kernelHz = 64000000;
        private double busyNs;                // completion time of the last frame on the bus
        private double eotNs = -1;            // completion of the transfer's last frame, -1 = none pending
        private double lastReady;             // completion time of the newest received frame
        private readonly Queue<double> rxReady = new Queue<double>();   // completion time per received frame
        public bool Timed { get; set; }

        public void SetKernelClock(long hz) { lock(sync) kernelHz = Math.Max(1, hz); }
        public long KernelClock { get { return kernelHz; } }
        public double FrameNs { get { lock(sync) return FrameTimeNs(); } }

        // the wake timer and the reference timer can differ by a few ns: at a wake-up the
        // time is at least the wake-up's own target (a monotonic floor), so a frame due at
        // that moment counts as complete and the timer is not re-armed for the same instant
        private double timeFloor, wakeTarget;
        private double NowNs() { return Math.Max((double)clockRef.Value, timeFloor); }
        private double FrameTimeNs()
        {
            int mbr = (int)((cfg1 >> 28) & 7);
            double fsck = kernelHz / (double)(2 << mbr);
            return Bits() * 1e9 / fsck;
        }
        private int ReadyCount()
        {
            if(!Timed) return rx.Count;
            double now = NowNs();
            int n = 0;
            foreach(var t in rxReady) { if(t <= now) n++; else break; }
            return n;
        }

        // arm the wake timer for the next completion that matters (RX DMA request, EOT, IRQ)
        private void ArmWake()
        {
            if(!Timed) return;
            // one wake-up per transfer, not per frame: at the transfer's end (EOT), or for an
            // endless transfer at the last frame in flight; the DMA then takes every frame
            // completed by then. The transfer time stays exact at a fraction of the events.
            double now = NowNs(), next = double.MaxValue;
            if((cfg1 & RXDMAEN) != 0)
            {
                // DMA: one wake-up per transfer (at EOT, or at the newest frame of an endless
                // transfer); the DMA then takes every completed frame
                if(eotNs >= 0 && eotNs > now) next = eotNs;
                else if(rxReady.Count > 0 && lastReady > now) next = lastReady;
            }
            else
            {
                // core polling: wake-ups so the reference time advances while the core polls
                // SR, batched to at least 2 us of frames (a frame becomes visible at most
                // 2 us late; the transfer's total time stays exact at its EOT)
                if(rxReady.Count > 0 && rxReady.Peek() > now)
                {
                    double target = rxReady.Peek() + 2000.0;
                    double last = rxReady.Peek();
                    foreach(var t in rxReady) { if(t <= target) last = t; else break; }
                    next = last;
                }
                if(eotNs >= 0 && eotNs > now) next = Math.Min(next, eotNs);
            }
            wake.Enabled = false;
            if(next == double.MaxValue) return;
            ulong d = (ulong)Math.Max(1.0, Math.Ceiling(next - now));
            wakeTarget = now + d;
            wake.Value = (1UL << 40) - d;
            wake.Enabled = true;
        }

        private void OnWake()
        {
            bool rxReq;
            lock(sync)
            {
                wake.Enabled = false;
                timeFloor = Math.Max(timeFloor, wakeTarget);
                rxReq = (cfg1 & RXDMAEN) != 0 && ReadyCount() > 0;
                ArmWake();
            }
            if(rxReq) Edge(RxDma);
            UpdateIrq();
        }

        public G7250Spi(IMachine machine, string spiName = "SPI", int fifoDepth = 16) : base(machine)
        {
            this.machine = machine;
            clockRef = new Antmicro.Renode.Peripherals.Timers.LimitTimer(machine.ClockSource, 1000000000UL, this, "spiclock", 1UL << 62,
                direction: Antmicro.Renode.Time.Direction.Ascending, enabled: true, workMode: Antmicro.Renode.Time.WorkMode.Periodic, eventEnabled: false);
            wake = new Antmicro.Renode.Peripherals.Timers.LimitTimer(machine.ClockSource, 1000000000UL, this, "spiwake", 1UL << 40,
                direction: Antmicro.Renode.Time.Direction.Ascending, enabled: false, workMode: Antmicro.Renode.Time.WorkMode.Periodic, eventEnabled: true);
            wake.LimitReached += OnWake;
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
                rx.Clear(); rxReady.Clear(); busyNs = 0; eotNs = -1;
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
            double ready = 0;
            if(Timed)
            {
                double now = NowNs();
                busyNs = Math.Max(now, busyNs) + FrameTimeNs();
                ready = busyNs;
            }
            if(Comm() != 1)
            {
                // RX FIFO overrun: frames received and not read; with timing, frames still in
                // flight on the bus are in the TX FIFO, not yet in the RX FIFO
                int inRx = Timed ? ReadyCount() : rx.Count;     // completed and unread now
                if(inRx >= fifoDepth) flags |= OVR;
                else { rx.Enqueue(reply); if(Timed) { rxReady.Enqueue(ready); lastReady = ready; } }
            }
            done++;
            if(Tsize() > 0 && done >= Tsize())
            {
                flags |= EOT | TXTF | TXC;
                if(Timed) eotNs = busyNs;
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
            bool eotDue = !Timed || eotNs < 0 || NowNs() >= eotNs;
            if(!eotDue) v &= ~(EOT | TXTF | TXC);
            int thr = (int)((cfg1 >> 5) & 0xF) + 1;
            int avail = ReadyCount();
            if(avail >= thr || (avail > 0 && (v & EOT) != 0)) v |= RXP;
            bool room = true;
            if(Timed)
            {
                double pending = (busyNs - NowNs()) / Math.Max(1e-3, FrameTimeNs());
                room = pending < fifoDepth;
            }
            if((cr1 & SPE) != 0 && Remaining() && room) v |= TXP;
            if((v & RXP) != 0 && (v & TXP) != 0) v |= DXP;
            if((Tsize() == 0 || !Running()) && eotDue) v |= TXC;
            int lvl = Math.Min(avail, 3);
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
                            if(Timed && rxReady.Count > 0) rxReady.Dequeue();
                        }
                        if(rx.Count == 0 && Comm() == 2 && (cfg1 & RXDMAEN) != 0 && Running() && Remaining())
                            ended |= Shift(0);                         // simplex receive by DMA: next frame
                        rxAgain = ReadyCount() > 0 && (cfg1 & RXDMAEN) != 0;
                        ArmWake();
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
                        if((value & SPE) == 0 && wasOn) { rx.Clear(); rxReady.Clear(); done = 0; eotNs = -1; }
                        // CSTART cannot be set while the SPI is disabled, and clearing SPE clears it
                        // (RM0468 SPI_CR1); a write of CSTART with SPE = 0 leaves CSTART clear, so
                        // the following write with SPE and CSTART is a start
                        if((value & SPE) == 0) value &= ~CSTART;
                        bool start = (value & CSTART) != 0 && (cr1 & CSTART) == 0 && (value & SPE) != 0;
                        cr1 = value;
                        if(start) { done = 0; flags &= ~(EOT | TXTF | TXC); eotNs = -1; started = true; }
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
                rxReq = (cfg1 & RXDMAEN) != 0 && ReadyCount() > 0;
                if(Comm() == 2 && started && (cfg1 & RXDMAEN) != 0 && Tsize() > 0)
                {
                    // simplex receive with DMA: the master clocks in the first frame
                    ended |= Shift(0);
                    rxReq = ReadyCount() > 0;
                }
                if((cfg1 & TXDMAEN) == 0 || !txOn) txReq = false;
                ArmWake();
            }
            if(ended) EndOfTransfer();
            bool txIdle;
            lock(sync) txIdle = !((cfg1 & TXDMAEN) != 0 && Running() && Remaining() && Comm() != 2);
            if(txIdle) TxDma.Unset();               // no TX request while the SPI cannot take data
            else if(txReq) Edge(TxDma);
            bool rxEmpty;
            lock(sync) rxEmpty = ReadyCount() == 0 || (cfg1 & RXDMAEN) == 0;
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
