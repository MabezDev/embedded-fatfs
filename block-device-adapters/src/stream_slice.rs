use core::cmp;
use core::fmt::Debug;
use embedded_io_async::{Read, Seek, SeekFrom, Write};

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug)]
#[non_exhaustive]
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
            StreamSliceError::Other(_) | StreamSliceError::WriteZero => {
                embedded_io_async::ErrorKind::Other
            }
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
    pub async fn new(
        mut inner: T,
        start_offset: u64,
        end_offset: u64,
    ) -> Result<Self, StreamSliceError<T::Error>> {
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

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn stream_test() {
        let _ = env_logger::builder().is_test(true).try_init();
        let buf = "BeforeTest dataAfter".to_string().into_bytes();
        let cur = std::io::Cursor::new(buf);
        let mut stream =
            StreamSlice::new(embedded_io_adapters::tokio_1::FromTokio::new(cur), 6, 6 + 9)
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
