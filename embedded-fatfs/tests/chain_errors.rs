//! A read error while a cluster chain is freed must not leave a directory entry or an open file
//! that still points at clusters that were already freed.

use std::sync::{Arc, Mutex};

use embedded_fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions};
use embedded_io_async::{ErrorKind, ErrorType, Read, Seek, SeekFrom, Write};

/// In-memory volume whose reads fail while they cover the byte at `fail_at`.
struct Storage {
    data: Arc<Mutex<Vec<u8>>>,
    pos: u64,
    fail_at: Arc<Mutex<Option<u64>>>,
}

impl ErrorType for Storage {
    type Error = ErrorKind;
}

impl Read for Storage {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let end = self.pos + buf.len() as u64;
        if let Some(offset) = *self.fail_at.lock().unwrap() {
            if (self.pos..end).contains(&offset) {
                return Err(ErrorKind::Other);
            }
        }
        let data = self.data.lock().unwrap();
        let start = self.pos as usize;
        let len = buf.len().min(data.len().saturating_sub(start));
        buf[..len].copy_from_slice(&data[start..start + len]);
        self.pos += len as u64;
        Ok(len)
    }
}

impl Write for Storage {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let mut data = self.data.lock().unwrap();
        let start = self.pos as usize;
        data[start..start + buf.len()].copy_from_slice(buf);
        self.pos += buf.len() as u64;
        Ok(buf.len())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl Seek for Storage {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        let len = self.data.lock().unwrap().len() as i64;
        self.pos = match pos {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::End(offset) => len + offset,
            SeekFrom::Current(offset) => self.pos as i64 + offset,
        } as u64;
        Ok(self.pos)
    }
}

fn fat32_entry(data: &[u8], fat_start: usize, cluster: u32) -> u32 {
    let offset = fat_start + cluster as usize * 4;
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) & 0x0FFF_FFFF
}

type Fs = FileSystem<Storage, embedded_fatfs::DefaultTimeProvider, embedded_fatfs::LossyOemCpConverter>;

/// A volume holding `LOG.TXT` with a three-cluster chain. A fresh volume puts the root directory
/// in cluster 2 and the file in 3 -> 4 -> 5. Freeing reads each entry before clearing it, so a
/// read failing at the entry of cluster 4 breaks the chain after cluster 3.
struct Volume {
    fs: Fs,
    data: Arc<Mutex<Vec<u8>>>,
    fail_at: Arc<Mutex<Option<u64>>>,
    fat_start: usize,
}

impl Volume {
    async fn new() -> Self {
        let data = Arc::new(Mutex::new(vec![0u8; 40 * 1024 * 1024]));
        let fail_at = Arc::new(Mutex::new(None));
        let mut storage = Storage {
            data: data.clone(),
            pos: 0,
            fail_at: fail_at.clone(),
        };
        embedded_fatfs::format_volume(&mut storage, FormatVolumeOptions::new().fat_type(FatType::Fat32))
            .await
            .unwrap();
        storage.pos = 0;
        let fs = FileSystem::new(storage, FsOptions::new()).await.unwrap();
        assert_eq!(fs.fat_type(), FatType::Fat32);

        let mut file = fs.root_dir().create_file("LOG.TXT").await.unwrap();
        file.write_all(&vec![0x5A; 3 * fs.cluster_size() as usize])
            .await
            .unwrap();
        file.flush().await.unwrap();
        drop(file);

        let fat_start = {
            let data = data.lock().unwrap();
            let bytes_per_sector = u16::from_le_bytes([data[11], data[12]]) as usize;
            let reserved_sectors = u16::from_le_bytes([data[14], data[15]]) as usize;
            bytes_per_sector * reserved_sectors
        };
        let volume = Self {
            fs,
            data,
            fail_at,
            fat_start,
        };
        assert_eq!(volume.entry(3), 4);
        assert_eq!(volume.entry(4), 5);
        volume
    }

    fn entry(&self, cluster: u32) -> u32 {
        fat32_entry(&self.data.lock().unwrap(), self.fat_start, cluster)
    }

    fn fail_reads_of_entry(&self, cluster: Option<u32>) {
        *self.fail_at.lock().unwrap() = cluster.map(|cluster| (self.fat_start + cluster as usize * 4) as u64);
    }
}

#[tokio::test]
async fn remove_with_failing_chain_read_deletes_the_entry_first() {
    let volume = Volume::new().await;
    let root = volume.fs.root_dir();

    volume.fail_reads_of_entry(Some(4));
    assert!(root.remove("LOG.TXT").await.is_err());
    volume.fail_reads_of_entry(None);

    assert!(matches!(
        root.open_file("LOG.TXT").await,
        Err(embedded_fatfs::Error::NotFound)
    ));
    assert_eq!(volume.entry(3), 0);
    assert_eq!(volume.entry(4), 5, "the rest of the chain is leaked, not freed");
}

#[tokio::test]
async fn truncate_with_failing_chain_read_forgets_the_chain() {
    let volume = Volume::new().await;
    let root = volume.fs.root_dir();
    let mut file = root.open_file("LOG.TXT").await.unwrap();

    volume.fail_reads_of_entry(Some(4));
    assert!(file.truncate().await.is_err());
    volume.fail_reads_of_entry(None);
    assert_eq!(volume.entry(3), 0);

    // Writing through the same handle must allocate a new chain, not reuse the freed cluster 3.
    file.write_all(b"hello").await.unwrap();
    file.flush().await.unwrap();
    drop(file);

    let mut file = root.open_file("LOG.TXT").await.unwrap();
    let mut content = Vec::new();
    let mut buf = [0u8; 16];
    loop {
        let read = file.read(&mut buf).await.unwrap();
        if read == 0 {
            break;
        }
        content.extend_from_slice(&buf[..read]);
    }
    assert_eq!(content, b"hello");
}
