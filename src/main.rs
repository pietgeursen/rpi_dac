use std::time::Duration;

use crossbeam::channel::{select, unbounded};
use rppal::gpio::{Gpio, OutputPin, Trigger};
use rppal::hal::Delay;
use rppal::spi::{Bus, Error as SpiError, Mode, SlaveSelect, Spi};
use src4392::reset::Reset;
use src4392::{
    AudioFormat, Deemphasis, InterpolationFilterGroupDelay, OutputDataSource, Port,
    PortClockSource, PortMasterClockDivider, ReadModifyWriteSpiRegister, Src4392, SrcClockSource,
    SrcSource, SrcRatio
};

use ad1955::{Ad1955, DataFormat, MclkMode, PcmSampleRate, OutputFormat, PcmDataWidth, PcmDataFormat};

fn main() {
    let mut spi = Spi::new(Bus::Spi0, SlaveSelect::Ss0, 1000000, Mode::Mode3).unwrap();
    let mut delay = Delay::new();
    let gpio = Gpio::new().unwrap();

    let src_cs = gpio.get(16).unwrap().into_output_high();
    let dac_cs = gpio.get(7).unwrap().into_output_high();
    let mut n_dac_reset = gpio.get(6).unwrap().into_output_high();
    let mut src_reset = gpio.get(5).unwrap().into_output_high();
    let mut n_src_int = gpio.get(13).unwrap().into_input();
    let mut n_lock = gpio.get(22).unwrap().into_input();
    let mut n_rdy = gpio.get(23).unwrap().into_input();

    std::thread::sleep(Duration::from_millis(10));
    src_reset.set_low();
    n_dac_reset.set_low();
    std::thread::sleep(Duration::from_millis(10));
    src_reset.set_high();
    n_dac_reset.set_high();
    std::thread::sleep(Duration::from_millis(10));

    n_src_int
        .set_async_interrupt(Trigger::FallingEdge, |_level| {
            println!("src interrupt");
        })
        .unwrap();

    let (ready_sender, ready_receiver) = unbounded();
    n_rdy
        .set_async_interrupt(Trigger::FallingEdge, move |_level| {
            ready_sender.send(()).unwrap();
        })
        .unwrap();

    let (lock_sender, lock_receiver) = unbounded();

    n_lock
        .set_async_interrupt(Trigger::FallingEdge, move |_level| {
            lock_sender.send(()).unwrap();
        })
        .unwrap();

    let mut ad1955 = Ad1955::new(dac_cs);

    ad1955.dac_control_1.data_format = DataFormat::Pcm;
    ad1955.dac_control_1.pcm_sample_rate = PcmSampleRate::_192kHz;
    ad1955.dac_control_1.is_power_down = false;
    ad1955.dac_control_1.is_muted = false;
    ad1955.dac_control_1.output_format = OutputFormat::Stereo;
    ad1955.dac_control_1.pcm_data_width = PcmDataWidth::_24bits;
    ad1955.dac_control_1.pcm_data_format = PcmDataFormat::I2S;

    ad1955.dac_control_2.mclk_mode = MclkMode::Fs512;
    ad1955.volume_left.volume = 0x3FFF.into(); // div by 4
    ad1955.volume_right.volume = 0x3FFF.into(); // div by 4
    //ad1955.volume.volume = 0x9FF.into(); // div by 4

    ad1955.update(&mut spi, &mut delay);

    let src_delay = Delay::new();
    let mut src4392 = Src4392::<OutputPin, Spi, SpiError, Delay, u16>::new(src_cs, src_delay);

    // PortB is the port connected to the PI
    src4392
        .configure_port(
            &mut spi,
            Port::B,
            AudioFormat::I2S,
            OutputDataSource::Loopback,
            PortMasterClockDivider::_128,
            PortClockSource::Mclk,
            false,
        )
        .unwrap();

    src4392
        .set_src(
            &mut spi,
            SrcSource::PortB,
            SrcClockSource::Mclk,
            InterpolationFilterGroupDelay::_64,
            Deemphasis::None,
            true,
        )
        .unwrap();

    // Port A is connected to the DAC
    src4392
        .configure_port(
            &mut spi,
            Port::A,
            AudioFormat::I2S,
            OutputDataSource::SRC,
            PortMasterClockDivider::_128,
            PortClockSource::Mclk,
            true,
        )
        .unwrap();

    src4392
        .modify_register(&mut spi, |r: &mut Reset| {
            r.n_pdsrc = true;
            r.n_pdpb = true;
            r.n_pdpa = true;
            r.n_pdrx = true;
            r.n_pdtx = false;
            r.n_pdall = true;
            r.reset = false;
        })
        .unwrap();

    println!("Hello, world!");

    loop {
        select! {
            recv(ready_receiver) -> _ => {
                println!("ready receiver");
                src4392.modify_register(&mut spi, |r: &mut SrcRatio|{
                    println!("src ratio: {:?}", r.as_f32());

                }).unwrap();
            }
            recv(lock_receiver) -> _ => {
                println!("lock receiver");
            }
        };
    }
}
