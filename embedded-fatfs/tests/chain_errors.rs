//! A read error while a cluster chain is freed must not leave a directory entry that still points
//! at clusters that were already freed.

use std::sync::{Arc, Mutex};

use embedded_fatfs::{FatType, FormatVolumeOptions, FsOptions};
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

#[tokio::test]
async fn remove_with_failing_chain_read_deletes_the_entry_first() {
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
    let fs = embedded_fatfs::FileSystem::new(storage, FsOptions::new())
        .await
        .unwrap();
    assert_eq!(fs.fat_type(), FatType::Fat32);

    let root = fs.root_dir();
    let mut file = root.create_file("LOG.TXT").await.unwrap();
    file.write_all(&vec![0x5A; 3 * fs.cluster_size() as usize])
        .await
        .unwrap();
    file.flush().await.unwrap();
    drop(file);

    // A fresh volume puts the root directory in cluster 2 and the file in 3 -> 4 -> 5.
    let fat_start = {
        let data = data.lock().unwrap();
        let bytes_per_sector = u16::from_le_bytes([data[11], data[12]]) as usize;
        let reserved_sectors = u16::from_le_bytes([data[14], data[15]]) as usize;
        let fat_start = bytes_per_sector * reserved_sectors;
        assert_eq!(fat32_entry(&data, fat_start, 3), 4);
        assert_eq!(fat32_entry(&data, fat_start, 4), 5);
        fat_start
    };

    // Freeing reads each entry before clearing it, so the chain breaks after cluster 3.
    *fail_at.lock().unwrap() = Some((fat_start + 4 * 4) as u64);
    assert!(root.remove("LOG.TXT").await.is_err());
    *fail_at.lock().unwrap() = None;

    assert!(matches!(
        root.open_file("LOG.TXT").await,
        Err(embedded_fatfs::Error::NotFound)
    ));
    let data = data.lock().unwrap();
    assert_eq!(fat32_entry(&data, fat_start, 3), 0);
    assert_eq!(
        fat32_entry(&data, fat_start, 4),
        5,
        "the rest of the chain is leaked, not freed"
    );
}
