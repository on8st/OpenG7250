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
// Format v1.8 (docs/FLASH-model.md): erase durations per PSIZE (DS13311 Rev 5 Table 48),
// the erase done here to 0xFF (Renode's controller does not see START), flash.log,
// flash.fault, flash.ecc (flags and FLASH_ECC_FA1R), option-byte writes refused.
// Not modelled: the flash-word programming duration, the 256-bit write buffer rules
// (all-ones writes, FW, INCERR), the bus error of a double ECC error, read-while-write on bank 2.
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
            cut.LimitReached += () =>
            {
                cut.Enabled = false;
                lock(sync)
                {
                    double f = eraseLenUs > 0 ? Math.Min(1.0, (NowUs() - eraseStartUs) / (double)eraseLenUs) : 0;
                    eraseResultExtra = ", \"fraction_done\": " + f.ToString("0.###", System.Globalization.CultureInfo.InvariantCulture);
                    SetEraseResult("cut", NowUs());
                }
                Stop("cut during erase");
            };
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
        public ulong EopReads { get; private set; }       // SR1 reads with EOP while PG (diagnostics)
        public ulong WordsNotFound { get; private set; }  // an EOP with no changed word found
        public string StopReason { get; private set; } = "";
        private ulong refreshesAtStart;

        public void CutAfterWords(long k) { lock(sync) { cutAfter = k; words = 0; } }
        public bool NotifyCut { get; set; }   // power.cut from the bench: the runner power-cycles

        // ---- format v1.8: erase durations per PSIZE from ST DS13311 Rev 5 Table 48, ----
        // ---- flash.log, the program address of every flash word, slot.corrupt      ----
        // sector (128 KiB) erase, typical and maximum, s, for PSIZE x8, x16, x32, x64 (x64 is
        // not listed: typical as x32, maximum as x8, "the longest listed figure")
        private static readonly double[] SectorTyp = { 2.0, 1.8, 1.1, 1.1 }, SectorMax = { 4.0, 3.6, 2.2, 4.0 };
        private static readonly double[] BankTyp = { 13, 8, 6, 5 }, BankMax = { 26, 16, 12, 10 };
        private bool stretch;                 // flash.timing {stretch: true}: every erase at its maximum
        private long eraseOverrideUs = -1;    // flash.timing {erase_us}: a fixed duration instead
        public void SetStretch(bool on) { lock(sync) stretch = on; }
        public void SetEraseUs(long us) { lock(sync) eraseOverrideUs = us; }
        public string TimingJson()
        {
            lock(sync)
                return "{\"erase_us\": " + (eraseOverrideUs >= 0 ? eraseOverrideUs.ToString() : "null") + ", \"stretch\": " + (stretch ? "true" : "false")
                    + ", \"program_us\": 0, \"t_erase_max_us\": " + (long)(SectorMax[3] * 1e6)
                    + ", \"source\": \"DS13311 Rev 5 Table 48 (x64 not listed: typical as x32, maximum as x8); program time not modelled\"}";
        }

        private ulong EraseUs(bool bank, int psize)
        {
            if(eraseOverrideUs >= 0) return (ulong)eraseOverrideUs;
            if(eraseSectorUs > 0 && !bank) return eraseSectorUs;     // the runner's --flash-erase-us
            double s = bank ? (stretch ? BankMax[psize] : BankTyp[psize]) : (stretch ? SectorMax[psize] : SectorTyp[psize]);
            return (ulong)(s * 1e6);
        }

        private readonly List<string> flashLog = new List<string>();
        private int flashLogDropped;
        private byte[] shadow;                 // the bank as last seen, to find each programmed word
        private ulong lastProg = 0;            // address of the last programmed flash word
        private int progEntry = -1;            // index of the open "program" entry (coalesced runs)
        private ulong progStart, progEnd; private int progWords; ulong progT;
        public string FlashLogRead(int from)
        {
            lock(sync)
            {
                CloseProgram();
                from = Math.Max(0, Math.Min(from, flashLog.Count));
                int to = Math.Min(flashLog.Count, from + 5000);
                return "{\"t_us\": " + NowUs() + ", \"count\": " + flashLog.Count + ", \"dropped\": " + flashLogDropped
                    + ", \"from\": " + from + ", \"next\": " + to + ", \"entries\": [" + string.Join(", ", flashLog.GetRange(from, to - from)) + "]}";
            }
        }
        private void Add(string e) { if(flashLog.Count >= 100000) flashLogDropped++; else flashLog.Add(e); }
        private static int Sector(ulong a) { return (int)((a - FlashBase) / SectorSize); }
        // fpga_bus at an operation's start: "open" while a bus cycle is open, i.e. STB (PE9,
        // his/80) is an output driven low; "idle" otherwise (GPIOE MODER and ODR, RM0468)
        private string FpgaBus()
        {
            uint moder = machine.SystemBus.ReadDoubleWord(0x58021000UL);
            uint odr = machine.SystemBus.ReadDoubleWord(0x58021014UL);
            bool stbOut = ((moder >> 18) & 3u) == 1u;
            return stbOut && ((odr >> 9) & 1u) == 0 ? "open" : "idle";
        }

        private void EnsureShadow()
        {
            if(shadow == null) shadow = machine.SystemBus.ReadBytes(FlashBase, (int)(FlashEnd - FlashBase));
        }

        // the flash word the last EOP belongs to: the one after the last programmed word
        // normally; else the first changed word in that sector; else anywhere in the bank
        private void FindProgrammed()
        {
            EnsureShadow();
            ulong found = 0;
            ulong cand = lastProg != 0 ? lastProg + 32 : 0;
            if(cand != 0 && cand + 32 <= FlashEnd && Changed(cand)) found = cand;
            if(found == 0 && lastProg != 0)
            {
                ulong s0 = FlashBase + (ulong)Sector(lastProg) * SectorSize;
                found = Scan(s0, s0 + SectorSize);
            }
            if(found == 0) found = Scan(FlashBase, FlashEnd);
            if(found == 0) { WordsNotFound++; return; }          // all ones written over erased cells: nothing changed
            if(TakeFault("program"))
            {
                // the word is not programmed: the array keeps its old content; the flag is set
                var old = new byte[32];
                Array.Copy(shadow, (int)(found - FlashBase), old, 0, 32);
                machine.SystemBus.WriteBytes(old, found);
                CloseProgram();
                if(!string.IsNullOrEmpty(faultFlag)) errFlags |= Flags[faultFlag];
                Add("{\"t_us\": " + NowUs() + ", \"op\": \"program\", \"bank\": 1, \"sector\": " + Sector(found)
                    + ", \"address\": \"0x" + found.ToString("X8") + "\", \"len\": 32, \"words\": 1, \"result\": \"error:" + faultFlag
                    + "\", \"fpga_bus\": \"" + FpgaBus() + "\"}");
                return;
            }
            var w = machine.SystemBus.ReadBytes(found, 32);
            Array.Copy(w, 0, shadow, (int)(found - FlashBase), 32);
            // coalesce back-to-back words at increasing addresses in one sector
            if(progEntry >= 0 && found == progEnd && Sector(found) == Sector(progStart)) { progEnd += 32; progWords++; }
            else
            {
                CloseProgram();
                progEntry = 1; progStart = found; progEnd = found + 32; progWords = 1; progT = NowUs();
            }
            lastProg = found;
        }

        private bool Changed(ulong a)
        {
            var w = machine.SystemBus.ReadBytes(a, 32);
            int o = (int)(a - FlashBase);
            for(int i = 0; i < 32; i++) if(w[i] != shadow[o + i]) return true;
            return false;
        }

        private ulong Scan(ulong lo, ulong hi)
        {
            var d = machine.SystemBus.ReadBytes(lo, (int)(hi - lo));
            int o = (int)(lo - FlashBase);
            for(int i = 0; i < d.Length; i += 32)
                for(int k = 0; k < 32; k++)
                    if(d[i + k] != shadow[o + i + k]) return lo + (ulong)i;
            return 0;
        }

        private void CloseProgram(string result = "ok")
        {
            if(progEntry < 0) return;
            Add("{\"t_us\": " + progT + ", \"op\": \"program\", \"bank\": 1, \"sector\": " + Sector(progStart)
                + ", \"address\": \"0x" + progStart.ToString("X8") + "\", \"len\": " + (progEnd - progStart) + ", \"words\": " + progWords
                + ", \"result\": \"" + result + "\"" + (result == "cut" ? ", \"words_written\": " + (progWords - 1) + ", \"torn\": \"0x" + lastProg.ToString("X8") + "\"" : "")
                + ", \"fpga_bus\": \"" + FpgaBus() + "\"}");
            progEntry = -1;
        }

        // slot.corrupt: one byte of the array changed directly (a disturbed cell, ECC consistent)
        private readonly Dictionary<ulong, byte> corrupted = new Dictionary<ulong, byte>();
        public string Corrupt(long address, int xor, bool restore)
        {
            ulong a = (ulong)address;
            lock(sync)
            {
                EnsureShadow();
                byte old = machine.SystemBus.ReadByte(a);
                byte nw;
                if(restore)
                {
                    if(!corrupted.ContainsKey(a)) return null;
                    nw = corrupted[a]; corrupted.Remove(a);
                }
                else { if(!corrupted.ContainsKey(a)) corrupted[a] = old; nw = (byte)(old ^ (xor & 0xFF)); }
                machine.SystemBus.WriteByte(a, nw);
                shadow[(int)(a - FlashBase)] = nw;
                CloseProgram();
                Add("{\"t_us\": " + NowUs() + ", \"op\": \"" + (restore ? "restore" : "corrupt") + "\", \"address\": \"0x" + a.ToString("X8") + "\", \"old\": " + old + ", \"new\": " + nw + "}");
                return "{\"address\": \"0x" + a.ToString("X8") + "\", \"old\": " + old + ", \"new\": " + nw + ", \"t_us\": " + NowUs() + "}";
            }
        }

        // ---- flash.fault, flash.ecc (format v1.8) ----
        // SR1 error flags, RM0468 bit positions (CCR1 clears them at the same positions)
        private static readonly Dictionary<string, uint> Flags = new Dictionary<string, uint>
        {
            { "WRPERR", 1u << 17 }, { "PGSERR", 1u << 18 }, { "STRBERR", 1u << 19 }, { "INCERR", 1u << 21 },
            { "RDPERR", 1u << 23 }, { "RDSERR", 1u << 24 }, { "SNECCERR", 1u << 25 }, { "DBECCERR", 1u << 26 },
            { "CRCRDERR", 1u << 28 }
        };
        private uint errFlags;
        private bool pendingEop;
        private string faultOp; private long faultAfter; private string faultFlag; private bool faultNeverEnd;
        private uint eccFa;
        public string Fault(string op, long after, string flag, bool neverEnd)
        {
            if(op != "erase" && op != "program") return "op must be erase or program";
            if(flag != null && flag != "" && !Flags.ContainsKey(flag)) return "unknown flag " + flag + " (RM0468 SR1 names)";
            if(string.IsNullOrEmpty(flag) && !neverEnd) return "a flag or never_end is needed";
            lock(sync) { faultOp = op; faultAfter = Math.Max(0, after); faultFlag = flag ?? ""; faultNeverEnd = neverEnd; }
            return null;
        }
        private bool TakeFault(string op)
        {
            if(faultOp != op) return false;
            if(faultAfter > 0) { faultAfter--; return false; }
            faultOp = null;
            return true;
        }

        // the ECC error words: a read of one sets SNECCERR or DBECCERR and FLASH_ECC_FA1R and is
        // a flash.log entry (once while the flag is set). Not modelled: the bus error a double
        // error gives on the AXI read (the data reads as stored)
        private readonly Dictionary<ulong, bool> eccWords = new Dictionary<ulong, bool>();
        public string AddEcc(long address, bool dbl)
        {
            ulong w = (ulong)address & ~31UL;
            if(w < FlashBase || w >= FlashEnd) return "address outside the flash bank";
            lock(sync)
            {
                if(eccWords.ContainsKey(w)) { eccWords[w] = dbl || eccWords[w]; return null; }
                eccWords[w] = dbl;
            }
            if(!eccLifted) InstallEcc(w);
            return null;
        }

        // Renode's controller watches the bank's writes itself while PG is set; a watchpoint of
        // ours at the same addresses left it blind (no EOP). The ECC watchpoints are lifted
        // while PG is set and put back at the first controller access after PG is cleared.
        private readonly List<Tuple<ulong, BusHookDelegate>> eccHooks = new List<Tuple<ulong, BusHookDelegate>>();
        private bool eccLifted;
        private void InstallEcc(ulong w)
        {
            var bus = machine.SystemBus;
            foreach(var width in new[] { SysbusAccessWidth.Byte, SysbusAccessWidth.Word, SysbusAccessWidth.DoubleWord, SysbusAccessWidth.QuadWord })
            {
                int step = width == SysbusAccessWidth.Byte ? 1 : width == SysbusAccessWidth.Word ? 2 : width == SysbusAccessWidth.DoubleWord ? 4 : 8;
                for(int o = 0; o < 32; o += step)
                {
                    BusHookDelegate h = (cpu, a, wd, v) => EccRead(w);
                    bus.AddWatchpointHook(w + (ulong)o, width, Access.Read, h);
                    eccHooks.Add(Tuple.Create(w + (ulong)o, h));
                }
            }
        }
        private void LiftEcc()
        {
            if(eccLifted || eccWords.Count == 0) { eccLifted = eccLifted || eccWords.Count > 0; return; }
            foreach(var t in eccHooks) machine.SystemBus.RemoveWatchpointHook(t.Item1, t.Item2);
            eccHooks.Clear();
            eccLifted = true;
        }
        private void RestoreEcc()
        {
            if(!eccLifted) return;
            eccLifted = false;
            foreach(var w in eccWords.Keys) InstallEcc(w);
        }
        public void AddEccText(string address, bool dbl) { AddEcc(Convert.ToInt64(address, 16), dbl); }
        private void EccRead(ulong w)
        {
            lock(sync)
            {
                bool dbl = eccWords[w];
                uint f = dbl ? Flags["DBECCERR"] : Flags["SNECCERR"];
                if((errFlags & f) != 0) return;
                errFlags |= f;
                eccFa = (uint)((w - FlashBase) / 32);
                CloseProgram();
                Add("{\"t_us\": " + NowUs() + ", \"op\": \"ecc\", \"type\": \"" + (dbl ? "double" : "single") + "\", \"address\": \"0x" + w.ToString("X8") + "\"}");
            }
        }

        private void Fill(ulong addr, ulong size)
        {
            var ff = new byte[size];
            for(ulong i = 0; i < size; i++) ff[i] = 0xFF;
            machine.SystemBus.WriteBytes(ff, addr);
        }

        private ulong eraseUs; private string eraseResultExtra = "";
        private void SetEraseResult(string result, ulong tEnd)
        {
            if(eraseEntry < 0 || eraseEntry >= flashLog.Count) return;
            bool bank = eraseSize > SectorSize;
            flashLog[eraseEntry] = "{\"t_us\": " + eraseT0 + ", \"op\": \"" + (bank ? "bank_erase" : "erase") + "\", \"bank\": 1"
                + (bank ? "" : ", \"sector\": " + Sector(eraseAddr) + ", \"address\": \"0x" + eraseAddr.ToString("X8") + "\", \"len\": " + eraseSize)
                + ", \"t_end_us\": " + tEnd + ", \"result\": \"" + result + "\"" + eraseResultExtra + ", \"fpga_bus\": \"" + eraseFpga + "\""
                + ", \"wall_s\": " + (DateTime.UtcNow - eraseWall0).TotalSeconds.ToString("0.000", System.Globalization.CultureInfo.InvariantCulture)
                + ", \"fetch_stalls\": " + FetchesDuringErase + "}";
        }
        private string eraseFpga = "unknown";

        // While an erase is busy the image polls SR1 (a C# bus access each time) and the
        // emulation ran at about 0.13 of real time. The core's MIPS is divided by
        // EraseMipsDivisor for the erase: each instruction then takes that much more
        // emulated time, so the erase passes faster in wall time; the image's own work during
        // an erase runs that much slower in emulated time (a speed device, docs/FLASH-model.md).
        public int EraseMipsDivisor { get; set; } = 10;
        private readonly Dictionary<object, uint> savedMips = new Dictionary<object, uint>();
        private void SlowCore(bool on)
        {
            if(EraseMipsDivisor <= 1) return;
            foreach(var cpu in machine.SystemBus.GetCPUs())
            {
                var pr = cpu.GetType().GetProperty("PerformanceInMips");
                if(pr == null || !pr.CanWrite) continue;
                if(on && !savedMips.ContainsKey(cpu))
                {
                    uint m = Convert.ToUInt32(pr.GetValue(cpu));
                    savedMips[cpu] = m;
                    pr.SetValue(cpu, Convert.ChangeType(Math.Max(1u, m / (uint)EraseMipsDivisor), pr.PropertyType));
                }
                else if(!on && savedMips.ContainsKey(cpu))
                {
                    pr.SetValue(cpu, Convert.ChangeType(savedMips[cpu], pr.PropertyType));
                    savedMips.Remove(cpu);
                }
            }
        }
        private DateTime eraseWall0;
        public ulong TornWord { get; private set; }

        // power.cut {at_address}: the cut lands on the programming of that flash word
        private ulong cutAtAddress;
        public void CutAtAddress(long a) { lock(sync) cutAtAddress = (ulong)a & ~31UL; }
        public void CutDuringErase(double fraction) { lock(sync) cutFraction = fraction; }

        public long Size { get { return 0x10; } }
        public uint ReadDoubleWord(long offset) { return offset == 0 ? (uint)Erases : offset == 4 ? (uint)words : 0u; }
        public void WriteDoubleWord(long offset, uint value) { }
        public void Reset()
        {
            lock(sync)
            {
                if(erasing) { SetEraseResult(neverEnding ? "reset" : "reset", NowUs()); }
                SlowCore(false);
                erasing = false; neverEnding = false; lastCr1 = LOCK; keyStep = 0; errFlags = 0; pendingEop = false; eopSeen = false;
            }
            busy.Enabled = false; Unstall();
        }

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
            lock(sync)
            {
                words++;
                FindProgrammed();
                stop = cutAfter > 0 && (long)words >= cutAfter;
                if(stop) cutAfter = -1;
                if(!stop && cutAtAddress != 0 && lastProg == cutAtAddress) { stop = true; cutAtAddress = 0; }
                if(stop) { TornWord = lastProg; CloseProgram("cut"); }
            }
            if(stop) Stop("cut after flash word " + words);
        }

        private void Stop(string why)
        {
            StoppedByCut = true;
            StopReason = why;
            foreach(var cpu in machine.SystemBus.GetCPUs()) cpu.IsHalted = true;
            this.Log(LogLevel.Info, "flash: core stopped ({0})", why);
            if(NotifyCut)
            {
                IPeripheral ag;
                if(machine.TryGetByName("sysbus.spi1.agent", out ag))
                {
                    var m = ag.GetType().GetMethod("FlashCut");
                    if(m != null) m.Invoke(ag, new object[] { why });
                }
            }
        }

        private uint BeforeWrite(uint value, long offset)
        {
            if(offset != CR1 && eccLifted && (lastCr1 & PG) == 0) lock(sync) RestoreEcc();
            if(offset == 0x004)
            {
                // KEYR1: the unlock sequence 0x45670123, 0xCDEF89AB clears LOCK
                lock(sync)
                {
                    if(value == 0x45670123u) keyStep = 1;
                    // the bank as it is before any programming of this boot (to find each programmed word)
                    else if(value == 0xCDEF89ABu && keyStep == 1) { keyStep = 0; lastCr1 &= ~LOCK; EnsureShadow(); }
                    else keyStep = 0;
                }
                return value;
            }
            if(offset == 0x014)
            {
                // CCR1: CLR_EOP and the CLR_ error bits (same bit positions as SR1)
                lock(sync) { if((value & EOP) != 0) { eopSeen = false; pendingEop = false; } errFlags &= ~value; }
                return value;
            }
            if(offset == 0x018 && (value & 0x2u) != 0)
            {
                // FLASH_OPTCR OPTSTART: an option-byte write is never carried out here
                lock(sync) { CloseProgram(); Add("{\"t_us\": " + NowUs() + ", \"op\": \"option_write\", \"result\": \"refused\"}"); }
                this.Log(LogLevel.Warning, "flash: option-byte write started by the image: refused (not modelled)");
                return value & ~0x2u;
            }
            if(offset != CR1) return value;
            lock(sync)
            {
                if((lastCr1 & LOCK) != 0) return lastCr1;           // locked: the write is ignored
                bool start = (value & START) != 0 && (value & (SER | BER)) != 0;
                lastCr1 = value;
                if((value & PG) != 0) LiftEcc();
                if(start)
                {
                    bool bank = (value & BER) != 0;
                    int snb = (int)((value >> 8) & 7);
                    int psize = (int)((value >> 4) & 3);
                    ulong us = EraseUs(bank, psize);
                    BeginErase(bank ? FlashBase : FlashBase + (ulong)snb * SectorSize, bank ? FlashEnd - FlashBase : SectorSize, us);
                    // the erase is the layer's (to the erased state 0xFF, RM0468): Renode's
                    // controller does not see START
                    return value & ~START;
                }
                if((value & (1u << 1)) != 0 && (value & ~LOCK) != 0) { }   // PG: programming through Renode's controller
                return value;
            }
        }

        private uint AfterRead(uint value, long offset)
        {
            if(offset == CR1) { lock(sync) { lastCr1 = value; if((value & PG) == 0) RestoreEcc(); } return value; }
            if(offset == 0x054) { lock(sync) return eccFa; }        // FLASH_ECC_FA1R
            if(offset == SR1)
            {
                bool word = false;
                lock(sync)
                {
                    value |= errFlags | (pendingEop ? EOP : 0u);
                    if(erasing) return value | BSY | QW;
                    // a flash word programmed: the controller reports EOP; counted once
                    // until the image clears EOP through CCR1
                    if((value & EOP) != 0 && (lastCr1 & PG) != 0) EopReads++;
                    if((value & EOP) != 0 && !eopSeen && (lastCr1 & PG) != 0) { eopSeen = true; word = true; }
                }
                if(word) Word();
            }
            return value;
        }

        private void BeginErase(ulong addr, ulong size, ulong us)
        {
            Erases++;
            CloseProgram();
            EnsureShadow();
            eraseEntry = flashLog.Count;
            eraseT0 = NowUs();
            eraseFpga = FpgaBus();
            eraseWall0 = DateTime.UtcNow;
            eraseResultExtra = "";
            bool fault = TakeFault("erase");
            eraseFaultFlag = fault ? faultFlag : null;
            bool never = fault && faultNeverEnd;
            Add(EraseEntry(addr, size, us, "running"));
            lastProg = 0;
            eraseAddr = addr; eraseSize = size; eraseUs = us;
            snapshot = machine.SystemBus.ReadBytes(addr, (int)size);
            PrimaskAtErase = Primask();
            refreshesAtStart = WatchdogRefreshes();
            if(never)
            {
                // flash.fault never_end: BSY stays set until a reset
                erasing = true; eraseStartUs = NowUs(); eraseLenUs = 0; neverEnding = true;
                SetBlockHook(true);
                this.Log(LogLevel.Info, "flash: erase of 0x{0:X8} never ends (flash.fault)", addr);
                return;
            }
            if(us == 0) { RefreshesDuringLastErase = 0; FinishErase(); return; }
            erasing = true;
            eraseStartUs = NowUs();
            eraseLenUs = us;
            SlowCore(true);
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

        private int eraseEntry = -1; private ulong eraseT0;
        private string EraseEntry(ulong addr, ulong size, ulong us, string result)
        {
            bool bank = size > SectorSize;
            return "{\"t_us\": " + eraseT0 + ", \"op\": \"" + (bank ? "bank_erase" : "erase") + "\", \"bank\": 1"
                + (bank ? "" : ", \"sector\": " + Sector(addr) + ", \"address\": \"0x" + addr.ToString("X8") + "\", \"len\": " + size)
                + ", \"t_end_us\": " + (eraseT0 + us) + ", \"result\": \"" + result + "\", \"fpga_bus\": \"" + FpgaBus() + "\"}";
        }

        private string eraseFaultFlag;
        private bool neverEnding;

        // the erase's end: the sector erased (0xFF), or with a fault flag left as it was
        private void FinishErase()
        {
            SlowCore(false);
            if(!string.IsNullOrEmpty(eraseFaultFlag))
            {
                errFlags |= Flags[eraseFaultFlag];
                SetEraseResult("error:" + eraseFaultFlag, NowUs());
            }
            else
            {
                Fill(eraseAddr, eraseSize);
                for(ulong i = 0; i < eraseSize; i++) shadow[(int)(eraseAddr - FlashBase + i)] = 0xFF;
                SetEraseResult("ok", NowUs());
            }
            eraseFaultFlag = null;
            pendingEop = true;
        }

        private void Done()
        {
            busy.Enabled = false;
            lock(sync) { erasing = false; FinishErase(); }
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
                    for(ulong i = 0; i < done; i++) data[eraseAddr - FlashBase + i] = 0xFF;
                    this.Log(LogLevel.Info, "flash: power lost during the erase at {0:P0}: the rest of the sector keeps its old content", f);
                }
            }
            System.IO.File.WriteAllBytes(path, data);
        }
    }
}
