#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

use embassy_executor::Spawner;
use esp32c6_hal::{
    clock::ClockControl,
    dma::DmaPriority,
    dma_descriptors, embassy,
    gdma::Gdma,
    peripherals::Peripherals,
    prelude::*,
    spi::{
        master::{prelude::*, Spi},
        SpiMode,
    },
    FlashSafeDma, IO,
};
use static_cell::make_static;
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

    let dma = Gdma::new(peripherals.DMA);
    let dma_channel = dma.channel0;

    let (mut descriptors, mut rx_descriptors) = dma_descriptors!(32000);

    let spi = Spi::new(peripherals.SPI2, 100u32.kHz(), SpiMode::Mode0, &clocks)
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

    let mut sd =
        sdmmc_spi_async::SpiSdmmc::new(spi, cs.into_push_pull_output(), Delay(embassy_time::Delay));

    sd.init().await.unwrap();

    log::info!("Initialization complete!");

    // we _must_ do this make static dance because the buffer might be placed in the cache
    // the DMA, used in async spi cannot write there, therefore read operations will fail.
    let mbr = make_static!([[0xAAu8; 512], [0xBB; 512]]);
    log::info!("Addr of buffer: {:p}", mbr.as_slice().as_ptr());

    sd.write(0, mbr).await.unwrap();
    mbr[0].fill(0); // reset the block for reading
    mbr[1].fill(0); // reset the block for reading
    sd.read(0, mbr).await.unwrap();

    log::info!("Contents of MBR: {:?}", mbr);

    loop {}
}

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
