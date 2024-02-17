#![no_std]

pub struct SpiSdmmc<SPI, CS, D>
where
    SPI: embedded_hal_async::spi::SpiBus,
    CS: embedded_hal::digital::OutputPin,
    D: embedded_hal_async::delay::DelayNs,
{
    spi: SPI,
    cs: CS,
    delay: D,
}

impl<SPI, CS, D> SpiSdmmc<SPI, CS, D>
where
    SPI: embedded_hal_async::spi::SpiBus,
    CS: embedded_hal::digital::OutputPin,
    D: embedded_hal_async::delay::DelayNs,
{
    pub fn new(spi: SPI, cs: CS, delay: D) -> Self {
        Self { spi, cs, delay }
    }
}

impl<SPI, CS, D> block_device_driver::BlockDevice for SpiSdmmc<SPI, CS, D>
where
    SPI: embedded_hal_async::spi::SpiBus,
    CS: embedded_hal::digital::OutputPin,
    D: embedded_hal_async::delay::DelayNs,
{
    type Error;

    async fn read(&mut self, block_address: u32, data: &mut [[u8; SIZE]]) -> Result<(), Self::Error> {
        todo!()
    }

    async fn write(&mut self, block_address: u32, data: &[[u8; SIZE]]) -> Result<(), Self::Error> {
        todo!()
    }

    async fn size(&mut self) -> Result<u64, Self::Error> {
        todo!()
    }
}

