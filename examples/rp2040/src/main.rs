#![no_std]
#![no_main]

use block_device_adapters::BufStream;
use block_device_adapters::BufStreamError;
use embassy_embedded_hal::shared_bus::asynch::spi::SpiDeviceWithConfig;
use embassy_executor::Spawner;
use embassy_rp::{
    gpio::{Level, Output},
    peripherals::*,
    spi::{Async, Config, Spi},
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use embedded_fatfs::FsOptions;
use embedded_hal_async::delay::DelayNs;
use embedded_io_async::{Read, Seek, Write};
use heapless::{String, Vec};
use sdspi::SdSpi;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

static SPI_BUS: StaticCell<Mutex<CriticalSectionRawMutex, Spi<'static, SPI0, Async>>> =
    StaticCell::new();

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    defmt::info!("Hello World!");

    let miso = p.PIN_16;
    let mosi = p.PIN_7;
    let clk = p.PIN_6;
    let cs = Output::new(p.PIN_5, Level::High);

    let mut config = Config::default();
    config.frequency = 400_000;

    let spi = Spi::new(
        p.SPI0,
        clk,
        mosi,
        miso,
        p.DMA_CH0,
        p.DMA_CH1,
        config.clone(),
    );
    let spi_bus = SPI_BUS.init(Mutex::new(spi));

    let spid = SpiDeviceWithConfig::new(spi_bus, cs, config);
    let mut sd = SdSpi::<_, _, aligned::A1>::new(spid, embassy_time::Delay);

    loop {
        // Initialize the card
        if let Ok(_) = sd.init().await {
            // Increase the speed up to the SD max of 25mhz

            let mut config = Config::default();
            config.frequency = 25_000_000;
            sd.spi().set_config(config);
            defmt::info!("Initialization complete!");

            break;
        }
        defmt::info!("Failed to init card, retrying...");
        embassy_time::Delay.delay_ns(5000u32).await;
    }

    let inner = BufStream::<_, 512>::new(sd);

    async {
        let fs = embedded_fatfs::FileSystem::new(inner, FsOptions::new()).await?;
        {
            let root = fs.root_dir();
            let mut iter = root.iter();
            loop {
                if let Some(Ok(entry)) = iter.next().await {
                    let name: String<256> = String::from_utf8(
                        Vec::from_slice(entry.short_file_name_as_bytes()).unwrap(),
                    )
                    .unwrap();
                    defmt::info!("Name:{} Length:{}", &name, entry.len());
                } else {
                    defmt::info!("end");
                    break;
                }
            }
        }
        fs.unmount().await?;

        Ok::<(), embedded_fatfs::Error<BufStreamError<sdspi::Error>>>(())
    }
    .await
    .expect("Filesystem tests failed!");

    loop {}
}
