// GPIOE port with the RF FPGA on its pins, for the Main (clean-room), see
// docs/FPGA-model-notes.md. Step 2 (FPGA) of docs/PRODUCT-TESTBENCH-PLAN.md:
// behaviour follows his/80-rf-fpga.md only, with no assumption about how a
// particular firmware drives the bus.
//
// Port: a full GPIO port (MODER, OTYPER, OSPEEDR, PUPDR, IDR, ODR, BSRR with set
// winning, LCKR, AFRL/AFRH); pin levels go to EXTI through outputs 0..15.
// Because the port computes IDR itself, the FPGA's data is read back on D0-D7
// whenever the MCU has those pins as inputs.
//
// FPGA (his/80): D0-D7 = PE0-PE7 (direction from MODER), CMD = PE8, STB = PE9,
// DRDY = PE10. A rising STB edge with CMD high latches a command byte; with CMD
// low it is a data write (D0-D7 outputs) or a data read (D0-D7 inputs).
//   0x00 loopback register: a written byte reads back
//   0x01 24-byte configuration frame: recorded (FrameHex, NcoWord(offset),
//        RateIndex = byte 23 & 3), logged at Info
//   0x02 status read: 7 bytes (flags + three 16-bit values), settable
//   0x03 TX I/Q write: bytes counted (TxBytes); TX is not modelled further
//   0x04 RX I/Q read: two signed 24-bit values per sample (I then Q), 8/4/2/1
//        samples per DRDY for rate index 0-3; source: a complex tone
//   0x08 version read: 3 bytes, settable (default 1.0.0)
//   0x05/0x06/0x09/0x0A: must not be sent (his/80); counted (ForbiddenCommands)
//        and logged as a warning
// DRDY: a rising edge per sample block at drdyRate in virtual time (his/80:
// about 48 kHz, MEDIUM).
// HYPOTHESIS (not in the HIS, documented): byte order of the 24-bit values
// (big-endian, as the 0x01 frame), status byte layout, the RX source.
using System;
using System.Collections.Generic;
using Antmicro.Renode.Core;
using Antmicro.Renode.Logging;
using Antmicro.Renode.Peripherals;
using Antmicro.Renode.Peripherals.Bus;
using Antmicro.Renode.Peripherals.Timers;
using Antmicro.Renode.Time;

namespace Antmicro.Renode.Peripherals.GPIOPort
{
    public class G7250GpioeFpga : IDoubleWordPeripheral, IKnownSize, INumberedGPIOOutput
    {
        private uint moder, otyper, ospeedr, pupdr, odr, lckr, afrl, afrh;
        private readonly GPIO[] pins = new GPIO[16];
        private readonly Dictionary<int, IGPIO> conn = new Dictionary<int, IGPIO>();
        private readonly object sync = new object();
        private readonly LimitTimer drdy;
        private readonly List<byte> frame = new List<byte>();
        private byte[] lastFrame = new byte[0];
        private readonly byte[] version = { 1, 0, 0 };
        private readonly byte[] status = new byte[7];
        private int cmd = -1, readIdx;
        private bool stbPrev;
        private byte testReg, dataOut;
        private ulong sample;
        private int sampleByte;
        private ulong txBytes, forbidden, drdyCount;
        private readonly ulong[] cmdCount = new ulong[256];
        private double toneHz, amplitude;

        public G7250GpioeFpga(IMachine machine, long drdyRate = 48000, double rxToneHz = 1000.0, double rxAmplitude = 0.01)
        {
            for(int i = 0; i < 16; i++) { pins[i] = new GPIO(); conn[i] = pins[i]; }
            Connections = conn;
            toneHz = rxToneHz;
            amplitude = rxAmplitude;
            drdy = new LimitTimer(machine.ClockSource, (ulong)drdyRate, this, "drdy", 1,
                                  direction: Direction.Descending, enabled: true,
                                  workMode: WorkMode.Periodic, eventEnabled: true);
            drdy.LimitReached += () => { drdyCount++; pins[10].Set(true); pins[10].Set(false); };
            Reset();
        }

        public long Size { get { return 0x400; } }
        public IReadOnlyDictionary<int, IGPIO> Connections { get; private set; }

        // ---- harness / test-bench access (interface observations only) ----
        public string FrameHex { get { return BitConverter.ToString(lastFrame).Replace("-", ""); } }
        public uint NcoWord(int offset)
        {
            if(offset < 0 || offset + 4 > lastFrame.Length) return 0;
            return ((uint)lastFrame[offset] << 24) | ((uint)lastFrame[offset + 1] << 16) | ((uint)lastFrame[offset + 2] << 8) | lastFrame[offset + 3];
        }
        public int RateIndex { get { return lastFrame.Length >= 24 ? lastFrame[23] & 3 : 0; } }
        public ulong TxBytes { get { return txBytes; } }
        public ulong ForbiddenCommands { get { return forbidden; } }
        public ulong CommandCount(int code) { return code >= 0 && code < 256 ? cmdCount[code] : 0; }
        public ulong DrdyCount { get { return drdyCount; } }
        public void SetVersion(int major, int minor, int patch) { version[0] = (byte)major; version[1] = (byte)minor; version[2] = (byte)patch; }
        public void SetStatusByte(int index, int value) { if(index >= 0 && index < status.Length) status[index] = (byte)value; }
        public void SetRxTone(double hz, double fullScaleFraction) { toneHz = hz; amplitude = fullScaleFraction; }

        public void Reset()
        {
            lock(sync)
            {
                otyper = ospeedr = pupdr = odr = lckr = afrl = afrh = 0;
                moder = 0xFFFFFFFFu;            // RM0468 reset value of GPIOE: every pin analog
                cmd = -1; readIdx = 0; stbPrev = false; testReg = dataOut = 0;
                frame.Clear(); lastFrame = new byte[0];
                sample = 0; sampleByte = 0; txBytes = forbidden = drdyCount = 0;
                Array.Clear(cmdCount, 0, cmdCount.Length);
            }
        }

        private bool IsOutput(int pin) { return ((moder >> (2 * pin)) & 3u) == 1u; }

        private uint PinLevel(int pin)
        {
            if(IsOutput(pin)) return (odr >> pin) & 1u;
            if(pin < 8) return (uint)((dataOut >> pin) & 1);          // the FPGA drives D0-D7
            return ((pupdr >> (2 * pin)) & 3u) == 1u ? 1u : 0u;      // pull-up reads 1
        }

        private uint Idr()
        {
            uint v = 0;
            for(int i = 0; i < 16; i++) v |= PinLevel(i) << i;
            return v;
        }

        private int SamplesPerDrdy() { return 8 >> RateIndex; }

        private byte RxByte()
        {
            double fs = 384000.0 / (1 << RateIndex);
            double ph = 2 * Math.PI * toneHz * sample / fs;
            double full = 8388607.0 * amplitude;
            int v = (int)Math.Round((sampleByte < 3 ? Math.Cos(ph) : Math.Sin(ph)) * full) & 0xFFFFFF;
            int shift = 16 - 8 * (sampleByte % 3);                 // big-endian (HYPOTHESIS)
            byte b = (byte)(v >> shift);
            if(++sampleByte == 6) { sampleByte = 0; sample++; }
            return b;
        }

        private byte NextReadByte()
        {
            switch(cmd)
            {
                case 0x00: return testReg;
                case 0x02: return status[(readIdx++) % status.Length];
                case 0x04: return RxByte();
                case 0x08: return version[(readIdx++) % 3];
                default: return 0;
            }
        }

        private void ProcessBus()
        {
            bool stb = ((odr >> 9) & 1u) != 0;
            bool cmdPin = ((odr >> 8) & 1u) != 0;
            if(stb && !stbPrev)
            {
                if(cmdPin)
                {
                    FlushFrame();
                    cmd = (int)(odr & 0xFF); readIdx = 0;
                    cmdCount[cmd]++;
                    if(cmd == 0x04) sampleByte = 0;
                    if(cmd == 0x05 || cmd == 0x06 || cmd == 0x09 || cmd == 0x0A)
                    {
                        forbidden++;
                        this.Log(LogLevel.Warning, "FPGA: command 0x{0:X2} received (his/80: must not be sent)", cmd);
                    }
                }
                else if(IsOutput(0))
                {
                    byte d = (byte)(odr & 0xFF);
                    if(cmd == 0x00) { testReg = d; dataOut = d; }
                    else if(cmd == 0x01 && frame.Count < 64) frame.Add(d);
                    else if(cmd == 0x03) txBytes++;
                }
                else dataOut = NextReadByte();
            }
            stbPrev = stb;
        }

        private void FlushFrame()
        {
            if(cmd == 0x01 && frame.Count > 0)
            {
                lastFrame = frame.ToArray();
                this.Log(LogLevel.Info, "FPGA cmd 0x01 frame {0} B: {1}", lastFrame.Length, FrameHex);
            }
            frame.Clear();
        }

        public uint ReadDoubleWord(long offset)
        {
            lock(sync)
            {
                switch(offset)
                {
                    case 0x00: return moder;
                    case 0x04: return otyper;
                    case 0x08: return ospeedr;
                    case 0x0C: return pupdr;
                    case 0x10: return Idr();
                    case 0x14: return odr;
                    case 0x1C: return lckr;
                    case 0x20: return afrl;
                    case 0x24: return afrh;
                    default: return 0;
                }
            }
        }

        public void WriteDoubleWord(long offset, uint value)
        {
            lock(sync)
            {
                switch(offset)
                {
                    case 0x00: moder = value; break;
                    case 0x04: otyper = value; break;
                    case 0x08: ospeedr = value; break;
                    case 0x0C: pupdr = value; break;
                    case 0x14: odr = value & 0xFFFF; ProcessBus(); break;
                    case 0x18:                                        // BSRR: set wins over reset
                        odr &= ~(value >> 16);
                        odr |= value & 0xFFFF;
                        ProcessBus(); break;
                    case 0x1C: lckr = value; break;
                    case 0x20: afrl = value; break;
                    case 0x24: afrh = value; break;
                }
            }
        }
    }
}
