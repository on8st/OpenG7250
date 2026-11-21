// CH394 Ethernet controller on SPI6 (clean-room), see docs/CH394-model.md.
// Register-compatible with the W5500 per the CH394 datasheet (WCH CH394DS1);
// register map and socket behaviour from the public W5500 datasheet; pins from
// his/40-ethernet.md (CS# PG15, RST# PB5, INT# PC13 active low).
//
// Modelled:
//   - SPI frames: [addr hi][addr lo][control = BSB<<3 | RWB<<2 | OM] then data,
//     auto-increment, framed by CS# (input 0); RST# low (input 1) resets the chip
//   - common registers incl. SHAR (the chip's own MAC, set at construction),
//     SIPR, IR/IMR, SIR/SIMR, PHYCFGR (link up, 100 Mbit/s full duplex by
//     default; SetLink changes it), VERSIONR
//   - 8 sockets: Sn_MR, Sn_CR (OPEN, CLOSE, SEND, SEND_MAC, RECV), Sn_IR, Sn_SR,
//     Sn_PORT, Sn_DIPR, Sn_DPORT, buffer sizes, TX_FSR, TX_RD/WR, RX_RSR, RX_RD/WR,
//     Sn_IMR; TX/RX buffers per socket with wrap-around
//   - UDP: SEND takes TX_RD..TX_WR as one datagram to Sn_DIPR:Sn_DPORT; a
//     received datagram goes into the RX buffer with the 8-byte header (source
//     IP, source port, length), Sn_IR.RECV; Sn_MR.BCASTB blocks broadcasts
//   - INT# (output IRQ, active low) from (SIR & SIMR) and (IR & IMR)
//   - the network: datagrams go to and come from the test-bench runner over a
//     local TCP link (bridgePort; 0 = no network), see docs/CH394-model.md; the
//     address the image programs (SIPR, SUBR) is reported on that link
// Not modelled: TCP and MACRAW (logged "not modelled: CH394/Sn_MR ..."), ARP and
// ICMP behaviour, timeouts and retransmission, IP fragmentation, wake-on-LAN,
// PPPoE, socket-less commands, timing.
using System;
using System.Collections.Generic;
using System.Net.Sockets;
using System.Threading;
using Antmicro.Renode.Core;
using Antmicro.Renode.Logging;

namespace Antmicro.Renode.Peripherals.SPI
{
    public class G7250Ch394 : ISPIPeripheral, Antmicro.Renode.Core.IGPIOReceiver
    {
        private const int Sockets = 8;
        private readonly object sync = new object();
        private readonly byte[] common = new byte[0x40];
        private readonly byte[][] sreg = new byte[Sockets][];
        private readonly byte[][] txbuf = new byte[Sockets][];
        private readonly byte[][] rxbuf = new byte[Sockets][];
        private readonly byte[] mac;
        private readonly int bridgePort;
        private int phase, bsb, addr;
        private bool write;
        private TcpClient link;
        private NetworkStream stream;
        private Thread reader;
        private bool linkUp = true, speed100 = true, softReset;
        private byte[] addressPending;
        private ulong sent, received, dropped;

        // socket log (RE-side evidence, e.g. DHCP borrows of a management socket): one
        // JSON line per OPEN and CLOSE with the socket, its source port (Sn_PORT) and the
        // emulated time, to the file in G7250_SOCKLOG (none when unset)
        private readonly IMachine machine;
        private readonly string sockLog = Environment.GetEnvironmentVariable("G7250_SOCKLOG");

        // net.log (format v1.6): every datagram at the radio in emulated time. to_radio:
        // the moment it is placed in the socket's receive buffer; from_radio: the moment
        // the image's SEND executes. Cap 100000 entries ("dropped" beyond it).
        private const int LogCap = 100000;
        private readonly List<string> netLog = new List<string>();
        private bool netLogOn;
        private int netLogDropped;
        // runner log: always on (the runner saves it per test as net-log.jsonl)
        private readonly List<string> runLog = new List<string>();
        private int runLogDropped;
        public string RunLogRead(int from) { lock(sync) return Page(runLog, from, runLogDropped, NowUs(), "entries"); }

        // key.log "hp": High Priority packets received on radio port 1027
        private readonly List<string> hpLog = new List<string>();
        private bool hpLogOn;
        private int hpLogDropped;

        private ulong NowUs() { return machine == null ? 0 : (ulong)machine.LocalTimeSource.ElapsedVirtualTime.TotalMicroseconds; }

        private static string Ip(byte[] b, int o) { return b[o] + "." + b[o + 1] + "." + b[o + 2] + "." + b[o + 3]; }

        private long curShift;      // the delivery shift of the datagram being delivered (0 = on its spacing)
        private double curWall;     // its host wall-clock arrival, epoch seconds (matches the bench's burst windows)

        private void NetRecord(string dir, int n, int radioPort, string peer, byte[] data, int off, int len, string reason = null)
        {
            var hx = new System.Text.StringBuilder();
            for(int i = 0; i < Math.Min(16, len); i++) hx.Append(data[off + i].ToString("x2"));
            string e = "{\"t_us\": " + NowUs() + ", \"dir\": \"" + dir + "\", \"socket\": " + n + ", \"radio_port\": " + radioPort
                + ", \"peer\": \"" + peer + "\", \"len\": " + len + ", \"head\": \"" + hx + "\""
                + (reason != null ? ", \"reason\": \"" + reason + "\"" : "")
                + (dir != "from_radio" && curShift != 0 ? ", \"shift_us\": " + curShift : "")
                + (dir != "from_radio" && curWall > 0 ? ", \"t_wall\": " + curWall.ToString("F6", System.Globalization.CultureInfo.InvariantCulture) : "")
                + (dir != "from_radio" && curInjected ? ", \"injected\": true" : "") + "}";
            if(runLog.Count >= 200000) runLogDropped++; else runLog.Add(e);
            if(!netLogOn) return;
            if(netLog.Count >= LogCap) { netLogDropped++; return; }
            netLog.Add(e);
        }

        private static string Page(List<string> log, int from, int dropped, ulong now, string key)
        {
            const int PageMax = 5000;
            from = Math.Max(0, Math.Min(from, log.Count));
            int to = Math.Min(log.Count, from + PageMax);
            return "{\"t_us\": " + now + ", \"count\": " + log.Count + ", \"dropped\": " + dropped + ", \"from\": " + from
                + ", \"next\": " + to + ", \"" + key + "\": [" + string.Join(", ", log.GetRange(from, to - from)) + "]}";
        }

        public string NetLogStart() { lock(sync) { netLog.Clear(); netLogDropped = 0; netLogOn = true; return "{\"t_us\": " + NowUs() + "}"; } }
        public string NetLogRead(int from) { lock(sync) return Page(netLog, from, netLogDropped, NowUs(), "entries"); }
        public string NetLogStop(int from) { lock(sync) { netLogOn = false; return Page(netLog, from, netLogDropped, NowUs(), "entries"); } }

        public void HpLogStart() { lock(sync) { hpLog.Clear(); hpLogDropped = 0; hpLogOn = true; } }
        public void HpLogStop() { lock(sync) hpLogOn = false; }
        public string HpLogRead(int from) { lock(sync) return Page(hpLog, from, hpLogDropped, NowUs(), "hp"); }

        // link changes as trace events (hw.trace kind "link"), through the bench agent
        private void LinkEvent()
        {
            IPeripheral ag;
            if(machine == null || !machine.TryGetByName("sysbus.spi1.agent", out ag)) return;
            var m = ag.GetType().GetMethod("ExternalEvent");
            if(m == null) return;
            try { m.Invoke(ag, new object[] { "\"kind\": \"link\", \"up\": " + (linkUp ? 1 : 0) + ", \"speed\": " + (linkUp ? (speed100 ? 100 : 10) : 0), true }); }
            catch(Exception) { }
        }

        private void LogSocket(string op, int n, byte[] r)
        {
            if(string.IsNullOrEmpty(sockLog)) return;
            ulong t = machine == null ? 0 : (ulong)machine.LocalTimeSource.ElapsedVirtualTime.TotalMicroseconds;
            try
            {
                System.IO.File.AppendAllText(sockLog, "{\"t_us\": " + t + ", \"op\": \"" + op + "\", \"socket\": " + n
                    + ", \"port\": " + U16(r, 0x04) + ", \"mode\": " + (r[0x00] & 0x0F) + "}\n");
            }
            catch(Exception) { }
        }

        public G7250Ch394(IMachine machine, string macAddress = "02:00:00:00:72:50", int bridgePort = 0)
        {
            this.machine = machine;
            mac = new byte[6];
            var parts = macAddress.Split(':');
            for(int i = 0; i < 6 && i < parts.Length; i++) mac[i] = Convert.ToByte(parts[i], 16);
            this.bridgePort = bridgePort;
            IRQ = new GPIO();
            for(int n = 0; n < Sockets; n++) { sreg[n] = new byte[0x40]; txbuf[n] = new byte[16384]; rxbuf[n] = new byte[16384]; }
            Reset();
            if(bridgePort > 0)
            {
                reader = new Thread(Connect) { IsBackground = true, Name = "ch394-bridge" };
                reader.Start();
            }
        }

        public GPIO IRQ { get; private set; }
        public ulong DatagramsSent { get { return sent; } }
        public ulong DatagramsReceived { get { return received; } }
        public ulong DatagramsDropped { get { return dropped; } }

        public void Reset()
        {
            lock(sync)
            {
                Array.Clear(common, 0, common.Length);
                Array.Copy(mac, 0, common, 0x09, 6);
                common[0x19] = 0x07; common[0x1A] = 0xD0;   // RTR 2000
                common[0x1B] = 0x08;                        // RCR
                common[0x39] = 0x04;                        // VERSIONR (W5500 value; HYPOTHESIS for the CH394)
                SetPhy();
                for(int n = 0; n < Sockets; n++)
                {
                    Array.Clear(sreg[n], 0, 0x40);
                    sreg[n][0x1E] = 2; sreg[n][0x1F] = 2;    // 2 KiB buffers
                    sreg[n][0x2C] = 0xFF;                    // Sn_IMR
                    sreg[n][0x12] = 0x05; sreg[n][0x13] = 0xB4;
                    sreg[n][0x16] = 0x80;                    // TTL
                }
                phase = 0;
            }
            UpdateIrq();
        }

        // test-bench fixture link.set: 0 = link down, 10 or 100 Mbit/s full duplex
        public void SetLink(bool up) { lock(sync) { linkUp = up; SetPhy(); } LinkEvent(); }
        public void SetLinkSpeed(int mbps)
        {
            lock(sync) { linkUp = mbps > 0; speed100 = mbps != 10; SetPhy(); }
            LinkEvent();
            this.Log(LogLevel.Info, "CH394: link {0}", mbps > 0 ? mbps + " Mbit/s" : "down");
        }
        public int LinkSpeed { get { return linkUp ? (speed100 ? 100 : 10) : 0; } }

        // PHYCFGR: RST/OPMD/OPMDC as written, DPX (bit 2), SPD (bit 1, 1 = 100 Mbit/s), LNK (bit 0)
        private void SetPhy() { common[0x2E] = (byte)(0xB8 | 0x04 | (speed100 ? 2 : 0) | (linkUp ? 1 : 0)); }

        private static int U16(byte[] r, int a) { return (r[a] << 8) | r[a + 1]; }
        private static void P16(byte[] r, int a, int v) { r[a] = (byte)(v >> 8); r[a + 1] = (byte)v; }
        private int BufSize(int n, bool tx) { int k = sreg[n][tx ? 0x1F : 0x1E]; return (k == 0 ? 0 : k) * 1024; }

        public byte Transmit(byte data)
        {
            byte outb = 0;
            bool cmd = false; int cmdSock = 0;
            lock(sync)
            {
                if(phase == 0) { addr = data << 8; phase = 1; }
                else if(phase == 1) { addr |= data; phase = 2; }
                else if(phase == 2) { bsb = (data >> 3) & 0x1F; write = ((data >> 2) & 1) != 0; phase = 3; }
                else
                {
                    int a = addr & 0xFFFF;
                    if(bsb == 0) { if(write) WriteCommon(a, data); else outb = ReadCommon(a); }
                    else
                    {
                        int n = (bsb - 1) / 4, kind = (bsb - 1) % 4;
                        if(n < Sockets)
                        {
                            if(kind == 0)
                            {
                                if(write) { cmd = WriteSocket(n, a & 0x3F, data); cmdSock = n; }
                                else outb = ReadSocket(n, a & 0x3F);
                            }
                            // buffer index = (16-bit address) mod size; size 0 = no buffer (writes ignored,
                            // reads 0). For a size that is not a power of two the mapping of the 16-bit
                            // pointer onto the buffer is not documented (CH394DS1): emulator assumption.
                            else if(kind == 1) { int sz = BufSize(n, true); if(sz > 0) { if(write) txbuf[n][(a & 0xFFFF) % sz] = data; else outb = txbuf[n][(a & 0xFFFF) % sz]; } else if(!write) outb = 0; }
                            else if(kind == 2) { int sz = BufSize(n, false); if(sz > 0) { if(write) rxbuf[n][(a & 0xFFFF) % sz] = data; else outb = rxbuf[n][(a & 0xFFFF) % sz]; } else if(!write) outb = 0; }
                        }
                    }
                    addr++;
                }
            }
            if(softReset) { softReset = false; bool up = linkUp; Reset(); SetLink(up); }
            var addrMsg = addressPending;
            if(addrMsg != null) { addressPending = null; ToBridge(3, addrMsg); }
            if(cmd) UpdateIrq();
            return outb;
        }

        private void WriteCommon(int a, byte v)
        {
            if(a >= common.Length) return;
            if(a == 0x15) { common[a] &= (byte)~v; return; }    // IR: write 1 to clear
            if(a == 0x17 || a == 0x39) return;                       // SIR, VERSIONR: read-only
            if(a == 0x2E) { common[a] = (byte)((common[a] & 0x07) | (v & 0xF8)); return; }
            common[a] = v;
            if(a == 0x12)
                this.Log(LogLevel.Info, "[CH394 model] source IP set to {0}.{1}.{2}.{3}", common[0x0F], common[0x10], common[0x11], common[0x12]);
            if((a == 0x12 || a == 0x08) && (common[0x0F] | common[0x10] | common[0x11] | common[0x12]) != 0)
            {
                var ip = new byte[8];
                Array.Copy(common, 0x0F, ip, 0, 4);
                Array.Copy(common, 0x05, ip, 4, 4);
                addressPending = ip;          // sent to the runner after this SPI byte (outside the lock)
            }
            if(a == 0x00 && (v & 0x80) != 0) softReset = true;      // MR.RST: done after this byte
        }

        private byte ReadCommon(int a)
        {
            if(a >= common.Length) return 0;
            if(a == 0x17)
            {
                byte sir = 0;
                for(int n = 0; n < Sockets; n++) if((sreg[n][0x02] & sreg[n][0x2C]) != 0) sir |= (byte)(1 << n);
                return sir;
            }
            return common[a];
        }

        private bool WriteSocket(int n, int a, byte v)
        {
            var r = sreg[n];
            switch(a)
            {
                case 0x01: Command(n, v); r[0x01] = 0; return true;
                case 0x02: r[0x02] &= (byte)~v; return true;              // Sn_IR: write 1 to clear
                case 0x03: case 0x20: case 0x21: case 0x22: case 0x23: case 0x26: case 0x27: case 0x2A: case 0x2B: return false;   // read-only
                default: r[a] = v; return false;
            }
        }

        private byte ReadSocket(int n, int a)
        {
            var r = sreg[n];
            switch(a)
            {
                case 0x20: case 0x21:
                {
                    int free = Math.Max(0, BufSize(n, true) - ((U16(r, 0x24) - U16(r, 0x22)) & 0xFFFF));
                    return (byte)(a == 0x20 ? free >> 8 : free);
                }
                case 0x26: case 0x27:
                {
                    int used = (U16(r, 0x2A) - U16(r, 0x28)) & 0xFFFF;
                    return (byte)(a == 0x26 ? used >> 8 : used);
                }
                default: return r[a];
            }
        }

        private void Command(int n, byte c)
        {
            var r = sreg[n];
            switch(c)
            {
                case 0x01: // OPEN
                {
                    int proto = r[0x00] & 0x0F;
                    P16(r, 0x22, 0); P16(r, 0x24, 0); P16(r, 0x28, 0); P16(r, 0x2A, 0);
                    LogSocket("open", n, r);
                    if(proto == 0x02) r[0x03] = 0x22;                      // SOCK_UDP
                    else
                    {
                        r[0x03] = proto == 0x01 ? (byte)0x13 : (byte)0x00;
                        this.Log(LogLevel.Warning, "not modelled: CH394/Sn_MR protocol 0x{0:X2} (socket {1})", proto, n);
                    }
                    break;
                }
                case 0x10: LogSocket("close", n, r); r[0x03] = 0x00; break;   // CLOSE
                case 0x20: case 0x21: Send(n); break;                      // SEND, SEND_MAC
                case 0x40:                                                 // RECV: RX_RD already updated
                    if(runLog.Count < 200000)
                        runLog.Add("{\"t_us\": " + NowUs() + ", \"dir\": \"recv\", \"socket\": " + n + ", \"radio_port\": " + U16(r, 0x04)
                            + ", \"left\": " + ((U16(r, 0x2A) - U16(r, 0x28)) & 0xFFFF) + "}");
                    break;
                default:
                    this.Log(LogLevel.Warning, "not modelled: CH394/Sn_CR command 0x{0:X2} (socket {1})", c, n);
                    break;
            }
        }

        private void Send(int n)
        {
            var r = sreg[n];
            int rd = U16(r, 0x22), wr = U16(r, 0x24), size = BufSize(n, true);
            int len = size == 0 ? 0 : (wr - rd) & 0xFFFF;
            var data = new byte[len];
            for(int i = 0; i < len; i++) data[i] = txbuf[n][((rd + i) & 0xFFFF) % size];
            P16(r, 0x22, wr);
            r[0x02] |= 0x10;                                               // SEND_OK
            if(r[0x03] != 0x22) return;
            if(!linkUp) { dropped++; return; }                        // no link: the datagram is lost
            var frame = new byte[8 + len];
            frame[0] = r[0x04]; frame[1] = r[0x05];                        // source port
            Array.Copy(r, 0x0C, frame, 2, 4);                              // destination IP
            frame[6] = r[0x10]; frame[7] = r[0x11];                        // destination port
            Array.Copy(data, 0, frame, 8, len);
            NetRecord("from_radio", n, U16(r, 0x04), Ip(r, 0x0C) + ":" + U16(r, 0x10), data, 0, len);
            sent++;
            ToBridge(1, frame);
        }

        // a datagram from the network: srcIP(4) srcPort(2) dstPort(2) flags(1) data
        private void Deliver(byte[] p) { Deliver(p, 0, 0); }

        private void Deliver(byte[] p, long shiftUs, double wall)
        {
            if(p.Length < 9) return;
            int dport = (p[6] << 8) | p[7];
            bool bcast = (p[8] & 1) != 0;
            int len = p.Length - 9;
            bool ok = false;
            lock(sync)
            {
                curShift = shiftUs; curWall = wall;
                curInjected = curInjectedNext; curInjectedNext = false;
                string peer = Ip(p, 0) + ":" + ((p[4] << 8) | p[5]);
                if(!linkUp) { dropped++; NetRecord("dropped", -1, dport, peer, p, 9, len, "link down"); return; }
                for(int n = 0; n < Sockets && !ok; n++)
                {
                    var r = sreg[n];
                    if(r[0x03] != 0x22 || U16(r, 0x04) != dport) continue;
                    if(bcast && (r[0x00] & 0x40) != 0) continue;              // BCASTB
                    int size = BufSize(n, false);
                    if(size == 0) { dropped++; NetRecord("dropped", n, dport, peer, p, 9, len, "no buffer"); ok = true; break; }   // no receive buffer
                    int used = (U16(r, 0x2A) - U16(r, 0x28)) & 0xFFFF;
                    if(size - used < 8 + len) { dropped++; NetRecord("dropped", n, dport, peer, p, 9, len, "buffer full"); ok = true; break; }
                    int wr = U16(r, 0x2A);
                    var hdr = new byte[] { p[0], p[1], p[2], p[3], p[4], p[5], (byte)(len >> 8), (byte)len };
                    for(int i = 0; i < 8; i++) rxbuf[n][((wr + i) & 0xFFFF) % size] = hdr[i];
                    for(int i = 0; i < len; i++) rxbuf[n][((wr + 8 + i) & 0xFFFF) % size] = p[9 + i];
                    P16(r, 0x2A, (wr + 8 + len) & 0xFFFF);
                    NetRecord("to_radio", n, dport, Ip(p, 0) + ":" + ((p[4] << 8) | p[5]), p, 9, len);
                    if(hpLogOn && dport == 1027)
                    {
                        if(hpLog.Count >= LogCap) hpLogDropped++;
                        else hpLog.Add("{\"t_us\": " + NowUs() + ", \"seq\": " + (len >= 4 ? (((uint)p[9] << 24) | ((uint)p[10] << 16) | ((uint)p[11] << 8) | p[12]).ToString() : "null")
                            + ", \"b4\": " + (len > 4 ? p[13].ToString() : "null") + ", \"b5\": " + (len > 5 ? p[14].ToString() : "null") + "}");
                    }
                    r[0x02] |= 0x04;                                         // RECV
                    received++;
                    ok = true;
                }
                if(!ok) { dropped++; NetRecord("dropped", -1, dport, peer, p, 9, len, "no socket"); }
            }
            UpdateIrq();
        }

        private void UpdateIrq()
        {
            bool pending;
            lock(sync)
            {
                byte sir = 0;
                for(int n = 0; n < Sockets; n++) if((sreg[n][0x02] & sreg[n][0x2C]) != 0) sir |= (byte)(1 << n);
                pending = (sir & common[0x18]) != 0 || (common[0x15] & common[0x16]) != 0;
            }
            IRQ.Set(!pending);   // INT# is active low
        }

        // input 0 = CS# (low starts a frame), input 1 = RST# (low resets the chip)
        public void OnGPIO(int number, bool value)
        {
            if(number == 0 && !value) { lock(sync) phase = 0; }
            else if(number == 1 && !value) { bool up = linkUp; Reset(); SetLink(up); }
        }

        public void FinishTransmission() { }

        // ---- arrival pacing in emulated time ----
        // A datagram enters the socket buffer at the emulated time that keeps its host
        // spacing: emulated anchor + (host arrival - host anchor), the anchor taken at the
        // start of a burst (100 ms of host silence). Never earlier than the emulated now.
        // When that is impossible (emulated time already past the target, or the target
        // more than 20 ms ahead: the burst is re-anchored), the datagram's shift is
        // recorded ("shift_us" in net.log and the runner log).
        private readonly System.Diagnostics.Stopwatch hostClock = System.Diagnostics.Stopwatch.StartNew();
        private double hostAnchorUs = -1, lastHostUs = -1e12;
        private ulong emuAnchorUs;
        private readonly Queue<Tuple<ulong, byte[], long, double>> pending = new Queue<Tuple<ulong, byte[], long, double>>();
        private Antmicro.Renode.Peripherals.Timers.LimitTimer paceTimer;
        private const double BurstGapUs = 100000, MaxLeadUs = 20000;
        // the endpoint's time_scale (emulated / wall) for runs where the bench stretches its
        // host spacing by 1 / time_scale: host offsets are scaled back into emulated time
        private readonly double timeScale = ParseScale(Environment.GetEnvironmentVariable("G7250_TIME_SCALE"));
        private static double ParseScale(string v)
        {
            double d;
            return double.TryParse(v, System.Globalization.NumberStyles.Float, System.Globalization.CultureInfo.InvariantCulture, out d) && d > 0 ? d : 1.0;
        }

        private void Arrive(byte[] p)
        {
            double wall = (DateTime.UtcNow - new DateTime(1970, 1, 1, 0, 0, 0, DateTimeKind.Utc)).TotalSeconds;
            if(machine == null) { Deliver(p, 0, wall); return; }
            double host = hostClock.Elapsed.TotalMilliseconds * 1000.0;
            ulong now = NowUs();
            long shift = 0;
            ulong target;
            lock(paceSync)
            {
                if(host - lastHostUs > BurstGapUs / timeScale || hostAnchorUs < 0) { hostAnchorUs = host; emuAnchorUs = now; }
                lastHostUs = host;
                double t = emuAnchorUs + (host - hostAnchorUs) * timeScale;
                if(t > now + MaxLeadUs)
                {
                    shift = (long)(now - t);                 // negative: delivered earlier than its spacing
                    hostAnchorUs = host; emuAnchorUs = now; t = now;
                }
                ulong ideal = (ulong)Math.Max(0, t);
                target = Math.Max(ideal, now);
                if(target > ideal) shift = (long)(target - ideal);   // positive: later than its spacing
                // keep the order of arrival
                if(pending.Count > 0) target = Math.Max(target, LastTarget());
                if(target <= now && pending.Count == 0) { }
                else
                {
                    pending.Enqueue(Tuple.Create(target, p, shift, wall));
                    // the timer only from the emulation thread (this is the bridge's thread;
                    // arming it here while the emulation's pace callback waits for paceSync
                    // deadlocks)
                    machine.LocalTimeSource.ExecuteInNearestSyncedState(ts => { lock(paceSync) ArmPace(); });
                    return;
                }
            }
            Deliver(p, shift, wall);
        }

        private readonly object paceSync = new object();
        private ulong lastTarget;
        private ulong LastTarget() { return lastTarget; }

        private void ArmPace()
        {
            if(pending.Count == 0) return;
            lastTarget = Math.Max(lastTarget, pending.Peek().Item1);
            foreach(var x in pending) lastTarget = Math.Max(lastTarget, x.Item1);
            if(paceTimer == null)
            {
                paceTimer = new Antmicro.Renode.Peripherals.Timers.LimitTimer(machine.ClockSource, 1000000, this, "ch394pace", 1UL << 40,
                    direction: Antmicro.Renode.Time.Direction.Ascending, enabled: false,
                    workMode: Antmicro.Renode.Time.WorkMode.Periodic, eventEnabled: true);
                paceTimer.LimitReached += OnPace;
            }
            ulong now = NowUs(), head = pending.Peek().Item1;
            ulong d = head > now ? head - now : 1;
            paceTimer.Enabled = false;
            paceTimer.Value = (1UL << 40) - d;
            paceTimer.Enabled = true;
        }

        private void OnPace()
        {
            var due = new List<Tuple<ulong, byte[], long, double>>();
            lock(paceSync)
            {
                paceTimer.Enabled = false;
                ulong now = NowUs();
                while(pending.Count > 0 && pending.Peek().Item1 <= now) due.Add(pending.Dequeue());
                if(pending.Count > 0) ArmPace();
            }
            foreach(var x in due) Deliver(x.Item2, x.Item3, x.Item4);
        }

        // ---- net.inject (format v1.7): datagrams placed at exact emulated offsets ----
        // The base time is taken on the emulation thread; every datagram then goes through
        // Deliver, the same receive path as a bridged one (socket by port, buffer size, the
        // drop rule, the header into Sn_RX, RECV), marked "injected" in the logs.
        private readonly List<Tuple<ulong, byte[]>> injQueue = new List<Tuple<ulong, byte[]>>();
        private Antmicro.Renode.Peripherals.Timers.LimitTimer injTimer;
        private bool curInjected;

        // srcIp "a.b.c.d", ports, offsets in us from now, payloads; returns the placing times
        public string Inject(string srcIp, int fromPort, int toPort, string offsetsCsv, string payloadsCsv)
        {
            var ip = srcIp.Split('.');
            var offs = offsetsCsv.Split(',');
            var pls = payloadsCsv.Split(',');
            if(offs.Length != pls.Length || ip.Length != 4) return null;
            var done = new System.Threading.ManualResetEventSlim(false);
            var times = new ulong[offs.Length];
            machine.LocalTimeSource.ExecuteInNearestSyncedState(ts =>
            {
                ulong now = NowUs();
                lock(paceSync)
                {
                    for(int k = 0; k < offs.Length; k++)
                    {
                        string hx = pls[k];
                        var data = new byte[hx.Length / 2];
                        for(int i = 0; i < data.Length; i++) data[i] = Convert.ToByte(hx.Substring(2 * i, 2), 16);
                        var p = new byte[9 + data.Length];
                        for(int i = 0; i < 4; i++) p[i] = byte.Parse(ip[i], System.Globalization.CultureInfo.InvariantCulture);
                        p[4] = (byte)(fromPort >> 8); p[5] = (byte)fromPort; p[6] = (byte)(toPort >> 8); p[7] = (byte)toPort; p[8] = 0;
                        Array.Copy(data, 0, p, 9, data.Length);
                        ulong at = now + ulong.Parse(offs[k], System.Globalization.CultureInfo.InvariantCulture);
                        times[k] = at;
                        injQueue.Add(Tuple.Create(at, p));
                    }
                    injQueue.Sort((a, b) => a.Item1.CompareTo(b.Item1));
                    ArmInject();
                }
                done.Set();
            });
            if(!done.Wait(30000)) return null;
            return string.Join(",", times);
        }

        private void ArmInject()
        {
            if(injQueue.Count == 0) return;
            if(injTimer == null)
            {
                injTimer = new Antmicro.Renode.Peripherals.Timers.LimitTimer(machine.ClockSource, 1000000, this, "ch394inject", 1UL << 40,
                    direction: Antmicro.Renode.Time.Direction.Ascending, enabled: false,
                    workMode: Antmicro.Renode.Time.WorkMode.Periodic, eventEnabled: true);
                injTimer.LimitReached += OnInject;
            }
            ulong now = NowUs(), head = injQueue[0].Item1;
            injTimer.Enabled = false;
            if(head <= now) { OnInject(); return; }
            injTimer.Value = (1UL << 40) - (head - now);
            injTimer.Enabled = true;
        }

        private void OnInject()
        {
            var due = new List<Tuple<ulong, byte[]>>();
            lock(paceSync)
            {
                if(injTimer != null) injTimer.Enabled = false;
                ulong now = NowUs();
                while(injQueue.Count > 0 && injQueue[0].Item1 <= now) { due.Add(injQueue[0]); injQueue.RemoveAt(0); }
            }
            foreach(var x in due)
            {
                long late = (long)(NowUs() - x.Item1);
                curInjectedNext = true;
                Deliver(x.Item2, late, 0);
            }
            lock(paceSync) ArmInject();
        }
        private bool curInjectedNext;

        // ---- local link to the test-bench runner ----
        private void Connect()
        {
            for(int tries = 0; tries < 600 && link == null; tries++)
            {
                try { var c = new TcpClient(); c.Connect("127.0.0.1", bridgePort); link = c; stream = c.GetStream(); }
                catch(Exception) { Thread.Sleep(100); }
            }
            if(stream == null) { this.Log(LogLevel.Warning, "CH394 bridge: no runner on port {0}", bridgePort); return; }
            this.Log(LogLevel.Info, "CH394 bridge connected on port {0}", bridgePort);
            var head = new byte[3];
            try
            {
                while(true)
                {
                    if(!ReadFull(head, 3)) break;
                    int len = (head[1] << 8) | head[2];
                    var p = new byte[len];
                    if(!ReadFull(p, len)) break;
                    if(head[0] == 2) Arrive(p);
                }
            }
            catch(Exception) { }
        }

        private bool ReadFull(byte[] b, int n)
        {
            int k = 0;
            while(k < n) { int r = stream.Read(b, k, n - k); if(r <= 0) return false; k += r; }
            return true;
        }

        private void ToBridge(byte type, byte[] p)
        {
            var s = stream;
            if(s == null) return;
            var f = new byte[3 + p.Length];
            f[0] = type; f[1] = (byte)(p.Length >> 8); f[2] = (byte)p.Length;
            Array.Copy(p, 0, f, 3, p.Length);
            try { lock(s) s.Write(f, 0, f.Length); } catch(Exception) { }
        }
    }
}
