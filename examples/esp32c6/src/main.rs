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
    dma::*,
    dma_buffers,
    gpio::{Io, Level, Output},
    peripherals::Peripherals,
    prelude::*,
    spi::{
        master::{Spi, SpiDmaBus},
        SpiMode,
    },
    system::SystemControl,
    timer::timg::TimerGroup,
};
use sdspi::{self, SdSpi};

#[esp_hal_embassy::main]
async fn main(_spawner: Spawner) {
    let peripherals = Peripherals::take();
    let system = SystemControl::new(peripherals.SYSTEM);
    let clocks = ClockControl::boot_defaults(system.clock_control).freeze();

    let timg0 = TimerGroup::new(peripherals.TIMG0, &clocks);
    esp_hal_embassy::init(&clocks, timg0.timer0);

    esp_println::logger::init_logger_from_env();
    log::info!("Hello world!");

    let io = Io::new(peripherals.GPIO, peripherals.IO_MUX);
    let sclk = io.pins.gpio18;
    let miso = io.pins.gpio20;
    let mosi = io.pins.gpio19;
    let mut cs = Output::new(io.pins.gpio9, Level::High);

    let dma = Dma::new(peripherals.DMA);
    let dma_channel = dma.channel0;

    let (rx_buffer, rx_descriptors, tx_buffer, tx_descriptors) = dma_buffers!(32000);
    let dma_rx_buf = DmaRxBuf::new(rx_descriptors, rx_buffer).unwrap();
    let dma_tx_buf = DmaTxBuf::new(tx_descriptors, tx_buffer).unwrap();

    // Initialize spi at the maxiumum SD initialization frequency of 400 khz
    let spi = Spi::new(peripherals.SPI2, 400u32.kHz(), SpiMode::Mode0, &clocks)
        .with_sck(sclk)
        .with_miso(miso)
        .with_mosi(mosi)
        .with_dma(dma_channel.configure_for_async(false, DmaPriority::Priority0));

    let mut spi = SpiDmaBus::new(spi, dma_tx_buf, dma_rx_buf);

    // Sd cards need to be clocked with a at least 74 cycles on their spi clock without the cs enabled,
    // sd_init is a helper function that does this for us.
    loop {
        match sdspi::sd_init(&mut spi, &mut cs).await {
            Ok(_) => break,
            Err(e) => {
                log::warn!("Sd init error: {:?}", e);
                embassy_time::Timer::after_millis(10).await;
            }
        }
    }

    let spid = ExclusiveDevice::new(spi, cs, embassy_time::Delay);
    let mut sd = SdSpi::<_, _, aligned::A1>::new(spid, embassy_time::Delay);

    loop {
        // Initialize the card
        if sd.init().await.is_ok() {
            // Increase the speed up to the SD max of 25mhz
            sd.spi()
                .bus_mut()
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
