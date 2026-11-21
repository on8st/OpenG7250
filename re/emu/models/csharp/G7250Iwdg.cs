// Generic STM32H7 independent watchdog IWDG1 and the RCC_RSR reset flags
// (clean-room, ST RM0468 public), see docs/IWDG-model.md. Replaces Renode
// 1.17's STM32_IndependentWatchdog, which cannot tell the firmware why it was
// reset.
//
// IWDG: KR (0xCCCC start, 0xAAAA reload, 0x5555 unlock PR/RLR/WINR), PR
// (divider 4 << PR), RLR (12 bit), SR (updates complete at once: 0), WINR
// (window: a reload while the counter is above WINR resets). Counter clock LSI
// 32 kHz; when the counter reaches 0 the MCU is reset.
//
// RCC_RSR (RCC + 0xD0), served through bus hooks on the RCC model:
//   after power-on   PORRSTF | BORRSTF | PINRSTF | D1RSTF (RM0468 reset value)
//   after an IWDG1 reset   IWDG1RSTF | PINRSTF
//   after any other reset (software, SYSRESETREQ)   SFT1RSTF | PINRSTF
//   RMVF (bit 16) written 1 clears the flags.
// The fixture reset.wdt (WatchdogReset) resets the MCU as an IWDG1 timeout.
// After every MCU reset the core boots from bootAddress (BOOT_ADD0 default
// 0x08000000): VTOR, SP and PC are taken from the vector table there.
// Not modelled: hardware start by option byte, freeze in debug/stop/standby,
// LSI accuracy, IWDG2 (dual core), the window early-warning of WWDG.
using System;
using Antmicro.Renode.Core;
using Antmicro.Renode.Logging;
using Antmicro.Renode.Peripherals;
using Antmicro.Renode.Peripherals.Bus;
using Antmicro.Renode.Peripherals.Timers;
using Antmicro.Renode.Time;

namespace Antmicro.Renode.Peripherals.Timers
{
    public class G7250Iwdg : IDoubleWordPeripheral, IKnownSize
    {
        private const uint PORRSTF = 1u << 23, PINRSTF = 1u << 22, BORRSTF = 1u << 21, D1RSTF = 1u << 19,
                           SFT1RSTF = 1u << 24, IWDG1RSTF = 1u << 26, RMVF = 1u << 16;
        private const long Lsi = 32000;

        private readonly IMachine machine;
        private readonly LimitTimer counter;
        private readonly object sync = new object();
        private readonly uint bootAddress;
        private uint pr, rlr, winr;
        private bool unlocked, running, hooked;
        private uint rsr = PORRSTF | BORRSTF | PINRSTF | D1RSTF;
        private string pendingCause;
        private ulong refreshes, resets;

        public G7250Iwdg(IMachine machine, uint bootAddress = 0x08000000)
        {
            this.machine = machine;
            this.bootAddress = bootAddress;
            machine.MachineReset += _ => Boot();
            counter = new LimitTimer(machine.ClockSource, (ulong)(Lsi / 4), this, "iwdg", 0xFFF,
                                     direction: Direction.Descending, enabled: false,
                                     workMode: WorkMode.Periodic, eventEnabled: true);
            counter.LimitReached += () => { counter.Enabled = false; Expire("wdt"); };
            Reset();
        }

        public long Size { get { return 0x400; } }
        public ulong Refreshes { get { return refreshes; } }
        public ulong Resets { get { return resets; } }
        public string LastResetCause { get; private set; } = "power";

        // fixture reset.wdt: reset the MCU as a watchdog timeout would
        public void WatchdogReset() { Expire("wdt"); }

        private void Expire(string cause)
        {
            lock(sync) pendingCause = cause;
            this.Log(LogLevel.Warning, "IWDG: MCU reset ({0})", cause);
            machine.RequestReset();
        }

        public void Reset()
        {
            lock(sync)
            {
                // a reset of the machine: set the reset flags for its cause
                if(hooked)
                {
                    string cause = pendingCause ?? "software";
                    rsr |= PINRSTF | (cause == "wdt" ? IWDG1RSTF : SFT1RSTF);
                    LastResetCause = cause;
                    resets++;
                }
                pendingCause = null;
                pr = 0; rlr = 0xFFF; winr = 0xFFF; unlocked = false; running = false;
                counter.Enabled = false;
            }
            Hook();
        }

        private void Hook()
        {
            if(hooked) return;
            IPeripheral rcc;
            if(!machine.TryGetByName("sysbus.rcc", out rcc) || !(rcc is IBusPeripheral)) return;
            var bus = machine.SystemBus;
            bus.SetHookAfterPeripheralRead<uint>((IBusPeripheral)rcc, (value, offset) => offset == 0xD0 ? rsr : value);
            bus.SetHookBeforePeripheralWrite<uint>((IBusPeripheral)rcc, (value, offset) =>
            {
                if(offset == 0xD0 && (value & RMVF) != 0) lock(sync) rsr = 0;
                return value;
            });
            hooked = true;
        }

        // after a reset the core boots from the boot address (option byte
        // BOOT_ADD0, default 0x0800 -> 0x08000000): VTOR, SP and PC from its
        // vector table, as the hardware does
        private void Boot()
        {
            var bus = machine.SystemBus;
            foreach(var cpu in bus.GetCPUs())
            {
                uint sp = bus.ReadDoubleWord(bootAddress), pc = bus.ReadDoubleWord(bootAddress + 4);
                try
                {
                    var t = cpu.GetType();
                    var vtor = t.GetProperty("VectorTableOffset");
                    if(vtor != null) vtor.SetValue(cpu, Convert.ChangeType(bootAddress, vtor.PropertyType));
                    var regs = cpu as Antmicro.Renode.Peripherals.CPU.ICPUWithRegisters;
                    if(regs != null) regs.SetRegister(13, Antmicro.Renode.Peripherals.CPU.RegisterValue.Create(sp, 32));
                    cpu.PC = pc & ~1u;
                }
                catch(Exception e) { this.Log(LogLevel.Error, "IWDG: boot after reset failed: {0}", e.Message); }
            }
        }

        private void Reload()
        {
            counter.Limit = rlr;
            counter.Value = rlr;
        }

        public uint ReadDoubleWord(long offset)
        {
            lock(sync)
            {
                switch(offset)
                {
                    case 0x04: return pr;
                    case 0x08: return rlr;
                    case 0x0C: return 0;        // SR: updates complete at once
                    case 0x10: return winr;
                    default: return 0;
                }
            }
        }

        public void WriteDoubleWord(long offset, uint value)
        {
            bool early = false;
            lock(sync)
            {
                switch(offset)
                {
                    case 0x00:
                        switch(value & 0xFFFF)
                        {
                            case 0xCCCC:
                                if(!running) { running = true; counter.Divider = 1; counter.Frequency = (ulong)(Lsi / (4 << (int)pr)); Reload(); counter.Enabled = true; }
                                break;
                            case 0xAAAA:
                                refreshes++;
                                if(running && winr < 0xFFF && counter.Value > winr) early = true;
                                else if(running) Reload();
                                break;
                            case 0x5555: unlocked = true; break;
                            default: unlocked = false; break;
                        }
                        break;
                    case 0x04: if(unlocked) { pr = value & 7; if(running) counter.Frequency = (ulong)(Lsi / (4 << (int)pr)); } break;
                    case 0x08: if(unlocked) rlr = value & 0xFFF; break;
                    case 0x10: if(unlocked) { winr = value & 0xFFF; if(running) Reload(); } break;
                }
            }
            if(early) Expire("wdt");
        }
    }
}
