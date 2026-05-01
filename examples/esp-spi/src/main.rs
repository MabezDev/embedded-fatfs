#![no_std]
#![no_main]

use block_device_adapters::BufStream;
use block_device_adapters::BufStreamError;
use embassy_executor::Spawner;
use embedded_fatfs::FsOptions;
use embedded_hal_bus::spi::ExclusiveDevice;
use embedded_io_async::{Read, Seek, Write};
use esp_backtrace as _;
use esp_hal::{
    dma::{DmaRxBuf, DmaTxBuf},
    dma_buffers,
    gpio::{Level, Output, OutputConfig},
    interrupt::software::SoftwareInterruptControl,
    spi::{
        master::{Config, Spi},
        Mode,
    },
    time::Rate,
    timer::timg::TimerGroup,
};
use sdspi::{self, SdSpi};

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) {
    esp_println::logger::init_logger_from_env();
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    log::info!("Hello world!");

    // Pin assignments - adjust for your board
    let sclk = peripherals.GPIO37;
    let miso = peripherals.GPIO35;
    let mosi = peripherals.GPIO36;
    let mut cs = Output::new(peripherals.GPIO38, Level::High, OutputConfig::default());

    cfg_if::cfg_if! {
        if #[cfg(any(feature = "esp32", feature = "esp32s2"))] {
            let dma_channel = peripherals.DMA_SPI2;
        } else {
            let dma_channel = peripherals.DMA_CH0;
        }
    }

    let (rx_buffer, rx_descriptors, tx_buffer, tx_descriptors) = dma_buffers!(32000);
    let dma_rx_buf = DmaRxBuf::new(rx_descriptors, rx_buffer).unwrap();
    let dma_tx_buf = DmaTxBuf::new(tx_descriptors, tx_buffer).unwrap();

    // Initialize SPI at the maximum SD initialization frequency of 400 kHz
    let mut spi = Spi::new(
        peripherals.SPI2,
        Config::default()
            .with_frequency(Rate::from_khz(400))
            .with_mode(Mode::_0),
    )
    .unwrap()
    .with_sck(sclk)
    .with_mosi(mosi)
    .with_miso(miso)
    .with_dma(dma_channel)
    .with_buffers(dma_rx_buf, dma_tx_buf)
    .into_async();

    // SD cards need at least 74 clock cycles on their SPI clock without CS asserted
    loop {
        match sdspi::sd_init(&mut spi, &mut cs).await {
            Ok(_) => break,
            Err(e) => {
                log::warn!("Sd init error: {:?}", e);
                embassy_time::Timer::after_millis(10).await;
            }
        }
    }

    let spid = ExclusiveDevice::new(spi, cs, embassy_time::Delay).unwrap();
    let mut sd = SdSpi::<_, _, aligned::A1>::new(spid, embassy_time::Delay);

    loop {
        if sd.init().await.is_ok() {
            log::info!("Initialization complete!");
            break;
        }
        log::info!("Failed to init card, retrying...");
        embassy_time::Timer::after_millis(5).await;
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
