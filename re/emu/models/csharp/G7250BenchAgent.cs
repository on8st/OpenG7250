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
        private static readonly string[] SafePins = { "PF4", "PF6", "PC7", "PH11", "PA10", "PH10", "PB14", "PB10", "PD12", "PB5", "PD5" };
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
        private string[] tracePins = SafePins;
        private readonly Dictionary<string, int> lastLevel = new Dictionary<string, int>();
        private readonly Dictionary<string, int> drivenLevel = new Dictionary<string, int>();
        private readonly LimitTimer sampler;

        public G7250BenchAgent(IMachine machine, int agentPort = 0, long sampleRate = 20000)
        {
            this.machine = machine;
            this.agentPort = agentPort;
            sampler = new LimitTimer(machine.ClockSource, (ulong)sampleRate, this, "sample", 1,
                                     direction: Direction.Descending, enabled: false,
                                     workMode: WorkMode.Periodic, eventEnabled: true);
            sampler.LimitReached += Sample;
            if(agentPort > 0) new Thread(Serve) { IsBackground = true, Name = "bench-agent" }.Start();
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
            }
            if(ev != null) Send("ev " + ev);
        }

        public void OnGPIO(int number, bool value)
        {
            if(number < 0 || number >= LatchPins.Length) return;
            string ev = null;
            lock(sync)
            {
                bool rising = value && !latchLevel[number];
                latchLevel[number] = value;
                if(tracing) ev = Gpio(LatchPins[number], value ? 1 : 0);
                if(rising && shifted.Count > 0)
                {
                    int n = number == 1 ? Math.Min(3, shifted.Count) : 1;
                    var sb = new StringBuilder();
                    for(int i = shifted.Count - n; i < shifted.Count; i++) sb.Append(shifted[i].ToString("x2"));
                    if(number == 0) chainA = shifted[shifted.Count - 1];
                    else if(number == 1) chainB = sb.ToString();
                    else { atuWrites[number - 2]++; atuLast[number - 2] = shifted[shifted.Count - 1]; }
                }
            }
            if(ev != null) Send("ev " + ev);
        }

        // ---- pins ----
        private static bool ParsePin(string pin, out int port, out int bit)
        {
            port = bit = -1;
            if(pin == null || pin.Length < 3 || pin[0] != 'P') return false;
            port = pin[1] - 'A';
            return int.TryParse(pin.Substring(2), out bit) && port >= 0 && port < 11 && bit >= 0 && bit < 16;
        }

        private int Level(string pin)
        {
            int driven;
            lock(sync) if(drivenLevel.TryGetValue(pin, out driven)) return driven;   // electrical level set by hw.input
            int port, bit;
            if(!ParsePin(pin, out port, out bit)) return -1;
            ulong b = 0x58020000UL + 0x400UL * (ulong)port;
            var bus = machine.SystemBus;
            uint moder = bus.ReadDoubleWord(b + 0x00);
            uint reg = ((moder >> (2 * bit)) & 3u) == 1u ? bus.ReadDoubleWord(b + 0x14) : bus.ReadDoubleWord(b + 0x10);
            return (int)((reg >> bit) & 1u);
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
            lock(sync)
            {
                drivenLevel[pin] = level != 0 ? 1 : 0;
                if(tracing) { lastLevel[pin] = drivenLevel[pin]; Send("ev " + Gpio(pin, drivenLevel[pin])); }
            }
            return "ok";
        }

        private ulong NowUs()
        {
            return (ulong)machine.LocalTimeSource.ElapsedVirtualTime.TotalMicroseconds;
        }

        private string Event(string body) { return "{\"seq\": " + (seq++) + ", \"t_us\": " + NowUs() + ", " + body + "}"; }
        private string Gpio(string pin, int level) { return Event("\"kind\": \"gpio\", \"pin\": \"" + pin + "\", \"level\": " + level); }

        private void Sample()
        {
            List<string> evs = null;
            lock(sync)
            {
                if(!tracing) return;
                foreach(var pin in tracePins)
                {
                    int l = Level(pin), prev;
                    if(lastLevel.TryGetValue(pin, out prev) && prev == l) continue;
                    lastLevel[pin] = l;
                    (evs = evs ?? new List<string>()).Add(Gpio(pin, l));
                }
            }
            if(evs != null) foreach(var e in evs) Send("ev " + e);
        }

        private string DacWords(out int resetLevel)
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
            int rst;
            string dac = DacWords(out rst);
            lock(sync)
            {
                return "{\"t_us\": " + NowUs()
                    + ", \"tr_lines\": {\"PF4\": " + Level("PF4") + ", \"PF6\": " + Level("PF6") + ", \"PC7\": " + Level("PC7") + ", \"PH11\": " + Level("PH11") + "}"
                    + ", \"spi1_a\": {\"oe_PA10\": " + Level("PA10") + ", \"last_frame\": " + (chainA < 0 ? "null" : chainA.ToString()) + "}"
                    + ", \"spi1_b\": {\"oe_PH10\": " + Level("PH10") + ", \"last_frame\": " + (chainB == null ? "null" : "\"" + chainB + "\"") + "}"
                    + ", \"atu_latch_writes\": {\"PD13\": " + atuWrites[0] + ", \"PG2\": " + atuWrites[1] + ", \"PG5\": " + atuWrites[2] + "}"
                    + ", \"pa_dac\": {\"reset_PB14\": " + rst + ", \"ch\": [" + dac + "]}"
                    + ", \"codec_reset_PB10\": " + Level("PB10") + ", \"PD12\": " + Level("PD12") + "}";
            }
        }

        // a reset of the MCU while the emulator keeps running (watchdog, software)
        private void OnMachineReset()
        {
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
                if(tracing) { ev = Event("\"kind\": \"reset\", \"cause\": \"" + cause + "\""); lastLevel.Clear(); }
            }
            if(ev != null) Send("ev " + ev);
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

        // the flash banks (0x08000000, 2 MiB) to a file: flash keeps its contents without power
        private string FlashDump(string path)
        {
            var data = machine.SystemBus.ReadBytes(0x08000000UL, 0x200000);
            System.IO.File.WriteAllBytes(path, data);
            return "ok";
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
                    case "wdt": return WatchdogReset();
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
                            lock(sync)
                            {
                                var pins = new List<string>(SafePins);
                                ulong first = 0;
                                for(int i = 2; i < w.Length; i++)
                                {
                                    if(w[i].StartsWith("@")) { first = ulong.Parse(w[i].Substring(1), CultureInfo.InvariantCulture); continue; }
                                    if(!pins.Contains(w[i])) pins.Add(w[i]);
                                }
                                tracePins = pins.ToArray();
                                lastLevel.Clear(); seq = first; tracing = true;
                                Array.Clear(atuWrites, 0, 3);
                            }
                            sampler.Enabled = true;
                            Sample();
                            return "ok";
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
