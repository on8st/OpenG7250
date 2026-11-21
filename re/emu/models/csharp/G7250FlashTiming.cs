// Flash timing, lock and power-loss layer on Renode 1.17's STM32H7_FlashController
// (clean-room, ST RM0468 flash chapter), see docs/FLASH-model.md.
// Renode's controller programs and erases correctly but instantly and accepts
// FLASH_CR1 writes while locked. Through bus hooks on the controller this layer:
//   - ignores FLASH_CR1 writes while CR1.LOCK is set (RM0468)
//   - keeps a sector erase (SER + START) or bank erase (BER + START) busy for
//     eraseSectorUs / eraseBankUs of emulated time: SR1 reads BSY and QW until
//     then (0 = instant, the controller's own behaviour). The durations are
//     parameters: the STM32H725/735 datasheet values are not set yet
//   - an instruction fetch from the flash bank (0x08000000..0x080FFFFF) while
//     the erase is busy stalls the core until it ends, as the bus does; each
//     such fetch is counted (FetchesDuringErase, FirstFetchPc)
//   - records at each erase start PRIMASK and the watchdog's refresh count, and
//     at the end the refreshes during the erase (ST-05 evidence)
//   - power loss during an erase: the sector content is the snapshot taken at
//     START with the first fraction (elapsed / duration) erased (HYPOTHESIS:
//     linear progress; the real content of an interrupted erase is undefined);
//     DumpFlash applies this before the runner keeps the flash over a power cycle
//   - counts programmed flash words by the controller's end-of-programming flag
//     (a new EOP seen in SR1 with PG set; re-armed when CCR1 clears EOP) and can
//     stop the core after the k-th one (CutAfterWords), for power-loss checks.
//     This relies on the image clearing EOP after each word; the harness checks
//     the count against the flash content.
//   - CutDuringErase(fraction): stops the core at that fraction of the next erase
// Not modelled: the flash-word programming duration, ECC (no ECC errors on
// reading half-erased or half-programmed words), read-while-write on bank 2.
using System;
using System.Collections.Generic;
using Antmicro.Renode.Core;
using Antmicro.Renode.Logging;
using Antmicro.Renode.Peripherals;
using Antmicro.Renode.Peripherals.Bus;
using Antmicro.Renode.Peripherals.Timers;
using Antmicro.Renode.Time;

namespace Antmicro.Renode.Peripherals.Miscellaneous
{
    public class G7250FlashTiming : IDoubleWordPeripheral, IKnownSize
    {
        private const long CR1 = 0x00C, SR1 = 0x010;
        private const uint LOCK = 1u << 0, SER = 1u << 2, BER = 1u << 3, START = 1u << 7;
        private const uint BSY = 1u << 0, QW = 1u << 2, EOP = 1u << 16, PG = 1u << 1;
        private const ulong FlashBase = 0x08000000UL, FlashEnd = 0x08100000UL, SectorSize = 0x20000UL;

        private readonly IMachine machine;
        private readonly LimitTimer busy, cut;
        private readonly object sync = new object();
        private readonly ulong eraseSectorUs, eraseBankUs;
        private bool erasing, stalled, hooked, blockHook;
        private uint lastCr1 = LOCK;          // CR1 reset value: locked
        private int keyStep;
        private bool eopSeen;
        private ulong eraseStartUs, eraseLenUs, eraseAddr, eraseSize;
        private byte[] snapshot;
        private double cutFraction = -1;
        private long cutAfter = -1;
        private ulong words;

        public G7250FlashTiming(IMachine machine, long eraseSectorUs = 0, long eraseBankUs = 0)
        {
            this.machine = machine;
            this.eraseSectorUs = (ulong)Math.Max(0, eraseSectorUs);
            this.eraseBankUs = (ulong)Math.Max(0, eraseBankUs);
            busy = new LimitTimer(machine.ClockSource, 1000000, this, "flash-busy", 1UL << 40,
                                  direction: Direction.Descending, enabled: false,
                                  workMode: WorkMode.Periodic, eventEnabled: true);
            busy.LimitReached += Done;
            cut = new LimitTimer(machine.ClockSource, 1000000, this, "flash-cut", 1UL << 40,
                                 direction: Direction.Descending, enabled: false,
                                 workMode: WorkMode.Periodic, eventEnabled: true);
            cut.LimitReached += () => { cut.Enabled = false; Stop("cut during erase"); };
            Hook();
        }

        // ---- evidence for the harness ----
        public ulong Erases { get; private set; }
        public ulong FetchesDuringErase { get; private set; }
        public ulong FirstFetchPc { get; private set; }
        public string PrimaskAtErase { get; private set; } = "";
        public ulong RefreshesDuringLastErase { get; private set; }
        public ulong WordsProgrammed { get { return words; } }
        public bool Erasing { get { return erasing; } }
        public bool StoppedByCut { get; private set; }
        public string StopReason { get; private set; } = "";
        private ulong refreshesAtStart;

        public void CutAfterWords(long k) { lock(sync) { cutAfter = k; words = 0; } }
        public void CutDuringErase(double fraction) { lock(sync) cutFraction = fraction; }

        public long Size { get { return 0x10; } }
        public uint ReadDoubleWord(long offset) { return offset == 0 ? (uint)Erases : offset == 4 ? (uint)words : 0u; }
        public void WriteDoubleWord(long offset, uint value) { }
        public void Reset() { lock(sync) { erasing = false; lastCr1 = LOCK; keyStep = 0; } busy.Enabled = false; Unstall(); }

        private ulong NowUs() { return (ulong)machine.LocalTimeSource.ElapsedVirtualTime.TotalMicroseconds; }

        private void Hook()
        {
            if(hooked) return;
            IPeripheral fc;
            if(!machine.TryGetByName("sysbus.flashController", out fc) || !(fc is IBusPeripheral)) return;
            var bus = machine.SystemBus;
            var p = (IBusPeripheral)fc;
            bus.SetHookBeforePeripheralWrite<uint>(p, (value, offset) => BeforeWrite(value, offset));
            bus.SetHookAfterPeripheralRead<uint>(p, (value, offset) => AfterRead(value, offset));
            hooked = true;
        }

        private void Word()
        {
            bool stop;
            lock(sync) { words++; stop = cutAfter > 0 && (long)words >= cutAfter; if(stop) cutAfter = -1; }
            if(stop) Stop("cut after flash word " + words);
        }

        private void Stop(string why)
        {
            StoppedByCut = true;
            StopReason = why;
            foreach(var cpu in machine.SystemBus.GetCPUs()) cpu.IsHalted = true;
            this.Log(LogLevel.Info, "flash: core stopped ({0})", why);
        }

        private uint BeforeWrite(uint value, long offset)
        {
            if(offset == 0x004)
            {
                // KEYR1: the unlock sequence 0x45670123, 0xCDEF89AB clears LOCK
                lock(sync)
                {
                    if(value == 0x45670123u) keyStep = 1;
                    else if(value == 0xCDEF89ABu && keyStep == 1) { keyStep = 0; lastCr1 &= ~LOCK; }
                    else keyStep = 0;
                }
                return value;
            }
            if(offset == 0x014) { if((value & EOP) != 0) lock(sync) eopSeen = false; return value; }   // CCR1: CLR_EOP
            if(offset != CR1) return value;
            lock(sync)
            {
                if((lastCr1 & LOCK) != 0) return lastCr1;           // locked: the write is ignored
                bool start = (value & START) != 0 && (value & (SER | BER)) != 0;
                lastCr1 = value;
                if(start)
                {
                    bool bank = (value & BER) != 0;
                    int snb = (int)((value >> 8) & 7);
                    ulong us = bank ? eraseBankUs : eraseSectorUs;
                    BeginErase(bank ? FlashBase : FlashBase + (ulong)snb * SectorSize, bank ? FlashEnd - FlashBase : SectorSize, us);
                }
                return value;
            }
        }

        private uint AfterRead(uint value, long offset)
        {
            if(offset == CR1) { lock(sync) lastCr1 = value; return value; }
            if(offset == SR1)
            {
                bool word = false;
                lock(sync)
                {
                    if(erasing) return value | BSY | QW;
                    // a flash word programmed: the controller reports EOP; counted once
                    // until the image clears EOP through CCR1
                    if((value & EOP) != 0 && !eopSeen && (lastCr1 & PG) != 0) { eopSeen = true; word = true; }
                }
                if(word) Word();
            }
            return value;
        }

        private void BeginErase(ulong addr, ulong size, ulong us)
        {
            Erases++;
            eraseAddr = addr; eraseSize = size;
            snapshot = machine.SystemBus.ReadBytes(addr, (int)size);
            PrimaskAtErase = Primask();
            refreshesAtStart = WatchdogRefreshes();
            if(us == 0) { RefreshesDuringLastErase = 0; return; }
            erasing = true;
            eraseStartUs = NowUs();
            eraseLenUs = us;
            Arm(busy, us);
            if(cutFraction >= 0)
            {
                ulong at = Math.Max(1UL, (ulong)(us * cutFraction));
                Arm(cut, at);
                cutFraction = -1;
            }
            SetBlockHook(true);
            this.Log(LogLevel.Info, "flash: erase of 0x{0:X8} (+0x{1:X}) busy for {2} us, PRIMASK {3}", addr, size, us, PrimaskAtErase);
        }

        private static void Arm(LimitTimer t, ulong us)
        {
            // the limit stays at its large construction value; the timer counts us down to 0
            t.Enabled = false;
            t.Value = us;
            t.Enabled = true;
        }

        private void Done()
        {
            busy.Enabled = false;
            lock(sync) erasing = false;
            RefreshesDuringLastErase = WatchdogRefreshes() - refreshesAtStart;
            SetBlockHook(false);
            Unstall();
            this.Log(LogLevel.Info, "flash: erase done; fetches from flash during it: {0}; watchdog refreshes during it: {1}", FetchesDuringErase, RefreshesDuringLastErase);
        }

        // an instruction fetch from the bank while the erase runs: stall until it ends
        private void SetBlockHook(bool on)
        {
            if(on == blockHook) return;
            blockHook = on;
            foreach(var cpu in machine.SystemBus.GetCPUs())
            {
                var m = cpu.GetType().GetMethod("SetHookAtBlockBegin");
                if(m == null) continue;
                Action<ulong, uint> h = null;
                if(on) h = (pc, size) =>
                {
                    if(erasing && pc >= FlashBase && pc < FlashEnd)
                    {
                        if(FetchesDuringErase == 0) FirstFetchPc = pc;
                        FetchesDuringErase++;
                        if(!stalled) { stalled = true; cpu.IsHalted = true; }
                    }
                };
                m.Invoke(cpu, new object[] { h });
            }
        }

        private void Unstall()
        {
            if(!stalled) return;
            stalled = false;
            if(!StoppedByCut) foreach(var cpu in machine.SystemBus.GetCPUs()) cpu.IsHalted = false;
        }

        private string Primask()
        {
            foreach(var cpu in machine.SystemBus.GetCPUs())
            {
                var p = cpu.GetType().GetProperty("PRIMASK");
                if(p != null) return Convert.ToString(p.GetValue(cpu));
            }
            return "unknown";
        }

        private ulong WatchdogRefreshes()
        {
            IPeripheral wd;
            if(!machine.TryGetByName("sysbus.watchdog", out wd)) return 0;
            var p = wd.GetType().GetProperty("Refreshes");
            return p != null ? (ulong)p.GetValue(wd) : 0;
        }

        // the flash as it is after power loss now: an erase in progress is only partly done
        public void DumpFlash(string path)
        {
            var data = machine.SystemBus.ReadBytes(FlashBase, (int)(FlashEnd - FlashBase) * 2);
            lock(sync)
            {
                if(erasing && snapshot != null && eraseLenUs > 0)
                {
                    double f = Math.Min(1.0, (NowUs() - eraseStartUs) / (double)eraseLenUs);
                    ulong done = (ulong)(eraseSize * f) & ~0x1FUL;
                    for(ulong i = done; i < eraseSize; i++) data[eraseAddr - FlashBase + i] = snapshot[i];
                    this.Log(LogLevel.Info, "flash: power lost during the erase at {0:P0}: the rest of the sector keeps its old content", f);
                }
            }
            System.IO.File.WriteAllBytes(path, data);
        }
    }
}
