//! An abstraction of block devices.

#![cfg_attr(not(test), no_std)]
#![warn(missing_docs)]
#![allow(async_fn_in_trait)]

use aligned::Aligned;

/// A trait for a block devices
///
/// [`BlockDevice<const SIZE: usize>`](BlockDevice) can be initialized with the following parameters.
///
/// - `SIZE`: The size of the block in the block device.
///
/// The generic parameter `SIZE` on [BlockDevice] is the number of _bytes_ in a block
/// for this block device.
///
/// All addresses are zero indexed, and the unit is blocks. For example to read bytes
/// from 1024 to 1536, the supplied block address would be 2.
///
/// This trait can be implemented multiple times to support various different block sizes.
pub trait BlockDevice<const SIZE: usize> {
    /// The error type for the BlockDevice implementation.
    type Error: core::fmt::Debug;

    /// The alignment requirements of the block buffers.
    type Align: aligned::Alignment;

    /// Read one or more blocks at the given block address.
    async fn read(
        &mut self,
        block_address: u32,
        data: &mut [Aligned<Self::Align, [u8; SIZE]>],
    ) -> Result<(), Self::Error>;

    /// Write one or more blocks at the given block address.
    async fn write(
        &mut self,
        block_address: u32,
        data: &[Aligned<Self::Align, [u8; SIZE]>],
    ) -> Result<(), Self::Error>;

    /// Report the size of the block device in bytes.
    async fn size(&mut self) -> Result<u64, Self::Error>;
}

impl<T: BlockDevice<SIZE>, const SIZE: usize> BlockDevice<SIZE> for &mut T {
    type Error = T::Error;
    type Align = T::Align;

    async fn read(
        &mut self,
        block_address: u32,
        data: &mut [Aligned<Self::Align, [u8; SIZE]>],
    ) -> Result<(), Self::Error> {
        (*self).read(block_address, data).await
    }

    async fn write(
        &mut self,
        block_address: u32,
        data: &[Aligned<Self::Align, [u8; SIZE]>],
    ) -> Result<(), Self::Error> {
        (*self).write(block_address, data).await
    }

    async fn size(&mut self) -> Result<u64, Self::Error> {
        (*self).size().await
    }
}
