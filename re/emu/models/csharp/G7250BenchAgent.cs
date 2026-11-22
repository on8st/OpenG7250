// Test-bench agent for product runs (clean-room), see docs/PRODUCT-RUNNER.md.
// Observes the Main's RF control interface as his/85-rf-control.md describes it
// and serves the runner's fixtures hw.observe, hw.trace and hw.input:
//   - SPI1 slave (transmit-only bus): one trace event per transfer (cs null);
//     latch inputs (GPIO) 0 = PA11 chain A (8 bit), 1 = PH12 chain B (24 bit),
//     2 = PD13, 3 = PG2, 4 = PG5 ATU latches (the CS line is the latch); a rising
//     edge latches the last 1 or 3 bytes
//   - pin levels read from the GPIO port registers (output: ODR, input: IDR)
//   - PA DAC words from the PA plant model (sysbus.spi2.pa), if present
//   - hw.input: drives a GPIO port input (OnGPIO on the port)
//   - emulated time in microseconds
// Line protocol over a local TCP link to the runner (agentPort):
//   "time" -> "time <us>"; "observe" -> one JSON line; "trace start [pins]" /
//   "trace stop" -> "ok"; while a trace runs, events are sent as "ev <json>";
//   "input <pin> <0|1>" -> "ok" or "err <text>".
// Pin changes in a trace are sampled every 1 / sampleRate of emulated time
// (default 20 kHz: 50 us resolution); latch edges and SPI words are exact.
using System;
using System.Collections.Generic;
using System.Globalization;
using System.Net.Sockets;
using System.Reflection;
using System.Text;
using System.Threading;
using Antmicro.Renode.Core;
using Antmicro.Renode.Logging;
using Antmicro.Renode.Peripherals;
using Antmicro.Renode.Peripherals.Timers;
using Antmicro.Renode.Time;

namespace Antmicro.Renode.Peripherals.SPI
{
    public class G7250BenchAgent : ISPIPeripheral, IGPIOReceiver
    {
        private static readonly string[] LatchPins = { "PA11", "PH12", "PD13", "PG2", "PG5" };
        // the HIS 85 safe pins, plus PH9 (held high in the firmware's safe state; added on request)
        private static readonly string[] SafePins = { "PF4", "PF6", "PC7", "PH11", "PA10", "PH10", "PB14", "PB10", "PD12", "PB5", "PD5", "PH9" };
        private static readonly string[] InputPins = { "PH6", "PH3", "PH4" };

        private readonly IMachine machine;
        private readonly object sync = new object();
        private readonly List<byte> shifted = new List<byte>();
        private readonly StringBuilder transfer = new StringBuilder();
        private readonly bool[] latchLevel = new bool[5];
        private int chainA = -1;
        private string chainB;
        private readonly int[] atuWrites = new int[3];
        private readonly int[] atuLast = { -1, -1, -1 };
        private readonly int agentPort;
        private TcpClient link;
        private NetworkStream stream;
        private bool tracing;
        private ulong seq, dropped;
        private static readonly string[] WatchedPins = Concat(SafePins, LatchPins);
        private static string[] Concat(string[] a, string[] b) { var r = new string[a.Length + b.Length]; a.CopyTo(r, 0); b.CopyTo(r, a.Length); return r; }
        private string[] tracePins = WatchedPins;
        private readonly Dictionary<string, int> lastLevel = new Dictionary<string, int>();
        private readonly Dictionary<string, int> drivenLevel = new Dictionary<string, int>();
        private readonly Dictionary<string, string> lastState = new Dictionary<string, string>();
        private readonly LimitTimer sampler;

        public G7250BenchAgent(IMachine machine, int agentPort = 0, long sampleRate = 20000, string consoleTimedPath = "")
        {
            this.machine = machine;
            this.agentPort = agentPort;
            timedPath = consoleTimedPath;
            sampler = new LimitTimer(machine.ClockSource, (ulong)sampleRate, this, "sample", 1,
                                     direction: Direction.Descending, enabled: false,
                                     workMode: WorkMode.Periodic, eventEnabled: true);
            sampler.LimitReached += Sample;
            if(agentPort > 0) new Thread(Serve) { IsBackground = true, Name = "bench-agent" }.Start();
            HookConsole();
            ArmBootHook();
            if(Environment.GetEnvironmentVariable("G7250_NO_PORT_HOOKS") != "1") HookPorts();
            HookIdr();
            machine.MachineReset += _ => OnMachineReset();
        }

        public void Reset()
        {
            lock(sync) { shifted.Clear(); chainA = -1; chainB = null; Array.Clear(atuWrites, 0, 3); atuLast[0] = atuLast[1] = atuLast[2] = -1; }
        }

        // ---- SPI1 ----
        public byte Transmit(byte data)
        {
            lock(sync)
            {
                shifted.Add(data); if(shifted.Count > 64) shifted.RemoveAt(0);
                transfer.Append(data.ToString("x2"));
            }
            return 0;
        }

        // end of one SPI1 transfer: a trace event with its bytes (SPI1 has no chip select)
        public void FinishTransmission()
        {
            string ev = null;
            lock(sync)
            {
                if(tracing && transfer.Length > 0)
                    ev = Event("\"kind\": \"spi\", \"bus\": \"SPI1\", \"cs\": null, \"data\": \"" + transfer + "\"");
                transfer.Clear();
                if(ev != null) Send("ev " + ev);
            }
        }

        public void OnGPIO(int number, bool value)
        {
            if(number < 0 || number >= LatchPins.Length) return;
            string ev = null;
            lock(sync)
            {
                bool rising = value && !latchLevel[number];
                latchLevel[number] = value;
                if(tracing)
                {
                    // the port write hook may have given this change already
                    var st = State(LatchPins[number]);
                    string key = st.Level + "/" + st.Mode + "/" + st.Pull, prev;
                    if(!(lastState.TryGetValue(LatchPins[number], out prev) && prev == key))
                    {
                        lastState[LatchPins[number]] = key;
                        ev = GpioState(LatchPins[number]);
                    }
                }
                if(rising && shifted.Count > 0)
                {
                    int n = number == 1 ? Math.Min(3, shifted.Count) : 1;
                    var sb = new StringBuilder();
                    for(int i = shifted.Count - n; i < shifted.Count; i++) sb.Append(shifted[i].ToString("x2"));
                    if(number == 0) chainA = shifted[shifted.Count - 1];
                    else if(number == 1) chainB = sb.ToString();
                    else { atuWrites[number - 2]++; atuLast[number - 2] = shifted[shifted.Count - 1]; }
                }
                if(ev != null) Send("ev " + ev);
            }
            if(number == 0 && keyLogOn) KeyCheck();
        }

        // ---- key.log (format v1.6): key = chain A's latched frame has bit 7 set and its
        // output enable PA10 is low; every change with the T/R levels and the bias state ----
        private const int KeyCap = 100000;
        private readonly List<string> keyLog = new List<string>();
        private volatile bool keyLogOn;
        private int keyLogDropped;
        private int lastKey = -1;

        private int KeyNow()
        {
            string m, pl;
            string pa10 = shadow.ContainsKey(0) ? ShadowLevel(0, 10, out m, out pl) : Level("PA10");
            return (chainA >= 0 && (chainA & 0x80) != 0 && pa10 == "0") ? 1 : 0;
        }

        private bool BiasNonzero()
        {
            IPeripheral pa;
            if(!machine.TryGetByName("sysbus.spi2.pa", out pa)) return false;
            var mi = pa.GetType().GetMethod("GetDacWord");
            if(mi == null) return false;
            foreach(int ch in new[] { 2, 7, 8 })
                if((int)mi.Invoke(pa, new object[] { ch }) > 0) return true;
            return false;
        }

        // CPU thread: after a chain A latch edge or a port A write
        private void KeyCheck()
        {
            int k;
            lock(sync) k = KeyNow();
            if(k == lastKey) return;
            string tr = "{\"PF4\": " + Level("PF4") + ", \"PF6\": " + Level("PF6") + ", \"PC7\": " + Level("PC7") + ", \"PH11\": " + Level("PH11") + "}";
            bool bias = BiasNonzero();
            lock(sync)
            {
                lastKey = k;
                if(!keyLogOn) return;
                if(keyLog.Count >= KeyCap) { keyLogDropped++; return; }
                keyLog.Add("{\"t_us\": " + NowUs() + ", \"on\": " + k + ", \"tr\": " + tr + ", \"bias_nonzero\": " + (bias ? "true" : "false") + "}");
            }
        }

        private object Ch394() { IPeripheral c; return machine.TryGetByName("sysbus.spi6.ch394", out c) ? c : null; }

        private string Call(object o, string method, params object[] args)
        {
            if(o == null) return null;
            var mi = o.GetType().GetMethod(method);
            return mi == null ? null : mi.Invoke(o, args) as string;
        }

        private string KeyLogCommand(string[] w)
        {
            var ch = Ch394();
            string act = w.Length > 1 ? w[1] : "";
            int fk = w.Length > 2 ? int.Parse(w[2], CultureInfo.InvariantCulture) : 0;
            int fh = w.Length > 3 ? int.Parse(w[3], CultureInfo.InvariantCulture) : 0;
            if(act == "start")
            {
                // port A shadow from the bus first (outside the lock), then exact hooks on port A
                ulong b = 0x58020000UL;
                var v = new uint[] { machine.SystemBus.ReadDoubleWord(b), machine.SystemBus.ReadDoubleWord(b + 4),
                                     machine.SystemBus.ReadDoubleWord(b + 0x0C), machine.SystemBus.ReadDoubleWord(b + 0x14) };
                Call(ch, "HpLogStart");
                lock(sync)
                {
                    shadow[0] = v;
                    keyLog.Clear(); keyLogDropped = 0;
                    lastKey = KeyNow();
                    keyLogOn = true;
                    return "ok {\"t_us\": " + NowUs() + "}";
                }
            }
            if(act != "read" && act != "stop") return "err key.log action";
            if(act == "stop") { keyLogOn = false; Call(ch, "HpLogStop"); }
            string hp = Call(ch, "HpLogRead", fh) ?? "{\"count\": 0, \"dropped\": 0, \"hp\": []}";
            lock(sync)
            {
                fk = Math.Max(0, Math.Min(fk, keyLog.Count));
                int to = Math.Min(keyLog.Count, fk + 5000);
                // hp page: take its fields from the CH394 answer
                string hpList = hp.Substring(hp.IndexOf("\"hp\": [") + 6);
                hpList = hpList.Substring(0, hpList.Length - 1);
                string hpCount = Field(hp, "count"), hpDropped = Field(hp, "dropped"), hpNext = Field(hp, "next");
                return "ok {\"t_us\": " + NowUs() + ", \"count_key\": " + keyLog.Count + ", \"count_hp\": " + hpCount
                    + ", \"dropped\": " + (keyLogDropped + int.Parse(hpDropped, CultureInfo.InvariantCulture))
                    + ", \"next_key\": " + to + ", \"next_hp\": " + hpNext
                    + ", \"key\": [" + string.Join(", ", keyLog.GetRange(fk, to - fk)) + "], \"hp\": " + hpList + "}";
            }
        }

        private static string Field(string json, string name)
        {
            int i = json.IndexOf("\"" + name + "\": ");
            if(i < 0) return "0";
            i += name.Length + 4;
            int j = i;
            while(j < json.Length && (char.IsDigit(json[j]) || json[j] == '-')) j++;
            return j > i ? json.Substring(i, j - i) : "0";
        }

        private string NetLogCommand(string[] w)
        {
            var ch = Ch394();
            if(ch == null) return "err no CH394";
            string act = w.Length > 1 ? w[1] : "";
            int from = w.Length > 2 ? int.Parse(w[2], CultureInfo.InvariantCulture) : 0;
            string r = act == "start" ? Call(ch, "NetLogStart") : act == "read" ? Call(ch, "NetLogRead", from)
                     : act == "runner" ? Call(ch, "RunLogRead", from)
                     : act == "stop" ? Call(ch, "NetLogStop", from) : null;
            return r == null ? "err net.log action" : "ok " + r;
        }

        // ---- exact scheduling in emulated time (gpio.force, dac.force) ----
        private readonly List<Tuple<ulong, Action>> schedule = new List<Tuple<ulong, Action>>();
        private LimitTimer schedTimer;

        // runs act at emulated now + delayUs on the emulation thread; returns that time
        private ulong ScheduleAt(ulong delayUs, Action act)
        {
            ulong at = 0;
            var done = new System.Threading.ManualResetEventSlim(false);
            machine.LocalTimeSource.ExecuteInNearestSyncedState(ts =>
            {
                at = NowUs() + delayUs;
                lock(schedule) { schedule.Add(Tuple.Create(at, act)); schedule.Sort((a, b) => a.Item1.CompareTo(b.Item1)); }
                ArmSchedule();
                done.Set();
            });
            done.Wait(30000);
            return at;
        }

        // from the emulation thread itself (a scheduled action scheduling another)
        private void ScheduleFromEmulation(ulong at, Action act)
        {
            lock(schedule) { schedule.Add(Tuple.Create(at, act)); schedule.Sort((a, b) => a.Item1.CompareTo(b.Item1)); }
        }

        private void ArmSchedule()
        {
            ulong head;
            lock(schedule) { if(schedule.Count == 0) return; head = schedule[0].Item1; }
            if(schedTimer == null)
            {
                schedTimer = new LimitTimer(machine.ClockSource, 1000000, this, "agentsched", 1UL << 40,
                    direction: Direction.Ascending, enabled: false, workMode: WorkMode.Periodic, eventEnabled: true);
                schedTimer.LimitReached += OnSchedule;
            }
            ulong now = NowUs();
            schedTimer.Enabled = false;
            if(head <= now) { OnSchedule(); return; }
            schedTimer.Value = (1UL << 40) - (head - now);
            schedTimer.Enabled = true;
        }

        private void OnSchedule()
        {
            var due = new List<Action>();
            lock(schedule)
            {
                if(schedTimer != null) schedTimer.Enabled = false;
                ulong now = NowUs();
                while(schedule.Count > 0 && schedule[0].Item1 <= now) { due.Add(schedule[0].Item2); schedule.RemoveAt(0); }
            }
            foreach(var a in due) { try { a(); } catch(Exception e) { this.Log(LogLevel.Warning, "agent: scheduled action: {0}", e.Message); } }
            ArmSchedule();
        }

        // ---- gpio.force (format v1.8): idr held (read hook), odr and moder written once ----
        private readonly Dictionary<int, uint> idrForceMask = new Dictionary<int, uint>();
        private readonly Dictionary<int, uint> idrForceVal = new Dictionary<int, uint>();
        private bool idrHooked;

        private void HookIdr()
        {
            if(idrHooked) return;
            for(int port = 0; port < 11; port++)
            {
                IPeripheral gp;
                if(!machine.TryGetByName("sysbus.gpioPort" + (char)('A' + port), out gp) || !(gp is Antmicro.Renode.Peripherals.Bus.IBusPeripheral)) continue;
                int pn = port;
                machine.SystemBus.SetHookAfterPeripheralRead<uint>((Antmicro.Renode.Peripherals.Bus.IBusPeripheral)gp, (value, offset) =>
                {
                    if(offset != 0x10) return value;
                    if(!firstIdrSeen && (pn == 0 || pn == 1 || pn == 2 || pn == 5 || pn == 7))
                    {
                        firstIdrSeen = true; firstIdrReadUs = NowUs(); firstIdrPort = "GPIO" + (char)('A' + pn);
                    }
                    lock(idrForceMask)
                    {
                        uint m;
                        if(!idrForceMask.TryGetValue(pn, out m) || m == 0) return value;
                        return (value & ~m) | (idrForceVal[pn] & m);
                    }
                });
            }
            idrHooked = true;
        }

        // a force from power-on: the runner puts this into the next emulator's start-up script,
        // before the core's first instruction (gpio.force at_power_on)
        public void ForceIdrAtBoot(string pin, int level)
        {
            int port, bit;
            if(!ParsePin(pin, out port, out bit)) return;
            HookIdr();
            ApplyForce(port, bit, pin, "idr", level.ToString(), false, null);
        }

        // the first read of a TX-path port's input register after each boot (GPIOA, B, C, F, H):
        // the emulator's view of when the image's first TX path check runs
        private ulong firstIdrReadUs, firstIdrBootUs;
        private string firstIdrPort;
        private bool firstIdrSeen = true;

        private void ForceEvent(string pin, string target, string val, string kind)
        {
            lock(sync) if(tracing) Send("ev " + Event("\"kind\": \"" + kind + "\", \"pin\": \"" + pin + "\", \"target\": \"" + target + "\", \"value\": " + val));
        }

        private void ApplyForce(int port, int bit, string pin, string target, string val, bool release, uint[] saved)
        {
            ulong b = 0x58020000UL + 0x400UL * (ulong)port;
            if(target == "idr")
            {
                lock(idrForceMask)
                {
                    uint m; idrForceMask.TryGetValue(port, out m);
                    uint v; idrForceVal.TryGetValue(port, out v);
                    if(release || val == "null") { m &= ~(1u << bit); }
                    else { m |= 1u << bit; v = val == "1" ? v | (1u << bit) : v & ~(1u << bit); }
                    idrForceMask[port] = m; idrForceVal[port] = v;
                }
            }
            else if(target == "odr")
            {
                uint odr = machine.SystemBus.ReadDoubleWord(b + 0x14);
                if(saved != null) saved[0] = (odr >> bit) & 1u;
                int lv = release ? (int)saved[0] : (val == "1" ? 1 : 0);
                machine.SystemBus.WriteDoubleWord(b + 0x18, lv == 1 ? (1u << bit) : (1u << (bit + 16)));
            }
            else if(target == "moder")
            {
                uint moder = machine.SystemBus.ReadDoubleWord(b);
                if(saved != null) saved[0] = (moder >> (2 * bit)) & 3u;
                uint m = release ? saved[0] : (val == "input" ? 0u : val == "output" ? 1u : val == "af" ? 2u : 3u);
                machine.SystemBus.WriteDoubleWord(b, (moder & ~(3u << (2 * bit))) | (m << (2 * bit)));
            }
            ForceEvent(pin, target, val == "null" || release ? "null" : (target == "moder" ? "\"" + val + "\"" : val), release || val == "null" ? "release" : "force");
        }

        // force <pin> <idr|odr|moder> <value> <from_us> <duration_us|-> -> ok <t_us>
        private string ForceCommand(string[] w)
        {
            if(w.Length < 6) return "err force arguments";
            int port, bit;
            if(!ParsePin(w[1], out port, out bit)) return "err bad pin " + w[1];
            string target = w[2], val = w[3];
            if(target != "idr" && target != "odr" && target != "moder") return "err target must be idr|odr|moder";
            if(target == "idr") HookIdr();
            ulong from = ulong.Parse(w[4], CultureInfo.InvariantCulture);
            var saved = new uint[1];
            long dur = w[5] == "-" ? -1 : long.Parse(w[5], CultureInfo.InvariantCulture);
            ulong at = ScheduleAt(from, () =>
            {
                ApplyForce(port, bit, w[1], target, val, false, saved);
                // the release exactly duration_us after the force itself (on the emulation thread)
                if(dur >= 0) ScheduleFromEmulation(NowUs() + (ulong)dur, () => ApplyForce(port, bit, w[1], target, val, true, saved));
            });
            return "ok " + at;
        }

        // dacforce <channel> <value> <from_us> -> ok <t_us>
        private string DacForceCommand(string[] w)
        {
            if(w.Length < 4) return "err dacforce arguments";
            int ch = int.Parse(w[1], CultureInfo.InvariantCulture), val = int.Parse(w[2], CultureInfo.InvariantCulture);
            IPeripheral pa;
            if(!machine.TryGetByName("sysbus.spi2.pa", out pa)) return "err no PA DAC model";
            var mi = pa.GetType().GetMethod("ForceDac");
            if(mi == null) return "err no ForceDac";
            ulong at = ScheduleAt(ulong.Parse(w[3], CultureInfo.InvariantCulture), () =>
            {
                mi.Invoke(pa, new object[] { ch, val });
                lock(sync) if(tracing) Send("ev " + Event("\"kind\": \"dac_force\", \"channel\": " + ch + ", \"value\": " + val));
            });
            return "ok " + at;
        }

        // advance_on / advance_off: Renode's GLOBAL advance-immediately (the master and every
        // machine's time source, as the monitor's "emulation SetGlobalAdvanceImmediately");
        // the fixture server switches it on, polls the emulated time to the target and
        // switches it off, so other fixture calls are answered during an advance
        // the 50 us pin sampler gives the clock an event every 50 us, so no idle time could be
        // skipped: it pauses during an advance and samples once at its end
        private bool samplerBeforeAdvance;

        // After an advance the emulated time is ahead of the host time the time source paces
        // against (its stopwatches), and in normal mode Renode would stall until the host
        // caught up. The stopwatches are moved forward by the time gained, so the emulation
        // goes on at real time from where the advance ended.
        private void CatchUpHostClock()
        {
            var mts = Antmicro.Renode.Core.EmulationManager.Instance.CurrentEmulation.MasterTimeSource;
            double virtUs = mts.ElapsedVirtualTime.TotalMicroseconds;
            for(var ty = mts.GetType(); ty != null && ty != typeof(object); ty = ty.BaseType)
                foreach(var fi in ty.GetFields(BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public))
                {
                    if(fi.FieldType != typeof(System.Diagnostics.Stopwatch)) continue;
                    var sw = fi.GetValue(mts) as System.Diagnostics.Stopwatch;
                    if(sw == null) continue;
                    double aheadUs = virtUs - sw.Elapsed.TotalMilliseconds * 1000.0;
                    if(aheadUs <= 0) continue;
                    long add = (long)(aheadUs * 1e-6 * System.Diagnostics.Stopwatch.Frequency);
                    var ef = typeof(System.Diagnostics.Stopwatch).GetField("_elapsed", BindingFlags.Instance | BindingFlags.NonPublic)
                          ?? typeof(System.Diagnostics.Stopwatch).GetField("elapsed", BindingFlags.Instance | BindingFlags.NonPublic);
                    if(ef == null) { this.Log(LogLevel.Warning, "agent: cannot move the host stopwatch (no elapsed field)"); continue; }
                    ef.SetValue(sw, (long)ef.GetValue(sw) + add);
                }
        }

        private string Advance(bool on)
        {
            if(on) { samplerBeforeAdvance = sampler.Enabled; sampler.Enabled = false; }
            else if(samplerBeforeAdvance) { sampler.Enabled = true; Sample(); }
            if(!on) CatchUpHostClock();
            var emu = Antmicro.Renode.Core.EmulationManager.Instance.CurrentEmulation;
            var setGlobal = emu.GetType().GetMethod("SetGlobalAdvanceImmediately");
            if(setGlobal != null) setGlobal.Invoke(emu, new object[] { on });
            else emu.MasterTimeSource.AdvanceImmediately = on;
            // the machine's own time source as well (a slave of the master)
            var lts = machine.LocalTimeSource;
            var pr = lts.GetType().GetProperty("AdvanceImmediately");
            if(pr != null && pr.CanWrite) pr.SetValue(lts, on);
            return "ok " + NowUs() + " global=" + (setGlobal != null) + " master=" + emu.MasterTimeSource.AdvanceImmediately
                + " local=" + (pr != null ? pr.GetValue(lts) : "n/a") + " localtype=" + lts.GetType().Name;
        }

        // ---- events from other models (CH394 link) ----
        private string lastLink;           // replayed at every trace start, after the boot phase

        public void ExternalEvent(string body, bool remember)
        {
            lock(sync)
            {
                string b = "\"t_us\": " + NowUs() + ", " + body;
                if(remember) lastLink = b;
                if(tracing) Send("ev {\"seq\": " + (seq++) + ", " + b + "}");
            }
        }

        // ---- pins ----
        private static bool ParsePin(string pin, out int port, out int bit)
        {
            port = bit = -1;
            if(pin == null || pin.Length < 3 || pin[0] != 'P') return false;
            port = pin[1] - 'A';
            return int.TryParse(pin.Substring(2), out bit) && port >= 0 && port < 11 && bit >= 0 && bit < 16;
        }

        // pin state per format v1.4: mode (output/input/af/analog), pull, and the
        // electrical level: 0/1 when driven (output, pull, or hw.input), "z" when
        // nothing drives it, null for an alternate-function pin the emulator does
        // not model at pin level
        private struct PinState { public string Mode, Pull, Level; }

        private PinState State(string pin)
        {
            var st = new PinState { Mode = "?", Pull = "none", Level = "null" };
            int port, bit;
            if(!ParsePin(pin, out port, out bit)) return st;
            ulong b = 0x58020000UL + 0x400UL * (ulong)port;
            var bus = machine.SystemBus;
            uint moder = (bus.ReadDoubleWord(b + 0x00) >> (2 * bit)) & 3u;
            uint otyper = (bus.ReadDoubleWord(b + 0x04) >> bit) & 1u;
            uint pupdr = (bus.ReadDoubleWord(b + 0x0C) >> (2 * bit)) & 3u;
            st.Mode = moder == 0 ? "input" : moder == 1 ? "output" : moder == 2 ? "af" : "analog";
            st.Pull = pupdr == 1 ? "up" : pupdr == 2 ? "down" : "none";
            int driven;
            bool byBench;
            lock(sync) byBench = drivenLevel.TryGetValue(pin, out driven);
            if(moder == 1)
            {
                uint odr = (bus.ReadDoubleWord(b + 0x14) >> bit) & 1u;
                if(otyper == 1 && odr == 1)      // open drain released
                    st.Level = pupdr == 1 ? "1" : pupdr == 2 ? "0" : (byBench ? driven.ToString() : "\"z\"");
                else st.Level = odr.ToString();
            }
            else if(moder == 0)
            {
                if(byBench) st.Level = driven.ToString();
                else st.Level = pupdr == 1 ? "1" : pupdr == 2 ? "0" : "\"z\"";
            }
            else if(moder == 3) st.Level = "\"z\"";
            else st.Level = "null";              // alternate function: driven by a peripheral the emulator does not model at pin level
            return st;
        }

        private string Level(string pin) { return State(pin).Level; }

        private string PinJson(string pin)
        {
            var s = State(pin);
            return "{\"level\": " + s.Level + ", \"mode\": \"" + s.Mode + "\", \"pull\": \"" + s.Pull + "\"}";
        }

        private string Input(string pin, int level)
        {
            if(Array.IndexOf(InputPins, pin) < 0) return "err pin not offered: " + pin;
            int port, bit;
            ParsePin(pin, out port, out bit);
            IPeripheral p;
            string name = "sysbus.gpioPort" + (char)('A' + port);
            if(!machine.TryGetByName(name, out p) || !(p is IGPIOReceiver)) return "err no port " + name;
            ((IGPIOReceiver)p).OnGPIO(bit, level != 0);
            lock(sync) drivenLevel[pin] = level != 0 ? 1 : 0;
            // bus reads outside the lock: the CPU thread takes it at every GPIO write
            var st = State(pin);
            lock(sync)
            {
                if(tracing) { lastState[pin] = st.Level + "/" + st.Mode + "/" + st.Pull; Send("ev " + GpioStateFrom(pin, st)); }
            }
            return "ok";
        }

        private ulong NowUs()
        {
            return (ulong)machine.LocalTimeSource.ElapsedVirtualTime.TotalMicroseconds;
        }

        private string Event(string body) { return "{\"seq\": " + (seq++) + ", \"t_us\": " + NowUs() + ", " + body + "}"; }
        private string Gpio(string pin, int level) { return Event("\"kind\": \"gpio\", \"pin\": \"" + pin + "\", \"level\": " + level); }
        private string GpioState(string pin) { return GpioStateFrom(pin, State(pin)); }
        private string GpioStateFrom(string pin, PinState s)
        {
            return Event("\"kind\": \"gpio\", \"pin\": \"" + pin + "\", \"level\": " + s.Level + ", \"mode\": \"" + s.Mode + "\", \"pull\": \"" + s.Pull + "\"");
        }

        private void Sample()
        {
            List<string> evs = null;
            string[] pins;
            lock(sync) { if(!tracing) return; pins = tracePins; }
            // bus reads outside the lock (the CPU thread takes it at every GPIO write)
            var states = new PinState[pins.Length];
            for(int i = 0; i < pins.Length; i++) states[i] = State(pins[i]);
            lock(sync)
            {
                if(!tracing) return;
                for(int i = 0; i < pins.Length; i++)
                {
                    var s = states[i];
                    string key = s.Level + "/" + s.Mode + "/" + s.Pull, prev;
                    if(lastState.TryGetValue(pins[i], out prev) && prev == key) continue;
                    lastState[pins[i]] = key;
                    (evs = evs ?? new List<string>()).Add(GpioStateFrom(pins[i], s));
                }
                if(evs != null) foreach(var e in evs) Send("ev " + e);
            }
        }

        private string DacWords(out string resetLevel)
        {
            resetLevel = Level("PB14");
            IPeripheral pa;
            var parts = new string[8];
            for(int i = 0; i < 8; i++) parts[i] = "null";
            if(machine.TryGetByName("sysbus.spi2.pa", out pa))
            {
                var m = pa.GetType().GetMethod("GetDacWord");
                if(m != null)
                    for(int ch = 1; ch <= 8; ch++) { int v = (int)m.Invoke(pa, new object[] { ch }); if(v >= 0) parts[ch - 1] = v.ToString(); }
            }
            return string.Join(", ", parts);
        }

        private string Observe()
        {
            string rst;
            string dac = DacWords(out rst);
            // pin states from the bus first, outside the lock (the CPU thread takes it at every GPIO write)
            string tr = ", \"tr_lines\": {\"PF4\": " + Level("PF4") + ", \"PF6\": " + Level("PF6") + ", \"PC7\": " + Level("PC7") + ", \"PH11\": " + Level("PH11") + "}";
            string oeA = Level("PA10"), oeB = Level("PH10"), pb10 = Level("PB10"), pd12 = Level("PD12");
            string pins = string.Join(", ", Array.ConvertAll(WatchedPins, x => "\"" + x + "\": " + PinJson(x)));
            lock(sync)
            {
                return "{\"t_us\": " + NowUs()
                    + tr
                    + ", \"spi1_a\": {\"oe_PA10\": " + oeA + ", \"last_frame\": " + (chainA < 0 ? "null" : chainA.ToString()) + "}"
                    + ", \"spi1_b\": {\"oe_PH10\": " + oeB + ", \"last_frame\": " + (chainB == null ? "null" : "\"" + chainB + "\"") + "}"
                    + ", \"atu_latch_writes\": {\"PD13\": " + atuWrites[0] + ", \"PG2\": " + atuWrites[1] + ", \"PG5\": " + atuWrites[2] + "}"
                    + ", \"pa_dac\": {\"reset_PB14\": " + rst + ", \"ch\": [" + dac + "]}"
                    + ", \"codec_reset_PB10\": " + pb10 + ", \"PD12\": " + pd12
                    + ", \"pins\": {" + pins + "}}";
            }
        }

        // ---- exact pin changes: write hooks on the GPIO ports ----
        // Each write to MODER/OTYPER/PUPDR/ODR/BSRR of a port is applied to a shadow
        // of that port, and every watched pin whose state changes gives a gpio event
        // at that instant. "since_boot_us" is the time since the boot event from
        // the executed instruction count at the core clock (one instruction a cycle:
        // an estimate; flash wait states and dual issue are not modelled).
        private readonly Dictionary<int, uint[]> shadow = new Dictionary<int, uint[]>();   // port -> MODER, OTYPER, PUPDR, ODR
        private readonly List<string> bootPhase = new List<string>();   // events of the first 100 ms after a boot
        private ulong bootInstr, bootUs;
        // during the boot phase each instruction is its own translation block, so the
        // executed-instruction count is exact at every port write (restored after 10 ms)
        private Antmicro.Renode.Peripherals.Timers.LimitTimer bootPhaseEnd;
        private object savedBlockSize;

        private void SetBlockSize(object value)
        {
            foreach(var cpu in machine.SystemBus.GetCPUs())
            {
                var p = cpu.GetType().GetProperty("MaximumBlockSize");
                if(p == null) continue;
                if(value == null) { if(savedBlockSize == null) savedBlockSize = p.GetValue(cpu); p.SetValue(cpu, Convert.ChangeType(1, p.PropertyType)); }
                else p.SetValue(cpu, value);
            }
        }

        private void StartBootPhase()
        {
            SetBlockSize(null);
            portHooksActive = true;
            if(bootPhaseEnd == null)
            {
                bootPhaseEnd = new Antmicro.Renode.Peripherals.Timers.LimitTimer(machine.ClockSource, 1000000, this, "bootphase", 1UL << 40,
                    direction: Direction.Ascending, enabled: false, workMode: Antmicro.Renode.Time.WorkMode.Periodic, eventEnabled: true);
                bootPhaseEnd.LimitReached += () =>
                {
                    bootPhaseEnd.Enabled = false;
                    portHooksActive = false;
                    if(savedBlockSize != null) SetBlockSize(savedBlockSize);
                    this.Log(LogLevel.Debug, "agent: boot phase ended, block size {0}", savedBlockSize);
                };
            }
            bootPhaseEnd.Enabled = false;
            bootPhaseEnd.Value = (1UL << 40) - 10000;
            bootPhaseEnd.Enabled = true;
        }
        private bool portsHooked;
        // exact events only in the boot phase (spec 0045); afterwards the 50 us sampler
        // reports changes, because the handler at every GPIO write slows emulation ~10x
        private volatile bool portHooksActive;

        private void HookPorts()
        {
            if(portsHooked) return;
            for(int port = 0; port < 11; port++)
            {
                IPeripheral gp;
                string name = "sysbus.gpioPort" + (char)('A' + port);
                if(!machine.TryGetByName(name, out gp) || !(gp is Antmicro.Renode.Peripherals.Bus.IBusPeripheral)) continue;
                int pn = port;
                ulong b = 0x58020000UL + 0x400UL * (ulong)port;
                shadow[pn] = new uint[] { machine.SystemBus.ReadDoubleWord(b), machine.SystemBus.ReadDoubleWord(b + 4),
                                          machine.SystemBus.ReadDoubleWord(b + 0x0C), machine.SystemBus.ReadDoubleWord(b + 0x14) };
                machine.SystemBus.SetHookBeforePeripheralWrite<uint>((Antmicro.Renode.Peripherals.Bus.IBusPeripheral)gp,
                    (value, offset) => { if(portHooksActive || (keyLogOn && pn == 0)) PortWrite(pn, offset, value); return value; });
            }
            portsHooked = true;
        }

        private string ShadowLevel(int port, int bit, out string mode, out string pull)
        {
            var s = shadow[port];
            uint m = (s[0] >> (2 * bit)) & 3u, ot = (s[1] >> bit) & 1u, pu = (s[2] >> (2 * bit)) & 3u, od = (s[3] >> bit) & 1u;
            mode = m == 0 ? "input" : m == 1 ? "output" : m == 2 ? "af" : "analog";
            pull = pu == 1 ? "up" : pu == 2 ? "down" : "none";
            string pin = "P" + (char)('A' + port) + bit;
            int driven;
            bool bench = drivenLevel.TryGetValue(pin, out driven);
            if(m == 1) return (ot == 1 && od == 1) ? (pu == 1 ? "1" : pu == 2 ? "0" : (bench ? driven.ToString() : "\"z\"")) : od.ToString();
            if(m == 0) return bench ? driven.ToString() : pu == 1 ? "1" : pu == 2 ? "0" : "\"z\"";
            return m == 3 ? "\"z\"" : "null";
        }

        private ulong Instructions()
        {
            foreach(var cpu in machine.SystemBus.GetCPUs())
            {
                var p = cpu.GetType().GetProperty("ExecutedInstructions");
                if(p != null) return Convert.ToUInt64(p.GetValue(cpu));
            }
            return 0;
        }

        private double CoreMhz()
        {
            foreach(var cpu in machine.SystemBus.GetCPUs())
            {
                var p = cpu.GetType().GetProperty("PerformanceInMips");
                if(p != null) return Math.Max(1.0, Convert.ToDouble(p.GetValue(cpu)));
            }
            return 64.0;
        }

        private void PortWrite(int port, long offset, uint value)
        {
            PortWriteInner(port, offset, value);
            if(port == 0 && keyLogOn) KeyCheck();
        }

        private void PortWriteInner(int port, long offset, uint value)
        {
            var s = shadow[port];
            string[] watchedNow = tracePins;
            var bits = new List<int>();
            for(int bit = 0; bit < 16; bit++)
            {
                string pn = "P" + (char)('A' + port) + bit;
                if(Array.IndexOf(WatchedPins, pn) >= 0 || Array.IndexOf(watchedNow, pn) >= 0) bits.Add(bit);
            }
            if(bits.Count == 0)
            {
                lock(sync)
                {
                    switch(offset)
                    {
                        case 0x00: s[0] = value; break;
                        case 0x04: s[1] = value; break;
                        case 0x0C: s[2] = value; break;
                        case 0x14: s[3] = value & 0xFFFF; break;
                        case 0x18: s[3] = (s[3] & ~(value >> 16)) | (value & 0xFFFF); break;
                    }
                }
                return;
            }
            var before = new Dictionary<int, string>();
            foreach(int bit in bits) { string m, pl; before[bit] = ShadowLevel(port, bit, out m, out pl) + "/" + m + "/" + pl; }
            switch(offset)
            {
                case 0x00: s[0] = value; break;
                case 0x04: s[1] = value; break;
                case 0x0C: s[2] = value; break;
                case 0x14: s[3] = value & 0xFFFF; break;
                case 0x18: s[3] = (s[3] & ~(value >> 16)) | (value & 0xFFFF); break;
                default: return;
            }
            // instruction count since the boot event at the core clock: excludes time in
            // WFI/WFE, so given only within 10 ms (emulated) of the boot event
            double since = (Instructions() - bootInstr) / CoreMhz();
            bool early = lastBoot != null && NowUs() - bootUs < 10000;
            var evs = new List<string>();
            lock(sync)
            {
                foreach(int bit in bits)
                {
                    string pin = "P" + (char)('A' + port) + bit;
                    string m, pl, lv = ShadowLevel(port, bit, out m, out pl);
                    if(lv + "/" + m + "/" + pl == before[bit]) continue;
                    lastState[pin] = lv + "/" + m + "/" + pl;
                    string body = string.Format(CultureInfo.InvariantCulture,
                        "\"t_us\": {0}, " + (early ? "\"since_boot_us\": {1:F3}, " : "") + "\"kind\": \"gpio\", \"pin\": \"{2}\", \"level\": {3}, \"mode\": \"{4}\", \"pull\": \"{5}\"",
                        NowUs(), since, pin, lv, m, pl);
                    if(early && bootPhase.Count < 512) bootPhase.Add(body);
                    if(tracing) evs.Add("{\"seq\": " + (seq++) + ", " + body + "}");
                }
                foreach(var e in evs) Send("ev " + e);
            }
        }

        // ---- boot event (spec 0045): the core executes its first instruction ----
        private string lastBoot;           // the last boot event body, re-emitted at every trace start
        private ulong bootHookPc;

        private uint BootAddress()
        {
            IPeripheral wd;
            if(machine.TryGetByName("sysbus.watchdog", out wd))
            {
                var f = wd.GetType().GetField("bootAddress", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic);
                if(f != null) return (uint)f.GetValue(wd);
            }
            return 0x08020000;
        }

        // the runner calls it after the image is loaded (sysbus.spi1.agent ArmBootHook)
        public void ArmBootHook()
        {
            uint vt = BootAddress();
            uint pc = machine.SystemBus.ReadDoubleWord(vt + 4) & ~1u;
            if(pc == 0 || pc == 0xFFFFFFFE) return;
            foreach(var cpu in machine.SystemBus.GetCPUs())
            {
                var add = cpu.GetType().GetMethod("AddHook");
                var remove = cpu.GetType().GetMethod("RemoveHooksAt");
                if(add == null) continue;
                if(bootHookPc != 0 && remove != null) remove.Invoke(cpu, new object[] { bootHookPc });
                bootHookPc = pc;
                var dt = add.GetParameters()[1].ParameterType;
                var mi = GetType().GetMethod("OnBootFetch", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic);
                var del = Delegate.CreateDelegate(dt, this, mi);
                add.Invoke(cpu, new object[] { (ulong)pc, del });
            }
        }

        private void OnBootFetch(Antmicro.Renode.Peripherals.CPU.ICpuSupportingGdb cpu, ulong pc)
        {
            string ev;
            lock(sync)
            {
                bootInstr = Instructions();
                bootUs = NowUs();
                firstIdrSeen = false; firstIdrBootUs = bootUs; firstIdrPort = null;
                if(lastLink == null)
                {
                    // the link as the CH394 reports it from power-on (later changes come
                    // from the model: link.set, a reset of the chip)
                    var ch = Ch394();
                    var pr = ch == null ? null : ch.GetType().GetProperty("LinkSpeed");
                    if(pr != null)
                    {
                        int sp = (int)pr.GetValue(ch);
                        lastLink = "\"t_us\": " + bootUs + ", \"kind\": \"link\", \"up\": " + (sp > 0 ? 1 : 0) + ", \"speed\": " + sp;
                    }
                }
                StartBootPhase();
                bootPhase.Clear();
                lastBoot = string.Format(CultureInfo.InvariantCulture,
                    "\"t_us\": {0}, \"kind\": \"boot\", \"at\": \"0x{1:X8}\", \"pc\": \"0x{2:X8}\"", NowUs(), BootAddress(), pc);
                ev = tracing ? "{\"seq\": " + (seq++) + ", " + lastBoot + "}" : null;
                if(ev != null) Send("ev " + ev);
            }
            // only the first instruction after a reset: remove until the next reset
            var remove = cpu.GetType().GetMethod("RemoveHooksAt");
            if(remove != null) remove.Invoke(cpu, new object[] { pc });
            bootHookPc = 0;
        }

        // a reset of the MCU while the emulator keeps running (watchdog, software)
        private int resetCount;     // MCU resets in this emulator process ("resets")

        private void OnMachineReset()
        {
            System.Threading.Interlocked.Increment(ref resetCount);
            string cause = "software";
            IPeripheral wd;
            if(machine.TryGetByName("sysbus.watchdog", out wd))
            {
                var pr = wd.GetType().GetProperty("LastResetCause");
                if(pr != null) cause = (string)pr.GetValue(wd);
            }
            string ev = null;
            lock(sync)
            {
                drivenLevel.Clear();
                if(tracing) { ev = Event("\"kind\": \"reset\", \"cause\": \"" + cause + "\""); lastState.Clear(); }
                if(ev != null) Send("ev " + ev);
            }
            // after the reset event: port shadows back to the reset values, boot hook re-armed
            try
            {
                for(int port = 0; port < 11; port++)
                {
                    if(!shadow.ContainsKey(port)) continue;
                    ulong b = 0x58020000UL + 0x400UL * (ulong)port;
                    var v = new uint[] { machine.SystemBus.ReadDoubleWord(b), machine.SystemBus.ReadDoubleWord(b + 4),
                                         machine.SystemBus.ReadDoubleWord(b + 0x0C), machine.SystemBus.ReadDoubleWord(b + 0x14) };
                    lock(sync) shadow[port] = v;
                }
                ArmBootHook();
                // ch394.silent counts MCU boots: the CH394 learns of each reset here
                IPeripheral ch;
                if(machine.TryGetByName("sysbus.spi6.ch394", out ch))
                {
                    var m = ch.GetType().GetMethod("McuBoot");
                    if(m != null) m.Invoke(ch, null);
                }
            }
            catch(Exception e) { this.Log(LogLevel.Warning, "agent: boot hook after reset: {0}", e.Message); }
        }

        // power.cut: at emulated time t_us the core stops at once (supply gone);
        // the runner then dumps the flash and powers the MCU on again
        private LimitTimer cutTimer;

        private string ScheduleCut(ulong atUs)
        {
            ulong now = NowUs();
            ulong delta = atUs > now ? atUs - now : 1;
            if(cutTimer == null)
            {
                cutTimer = new LimitTimer(machine.ClockSource, 1000000, this, "cut", delta,
                                          direction: Direction.Descending, enabled: false,
                                          workMode: WorkMode.Periodic, eventEnabled: true);
                cutTimer.LimitReached += () =>
                {
                    cutTimer.Enabled = false;
                    foreach(var cpu in machine.SystemBus.GetCPUs()) cpu.IsHalted = true;
                    Send("cut " + NowUs());
                };
            }
            cutTimer.Enabled = false;
            cutTimer.Limit = delta;
            cutTimer.Value = delta;
            cutTimer.Enabled = true;
            return "ok " + (now + delta);
        }

        // a power.cut placed by the flash layer (after a flash word, during an erase): as the timed cut
        public void FlashCut(string why)
        {
            Send("cut " + NowUs());
        }

        // the flash banks (0x08000000, 2 MiB) to a file: flash keeps its contents without power
        private string FlashDump(string path)
        {
            // through the flash timing layer if present: an erase in progress is only partly done
            IPeripheral ft;
            if(machine.TryGetByName("sysbus.flashtiming", out ft))
            {
                var m = ft.GetType().GetMethod("DumpFlash");
                if(m != null) { m.Invoke(ft, new object[] { path }); return "ok"; }
            }
            var data = machine.SystemBus.ReadBytes(0x08000000UL, 0x200000);
            System.IO.File.WriteAllBytes(path, data);
            return "ok";
        }

        // the flash timing layer (store checks): "ft info", "ft cut-words K", "ft cut-erase F"
        private string FlashTiming(string[] w)
        {
            IPeripheral ft;
            if(!machine.TryGetByName("sysbus.flashtiming", out ft)) return "err no flash timing layer";
            var t = ft.GetType();
            if(w.Length >= 2 && w[1] == "info")
            {
                var parts = new List<string>();
                foreach(var n in new[] { "Erases", "FetchesDuringErase", "FirstFetchPc", "RefreshesDuringLastErase", "WordsProgrammed", "EopReads", "WordsNotFound", "EccSpans", "EccSpanInstr", "EccSpanInstrMax" })
                    parts.Add("\"" + n + "\": " + Convert.ToString(t.GetProperty(n).GetValue(ft), CultureInfo.InvariantCulture));
                foreach(var n in new[] { "PrimaskAtErase", "StopReason" })
                    parts.Add("\"" + n + "\": \"" + t.GetProperty(n).GetValue(ft) + "\"");
                foreach(var n in new[] { "Erasing", "StoppedByCut" })
                    parts.Add("\"" + n + "\": " + ((bool)t.GetProperty(n).GetValue(ft) ? "true" : "false"));
                return "info {" + string.Join(", ", parts) + "}";
            }
            if(w.Length >= 3 && w[1] == "cut-words") { t.GetMethod("CutAfterWords").Invoke(ft, new object[] { long.Parse(w[2], CultureInfo.InvariantCulture) }); return "ok"; }
            if(w.Length >= 3 && w[1] == "cut-erase") { t.GetMethod("CutDuringErase").Invoke(ft, new object[] { double.Parse(w[2], CultureInfo.InvariantCulture) }); return "ok"; }
            if(w.Length >= 3 && w[1] == "log") return "ok " + (string)t.GetMethod("FlashLogRead").Invoke(ft, new object[] { int.Parse(w[2], CultureInfo.InvariantCulture) });
            if(w.Length >= 3 && w[1] == "erase-us") { t.GetMethod("SetEraseUs").Invoke(ft, new object[] { long.Parse(w[2], CultureInfo.InvariantCulture) }); return "ok " + (string)t.GetMethod("TimingJson").Invoke(ft, null); }
            if(w.Length >= 2 && w[1] == "timing") return "ok " + (string)t.GetMethod("TimingJson").Invoke(ft, null);
            if(w.Length >= 6 && w[1] == "fault")
            {
                var e = (string)t.GetMethod("Fault").Invoke(ft, new object[] { w[2], long.Parse(w[3], CultureInfo.InvariantCulture), w[4] == "-" ? "" : w[4], w[5] == "1" });
                return e == null ? "ok" : "err " + e;
            }
            if(w.Length >= 2 && w[1] == "ecclist") return "ok " + t.GetMethod("EccList").Invoke(ft, null);
            if(w.Length >= 4 && w[1] == "ecc")
            {
                var e = (string)t.GetMethod("AddEcc").Invoke(ft, new object[] { Convert.ToInt64(w[2], 16), w[3] == "double" });
                return e == null ? "ok" : "err " + e;
            }
            if(w.Length >= 4 && w[1] == "corrupt")
            {
                var r = (string)t.GetMethod("Corrupt").Invoke(ft, new object[] { Convert.ToInt64(w[2], 16), int.Parse(w[3], CultureInfo.InvariantCulture), false });
                if(r == null) return "err not changed";
                // "ft corrupt ADDR XOR KIND SLOT": a hw.trace event {kind: slot_corrupt|slot_restore, slot, address}
                if(w.Length >= 6) ExternalEvent("\"kind\": \"" + w[4] + "\", \"slot\": " + w[5] + ", \"address\": \"0x" + Convert.ToInt64(w[2], 16).ToString("X8") + "\"", false);
                return "ok " + r;
            }
            if(w.Length >= 3 && w[1] == "cut-words-notify") { t.GetProperty("NotifyCut").SetValue(ft, true); t.GetMethod("CutAfterWords").Invoke(ft, new object[] { long.Parse(w[2], CultureInfo.InvariantCulture) }); return "ok"; }
            if(w.Length >= 3 && w[1] == "cut-erase-notify") { t.GetProperty("NotifyCut").SetValue(ft, true); t.GetMethod("CutDuringErase").Invoke(ft, new object[] { double.Parse(w[2], CultureInfo.InvariantCulture) }); return "ok"; }
            if(w.Length >= 3 && w[1] == "cut-address-notify") { t.GetProperty("NotifyCut").SetValue(ft, true); t.GetMethod("CutAtAddress").Invoke(ft, new object[] { Convert.ToInt64(w[2], 16) }); return "ok"; }
            return "err usage: ft info | cut-words K | cut-erase F | log FROM | timing | erase-us US | fault OP AFTER FLAG NEVER | ecc ADDR TYPE | corrupt ADDR XOR [restore]";
        }

        private string WatchdogReset()
        {
            IPeripheral wd;
            if(!machine.TryGetByName("sysbus.watchdog", out wd)) return "err no watchdog";
            var m = wd.GetType().GetMethod("WatchdogReset");
            if(m == null) return "err watchdog model has no reset";
            m.Invoke(wd, null);
            return "ok";
        }

        // ---- console with emulated-time stamps (a second artifact; console.txt stays as the image wrote it) ----
        private readonly string timedPath;
        private readonly StringBuilder consoleLine = new StringBuilder();
        private ulong lineStartUs;
        private bool consoleHooked;

        // console.log (format v1.8): every line from power-on, emulated time of its first and
        // last character; the runner collects it across power cycles
        private readonly List<string> consoleList = new List<string>();
        private int consoleDropped;

        private string ConsoleLogRead(int from)
        {
            lock(consoleList)
            {
                from = Math.Max(0, Math.Min(from, consoleList.Count));
                int to = Math.Min(consoleList.Count, from + 5000);
                return "{\"t_us\": " + NowUs() + ", \"count\": " + consoleList.Count + ", \"dropped\": " + consoleDropped
                    + ", \"from\": " + from + ", \"next\": " + to + ", \"entries\": [" + string.Join(", ", consoleList.GetRange(from, to - from)) + "]}";
            }
        }

        private void HookConsole()
        {
            if(consoleHooked || string.IsNullOrEmpty(timedPath)) return;
            IPeripheral u;
            if(!machine.TryGetByName("sysbus.usart1", out u)) return;
            var ev = u.GetType().GetEvent("CharReceived");
            if(ev == null) return;
            ev.AddEventHandler(u, new Action<byte>(OnConsoleChar));
            consoleHooked = true;
        }

        private void OnConsoleChar(byte b)
        {
            string line = null;
            lock(consoleLine)
            {
                if(consoleLine.Length == 0) lineStartUs = NowUs();
                if(b == (byte)'\n') { line = consoleLine.ToString().TrimEnd('\r'); consoleLine.Clear(); }
                else consoleLine.Append((char)b);
            }
            if(line != null)
            {
                var esc = new StringBuilder();
                foreach(char ch in line)
                {
                    if(ch == '"' || ch == '\\') esc.Append('\\').Append(ch);
                    else if(ch < 0x20 || ch > 0x7E) esc.AppendFormat("\\u{0:x4}", (int)ch);
                    else esc.Append(ch);
                }
                lock(consoleList)
                {
                    if(consoleList.Count >= 100000) consoleDropped++;
                    else consoleList.Add("{\"t_first_us\": " + lineStartUs + ", \"t_us\": " + NowUs() + ", \"line\": \"" + esc + "\"}");
                }
                try
                {
                    System.IO.File.AppendAllText(timedPath, string.Format(CultureInfo.InvariantCulture, "{0,12:F6} {1}\n", lineStartUs / 1e6, line));
                }
                catch(Exception) { }
            }
        }

        // ---- link to the runner ----
        private void Serve()
        {
            for(int tries = 0; tries < 600 && stream == null; tries++)
            {
                try { var c = new TcpClient(); c.Connect("127.0.0.1", agentPort); link = c; stream = c.GetStream(); }
                catch(Exception) { Thread.Sleep(100); }
            }
            if(stream == null) return;
            var buf = new StringBuilder();
            var one = new byte[1];
            try
            {
                while(stream.Read(one, 0, 1) == 1)
                {
                    if(one[0] != (byte)'\n') { buf.Append((char)one[0]); continue; }
                    string line = buf.ToString().Trim(); buf.Clear();
                    Send(Handle(line));
                }
            }
            catch(Exception) { }
        }

        private string Handle(string line)
        {
            var w = line.Split(' ');
            try
            {
                switch(w[0])
                {
                    case "time": return "time " + NowUs();
                    case "ft": return FlashTiming(w);
                    case "ch394silent":
                    {
                        IPeripheral ch;
                        if(!machine.TryGetByName("sysbus.spi6.ch394", out ch)) return "err no CH394";
                        var pr = ch.GetType().GetProperty("SilentBoots");
                        if(w.Length >= 2 && w[1] != "?") pr.SetValue(ch, int.Parse(w[1], CultureInfo.InvariantCulture));
                        return "ok " + pr.GetValue(ch) + " " + ch.GetType().GetProperty("SilentNow").GetValue(ch);
                    }
                    case "wdt": return WatchdogReset();
                    case "resets": return "resets " + resetCount;
                    case "keylog": return KeyLogCommand(w);
                    case "peek":     // diagnostics: one 32-bit bus read
                        return "ok 0x" + machine.SystemBus.ReadDoubleWord(ulong.Parse(w[1].Replace("0x", ""), NumberStyles.HexNumber)).ToString("X8");
                    case "consolelog": return "ok " + ConsoleLogRead(w.Length > 1 ? int.Parse(w[1], CultureInfo.InvariantCulture) : 0);
                    case "firstcheck":
                        return firstIdrPort == null ? "ok {}" : "ok {\"boot_t_us\": " + firstIdrBootUs + ", \"first_idr_read_us\": " + firstIdrReadUs
                            + ", \"after_boot_us\": " + (firstIdrReadUs - firstIdrBootUs) + ", \"port\": \"" + firstIdrPort + "\"}";
                    case "force": return ForceCommand(w);
                    case "dacforce": return DacForceCommand(w);
                    case "tsfields":     // diagnostics: the master time source's fields
                    {
                        var mts = Antmicro.Renode.Core.EmulationManager.Instance.CurrentEmulation.MasterTimeSource;
                        var sb = new StringBuilder("ok");
                        for(var ty = mts.GetType(); ty != null && ty != typeof(object); ty = ty.BaseType)
                            foreach(var fi in ty.GetFields(BindingFlags.Instance | BindingFlags.NonPublic | BindingFlags.Public))
                            {
                                object v = null; try { v = fi.GetValue(mts); } catch(Exception) { }
                                sb.Append(" | " + ty.Name + "." + fi.Name + ":" + fi.FieldType.Name + "=" + (v == null ? "null" : v.ToString().Replace(" ", "_")));
                            }
                        return sb.ToString();
                    }
                    case "advance_on": return Advance(true);
                    case "advance_off": return Advance(false);
                    case "socklog":
                    {
                        var r = Call(Ch394(), "SockLogRead", w.Length > 1 ? int.Parse(w[1], CultureInfo.InvariantCulture) : 0);
                        return r == null ? "err no CH394" : "ok " + r;
                    }
                    case "inject":
                    {
                        // inject <srcip> <fromport> <toport> <offsets csv> <payloads hex csv>
                        if(w.Length < 6) return "err inject arguments";
                        var r = Call(Ch394(), "Inject", w[1], int.Parse(w[2], CultureInfo.InvariantCulture), int.Parse(w[3], CultureInfo.InvariantCulture), w[4], w[5]);
                        return r == null ? "err inject failed" : "ok " + r;
                    }
                    case "wait":
                    {
                        // wait <us>: returns when emulated time has advanced by us
                        ulong target = NowUs() + ulong.Parse(w[1], CultureInfo.InvariantCulture);
                        var sw = System.Diagnostics.Stopwatch.StartNew();
                        while(NowUs() < target && sw.Elapsed.TotalSeconds < 600) System.Threading.Thread.Sleep(1);
                        return NowUs() >= target ? "ok " + NowUs() : "err wait timed out at " + NowUs();
                    }
                    case "netlog": return NetLogCommand(w);
                    case "blk":
                    {
                        // diagnostics: the core's translation block size and the boot-phase timer
                        foreach(var cpu in machine.SystemBus.GetCPUs())
                        {
                            var pr = cpu.GetType().GetProperty("MaximumBlockSize");
                            return "blk " + (pr != null ? pr.GetValue(cpu) : "?") + " saved " + savedBlockSize
                                + " timer " + (bootPhaseEnd == null ? "none" : bootPhaseEnd.Enabled + "/" + bootPhaseEnd.Value);
                        }
                        return "blk ?";
                    }
                    case "cutclear":     // power.cut: whatever is still armed is replaced
                    {
                        if(cutTimer != null) cutTimer.Enabled = false;
                        IPeripheral fl;
                        if(machine.TryGetByName("sysbus.flashtiming", out fl)) fl.GetType().GetMethod("ClearCuts").Invoke(fl, null);
                        return "ok";
                    }
                    case "cut": return ScheduleCut(ulong.Parse(w[1], CultureInfo.InvariantCulture));
                    case "flashdump": return FlashDump(line.Substring(10));
                    case "halt":
                        foreach(var cpu in machine.SystemBus.GetCPUs()) cpu.IsHalted = true;
                        return "ok";
                    case "link":
                    {
                        IPeripheral ch;
                        if(!machine.TryGetByName("sysbus.spi6.ch394", out ch)) return "err no Ethernet controller";
                        var m = ch.GetType().GetMethod("SetLinkSpeed");
                        if(m == null) return "err no link control";
                        m.Invoke(ch, new object[] { int.Parse(w[1], CultureInfo.InvariantCulture) });
                        return "ok";
                    }
                    case "halted":
                        foreach(var cpu in machine.SystemBus.GetCPUs()) if(!cpu.IsHalted) return "halted 0";
                        return "halted 1";
                    case "observe": return "obs " + Observe();
                    case "input": return w.Length == 3 ? Input(w[1], int.Parse(w[2], CultureInfo.InvariantCulture)) : "err usage";
                    case "trace":
                        if(w.Length >= 2 && w[1] == "start")
                        {
                            ulong startUs;
                            lock(sync)
                            {
                                startUs = NowUs();     // replayed boot events lie before it
                                var pins = new List<string>(WatchedPins);
                                ulong first = 0;
                                for(int i = 2; i < w.Length; i++)
                                {
                                    if(w[i].StartsWith("@")) { first = ulong.Parse(w[i].Substring(1), CultureInfo.InvariantCulture); continue; }
                                    if(!pins.Contains(w[i])) pins.Add(w[i]);
                                }
                                tracePins = pins.ToArray();
                                lastState.Clear(); seq = first; tracing = true;
                                if(lastBoot != null) Send("ev {\"seq\": " + (seq++) + ", " + lastBoot + "}");
                                foreach(var b in bootPhase) Send("ev {\"seq\": " + (seq++) + ", " + b + "}");
                                if(lastLink != null) Send("ev {\"seq\": " + (seq++) + ", " + lastLink + "}");
                                Array.Clear(atuWrites, 0, 3);
                            }
                            sampler.Enabled = true;
                            Sample();
                            return "ok " + startUs;
                        }
                        lock(sync) tracing = false;
                        sampler.Enabled = false;
                        return "ok " + seq + " " + dropped;
                    default: return "err unknown " + w[0];
                }
            }
            catch(Exception e) { return "err " + e.Message; }
        }

        private void Send(string line)
        {
            var s = stream;
            if(s == null) { dropped++; return; }
            var b = Encoding.ASCII.GetBytes(line + "\n");
            try { lock(s) s.Write(b, 0, b.Length); } catch(Exception) { dropped++; }
        }
    }
}
