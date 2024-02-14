//! Device helper structs

use core::cmp;
use core::fmt::Debug;
use elain::{Align, Alignment};
use embedded_io_async::{ErrorKind, Read, Seek, SeekFrom, Write};

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug)]
pub enum StreamSliceError<T: Debug> {
    InvalidSeek(i64),
    WriteZero,
    Other(T),
}

impl<E: Debug> From<E> for StreamSliceError<E> {
    fn from(e: E) -> Self {
        Self::Other(e)
    }
}

/// Stream wrapper for accessing limited segment of data from underlying file or device.
#[derive(Clone)]
pub struct StreamSlice<T: Read + Write + Seek> {
    inner: T,
    start_offset: u64,
    current_offset: u64,
    size: u64,
}

impl<E: Debug> embedded_io_async::Error for StreamSliceError<E> {
    fn kind(&self) -> embedded_io_async::ErrorKind {
        match self {
            StreamSliceError::InvalidSeek(_) => embedded_io_async::ErrorKind::InvalidInput,
            StreamSliceError::Other(_) | StreamSliceError::WriteZero => embedded_io_async::ErrorKind::Other,
        }
    }
}

impl<T: Read + Write + Seek> embedded_io_async::ErrorType for StreamSlice<T> {
    type Error = StreamSliceError<T::Error>;
}

impl<T: Read + Write + Seek> StreamSlice<T> {
    /// Creates new `StreamSlice` from inner stream and offset range.
    ///
    /// `start_offset` is inclusive offset of the first accessible byte.
    /// `end_offset` is exclusive offset of the first non-accessible byte.
    /// `start_offset` must be lower or equal to `end_offset`.
    pub async fn new(mut inner: T, start_offset: u64, end_offset: u64) -> Result<Self, StreamSliceError<T::Error>> {
        debug_assert!(end_offset >= start_offset);
        inner.seek(SeekFrom::Start(start_offset)).await?;
        let size = end_offset - start_offset;
        Ok(StreamSlice {
            start_offset,
            size,
            inner,
            current_offset: 0,
        })
    }

    /// Returns inner object
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T: Read + Write + Seek> Read for StreamSlice<T> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, StreamSliceError<T::Error>> {
        let max_read_size = cmp::min((self.size - self.current_offset) as usize, buf.len());
        let bytes_read = self.inner.read(&mut buf[..max_read_size]).await?;
        self.current_offset += bytes_read as u64;
        Ok(bytes_read)
    }
}

impl<T: Read + Write + Seek> Write for StreamSlice<T> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, StreamSliceError<T::Error>> {
        let max_write_size = cmp::min((self.size - self.current_offset) as usize, buf.len());
        let bytes_written = self.inner.write(&buf[..max_write_size]).await?;
        if bytes_written == 0 {
            return Err(StreamSliceError::WriteZero);
        }
        self.current_offset += bytes_written as u64;
        Ok(bytes_written)
    }

    async fn flush(&mut self) -> Result<(), StreamSliceError<T::Error>> {
        self.inner.flush().await?;
        Ok(())
    }
}

impl<T: Read + Write + Seek> Seek for StreamSlice<T> {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, StreamSliceError<T::Error>> {
        let new_offset = match pos {
            SeekFrom::Current(x) => self.current_offset as i64 + x,
            SeekFrom::Start(x) => x as i64,
            SeekFrom::End(x) => self.size as i64 + x,
        };
        if new_offset < 0 || new_offset as u64 > self.size {
            Err(StreamSliceError::InvalidSeek(new_offset))
        } else {
            self.inner
                .seek(SeekFrom::Start(self.start_offset + new_offset as u64))
                .await?;
            self.current_offset = new_offset as u64;
            Ok(self.current_offset)
        }
    }
}

/// A trait for a block devices
///
/// The generic parameter `SIZE` is used by [`BlockDevice`] to determine the block size of the device.
///
/// This trait can be implemented multiple times to support various different block sizes.
pub trait Device<const SIZE: usize> {
    type Error: core::fmt::Debug;

    /// Read one or more blocks at the given block address.
    async fn read(&mut self, block_address: u64, data: &mut [[u8; SIZE]]) -> Result<(), Self::Error>;

    /// Write one or more blocks at the given block address.
    async fn write(&mut self, block_address: u64, data: &[[u8; SIZE]]) -> Result<(), Self::Error>;

    // Report the size of the device.
    async fn size(&mut self) -> Result<u64, Self::Error>;
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum BlockDeviceError<T> {
    Io(T),
}

impl<T> From<T> for BlockDeviceError<T> {
    fn from(t: T) -> Self {
        BlockDeviceError::Io(t)
    }
}

impl<T: core::fmt::Debug> embedded_io_async::Error for BlockDeviceError<T> {
    fn kind(&self) -> ErrorKind {
        ErrorKind::Other
    }
}

/// A Stream wrapper for accessing a stream in block sized chunks.
///
/// [`BlockDevice<T, const SIZE: usize, const ALIGN: usize`](BlockDevice) can be initialized with the following parameters.
///
/// - `T`: The inner stream.
/// - `SIZE`: The size of the block, this dictates the size of the internal buffer.
/// - `ALIGN`: The alignment of the internal buffer.
///
/// If the `buf` provided to either [`Read::read`] or [`Write::write`] meets the following conditions the `buf`
/// will be used directly instead of the intermediate buffer to avoid unnecessary copies:
///
/// - `buf.len()` is a multiple of block size
/// - `buf` has the same alignment as the internal buffer
/// - The byte address of the inner device is aligned to a block size.
///
/// [`BlockDevice<T, const SIZE: usize, const ALIGN: usize`](BlockDevice) implements the [`embedded_io_async`] traits, and implicitly
/// handles the RMW (Read, Modify, Write) cycle for you.
#[derive(Clone)]
pub struct BlockDevice<T: Device<SIZE>, const SIZE: usize, const ALIGN: usize>
where
    Align<ALIGN>: Alignment,
{
    inner: T,
    buffer: AlignedBuffer<SIZE, ALIGN>,
    current_block: u64,
    current_offset: u64,
}

impl<T: Device<SIZE>, const SIZE: usize, const ALIGN: usize> BlockDevice<T, SIZE, ALIGN>
where
    Align<ALIGN>: Alignment,
{
    /// Create a new [`BlockDevice`] around a hardware block device.
    pub async fn new(inner: T) -> Result<Self, T::Error> {
        let mut s = Self {
            inner,
            current_block: u64::MAX,
            current_offset: 0,
            buffer: AlignedBuffer::new(),
        };

        // Load the initial buffer at sector 0, so that flush functions correctly
        s.check_cache().await?;

        Ok(s)
    }

    /// Returns inner object.
    pub fn into_inner(self) -> T {
        self.inner
    }

    #[inline]
    fn pointer_block_start_addr(&self) -> u64 {
        (self.current_offset / SIZE as u64) * SIZE as u64
    }

    #[inline]
    fn pointer_block_start(&self) -> u64 {
        self.pointer_block_start_addr() / SIZE as u64
    }

    async fn check_cache(&mut self) -> Result<(), T::Error> {
        let block_start = self.pointer_block_start();
        if block_start != self.current_block {
            // We have seeked to a new block, read it
            let buf = &mut self.buffer[..];
            // Note unsafe: the internal buffer already has the correct size and alignment
            self.inner.read(block_start, slice_to_blocks_mut(buf)).await?;
            self.current_block = block_start;
        }
        Ok(())
    }
}

impl<T: Device<SIZE>, const SIZE: usize, const ALIGN: usize> embedded_io_async::ErrorType
    for BlockDevice<T, SIZE, ALIGN>
where
    Align<ALIGN>: Alignment,
{
    type Error = BlockDeviceError<T::Error>;
}

impl<T: Device<SIZE>, const SIZE: usize, const ALIGN: usize> Read for BlockDevice<T, SIZE, ALIGN>
where
    Align<ALIGN>: Alignment,
{
    async fn read(&mut self, mut buf: &mut [u8]) -> Result<usize, Self::Error> {
        let mut total = 0;
        let target = buf.len();
        loop {
            if buf.len() % SIZE == 0
                && &buf[0] as *const _ as usize % ALIGN == 0
                && self.current_offset % SIZE as u64 == 0
            {
                let block = self.pointer_block_start();
                // Note unsafe: we check the buf has the correct SIZE and ALIGNment before casting
                self.inner.read(block, slice_to_blocks_mut(buf)).await?;
                total += buf.len();
            } else {
                let block_start = self.pointer_block_start_addr();
                let block_end = block_start + SIZE as u64;
                trace!(
                    "offset {}, block_start {}, block_end {}",
                    self.current_offset,
                    block_start,
                    block_end
                );

                self.check_cache().await?;

                // copy as much as possible, up to the block boundary
                let buffer_offset = (self.current_offset - block_start) as usize;
                let bytes_to_read = buf.len();

                let end = core::cmp::min(buffer_offset + bytes_to_read, SIZE);
                trace!("buffer_offset {}, end {}", buffer_offset, end);
                let bytes_read = end - buffer_offset;
                buf[..bytes_read].copy_from_slice(&self.buffer[buffer_offset..end]);
                buf = &mut buf[bytes_read..]; // move the buffer along

                self.current_offset += bytes_read as u64;
                total += bytes_read;
            }

            if total == target {
                return Ok(total);
            }
        }
    }
}

impl<T: Device<SIZE>, const SIZE: usize, const ALIGN: usize> Write for BlockDevice<T, SIZE, ALIGN>
where
    Align<ALIGN>: Alignment,
{
    async fn write(&mut self, mut buf: &[u8]) -> Result<usize, Self::Error> {
        let mut total = 0;
        let target = buf.len();
        loop {
            if buf.len() % SIZE == 0
                && &buf[0] as *const _ as usize % ALIGN == 0
                && self.current_offset % SIZE as u64 == 0
            {
                // If the provided buffer has a suitable length and alignment use it directly
                let block = self.pointer_block_start();
                // Note unsafe: we check the buf has the correct SIZE and ALIGNment before casting
                self.inner.write(block, slice_to_blocks(buf)).await?;
                total += buf.len();
            } else {
                let block_start = self.pointer_block_start_addr();
                let block_end = block_start + SIZE as u64;
                trace!(
                    "offset {}, block_start {}, block_end {}",
                    self.current_offset,
                    block_start,
                    block_end
                );

                self.check_cache().await?;

                // copy as much as possible, up to the block boundary
                let buffer_offset = (self.current_offset - block_start) as usize;
                let bytes_to_write = buf.len();

                let end = core::cmp::min(buffer_offset + bytes_to_write, SIZE);
                trace!("buffer_offset {}, end {}", buffer_offset, end);
                let bytes_written = end - buffer_offset;
                self.buffer[buffer_offset..buffer_offset + bytes_written].copy_from_slice(&buf[..bytes_written]);
                buf = &buf[bytes_written..]; // move the buffer along

                // write out the whole block with the modified data
                self.flush().await?;

                self.current_offset += bytes_written as u64;
                total += bytes_written;
            }

            if total == target {
                return Ok(total);
            }
        }
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        // flush the internal buffer if we have modified the buffer
        self.inner
            .write(self.current_block, slice_to_blocks(&self.buffer[..]))
            .await?;
        Ok(())
    }
}

fn slice_to_blocks<const SIZE: usize>(slice: &[u8]) -> &[[u8; SIZE]] {
    assert!(slice.len() % SIZE == 0);
    unsafe { core::slice::from_raw_parts(slice.as_ptr() as *const [u8; SIZE], slice.len() / SIZE) }
}

fn slice_to_blocks_mut<const SIZE: usize>(slice: &mut [u8]) -> &mut [[u8; SIZE]] {
    assert!(slice.len() % SIZE == 0);
    unsafe { core::slice::from_raw_parts_mut(slice.as_mut_ptr() as *mut [u8; SIZE], slice.len() / SIZE) }
}

impl<T: Device<SIZE>, const SIZE: usize, const ALIGN: usize> Seek for BlockDevice<T, SIZE, ALIGN>
where
    Align<ALIGN>: Alignment,
{
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        self.current_offset = match pos {
            SeekFrom::Start(x) => x,
            SeekFrom::End(x) => (self.inner.size().await? as i64 - x) as u64,
            SeekFrom::Current(x) => (self.current_offset as i64 + x) as u64,
        };
        Ok(self.current_offset)
    }
}

#[derive(Clone)]
struct AlignedBuffer<const SIZE: usize, const ALIGN: usize>
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
mod tests {
    use embedded_io_async::ErrorType;

    use super::{BlockDevice, *};

    struct TestBlockDevice<T: Read + Write + Seek>(T);

    impl<T: Read + Write + Seek> ErrorType for TestBlockDevice<T> {
        type Error = T::Error;
    }

    impl<T: Read + Write + Seek> Read for TestBlockDevice<T> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            Ok(self.0.read(buf).await?)
        }
    }

    impl<T: Read + Write + Seek> Write for TestBlockDevice<T> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            Ok(self.0.write(buf).await?)
        }
    }

    impl<T: Read + Write + Seek> Seek for TestBlockDevice<T> {
        async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
            Ok(self.0.seek(pos).await?)
        }
    }

    impl<T: Read + Write + Seek> Device<512> for TestBlockDevice<T> {
        type Error = T::Error;

        /// Read one or more blocks at the given block address.
        async fn read(&mut self, block_address: u64, data: &mut [[u8; 512]]) -> Result<(), Self::Error> {
            self.0.seek(SeekFrom::Start(block_address * 512 as u64)).await?;
            for b in data {
                self.0.read(b).await?;
            }
            Ok(())
        }

        /// Write one or more blocks at the given block address.
        async fn write(&mut self, block_address: u64, data: &[[u8; 512]]) -> Result<(), Self::Error> {
            self.0.seek(SeekFrom::Start(block_address * 512 as u64)).await?;
            for b in data {
                self.0.write(b).await?;
            }
            Ok(())
        }

        async fn size(&mut self) -> Result<u64, Self::Error> {
            Ok(u64::MAX)
        }
    }

    #[tokio::test]
    async fn stream_test() {
        let _ = env_logger::builder().is_test(true).try_init();
        let buf = "BeforeTest dataAfter".to_string().into_bytes();
        let cur = std::io::Cursor::new(buf);
        let mut stream = StreamSlice::new(embedded_io_adapters::tokio_1::FromTokio::new(cur), 6, 6 + 9)
            .await
            .unwrap();

        let data = read_to_string(&mut stream).await.unwrap();
        assert_eq!(data, "Test data");

        stream.seek(SeekFrom::Start(5)).await.unwrap();
        let data = read_to_string(&mut stream).await.unwrap();
        assert_eq!(data, "data");

        stream.seek(SeekFrom::Start(5)).await.unwrap();
        stream.write_all("Rust".as_bytes()).await.unwrap();
        assert!(stream.write_all("X".as_bytes()).await.is_err());
        stream.seek(SeekFrom::Start(0)).await.unwrap();
        let data = read_to_string(&mut stream).await.unwrap();
        assert_eq!(data, "Test Rust");
    }

    #[tokio::test]
    async fn block_512_read_test() {
        let _ = env_logger::builder().is_test(true).try_init();
        let buf = ("A".repeat(512) + "B".repeat(512).as_str()).into_bytes();
        let cur = std::io::Cursor::new(buf);
        let mut block: BlockDevice<_, 512, 4> =
            BlockDevice::new(TestBlockDevice(embedded_io_adapters::tokio_1::FromTokio::new(cur)))
                .await
                .unwrap();

        // Test sector aligned access
        let mut buf = vec![0; 128];
        block.seek(SeekFrom::Start(0)).await.unwrap();
        block.read_exact(&mut buf[..]).await.unwrap();
        assert_eq!(buf, "A".repeat(128).into_bytes());

        let mut buf = vec![0; 128];
        block.seek(SeekFrom::Start(512)).await.unwrap();
        block.read_exact(&mut buf[..]).await.unwrap();
        assert_eq!(buf, "B".repeat(128).into_bytes());

        // Read across sectors
        let mut buf = vec![0; 128];
        block.seek(SeekFrom::Start(512 - 64)).await.unwrap();
        block.read_exact(&mut buf[..]).await.unwrap();
        assert_eq!(buf, ("A".repeat(64) + "B".repeat(64).as_str()).into_bytes());
    }

    #[tokio::test]
    async fn block_512_read_successive() {
        let _ = env_logger::builder().is_test(true).try_init();
        let buf = ("A".repeat(64) + "B".repeat(64).as_str()).repeat(16).into_bytes();
        let cur = std::io::Cursor::new(buf);
        let mut block: BlockDevice<_, 512, 4> =
            BlockDevice::new(TestBlockDevice(embedded_io_adapters::tokio_1::FromTokio::new(cur)))
                .await
                .unwrap();

        // Test sector aligned access
        let mut buf = vec![0; 64];
        block.seek(SeekFrom::Start(0)).await.unwrap();
        block.read_exact(&mut buf[..]).await.unwrap();
        assert_eq!(buf, "A".repeat(64).into_bytes());

        let mut buf = vec![0; 64];
        block.seek(SeekFrom::Start(64)).await.unwrap();
        block.read_exact(&mut buf[..]).await.unwrap();
        assert_eq!(buf, "B".repeat(64).into_bytes());

        let mut buf = vec![0; 64];
        block.seek(SeekFrom::Start(32)).await.unwrap();
        block.read_exact(&mut buf[..]).await.unwrap();
        assert_eq!(buf, ("A".repeat(32) + "B".repeat(32).as_str()).into_bytes());
    }

    #[tokio::test]
    async fn block_512_write_single_sector() {
        let _ = env_logger::builder().is_test(true).try_init();
        let buf = vec![0; 2048];
        let cur = std::io::Cursor::new(buf);
        let mut block: BlockDevice<_, 512, 4> =
            BlockDevice::new(TestBlockDevice(embedded_io_adapters::tokio_1::FromTokio::new(cur)))
                .await
                .unwrap();

        // Test sector aligned access
        let data_a = "A".repeat(512).into_bytes();
        block.seek(SeekFrom::Start(0)).await.unwrap();
        block.write_all(&data_a).await.unwrap();
        assert_eq!(&block.into_inner().0.into_inner().into_inner()[..512], data_a)
    }

    #[tokio::test]
    async fn block_512_write_across_sectors() {
        let _ = env_logger::builder().is_test(true).try_init();
        let buf = vec![0; 2048];
        let cur = std::io::Cursor::new(buf);
        let mut block: BlockDevice<_, 512, 4> =
            BlockDevice::new(TestBlockDevice(embedded_io_adapters::tokio_1::FromTokio::new(cur)))
                .await
                .unwrap();

        // Test sector aligned access
        let data_a = "A".repeat(512).into_bytes();
        block.seek(SeekFrom::Start(256)).await.unwrap();
        block.write_all(&data_a).await.unwrap();
        let buf = block.into_inner().0.into_inner().into_inner();
        assert_eq!(&buf[..256], [0; 256]);
        assert_eq!(&buf[256..768], data_a);
        assert_eq!(&buf[768..1024], [0; 256]);
    }

    #[tokio::test]
    async fn aligned_write_block_optimization() {
        let _ = env_logger::builder().is_test(true).try_init();
        let buf = vec![0; 2048];
        let cur = std::io::Cursor::new(buf);
        let mut block: BlockDevice<_, 512, 4> =
            BlockDevice::new(TestBlockDevice(embedded_io_adapters::tokio_1::FromTokio::new(cur)))
                .await
                .unwrap();

        let mut aligned_buffer: AlignedBuffer<2048, 4> = AlignedBuffer::new();
        let data_a = "A".repeat(512).into_bytes();
        aligned_buffer[..512].copy_from_slice(&data_a[..]);
        block.seek(SeekFrom::Start(0)).await.unwrap();
        block.write_all(&aligned_buffer[..]).await.unwrap();

        // if we wrote directly, the block buffer will be empty
        assert_eq!(&block.buffer[..], [0u8; 512]);
        // the write suceeded
        assert_eq!(&block.into_inner().0.into_inner().into_inner()[..512], &data_a)
    }

    #[tokio::test]
    async fn aligned_write_block_optimization_misaligned_block() {
        let _ = env_logger::builder().is_test(true).try_init();
        let buf = vec![0; 2048];
        let cur = std::io::Cursor::new(buf);
        let mut block: BlockDevice<_, 512, 4> =
            BlockDevice::new(TestBlockDevice(embedded_io_adapters::tokio_1::FromTokio::new(cur)))
                .await
                .unwrap();

        let mut aligned_buffer: AlignedBuffer<2048, 4> = AlignedBuffer::new();
        let data_a = "A".repeat(512).into_bytes();
        aligned_buffer[..512].copy_from_slice(&data_a[..]);
        // seek away from aligned block address
        block.seek(SeekFrom::Start(3)).await.unwrap();
        // attempt write all
        block.write_all(&aligned_buffer[..512]).await.unwrap();

        // because the addr was not block aligned, we will have used the cache
        assert_ne!(&block.buffer[..], [0u8; 512]);
        // the write suceeded
        assert_eq!(&block.into_inner().0.into_inner().into_inner()[3..515], &data_a)
    }

    #[tokio::test]
    async fn aligned_read_block_optimization() {
        let _ = env_logger::builder().is_test(true).try_init();
        let buf = "A".repeat(2048).into_bytes();
        let cur = std::io::Cursor::new(buf);
        let mut block: BlockDevice<_, 512, 4> =
            BlockDevice::new(TestBlockDevice(embedded_io_adapters::tokio_1::FromTokio::new(cur)))
                .await
                .unwrap();

        let mut aligned_buffer: AlignedBuffer<512, 4> = AlignedBuffer::new();
        block.seek(SeekFrom::Start(0)).await.unwrap();
        block.read_exact(&mut aligned_buffer[..]).await.unwrap();

        // if we read directly, the block buffer will be empty
        assert_eq!(&block.buffer[..], [0u8; 512]);
        // the write suceeded
        assert_eq!(
            &block.into_inner().0.into_inner().into_inner()[..512],
            &aligned_buffer[..]
        )
    }

    #[tokio::test]
    async fn aligned_read_block_optimization_misaligned() {
        let _ = env_logger::builder().is_test(true).try_init();
        let buf = "A".repeat(2048).into_bytes();
        let cur = std::io::Cursor::new(buf);
        let mut block: BlockDevice<_, 512, 4> =
            BlockDevice::new(TestBlockDevice(embedded_io_adapters::tokio_1::FromTokio::new(cur)))
                .await
                .unwrap();

        let mut aligned_buffer: AlignedBuffer<512, 4> = AlignedBuffer::new();
        // seek away from aligned block
        block.seek(SeekFrom::Start(3)).await.unwrap();
        // pass an aligned buffer with correct sizing
        block.read_exact(&mut aligned_buffer[..]).await.unwrap();

        // now, we must seek back and read the entire block
        // meaning our block cache will be written to:
        assert_ne!(&block.buffer[..], [0u8; 512]);

        // the read suceeded
        assert_eq!(
            &block.into_inner().0.into_inner().into_inner()[3..512],
            &aligned_buffer[3..]
        )
    }

    #[tokio::test]
    async fn write_seek_read_write() {
        let _ = env_logger::builder().is_test(true).try_init();
        let buf = "A".repeat(2048).into_bytes();
        let cur = std::io::Cursor::new(buf);
        let mut block: BlockDevice<_, 512, 4> =
            BlockDevice::new(TestBlockDevice(embedded_io_adapters::tokio_1::FromTokio::new(cur)))
                .await
                .unwrap();

        block.seek(SeekFrom::Start(524)).await.unwrap();
        block.write_all(&"B".repeat(512).into_bytes()).await.unwrap();

        block.seek(SeekFrom::Start(0)).await.unwrap();
        let mut tmp = [0u8; 256];
        block.read(&mut tmp[..]).await.unwrap();

        assert_eq!(&tmp[..], "A".repeat(256).into_bytes().as_slice());

        block.seek(SeekFrom::Start(524 + 512)).await.unwrap();
        block.write_all(&"C".repeat(512).into_bytes()).await.unwrap();

        let buf = block.into_inner().0.into_inner().into_inner();

        assert_eq!(
            buf,
            ("A".repeat(524) + &"B".repeat(512) + &"C".repeat(512) + &"A".repeat(500)).into_bytes()
        )
    }

    async fn read_to_string<IO: embedded_io_async::Read>(io: &mut IO) -> Result<String, IO::Error> {
        let mut buf = Vec::new();
        loop {
            let mut tmp = [0; 256];
            match io.read(&mut tmp).await {
                Ok(0) => break,
                Ok(n) => buf.extend(&tmp[..n]),
                Err(e) => return Err(e),
            }
        }

        Ok(String::from_utf8(buf).unwrap())
    }
}
