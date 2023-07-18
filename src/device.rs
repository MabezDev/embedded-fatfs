use core::cmp;
use core::fmt::Debug;
use elain::{Align, Alignment};
use embedded_io as io;
use embedded_io::{Read, ReadExactError, Seek, Write, WriteAllError};

#[derive(Debug)]
pub enum StreamSliceError<T: Debug> {
    InvalidSeek(i64),
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

impl<E: Debug> embedded_io::Error for StreamSliceError<E> {
    fn kind(&self) -> io::ErrorKind {
        match self {
            StreamSliceError::InvalidSeek(_) => io::ErrorKind::InvalidInput,
            StreamSliceError::Other(_) => io::ErrorKind::Other,
        }
    }
}

impl<T: Read + Write + Seek> embedded_io::ErrorType for StreamSlice<T> {
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
        inner.seek(io::SeekFrom::Start(start_offset)).await?;
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
        self.current_offset += bytes_written as u64;
        Ok(bytes_written)
    }

    async fn flush(&mut self) -> Result<(), StreamSliceError<T::Error>> {
        self.inner.flush().await?;
        Ok(())
    }
}

impl<T: Read + Write + Seek> Seek for StreamSlice<T> {
    async fn seek(&mut self, pos: io::SeekFrom) -> Result<u64, StreamSliceError<T::Error>> {
        let new_offset = match pos {
            io::SeekFrom::Current(x) => self.current_offset as i64 + x,
            io::SeekFrom::Start(x) => x as i64,
            io::SeekFrom::End(x) => self.size as i64 + x,
        };
        if new_offset < 0 || new_offset as u64 > self.size {
            Err(StreamSliceError::InvalidSeek(new_offset))
        } else {
            self.inner
                .seek(io::SeekFrom::Start(self.start_offset + new_offset as u64))
                .await?;
            self.current_offset = new_offset as u64;
            Ok(self.current_offset)
        }
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
/// - `buf.len()` has the same alignment as the internal buffer
/// 
#[derive(Clone)]
pub struct BlockDevice<T: Read + Write + Seek, const SIZE: usize, const ALIGN: usize>
where
    Align<ALIGN>: Alignment,
{
    inner: T,
    buffer: AlignedBuffer<SIZE, ALIGN>,
    current_block: u64,
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

impl<T: Read + Write + Seek, const SIZE: usize, const ALIGN: usize> BlockDevice<T, SIZE, ALIGN>
where
    Align<ALIGN>: Alignment,
{
    pub const fn new(inner: T) -> Self {
        Self {
            inner,
            current_block: u64::MAX,
            buffer: AlignedBuffer::new(),
        }
    }

    /// Returns inner object
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T: Read + Write + Seek, const SIZE: usize, const ALIGN: usize> embedded_io::ErrorType
    for BlockDevice<T, SIZE, ALIGN>
where
    Align<ALIGN>: Alignment,
{
    type Error = T::Error;
}

impl<T: Read + Write + Seek, const SIZE: usize, const ALIGN: usize> Read for BlockDevice<T, SIZE, ALIGN>
where
    Align<ALIGN>: Alignment,
    T::Error: From<ReadExactError<T::Error>> + From<WriteAllError<T::Error>>,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, T::Error> {
        Ok(if buf.len() % SIZE == 0 && &buf[0] as *const _ as usize % ALIGN == 0 {
            // If the provided buffer has a suitable length and alignment use it directly
            self.inner.read_exact(buf).await?;
            buf.len()
        } else {
            let offset = self.inner.seek(io::SeekFrom::Current(0)).await?;
            let block_start = (offset / SIZE as u64) * SIZE as u64;
            let block_end = block_start + SIZE as u64;
            log::info!("offset {offset}, block_start {block_start}, block_end {block_end}");

            if block_start != self.current_block {
                // We have seeked to a new block, read it
                self.inner.seek(io::SeekFrom::Start(block_start)).await?;
                self.inner.read_exact(&mut self.buffer[..]).await?;
            }

            // copy as much as possible, up to the block boundary
            let buffer_offset = (offset - block_start) as usize;
            let bytes_to_read = buf.len();
            let end = core::cmp::min(buffer_offset + bytes_to_read, SIZE);
            log::info!("buffer_offset {buffer_offset}, end {end}");
            let bytes_read = end - buffer_offset;
            buf[..bytes_read].copy_from_slice(&self.buffer[buffer_offset..end]);

            self.inner.seek(io::SeekFrom::Start(offset + bytes_read as u64)).await?;

            bytes_read
        })
    }
}

impl<T: Read + Write + Seek, const SIZE: usize, const ALIGN: usize> Write for BlockDevice<T, SIZE, ALIGN>
where
    Align<ALIGN>: Alignment,
    T::Error: From<ReadExactError<T::Error>> + From<WriteAllError<T::Error>>,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, T::Error> {
        Ok(if buf.len() % SIZE == 0 && &buf[0] as *const _ as usize % ALIGN == 0 {
            // If the provided buffer has a suitable length and alignment use it directly
            self.inner.write_all(buf).await?;
            buf.len()
        } else {
            let offset = self.inner.seek(io::SeekFrom::Current(0)).await?;
            let block_start = (offset / SIZE as u64) * SIZE as u64;
            let block_end = block_start + SIZE as u64;
            log::info!("offset {offset}, block_start {block_start}, block_end {block_end}");

            if block_start != self.current_block {
                // We have seeked to a new block, read it
                self.inner.seek(io::SeekFrom::Start(block_start)).await?;
                self.inner.read_exact(&mut self.buffer[..]).await?;
            }

            // copy as much as possible, up to the block boundary
            let buffer_offset = (offset - block_start) as usize;
            let bytes_to_write = buf.len();
            let end = core::cmp::min(buffer_offset + bytes_to_write, SIZE);
            log::info!("buffer_offset {buffer_offset}, end {end}");
            let bytes_written = end - buffer_offset;
            self.buffer[buffer_offset..buffer_offset + bytes_written].copy_from_slice(&buf[..bytes_written]);

            // write out the whole block with the modified data
            self.inner.seek(io::SeekFrom::Start(block_start)).await?;
            self.inner.write_all(&self.buffer[..]).await?;

            self.inner
                .seek(io::SeekFrom::Start(offset + bytes_written as u64))
                .await?;

            bytes_written
        })
    }

    async fn flush(&mut self) -> Result<(), T::Error> {
        self.inner.flush().await
    }
}

impl<T: Read + Write + Seek, const SIZE: usize, const ALIGN: usize> Seek for BlockDevice<T, SIZE, ALIGN>
where
    Align<ALIGN>: Alignment,
{
    async fn seek(&mut self, pos: io::SeekFrom) -> Result<u64, T::Error> {
        self.inner.seek(pos).await
    }
}

#[cfg(test)]
mod tests {
    use super::{BlockDevice, *};

    #[tokio::test]
    async fn stream_smoke_test() {
        let _ = env_logger::builder().is_test(true).try_init();
        let buf = "BeforeTest dataAfter".to_string().into_bytes();
        let cur = std::io::Cursor::new(buf);
        let mut stream = StreamSlice::new(embedded_io_adapters::tokio_1::FromTokio::new(cur), 6, 6 + 9)
            .await
            .unwrap();

        let data = read_to_string(&mut stream).await.unwrap();
        assert_eq!(data, "Test data");

        stream.seek(io::SeekFrom::Start(5)).await.unwrap();
        let data = read_to_string(&mut stream).await.unwrap();
        assert_eq!(data, "data");

        stream.seek(io::SeekFrom::Start(5)).await.unwrap();
        stream.write_all("Rust".as_bytes()).await.unwrap();
        assert!(stream.write_all("X".as_bytes()).await.is_err());
        stream.seek(io::SeekFrom::Start(0)).await.unwrap();
        let data = read_to_string(&mut stream).await.unwrap();
        assert_eq!(data, "Test Rust");
    }

    #[tokio::test]
    async fn block_512_read_test() {
        let _ = env_logger::builder().is_test(true).try_init();
        let buf = ("A".repeat(512) + "B".repeat(512).as_str()).into_bytes();
        let cur = std::io::Cursor::new(buf);
        let mut block: BlockDevice<_, 512, 4> = BlockDevice::new(embedded_io_adapters::tokio_1::FromTokio::new(cur));

        // Test sector aligned access
        let mut buf = vec![0; 128];
        block.seek(io::SeekFrom::Start(0)).await.unwrap();
        block.read_exact(&mut buf[..]).await.unwrap();
        assert_eq!(buf, "A".repeat(128).into_bytes());

        let mut buf = vec![0; 128];
        block.seek(io::SeekFrom::Start(512)).await.unwrap();
        block.read_exact(&mut buf[..]).await.unwrap();
        assert_eq!(buf, "B".repeat(128).into_bytes());

        // Read across sectors
        let mut buf = vec![0; 128];
        block.seek(io::SeekFrom::Start(512 - 64)).await.unwrap();
        block.read_exact(&mut buf[..]).await.unwrap();
        assert_eq!(buf, ("A".repeat(64) + "B".repeat(64).as_str()).into_bytes());
    }

    #[tokio::test]
    async fn block_512_read_successive() {
        let _ = env_logger::builder().is_test(true).try_init();
        let buf = ("A".repeat(64) + "B".repeat(64).as_str()).repeat(16).into_bytes();
        let cur = std::io::Cursor::new(buf);
        let mut block: BlockDevice<_, 512, 4> = BlockDevice::new(embedded_io_adapters::tokio_1::FromTokio::new(cur));

        // Test sector aligned access
        let mut buf = vec![0; 64];
        block.seek(io::SeekFrom::Start(0)).await.unwrap();
        block.read_exact(&mut buf[..]).await.unwrap();
        assert_eq!(buf, "A".repeat(64).into_bytes());

        let mut buf = vec![0; 64];
        block.seek(io::SeekFrom::Start(64)).await.unwrap();
        block.read_exact(&mut buf[..]).await.unwrap();
        assert_eq!(buf, "B".repeat(64).into_bytes());

        let mut buf = vec![0; 64];
        block.seek(io::SeekFrom::Start(32)).await.unwrap();
        block.read_exact(&mut buf[..]).await.unwrap();
        assert_eq!(buf, ("A".repeat(32) + "B".repeat(32).as_str()).into_bytes());
    }

    #[tokio::test]
    async fn block_512_write_single_sector() {
        let _ = env_logger::builder().is_test(true).try_init();
        let buf = vec![0; 2048];
        let cur = std::io::Cursor::new(buf);
        let mut block: BlockDevice<_, 512, 4> = BlockDevice::new(embedded_io_adapters::tokio_1::FromTokio::new(cur));

        // Test sector aligned access
        let data_a = "A".repeat(512).into_bytes();
        block.seek(io::SeekFrom::Start(0)).await.unwrap();
        block.write_all(&data_a).await.unwrap();
        assert_eq!(&block.into_inner().into_inner().into_inner()[..512], data_a)
    }

    #[tokio::test]
    async fn block_512_write_across_sectors() {
        let _ = env_logger::builder().is_test(true).try_init();
        let buf = vec![0; 2048];
        let cur = std::io::Cursor::new(buf);
        let mut block: BlockDevice<_, 512, 4> = BlockDevice::new(embedded_io_adapters::tokio_1::FromTokio::new(cur));

        // Test sector aligned access
        let data_a = "A".repeat(512).into_bytes();
        block.seek(io::SeekFrom::Start(256)).await.unwrap();
        block.write_all(&data_a).await.unwrap();
        let buf = block.into_inner().into_inner().into_inner();
        assert_eq!(&buf[..256], [0; 256]);
        assert_eq!(&buf[256..768], data_a);
        assert_eq!(&buf[768..1024], [0; 256]);
    }

    #[tokio::test]
    async fn aligned_write_block_optimization() {
        let _ = env_logger::builder().is_test(true).try_init();
        let buf = vec![0; 2048];
        let cur = std::io::Cursor::new(buf);
        let mut block: BlockDevice<_, 512, 4> = BlockDevice::new(embedded_io_adapters::tokio_1::FromTokio::new(cur));

        let mut aligned_buffer: AlignedBuffer<2048, 4> = AlignedBuffer::new();
        let data_a = "A".repeat(512).into_bytes();
        aligned_buffer[..512].copy_from_slice(&data_a[..]);
        block.seek(io::SeekFrom::Start(0)).await.unwrap();
        block.write_all(&aligned_buffer[..]).await.unwrap();

        // if we wrote directly, the block buffer will be empty
        assert_eq!(&block.buffer[..], [0u8; 512]);
        // the write suceeded
        assert_eq!(&block.into_inner().into_inner().into_inner()[..512], &data_a)
    }

    #[tokio::test]
    async fn aligned_read_block_optimization() {
        let _ = env_logger::builder().is_test(true).try_init();
        let buf = "A".repeat(2048).into_bytes();
        let cur = std::io::Cursor::new(buf);
        let mut block: BlockDevice<_, 512, 4> = BlockDevice::new(embedded_io_adapters::tokio_1::FromTokio::new(cur));

        let mut aligned_buffer: AlignedBuffer<512, 4> = AlignedBuffer::new();
        block.seek(io::SeekFrom::Start(0)).await.unwrap();
        block.read_exact(&mut aligned_buffer[..]).await.unwrap();

        // if we read directly, the block buffer will be empty
        assert_eq!(&block.buffer[..], [0u8; 512]);
        // the write suceeded
        assert_eq!(
            &block.into_inner().into_inner().into_inner()[..512],
            &aligned_buffer[..]
        )
    }

    async fn read_to_string<IO: embedded_io::Read>(io: &mut IO) -> Result<String, IO::Error> {
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
