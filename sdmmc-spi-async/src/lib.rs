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
