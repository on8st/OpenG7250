// Generic STM32H72x ADC1 (ADC1/ADC2 block at 0x40022000), clean-room from ST
// RM0468 (public). See docs/ADC-model.md. Replaces Renode 1.17's STM32H7_ADC,
// which returned wrong raw values. Modelled as the hardware behaves:
//   - ADC1 registers ISR, IER, CR, CFGR, CFGR2, SMPR1/2, PCSEL, LTR/HTR, SQR1-4,
//     DR, JSQR, OFR1-4, JDR1-4, AWD2CR/AWD3CR, DIFSEL, CALFACT; ADC2 registers are
//     kept but do not convert; common registers CSR, CCR, CDR at +0x300
//   - power-up: DEEPPWD, ADVREGEN (LDORDY), ADCAL (completes at once), ADEN
//     (ADRDY), ADDIS, ADSTART, ADSTP
//   - regular sequence of up to 16 ranks (SQR1-4), single and continuous mode,
//     resolution 16/14/12/10/8 bit (RES, both RM0468 encodings), left alignment
//     (LSHIFT) not modelled
//   - raw = round(V / Vref * (2^bits - 1)) from per-channel input voltages set by
//     the harness or a plant (SetChannelVoltage); unset channels read 0 V
//   - EOC, EOS, OVR (DR not read before the next result, DMNGT = 0), with the
//     interrupt line from IER; DR read clears EOC
//   - DMA: a request edge per conversion when DMNGT is one-shot (01) or
//     circular (11)
// Not modelled: conversion timing from sampling time and clock (each conversion
// takes 1 / conversionRate of virtual time), external triggers (a sequence
// started with EXTEN != 0 runs at the same fixed rate and logs that once), injected conversions, oversampling, offsets, analog watchdogs,
// differential inputs, dual mode, ADC2 conversions, calibration values.
using System;
using System.Collections.Generic;
using Antmicro.Renode.Core;
using Antmicro.Renode.Logging;
using Antmicro.Renode.Peripherals.Bus;
using Antmicro.Renode.Peripherals.Timers;
using Antmicro.Renode.Time;

namespace Antmicro.Renode.Peripherals.Analog
{
    public class G7250Adc : IDoubleWordPeripheral, IWordPeripheral, IKnownSize
    {
        // ISR / IER bits
        private const uint ADRDY = 1u << 0, EOSMP = 1u << 1, EOC = 1u << 2, EOS = 1u << 3, OVR = 1u << 4,
                           LDORDY = 1u << 12;
        // CR bits
        private const uint ADEN = 1u << 0, ADDIS = 1u << 1, ADSTART = 1u << 2, ADSTP = 1u << 4,
                           ADVREGEN = 1u << 28, DEEPPWD = 1u << 29, ADCAL = 1u << 31;
        private const uint CONT = 1u << 13;

        private readonly IMachine machine;
        private readonly object sync = new object();
        private readonly uint[] adc1 = new uint[0x100 / 4], adc2 = new uint[0x100 / 4], common = new uint[0x10 / 4];
        private readonly double[] volts = new double[20];
        private readonly LimitTimer timer;
        private readonly double vref;
        private uint isr, dr;
        private int rank;
        private bool running, triggerNoted;
        private ulong conversions;

        public G7250Adc(IMachine machine, double referenceVoltage = 3.3, long conversionRate = 20000)
        {
            this.machine = machine;
            vref = referenceVoltage;
            IRQ = new GPIO();
            DmaRequest = new GPIO();
            timer = new LimitTimer(machine.ClockSource, (ulong)conversionRate, this, "conversion", 1,
                                   direction: Direction.Descending, enabled: false,
                                   workMode: WorkMode.Periodic, eventEnabled: true);
            timer.LimitReached += Conversion;
            Reset();
        }

        public GPIO IRQ { get; private set; }
        public GPIO DmaRequest { get; private set; }
        public long Size { get { return 0x400; } }

        // input voltage of channel INPx (harness, fixture or plant model)
        public void SetChannelVoltage(int channel, double value)
        {
            if(channel < 0 || channel >= volts.Length) return;
            lock(sync) volts[channel] = value;
        }

        public double GetChannelVoltage(int channel)
        {
            return channel >= 0 && channel < volts.Length ? volts[channel] : 0.0;
        }

        public ulong Conversions { get { return conversions; } }

        public void Reset()
        {
            lock(sync)
            {
                Array.Clear(adc1, 0, adc1.Length);
                Array.Clear(adc2, 0, adc2.Length);
                Array.Clear(common, 0, common.Length);
                adc1[0x08 / 4] = DEEPPWD;     // CR reset value: deep power-down
                adc2[0x08 / 4] = DEEPPWD;
                isr = 0; dr = 0; rank = 0; running = false; conversions = 0;
                timer.Enabled = false;
            }
            IRQ.Unset(); DmaRequest.Unset();
        }

        private uint Cr { get { return adc1[0x08 / 4]; } set { adc1[0x08 / 4] = value; } }
        private uint Cfgr { get { return adc1[0x0C / 4]; } }

        private int Bits()
        {
            switch((Cfgr >> 2) & 7)
            {
                case 0: return 16;
                case 1: case 5: return 14;
                case 2: case 6: return 12;
                case 3: return 10;
                case 7: return 8;
                default: return 12;
            }
        }

        private int SeqLength() { return (int)(adc1[0x30 / 4] & 0xF) + 1; }

        private int SeqChannel(int r)
        {
            // SQ1..SQ4 in SQR1 (bits 6, 12, 18, 24), SQ5..SQ9 in SQR2, SQ10..SQ14 in SQR3, SQ15..SQ16 in SQR4
            uint reg; int pos;
            if(r < 4) { reg = adc1[0x30 / 4]; pos = 6 + 6 * r; }
            else if(r < 9) { reg = adc1[0x34 / 4]; pos = 6 * (r - 4); }
            else if(r < 14) { reg = adc1[0x38 / 4]; pos = 6 * (r - 9); }
            else { reg = adc1[0x3C / 4]; pos = 6 * (r - 14); }
            return (int)((reg >> pos) & 0x1F);
        }

        private uint Convert(int ch)
        {
            int bits = Bits();
            double full = (1u << bits) - 1;
            double v = ch < volts.Length ? volts[ch] : 0.0;
            double raw = Math.Round(v / vref * full);
            if(raw < 0) raw = 0;
            if(raw > full) raw = full;
            return (uint)raw;
        }

        // one conversion per timer event (virtual time)
        private void Conversion()
        {
            bool dma, stop = false;
            lock(sync)
            {
                if(!running) { timer.Enabled = false; return; }
                int n = SeqLength();
                int ch = SeqChannel(rank);
                uint dmngt = Cfgr & 3;
                if((isr & EOC) != 0 && dmngt == 0)
                {
                    isr |= OVR;
                    if((Cfgr & (1u << 12)) != 0) dr = Convert(ch);   // OVRMOD = 1: overwrite
                }
                else dr = Convert(ch);
                isr |= EOC | EOSMP;
                conversions++;
                rank++;
                if(rank >= n)
                {
                    rank = 0;
                    isr |= EOS;
                    if((Cfgr & CONT) == 0)
                    {
                        running = false;
                        Cr &= ~ADSTART;
                        timer.Enabled = false;
                        stop = true;
                    }
                }
                dma = dmngt == 1 || dmngt == 3;
            }
            if(dma) { DmaRequest.Unset(); DmaRequest.Set(); }
            UpdateIrq();
            if(stop) UpdateIrq();
        }

        public ushort ReadWord(long offset) { return (ushort)ReadDoubleWord(offset & ~3L); }
        public void WriteWord(long offset, ushort value) { WriteDoubleWord(offset & ~3L, value); }

        public uint ReadDoubleWord(long offset)
        {
            uint v;
            bool irq = false;
            lock(sync)
            {
                if(offset >= 0x300)
                {
                    long c = offset - 0x300;
                    if(c == 0x00) v = (isr & 0x7F);                 // CSR: ADC1 flags in the master bits
                    else if(c == 0x0C) v = dr;                       // CDR (single mode: master data)
                    else v = c < 0x10 ? common[c / 4] : 0u;
                }
                else if(offset >= 0x100) v = offset - 0x100 < 0x100 ? adc2[(offset - 0x100) / 4] : 0u;
                else if(offset == 0x00) v = isr;
                else if(offset == 0x40)
                {
                    v = dr;
                    if((isr & EOC) != 0) { isr &= ~EOC; irq = true; }
                }
                else v = adc1[offset / 4];
            }
            if(irq) UpdateIrq();
            return v;
        }

        public void WriteDoubleWord(long offset, uint value)
        {
            lock(sync)
            {
                if(offset >= 0x300) { long c = offset - 0x300; if(c < 0x10 && c != 0x00 && c != 0x0C) common[c / 4] = value; return; }
                if(offset >= 0x100) { if(offset - 0x100 < 0x100) adc2[(offset - 0x100) / 4] = value; return; }
                switch(offset)
                {
                    case 0x00: isr &= ~value; break;                  // ISR: write 1 to clear
                    case 0x08: WriteCr(value); break;
                    case 0x40: break;                                   // DR is read-only
                    default: adc1[offset / 4] = value; break;
                }
            }
            UpdateIrq();
        }

        private void WriteCr(uint value)
        {
            uint cr = Cr;
            // DEEPPWD / ADVREGEN / BOOST and the other configuration bits as written
            uint keep = value & ~(ADEN | ADDIS | ADSTART | ADSTP | ADCAL | (1u << 3) | (1u << 5));
            cr = (cr & (ADEN | ADSTART)) | keep;
            if((cr & ADVREGEN) != 0 && (cr & DEEPPWD) == 0) isr |= LDORDY; else isr &= ~LDORDY;
            if((value & ADCAL) != 0) { /* calibration completes at once: ADCAL reads 0 */ }
            if((value & ADEN) != 0 && (cr & ADVREGEN) != 0) { cr |= ADEN; isr |= ADRDY; }
            if((value & ADDIS) != 0) { cr &= ~(ADEN | ADSTART); running = false; timer.Enabled = false; }
            if((value & ADSTP) != 0) { cr &= ~ADSTART; running = false; timer.Enabled = false; }
            if((value & ADSTART) != 0 && (cr & ADEN) != 0 && !running)
            {
                cr |= ADSTART;
                running = true;
                rank = 0;
                if(((Cfgr >> 10) & 3) != 0 && !triggerNoted)
                {
                    triggerNoted = true;
                    this.Log(LogLevel.Warning, "not modelled: ADC1/CFGR EXTEN (external trigger); conversions run at the fixed rate");
                }
                timer.Value = 1;
                timer.Enabled = true;
            }
            Cr = cr;
        }

        private void UpdateIrq()
        {
            bool on;
            lock(sync) on = (isr & adc1[0x04 / 4] & 0x7FF) != 0;
            IRQ.Set(on);
        }
    }
}
