#![no_std]

use core::fmt::Debug;
use core::future::Future;
use embassy_futures::select::{select, Either};
use sdio_host::common_cmd::*;

// MUST be the first module listed
mod fmt;

/// Status for card in the ready state
pub const R1_READY_STATE: u8 = 0x00;
/// Status for card in the idle state
pub const R1_IDLE_STATE: u8 = 0x01;
/// Status bit for illegal command
pub const R1_ILLEGAL_COMMAND: u8 = 0x04;
/// Start data token for read or write single block*/
pub const DATA_START_BLOCK: u8 = 0xFE;
/// Stop token for write multiple blocks*/
pub const STOP_TRAN_TOKEN: u8 = 0xFD;
/// Start data token for write multiple blocks*/
pub const WRITE_MULTIPLE_TOKEN: u8 = 0xFC;
/// Mask for data response tokens after a write block operation
pub const DATA_RES_MASK: u8 = 0x1F;
/// Write data accepted token
pub const DATA_RES_ACCEPTED: u8 = 0x05;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Error {
    ChipSelect,
    SpiError,
    Timeout,
}

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
    D: embedded_hal_async::delay::DelayNs + Clone,
{
    pub fn new(spi: SPI, cs: CS, delay: D) -> Self {
        Self { spi, cs, delay }
    }

    pub async fn init(&mut self) -> Result<(), Error> {
        self.cs.set_high().map_err(|_| Error::ChipSelect)?;
        // Supply minimum of 74 clock cycles without CS asserted.
        self.spi
            .write(&[0xFF; 10])
            .await
            .map_err(|_| Error::SpiError)?;
        self.cs.set_low().map_err(|_| Error::ChipSelect)?;

        with_timeout(self.delay.clone(), 1000, async {
            loop {
                let r = self.cmd(idle()).await?;
                trace!("Idle resp: {}", r);
                if r == R1_IDLE_STATE {
                    return Ok(());
                }
            }
        })
        .await??;

        Ok(())
    }

    async fn cmd<R: Resp>(&mut self, cmd: Cmd<R>) -> Result<u8, Error> {
        if cmd.cmd != idle().cmd {
            self.wait_idle().await?;
        }

        let mut buf = [
            0x40 | cmd.cmd,
            (cmd.arg >> 24) as u8,
            (cmd.arg >> 16) as u8,
            (cmd.arg >> 8) as u8,
            cmd.arg as u8,
            0,
        ];
        buf[5] = crc7(&buf[0..5]);

        self.spi.write(&buf).await.map_err(|_| Error::SpiError)?;

        // skip stuff byte for stop read
        if cmd.cmd == stop_transmission().cmd {
            self.spi
                .transfer_in_place(&mut [0xFF])
                .await
                .map_err(|_| Error::SpiError)?;
        }

        let byte = with_timeout(self.delay.clone(), 1000, async {
            loop {
                let byte = self.read_byte().await?;
                if byte & 0x80 == 0 {
                    return Ok(byte);
                }
            }
        })
        .await??;

        Ok(byte)
    }

    async fn wait_idle(&mut self) -> Result<(), Error> {
        with_timeout(self.delay.clone(), 5000, async {
            while self.read_byte().await? != 0xFF {}
            Ok(())
        })
        .await?
    }

    async fn read_byte(&mut self) -> Result<u8, Error> {
        let mut buf = [0xFFu8; 1];
        self.spi
            .transfer_in_place(&mut buf[..])
            .await
            .map_err(|_| Error::SpiError)?;

        Ok(buf[0])
    }
}

impl<SPI, CS, D, const SIZE: usize> block_device_driver::BlockDevice<SIZE> for SpiSdmmc<SPI, CS, D>
where
    SPI: embedded_hal_async::spi::SpiBus,
    CS: embedded_hal::digital::OutputPin,
    D: embedded_hal_async::delay::DelayNs,
{
    type Error = Error;

    async fn read(
        &mut self,
        _block_address: u32,
        _data: &mut [[u8; SIZE]],
    ) -> Result<(), Self::Error> {
        todo!()
    }

    async fn write(&mut self, _block_address: u32, _data: &[[u8; SIZE]]) -> Result<(), Self::Error> {
        todo!()
    }

    async fn size(&mut self) -> Result<u64, Self::Error> {
        todo!()
    }
}

async fn with_timeout<D: embedded_hal_async::delay::DelayNs, F: Future>(
    mut delay: D,
    timeout: u32,
    fut: F,
) -> Result<F::Output, Error> {
    match select(fut, delay.delay_ms(timeout)).await {
        Either::First(r) => Ok(r),
        Either::Second(_) => Err(Error::Timeout),
    }
}

/// Perform the 7-bit CRC used on the SD card
fn crc7(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for mut d in data.iter().cloned() {
        for _bit in 0..8 {
            crc <<= 1;
            if ((d & 0x80) ^ (crc & 0x80)) != 0 {
                crc ^= 0x09;
            }
            d <<= 1;
        }
    }
    (crc << 1) | 1
}

/// Perform the X25 CRC calculation, as used for data blocks.
fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in data {
        crc = ((crc >> 8) & 0xFF) | (crc << 8);
        crc ^= u16::from(byte);
        crc ^= (crc & 0xFF) >> 4;
        crc ^= crc << 12;
        crc ^= (crc & 0xFF) << 5;
    }
    crc
}
