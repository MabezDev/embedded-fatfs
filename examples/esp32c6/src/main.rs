#![no_std]
#![no_main]

use block_device_adapters::BufStream;
use block_device_adapters::BufStreamError;
use embassy_executor::Spawner;
use embedded_fatfs::FsOptions;
use embedded_hal_async::delay::DelayNs;
use embedded_hal_bus::spi::ExclusiveDevice;
use embedded_io_async::{Read, Seek, Write};
use esp_backtrace as _;
use esp_hal::{
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
use sdspi::SdSpi;

#[main]
async fn main(_spawner: Spawner) {
    let peripherals = Peripherals::take();
    let system = peripherals.SYSTEM.split();
    let clocks = ClockControl::max(system.clock_control).freeze();

    embassy::init(
        &clocks,
        esp_hal::systimer::SystemTimer::new(peripherals.SYSTIMER),
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

    let spid = ExclusiveDevice::new(spi, cs.into_push_pull_output(), embassy_time::Delay);
    let mut sd = SdSpi::<_, _, aligned::A1>::new(spid, embassy_time::Delay);

    loop {
        // Initialize the card
        if let Ok(_) = sd.init().await {
            // Increase the speed up to the SD max of 25mhz
            sd.spi()
                .bus_mut()
                .inner_mut()
                .change_bus_frequency(25u32.MHz(), &clocks);
            log::info!("Initialization complete!");

            break;
        }
        log::info!("Failed to init card, retrying...");
        embassy_time::Delay.delay_ns(5000u32).await;
    }

    let inner = BufStream::<_, 512>::new(sd);

    async {
        let fs = embedded_fatfs::FileSystem::new(inner, FsOptions::new()).await?;
        {
            let mut f = fs.root_dir().create_file("test.log").await?;
            let hello = b"Hello world!";
            log::info!("Writing to file...");
            f.write_all(hello).await?;
            f.flush().await?;

            let mut buf = [0u8; 12];
            f.rewind().await?;
            f.read_exact(&mut buf[..]).await?;
            log::info!(
                "Read from file: {}",
                core::str::from_utf8(&buf[..]).unwrap()
            );
        }
        fs.unmount().await?;

        Ok::<(), embedded_fatfs::Error<BufStreamError<sdspi::Error>>>(())
    }
    .await
    .expect("Filesystem tests failed!");

    loop {}
}
