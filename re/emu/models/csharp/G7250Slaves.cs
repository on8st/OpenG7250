// G7250 functional bus slaves (clean-room, compiled into Renode at runtime via
// the monitor `i @models/csharp/G7250Slaves.cs`). Semantics mirror the
// host-tested logic in models/devices/*.py and are traced to the public
// datasheets + g7250-base RE docs (used as data). No stock firmware code.
using System;
using Antmicro.Renode.Peripherals.I2C;
using Antmicro.Renode.Peripherals.SPI;
using Antmicro.Renode.Logging;

namespace Antmicro.Renode.Peripherals.I2C
{
    // 24Cxx I2C EEPROM with 16-bit (2-byte) memory addressing. The Main's
    // settings loader treats the 0x1A8-byte block at 0x0000 as valid iff bytes
    // 0x000, 0x1A3 and 0x1A4 are 'd' (0x64) (g7250-base main_unit/docs +
    // OpenG7250 RE). We seed those markers so the boot logs valid settings
    // instead of "[ERR] EEPROM Settings data incorrect". Erased bytes read 0xFF.
    public class G7250Eeprom24C : II2CPeripheral
    {
        private readonly byte[] mem;
        private int pointer;
        private int addrBytesSeen;
        private bool addrPhase;

        public G7250Eeprom24C()
        {
            mem = new byte[0x4000]; // >= 24C128 (highest ref 0x24A8)
            Reset();
        }

        public void Reset()
        {
            for(int i = 0; i < mem.Length; i++) mem[i] = 0xFF;
            // settings block validity markers ('d' = 0x64)
            mem[0x000] = 0x64; mem[0x1A3] = 0x64; mem[0x1A4] = 0x64;
            pointer = 0; addrPhase = true; addrBytesSeen = 0;
        }

        public void Write(byte[] data)
        {
            // Each write transaction begins with the 2-byte memory address,
            // then optional data bytes (auto-incrementing).
            int i = 0;
            if(addrPhase)
            {
                while(i < data.Length && addrBytesSeen < 2)
                {
                    pointer = ((pointer << 8) | data[i]) & (mem.Length - 1);
                    addrBytesSeen++; i++;
                }
                if(addrBytesSeen >= 2) addrPhase = false;
            }
            for(; i < data.Length; i++)
            {
                mem[pointer] = data[i];
                pointer = (pointer + 1) & (mem.Length - 1);
            }
        }

        public byte[] Read(int count)
        {
            var o = new byte[count];
            for(int i = 0; i < count; i++)
            {
                o[i] = mem[pointer];
                pointer = (pointer + 1) & (mem.Length - 1);
            }
            return o;
        }

        public void FinishTransmission()
        {
            // next transaction starts with a fresh address phase
            addrPhase = true; addrBytesSeen = 0;
        }
    }
}

namespace Antmicro.Renode.Peripherals.SPI
{
    // The CH394 model moved to models/csharp/G7250Ch394.cs (step 3).
}

namespace Antmicro.Renode.Peripherals.Miscellaneous
{
    // Headless LTDC stub for the Control: the real Video.STM32LTDC in this build
    // does not implement CDSR (offset 0x48) and never toggles VSYNC headless, so
    // the GUI framebuffer-swap spins forever (docs/M2-control-status.md). We
    // register this in its place (after sysbus UnregisterFromAddress 0x50001000)
    // purely so the boot gets past the VSYNC wait into the GUI service loop; we
    // do not render video. Writes are echoed; CDSR bit0 (VSYNCS) toggles each
    // read so both `while(!(CDSR&1))` and `while(CDSR&1)` complete.
    public class G7250LtdcStub : Antmicro.Renode.Peripherals.Bus.IDoubleWordPeripheral,
                                 Antmicro.Renode.Peripherals.IKnownSize
    {
        private readonly System.Collections.Generic.Dictionary<long, uint> regs =
            new System.Collections.Generic.Dictionary<long, uint>();
        private uint vtick;
        public long Size { get { return 0x1000; } }
        public void Reset() { regs.Clear(); vtick = 0; }
        public uint ReadDoubleWord(long offset)
        {
            if(offset == 0x48) { vtick++; return (vtick & 1u) != 0 ? 0x0Fu : 0x00u; } // CDSR status toggle (VSYNCS/HSYNCS/VDES/HDES)
            uint v; return regs.TryGetValue(offset, out v) ? v : 0u;
        }
        public void WriteDoubleWord(long offset, uint value) { regs[offset] = value; }
    }
}

namespace Antmicro.Renode.Peripherals.Miscellaneous
{
    // G7250 RF FPGA — 8-bit bit-banged GPIOE bus (HIS 80). Minimal fused model:
    // handles the boot loopback bus-test (command 0x00 echoes the written byte)
    // and the 3-byte version read (0x08), so the Main stops logging FPGA bus
    // errors. Live 24-bit I/Q + DRDY streaming is the full M3 deliverable.
    //
    // Wiring (repl): gpioPortE.[0-9] -> fpga.[0-9]  (MCU writes reach us) and
    // fpga.[0-7] -> gpioPortE.[0-7] (we drive the data lines on reads). Data
    // direction is inferred: an STB pulse whose data pins did not change since
    // the previous pulse is a read (we drive the stored byte), else a write.
    public class G7250Fpga : Antmicro.Renode.Core.IGPIOReceiver,
                             Antmicro.Renode.Core.INumberedGPIOOutput
    {
        private readonly Antmicro.Renode.Core.GPIO[] outs;
        private int dataIn, prevLatched, cmd;
        private bool stbPrev, cmdPin;
        private bool dataChangedSinceStb;
        private byte testReg;
        private readonly byte[] version = { 1, 0, 0 };
        private int verIdx;

        public G7250Fpga()
        {
            outs = new Antmicro.Renode.Core.GPIO[11];
            for(int i = 0; i < outs.Length; i++) outs[i] = new Antmicro.Renode.Core.GPIO();
            Reset();
        }

        public System.Collections.Generic.IReadOnlyDictionary<int, Antmicro.Renode.Core.IGPIO> Connections
        {
            get
            {
                var d = new System.Collections.Generic.Dictionary<int, Antmicro.Renode.Core.IGPIO>();
                for(int i = 0; i < outs.Length; i++) d[i] = outs[i];
                return d;
            }
        }

        public void Reset()
        {
            dataIn = prevLatched = cmd = 0; stbPrev = cmdPin = false;
            dataChangedSinceStb = false; testReg = 0; verIdx = 0;
        }

        private void DriveData(byte b)
        {
            for(int i = 0; i < 8; i++) outs[i].Set(((b >> i) & 1) != 0);
        }

        public void OnGPIO(int number, bool value)
        {
            if(number >= 0 && number <= 7)
            {
                int m = 1 << number;
                int nv = value ? (dataIn | m) : (dataIn & ~m);
                if(nv != dataIn) { dataIn = nv; dataChangedSinceStb = true; }
                return;
            }
            if(number == 8) { cmdPin = value; return; }   // CMD
            if(number == 9)                                // STB
            {
                bool rising = value && !stbPrev;
                stbPrev = value;
                if(!rising) return;
                if(cmdPin)                                 // CMD+STB: latch command
                {
                    cmd = dataIn & 0xFF; verIdx = 0;
                    dataChangedSinceStb = false;
                    return;
                }
                if(dataChangedSinceStb)                     // a write strobe
                {
                    if(cmd == 0x00) testReg = (byte)(dataIn & 0xFF);
                    dataChangedSinceStb = false;
                }
                else                                        // a read strobe: drive data
                {
                    if(cmd == 0x00) DriveData(testReg);
                    else if(cmd == 0x08) DriveData(version[(verIdx++) % 3]);
                    else DriveData(0);
                }
            }
        }
    }
}
