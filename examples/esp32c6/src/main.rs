#![no_std]
#![no_main]

use block_device_adapters::BufStream;
use embassy_executor::Spawner;
use embedded_fatfs::FsOptions;
use embedded_io_async::{Read, Write};
use esp32c6_hal::{
    clock::ClockControl,
    dma::Dma,
    dma::DmaPriority,
    dma_descriptors, embassy,
    peripherals::Peripherals,
    prelude::*,
    spi::{
        master::{prelude::*, Spi},
        SpiMode,
    },
    FlashSafeDma, IO,
};
use esp_backtrace as _;

#[main]
async fn main(_spawner: Spawner) {
    let peripherals = Peripherals::take();
    let system = peripherals.SYSTEM.split();
    let clocks = ClockControl::max(system.clock_control).freeze();

    embassy::init(
        &clocks,
        esp32c6_hal::systimer::SystemTimer::new(peripherals.SYSTIMER),
    );

    esp_println::logger::init_logger_from_env();
    log::info!("Hello world!");

    let io = IO::new(peripherals.GPIO, peripherals.IO_MUX);
    let sclk = io.pins.gpio18;
    let miso = io.pins.gpio20;
    let mosi = io.pins.gpio19;
    let cs = io.pins.gpio9;

    let dma = Dma::new(peripherals.DMA);
    let dma_channel = dma.channel0;

    let (mut descriptors, mut rx_descriptors) = dma_descriptors!(32000);

    // Initialize spi at the maxiumum SD initialization frequency of 400 khz
    let spi = Spi::new(peripherals.SPI2, 400u32.kHz(), SpiMode::Mode0, &clocks)
        .with_sck(sclk)
        .with_miso(miso)
        .with_mosi(mosi)
        .with_dma(dma_channel.configure(
            false,
            &mut descriptors,
            &mut rx_descriptors,
            DmaPriority::Priority0,
        ));

    let spi = FlashSafeDma::<_, 512>::new(spi);

    let mut sd = sdspi::SdSpi::new(spi, cs.into_push_pull_output(), Delay(embassy_time::Delay));

    // Initialize the card
    sd.init().await.unwrap();

    // Increase the speed up to the SD max of 25mhz
    sd.spi()
        .inner_mut()
        .change_bus_frequency(25u32.MHz(), &clocks);

    log::info!("Initialization complete!");

    let inner = BufStream::<_, 512, 4>::new(sd);

    let fs = embedded_fatfs::FileSystem::new(inner, FsOptions::new())
        .await
        .unwrap();

    {
        let mut f = fs.root_dir().create_file("test.log").await.unwrap();
        let hello = b"Hello world!";
        f.write_all(hello).await.unwrap();
        f.flush().await.unwrap();
    }

    // See https://github.com/MabezDev/embedded-fatfs/issues/19 for why this re open is currently needed
    {
        let mut f = fs.root_dir().open_file("test.log").await.unwrap();
        let mut buf = [0u8; 12];
        f.read_exact(&mut buf[..]).await.unwrap();
        log::info!(
            "Read from file: {}",
            core::str::from_utf8(&buf[..]).unwrap()
        );
    }

    fs.unmount().await.unwrap();

    loop {}
}

// We can remove this once https://github.com/embassy-rs/embassy/pull/2593 is released
pub struct Delay(embassy_time::Delay);

impl Clone for Delay {
    fn clone(&self) -> Self {
        Self(embassy_time::Delay)
    }
}

impl embedded_hal_async::delay::DelayNs for Delay {
    async fn delay_ns(&mut self, ns: u32) {
        embedded_hal_async::delay::DelayNs::delay_ns(&mut self.0, ns).await
    }

    async fn delay_us(&mut self, us: u32) {
        embedded_hal_async::delay::DelayNs::delay_us(&mut self.0, us).await
    }

    async fn delay_ms(&mut self, ms: u32) {
        embedded_hal_async::delay::DelayNs::delay_ms(&mut self.0, ms).await
    }
}
