#![no_std]
#![allow(async_fn_in_trait)]

/// A trait for a block devices
///
/// This trait can be implemented multiple times to support various different block sizes.
pub trait BlockDevice<const SIZE: usize> {
    type Error: core::fmt::Debug;

    /// Read one or more blocks at the given block address.
    async fn read(&mut self, block_address: u32, data: &mut [[u8; SIZE]]) -> Result<(), Self::Error>;

    /// Write one or more blocks at the given block address.
    async fn write(&mut self, block_address: u32, data: &[[u8; SIZE]]) -> Result<(), Self::Error>;

    // Report the size of the block device.
    async fn size(&mut self) -> Result<u64, Self::Error>;
}