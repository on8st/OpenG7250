// Generic STM32H7 USART (clean-room, ST RM0468 public register map), see
// docs/USART-model.md. Replaces Renode 1.17's STM32F7_USART where DMA matters:
// that model raises no DMA requests, so a DMA receive ring never fills.
// Modelled as the hardware behaves:
//   - CR1/CR2/CR3/BRR/GTPR/RTOR/RQR/ISR/ICR/RDR/TDR/PRESC
//   - transmit: a TDR write with UE+TE sends the byte at once (TXE/TC stay 1)
//   - receive: bytes from the connected terminal or hub are queued; RXNE while
//     the queue is not empty; RDR read pops one byte; IDLE when the queue runs
//     empty after reception; RQR.RXFRQ flushes
//   - DMA: RxDma request edge per received byte while CR3.DMAR (and again
//     after an RDR read while more bytes wait); TxDma request edge whenever
//     TXE is set while CR3.DMAT
//   - TEACK/REACK follow TE/RE while UE; one interrupt line from
//     RXNEIE/TXEIE/TCIE/IDLEIE/EIE
// Not modelled: baud-rate timing (bytes move instantly), overrun (the receive
// queue has no depth limit), parity/framing/noise errors, LIN, smartcard,
// IrDA, synchronous mode, auto-baud, receiver timeout, wake-up from Stop.
using System;
using System.Collections.Generic;
using Antmicro.Renode.Core;
using Antmicro.Renode.Logging;
using Antmicro.Renode.Peripherals.Bus;

namespace Antmicro.Renode.Peripherals.UART
{
    public class G7250Usart : IUART, IDoubleWordPeripheral, IWordPeripheral, IBytePeripheral, IKnownSize
    {
        private const uint UE = 1u << 0, RE = 1u << 2, TE = 1u << 3, IDLEIE = 1u << 4, RXNEIE = 1u << 5,
                           TCIE = 1u << 6, TXEIE = 1u << 7;
        private const uint EIE = 1u << 0, DMAR = 1u << 6, DMAT = 1u << 7;
        private const uint ORE = 1u << 3, IDLE = 1u << 4, RXNE = 1u << 5, TC = 1u << 6, TXE = 1u << 7,
                           TEACK = 1u << 21, REACK = 1u << 22;

        private readonly object sync = new object();
        private readonly Queue<byte> rx = new Queue<byte>();
        private readonly string label;
        private readonly long frequency;
        private uint cr1, cr2, cr3, brr, gtpr, rtor, presc, flags;
        private ulong sent, received;

        public G7250Usart(string usartName = "USART", long frequency = 100000000)
        {
            label = usartName;
            this.frequency = frequency;
            IRQ = new GPIO(); RxDma = new GPIO(); TxDma = new GPIO();
            Reset();
        }

        public GPIO IRQ { get; private set; }
        public GPIO RxDma { get; private set; }
        public GPIO TxDma { get; private set; }
        public long Size { get { return 0x400; } }
        public uint BaudRate { get { return brr == 0 ? 0u : (uint)(frequency / brr); } }
        public Bits StopBits { get { return Bits.One; } }
        public Parity ParityBit { get { return Parity.None; } }
        public event Action<byte> CharReceived;

        public void Reset()
        {
            lock(sync)
            {
                cr1 = cr2 = cr3 = brr = gtpr = rtor = presc = 0;
                flags = TXE | TC;
                rx.Clear();
            }
            IRQ.Unset(); RxDma.Unset(); TxDma.Unset();
        }

        private uint Isr()
        {
            uint v = flags & ~(RXNE | TEACK | REACK);
            if(rx.Count > 0) v |= RXNE;
            if((cr1 & UE) != 0 && (cr1 & TE) != 0) v |= TEACK;
            if((cr1 & UE) != 0 && (cr1 & RE) != 0) v |= REACK;
            return v;
        }

        public uint ReadDoubleWord(long offset)
        {
            bool rxAgain = false;
            uint v;
            lock(sync)
            {
                switch(offset)
                {
                    case 0x00: v = cr1; break;
                    case 0x04: v = cr2; break;
                    case 0x08: v = cr3; break;
                    case 0x0C: v = brr; break;
                    case 0x10: v = gtpr; break;
                    case 0x14: v = rtor; break;
                    case 0x1C: v = Isr(); break;
                    case 0x24:
                        v = rx.Count > 0 ? rx.Dequeue() : 0u;
                        if(rx.Count == 0 && received > 0) flags |= IDLE;
                        rxAgain = rx.Count > 0 && (cr3 & DMAR) != 0;
                        break;
                    case 0x2C: v = presc; break;
                    default: v = 0; break;
                }
            }
            if(offset == 0x24) { if(rxAgain) Edge(RxDma); else RxDma.Unset(); UpdateIrq(); }
            return v;
        }

        // byte and half-word accesses (a DMA with byte data size, HAL code) act on
        // the register at that offset, as on the hardware's APB interface
        public byte ReadByte(long offset) { return (byte)ReadDoubleWord(offset & ~3L); }
        public void WriteByte(long offset, byte value) { WriteDoubleWord(offset & ~3L, value); }
        public ushort ReadWord(long offset) { return (ushort)ReadDoubleWord(offset & ~3L); }
        public void WriteWord(long offset, ushort value) { WriteDoubleWord(offset & ~3L, value); }

        public void WriteDoubleWord(long offset, uint value)
        {
            bool txReq = false, rxReq = false;
            byte? tx = null;
            lock(sync)
            {
                switch(offset)
                {
                    case 0x00: cr1 = value; break;
                    case 0x04: cr2 = value; break;
                    case 0x08:
                        txReq = (value & DMAT) != 0 && (cr3 & DMAT) == 0;
                        rxReq = (value & DMAR) != 0 && (cr3 & DMAR) == 0 && rx.Count > 0;
                        cr3 = value;
                        break;
                    case 0x0C: brr = value & 0xFFFF; break;
                    case 0x10: gtpr = value; break;
                    case 0x14: rtor = value; break;
                    case 0x18:
                        if((value & (1u << 3)) != 0) rx.Clear();    // RXFRQ
                        break;
                    case 0x20: flags &= ~(value & 0x1F5Fu & ~TXE); break;   // ICR (TXE is not a clearable flag)
                    case 0x28:
                        if((cr1 & UE) != 0 && (cr1 & TE) != 0) { tx = (byte)value; sent++; }
                        flags |= TXE | TC;
                        txReq = (cr3 & DMAT) != 0;
                        break;
                    case 0x2C: presc = value & 0xF; break;
                    default:
                        this.Log(LogLevel.Warning, "not modelled: {0}/offset 0x{1:X3} (write)", label, offset);
                        break;
                }
            }
            if(offset == 0x08)
            {
                if((value & DMAT) == 0) TxDma.Unset();
                if((value & DMAR) == 0) RxDma.Unset();
            }
            if(tx.HasValue) CharReceived?.Invoke(tx.Value);
            if(txReq) Edge(TxDma);
            if(rxReq) Edge(RxDma);
            UpdateIrq();
        }

        // a byte arriving on the line (from the connected terminal or hub)
        public void WriteChar(byte value)
        {
            bool req;
            lock(sync)
            {
                if((cr1 & UE) == 0 || (cr1 & RE) == 0) return;   // receiver off: the byte is lost
                rx.Enqueue(value);
                received++;
                flags &= ~IDLE;
                req = (cr3 & DMAR) != 0;
            }
            if((received & (received - 1)) == 0)
                this.Log(LogLevel.Info, "{0}: {1} bytes received", label, received);
            if(req) Edge(RxDma);
            UpdateIrq();
        }

        private static void Edge(GPIO g) { g.Unset(); g.Set(); }

        private void UpdateIrq()
        {
            bool on;
            lock(sync)
            {
                uint isr = Isr();
                on = ((isr & RXNE) != 0 && (cr1 & RXNEIE) != 0)
                  || ((isr & TXE) != 0 && (cr1 & TXEIE) != 0)
                  || ((isr & TC) != 0 && (cr1 & TCIE) != 0)
                  || ((isr & IDLE) != 0 && (cr1 & IDLEIE) != 0)
                  || ((isr & ORE) != 0 && ((cr1 & RXNEIE) != 0 || (cr3 & EIE) != 0));
            }
            IRQ.Set(on);
        }
    }
}
