//! Builds the known-good FAT32 volume that every corruption case starts from.
//!
//! The volume is built with `embedded_fatfs` itself, so it is exactly what an
//! embedded device would have written, and it is built deterministically
//! (`NullTimeProvider`, fixed volume id, fixed order of operations) so that
//! byte-for-byte comparison against a repaired image is meaningful.

use std::cell::RefCell;
use std::rc::Rc;

use embedded_fatfs::{FormatVolumeOptions, FsOptions, LossyOemCpConverter, NullTimeProvider};
use embedded_io_async::{ErrorType, Read, Seek, SeekFrom, Write};

use super::image::Fat32Image;

/// Sector size of every image in the corpus.
pub const BYTES_PER_SECTOR: u16 = 512;
/// One sector per cluster keeps the images small while still giving multi-cluster
/// files: a "3 cluster" file is only 1536 bytes.
pub const BYTES_PER_CLUSTER: u32 = 512;
/// 40 MiB is the smallest round size that still yields the >= 65525 clusters a
/// FAT32 volume requires at this cluster size.
pub const TOTAL_BYTES: u64 = 40 * 1024 * 1024;

/// Contents of the files in the pristine volume: (path, length in bytes).
///
/// Sizes are chosen so the corpus has an exactly-one-cluster file, a
/// multi-cluster file, a file with a partly-used last cluster, and an empty file.
pub const FILES: &[(&str, usize)] = &[
    ("BOOT.BIN", 1536),                // 3 clusters, exactly full
    ("LOG.TXT", 512),                  // 1 cluster, exactly full
    ("EMPTY.TXT", 0),                  // no cluster at all
    ("DATA/READINGS.CSV", 1000),       // 2 clusters, last one partly used
    ("DATA/sensor readings.txt", 200), // 1 cluster, has LFN entries
];

/// Directories in the pristine volume, in creation order.
pub const DIRS: &[&str] = &["DATA", "DATA/SUB"];

/// Deterministic filler so a cluster mix-up shows up as wrong data rather than
/// as more zeros.
pub fn file_content(path: &str, len: usize) -> Vec<u8> {
    let mut state = path.bytes().fold(0x1234_5678_u32, |s, b| {
        s.rotate_left(5) ^ u32::from(b).wrapping_mul(0x9E37_79B1)
    });
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect()
}

/// Build the pristine image. Deterministic: two calls return identical bytes.
pub async fn build() -> Fat32Image {
    let disk = MemDisk::new(TOTAL_BYTES as usize);
    let buffer = disk.buffer();

    let mut fmt_disk = disk.clone();
    embedded_fatfs::format_volume(
        &mut fmt_disk,
        FormatVolumeOptions::new()
            .fat_type(embedded_fatfs::FatType::Fat32)
            .bytes_per_sector(BYTES_PER_SECTOR)
            .bytes_per_cluster(BYTES_PER_CLUSTER)
            .fats(2)
            .volume_id(0x1234_5678)
            .volume_label(*b"FSCKCORPUS "),
    )
    .await
    .expect("format volume");

    let options = FsOptions::new()
        .time_provider(NullTimeProvider::new())
        .oem_cp_converter(LossyOemCpConverter::new());
    let fs = embedded_fatfs::FileSystem::new(disk, options).await.expect("mount");

    {
        let root = fs.root_dir();
        for dir in DIRS {
            root.create_dir(dir).await.expect("create_dir");
        }
        for &(path, len) in FILES {
            let mut file = root.create_file(path).await.expect("create_file");
            file.truncate().await.expect("truncate");
            if len > 0 {
                file.write_all(&file_content(path, len)).await.expect("write");
            }
            file.flush().await.expect("flush file");
        }
        // Force the free-cluster count to be computed and written, so FSInfo in
        // the pristine image is correct rather than merely plausible.
        fs.stats().await.expect("stats");
    }
    fs.unmount().await.expect("unmount");

    let data = buffer.borrow().clone();
    Fat32Image::parse(data)
}

/// An in-memory block device. Unlike a `Cursor`, the buffer is shared, so it can
/// still be read after `FileSystem::unmount` consumes the device.
#[derive(Clone)]
pub struct MemDisk {
    buffer: Rc<RefCell<Vec<u8>>>,
    pos: u64,
}

impl MemDisk {
    pub fn new(size: usize) -> Self {
        Self::from_bytes(vec![0; size])
    }

    pub fn from_bytes(data: Vec<u8>) -> Self {
        Self {
            buffer: Rc::new(RefCell::new(data)),
            pos: 0,
        }
    }

    pub fn buffer(&self) -> Rc<RefCell<Vec<u8>>> {
        Rc::clone(&self.buffer)
    }
}

impl ErrorType for MemDisk {
    type Error = embedded_io_async::ErrorKind;
}

impl Read for MemDisk {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let data = self.buffer.borrow();
        let pos = usize::try_from(self.pos).unwrap().min(data.len());
        let n = buf.len().min(data.len() - pos);
        buf[..n].copy_from_slice(&data[pos..pos + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Write for MemDisk {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let mut data = self.buffer.borrow_mut();
        let pos = usize::try_from(self.pos).unwrap();
        if pos + buf.len() > data.len() {
            data.resize(pos + buf.len(), 0);
        }
        data[pos..pos + buf.len()].copy_from_slice(buf);
        self.pos += buf.len() as u64;
        Ok(buf.len())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl Seek for MemDisk {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        let len = self.buffer.borrow().len() as i64;
        let new = match pos {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::End(n) => len + n,
            SeekFrom::Current(n) => self.pos as i64 + n,
        };
        if new < 0 {
            return Err(embedded_io_async::ErrorKind::InvalidInput);
        }
        self.pos = new as u64;
        Ok(self.pos)
    }
}
