// PA bias DAC and PA plant for the Main, at bus and pin level (clean-room; see
// docs/PA-plant-model.md). Step 2 (PA) of docs/PRODUCT-TESTBENCH-PLAN.md.
//
// 1. Bias DAC on SPI2 (8-channel 10-bit serial DAC of the BU2506FV type, word
//    format per the Rohm datasheet and his/85-rf-control.md): 4-bit address
//    (bit-reversed channel) << 2 | value bits 9..8, then bits 7..0; LD rising
//    edge (PB12) latches; RESET low (PB14) forces all outputs to 0.
// 2. PA plant: drain current Idd from the bias codes (static current plus, per
//    bias stage, gain x (code - dead zone) above the dead zone; monotonic, zero
//    extra current at code 0).
// 3. Analog outputs: the plant sets the input voltages of the ADC1 channels
//    whose pins his/85 names (PA1_C INP1 supply, PF11 INP2 forward, PA6 INP3
//    reflected, PC4 INP4 Idd, PB1 INP5 PA NTC, PC5 INP8 EALC, PA2 INP14 ALC) on
//    the generic ADC model (sysbus.adc12). Nothing is written into RAM; the
//    firmware reads the values through its own ADC and DMA set-up.
// Board scaling (volts at the pin per ampere or volt) and all plant values are
// HYPOTHESIS placeholders until measured on the radio.
using System;
using Antmicro.Renode.Core;
using Antmicro.Renode.Logging;
using Antmicro.Renode.Peripherals;

namespace Antmicro.Renode.Peripherals.SPI
{
    public class G7250PaPlant : ISPIPeripheral, Antmicro.Renode.Core.IGPIOReceiver
    {
        // HYPOTHESIS: placeholder plant parameters
        private const double StaticIdd = 0.05;                      // A
        private static readonly int[] BiasCh = { 2, 8, 7 };          // driver, final 1, final 2
        private static readonly int[] DeadZone = { 450, 800, 800 };  // DAC codes, below the 10-bit clamp
        private static readonly double[] Gain = { 0.5 / 150, 0.9 / 150, 0.9 / 150 }; // A per code
        private const double SupplyVolts = 13.8;
        // HYPOTHESIS: board scaling to the ADC pin (3.3 V full scale)
        private const double PinVoltsPerAmpIdd = 3.3 / 4095 / 0.008059;
        private const double PinVoltsPerVoltSupply = 3.3 / 4095 / 0.004835;
        private const double NtcPinVolts25C = 3.3 * 2047 / 4095;

        private readonly IMachine machine;
        private readonly int[] dac = new int[9];
        private readonly bool[] written = new bool[9];
        private readonly byte[] word = new byte[2];
        private int nbytes;
        private bool ldPrev, rstPrev = true, warned;
        private double forward, reflected, ealc, alc, supply = SupplyVolts, ntc = NtcPinVolts25C;

        public G7250PaPlant(IMachine machine)
        {
            this.machine = machine;
            Reset();
        }

        public void Reset()
        {
            for(int i = 0; i < dac.Length; i++) { dac[i] = 0; written[i] = false; }
            nbytes = 0;
            Publish();
        }

        // test-bench fixture hooks (sensor.set), in plant units
        public void SetSupplyVolts(double v) { supply = v; Publish(); }
        public void SetNtcPinVolts(double v) { ntc = v; Publish(); }
        public void SetRfPinVolts(double fwd, double rev) { forward = fwd; reflected = rev; Publish(); }
        public int GetDacCode(int channel) { return channel >= 1 && channel <= 8 ? dac[channel] : -1; }
        // last latched word value per channel, -1 if never written since reset
        public int GetDacWord(int channel) { return channel >= 1 && channel <= 8 && written[channel] ? dac[channel] : -1; }
        public double Idd { get { return ComputeIdd(); } }

        public byte Transmit(byte data)
        {
            word[nbytes & 1] = data;   // keep the last two bytes of the word
            nbytes++;
            return 0;
        }

        public void FinishTransmission() { }

        private static int Reverse4(int v)
        {
            return ((v & 1) << 3) | ((v & 2) << 1) | ((v & 4) >> 1) | ((v & 8) >> 3);
        }

        // GPIO 0 = LD (PB12, rising edge latches), GPIO 1 = RESET (PB14, low clears)
        public void OnGPIO(int number, bool value)
        {
            if(number == 0)
            {
                if(value && !ldPrev && nbytes >= 2)
                {
                    byte hi = word[(nbytes - 2) & 1], lo = word[(nbytes - 1) & 1];
                    int ch = Reverse4((hi >> 2) & 0xF);
                    int val = ((hi & 3) << 8) | lo;
                    if(ch >= 1 && ch <= 8) { dac[ch] = val; written[ch] = true; }
                    this.Log(LogLevel.Debug, "DAC ch {0} = {1}, Idd {2:F3} A", ch, val, ComputeIdd());
                    nbytes = 0;
                    Publish();
                }
                ldPrev = value;
            }
            else if(number == 1)
            {
                if(!value && rstPrev) { for(int i = 0; i < dac.Length; i++) dac[i] = 0; Publish(); }
                rstPrev = value;
            }
        }

        private double ComputeIdd()
        {
            double i = StaticIdd;
            for(int k = 0; k < BiasCh.Length; k++)
            {
                int above = dac[BiasCh[k]] - DeadZone[k];
                if(above > 0) i += Gain[k] * above;
            }
            return i;
        }

        // push the analog values to the ADC inputs (the ADC model may be loaded
        // after this plant; then the first publish is retried by the runner)
        public void Publish()
        {
            IPeripheral adc;
            if(machine == null || !machine.TryGetByName("sysbus.adc12", out adc))
            {
                if(!warned && machine != null) { warned = true; this.Log(LogLevel.Debug, "no sysbus.adc12 yet"); }
                return;
            }
            var set = adc.GetType().GetMethod("SetChannelVoltage");
            if(set == null) return;
            Action<int, double> v = (ch, volts) => set.Invoke(adc, new object[] { ch, volts });
            v(1, supply * PinVoltsPerVoltSupply);
            v(2, forward);
            v(3, reflected);
            v(4, ComputeIdd() * PinVoltsPerAmpIdd);
            v(5, ntc);
            v(8, ealc);
            v(14, alc);
        }
    }
}
