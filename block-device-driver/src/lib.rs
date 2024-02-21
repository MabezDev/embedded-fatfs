//! An abstraction of block devices.

#![cfg_attr(not(test), no_std)]
#![warn(missing_docs)]
#![allow(async_fn_in_trait)]

use elain::{Align, Alignment};

/// A trait for a block devices
///
/// [`BlockDevice<const SIZE: usize, const ALIGN: usize>`](BlockDevice) can be initialized with the following parameters.
///
/// - `SIZE`: The size of the block, this dictates the size of the internal buffer.
/// - `ALIGN`: The alignment of the block buffers, this defaults to 1 unless
///            specified in the trait impl. Alignment must be a power of two.
///
/// The generic parameter `SIZE` on [BlockDevice] is the number of _bytes_ in a block 
/// for this block device.
///
/// All addresses are zero indexed, and the unit is blocks. For example to read bytes
/// from 1024 to 1536, the supplied block address would be 2.
///
/// This trait can be implemented multiple times to support various different block sizes.
pub trait BlockDevice<const SIZE: usize, const ALIGN: usize = 1>
where
    Align<ALIGN>: Alignment,
{
    /// The error type for the BlockDevice implementation.
    type Error: core::fmt::Debug;

    /// Read one or more blocks at the given block address.
    async fn read(
        &mut self,
        block_address: u32,
        data: &mut [AlignedBuffer<SIZE, ALIGN>],
    ) -> Result<(), Self::Error>;

    /// Write one or more blocks at the given block address.
    async fn write(
        &mut self,
        block_address: u32,
        data: &[AlignedBuffer<SIZE, ALIGN>],
    ) -> Result<(), Self::Error>;

    /// Report the size of the block device in bytes.
    async fn size(&mut self) -> Result<u64, Self::Error>;
}

impl<T: BlockDevice<SIZE, ALIGN>, const SIZE: usize, const ALIGN: usize> BlockDevice<SIZE, ALIGN>
    for &mut T
where
    Align<ALIGN>: Alignment,
{
    type Error = T::Error;

    async fn read(
        &mut self,
        block_address: u32,
        data: &mut [AlignedBuffer<SIZE, ALIGN>],
    ) -> Result<(), Self::Error> {
        (*self).read(block_address, data).await
    }

    async fn write(
        &mut self,
        block_address: u32,
        data: &[AlignedBuffer<SIZE, ALIGN>],
    ) -> Result<(), Self::Error> {
        (*self).write(block_address, data).await
    }

    async fn size(&mut self) -> Result<u64, Self::Error> {
        (*self).size().await
    }
}

/// AlignedBuffer
///
/// A representation of an buffer with specific length and alignment requirements.
#[derive(Clone)]
#[repr(C)]
pub struct AlignedBuffer<const SIZE: usize, const ALIGN: usize>
where
    Align<ALIGN>: Alignment,
{
    _align: Align<ALIGN>,
    buffer: [u8; SIZE],
}

impl<const SIZE: usize, const ALIGN: usize> AlignedBuffer<SIZE, ALIGN>
where
    Align<ALIGN>: Alignment,
{
    /// Creates a new AlignedBuffer.
    pub const fn new() -> Self {
        Self {
            _align: Align::NEW,
            buffer: [0; SIZE],
        }
    }
}

impl<const SIZE: usize, const ALIGN: usize> core::ops::Deref for AlignedBuffer<SIZE, ALIGN>
where
    Align<ALIGN>: Alignment,
{
    type Target = [u8; SIZE];

    fn deref(&self) -> &Self::Target {
        &self.buffer
    }
}

impl<const SIZE: usize, const ALIGN: usize> core::ops::DerefMut for AlignedBuffer<SIZE, ALIGN>
where
    Align<ALIGN>: Alignment,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.buffer
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn basic() {
        assert!(core::mem::size_of::<AlignedBuffer<512, 4>>() == 512);

        let buf: AlignedBuffer<512, 4> = AlignedBuffer::new();
        assert!(buf.as_ptr().cast::<u8>() as usize % 4 == 0);
    }
}
