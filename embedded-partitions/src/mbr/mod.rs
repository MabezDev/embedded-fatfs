//! Master Boot Record (MBR) parsing.
//!
//! The MBR is a 512-byte structure stored at the very beginning of a
//! storage device. It contains a boot signature, an optional 4-byte disk
//! signature, and a four-entry primary partition table.
//!
//! # Limitations
//!
//! * Only the four primary partitions are enumerated. Extended partitions
//!   ([`PartitionType::ExtendedChs`] / [`PartitionType::ExtendedLba`])
//!   point to chained Extended Boot Records (EBRs); this crate does not
//!   currently follow that chain. Their `start_lba` / `sector_count`
//!   describe the *container*, not the logical partitions inside it.
//! * MBR partition LBAs are interpreted using 512-byte logical sectors, as
//!   per convention. For non-standard 4Kn drives the byte offset helpers
//!   on [`PartitionEntry`] accept a custom sector size.
//!
//! # Auto-detection
//!
//! When the caller doesn't know in advance whether a device carries a
//! partition table at all, [`Scheme::detect`] / [`Scheme::open`]
//! classify the first 512 bytes as one of [`Layout::Mbr`],
//! [`Layout::Superfloppy`] (a FAT BPB at LBA 0, no partition table)
//! or [`Layout::Unknown`].

mod entry;
mod error;
mod parse;

pub use entry::{
    Chs, PartitionEntry, PartitionType, MBR_SIZE, PARTITION_COUNT, PARTITION_ENTRY_SIZE,
    SECTOR_SIZE,
};
pub use error::Error;
pub use parse::Mbr;

use embedded_io_async::{Read, ReadExactError, Seek, SeekFrom};

use parse::{BOOT_SIGNATURE, BOOT_SIGNATURE_OFFSET, PARTITION_TABLE_OFFSET};

/// Bare classification produced by [`Scheme::detect`].
///
/// See [`Scheme`] for a richer variant that also carries the parsed
/// `Mbr` (or the untouched `IO`) for the `Mbr` / `Superfloppy` /
/// `Unknown` cases respectively.
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Layout {
    /// First sector ends in `0x55AA` and the four partition status
    /// bytes are all `0x00` or `0x80` — a plausible MBR.
    Mbr,
    /// First sector ends in `0x55AA` but the status bytes are not
    /// valid; the sector instead looks like a FAT BPB. The device
    /// has no partition table and is meant to be mounted directly.
    Superfloppy,
    /// First sector matches no recognised layout (no `0x55AA`, or
    /// `0x55AA` with neither MBR-shaped nor FAT-shaped contents).
    Unknown,
}

/// Auto-detected partitioning scheme, paired with the data needed to
/// actually use it.
///
/// Produced by [`Scheme::open`]. Each variant carries either a
/// ready-to-use parser (for `Mbr`) or the original `io` (for
/// `Superfloppy` / `Unknown`), so the caller can route to the right
/// next step without re-reading the first sector.
pub enum Scheme<IO> {
    /// Plain Master Boot Record. Use the contained [`Mbr`] to iterate
    /// or open partitions.
    Mbr(Mbr<IO>),
    /// FAT BPB at LBA 0 (no partition table). Hand the contained
    /// `io` straight to a filesystem layer such as
    /// `embedded_fatfs::FileSystem::new`.
    Superfloppy(IO),
    /// First sector matched no recognised layout. The contained `io`
    /// is untouched (cursor at byte 0).
    Unknown(IO),
}

impl<IO: Read + Seek> Scheme<IO> {
    /// Classify the layout of `io` without consuming it.
    ///
    /// Reads the first 512-byte sector, applies the heuristics
    /// described on [`Layout`], and restores the cursor to byte 0.
    ///
    /// # Errors
    ///
    /// * [`Error::Io`] — underlying storage I/O error.
    /// * [`Error::DiskTooSmall`] — fewer than 512 bytes could be read.
    pub async fn detect(io: &mut IO) -> Result<Layout, Error<IO::Error>> {
        let mut buf = [0u8; MBR_SIZE];
        read_first_sector(io, &mut buf).await?;
        Ok(classify(&buf))
    }

    /// Classify the layout of `io` and take ownership.
    ///
    /// On `Mbr`, the returned [`Scheme::Mbr`] carries a fully parsed
    /// [`Mbr`] ready for [`Mbr::iter`] / [`Mbr::open_partition`] /
    /// [`Mbr::into_partition`]. On `Superfloppy` / `Unknown` the
    /// original `io` is returned unchanged with its cursor at byte 0.
    ///
    /// # Errors
    ///
    /// * [`Error::Io`] — underlying storage I/O error.
    /// * [`Error::DiskTooSmall`] — fewer than 512 bytes could be read.
    pub async fn open(mut io: IO) -> Result<Self, Error<IO::Error>> {
        let mut buf = [0u8; MBR_SIZE];
        let disk_size = read_first_sector_with_size(&mut io, &mut buf).await?;
        Ok(match classify(&buf) {
            Layout::Mbr => Scheme::Mbr(Mbr::from_sector(io, &buf, disk_size)),
            Layout::Superfloppy => Scheme::Superfloppy(io),
            Layout::Unknown => Scheme::Unknown(io),
        })
    }
}

/// Reads the first 512-byte sector of `io` into `buf`, restoring the
/// cursor to byte 0 on both success and error paths.
pub(super) async fn read_first_sector<IO: Read + Seek>(
    io: &mut IO,
    buf: &mut [u8; MBR_SIZE],
) -> Result<(), Error<IO::Error>> {
    io.seek(SeekFrom::Start(0)).await.map_err(Error::Io)?;
    // Seek-back runs unconditionally so a partial read (e.g.
    // `DiskTooSmall`) still leaves the cursor at 0. The read error
    // wins if both fail, since it describes the root cause.
    let read_result = io.read_exact(buf).await.map_err(|e| match e {
        ReadExactError::UnexpectedEof => Error::DiskTooSmall,
        ReadExactError::Other(inner) => Error::Io(inner),
    });
    let seek_result = io.seek(SeekFrom::Start(0)).await.map_err(Error::Io);
    read_result?;
    seek_result?;
    Ok(())
}

/// Same as [`read_first_sector`], but also returns the device size.
pub(super) async fn read_first_sector_with_size<IO: Read + Seek>(
    io: &mut IO,
    buf: &mut [u8; MBR_SIZE],
) -> Result<u64, Error<IO::Error>> {
    let disk_size = io.seek(SeekFrom::End(0)).await.map_err(Error::Io)?;
    read_first_sector(io, buf).await?;
    Ok(disk_size)
}

/// Classify a 512-byte first sector. See [`Layout`] for the rules.
fn classify(buf: &[u8; MBR_SIZE]) -> Layout {
    if buf[BOOT_SIGNATURE_OFFSET..BOOT_SIGNATURE_OFFSET + 2] != BOOT_SIGNATURE {
        return Layout::Unknown;
    }
    if (0..PARTITION_COUNT).all(|i| {
        let s = buf[PARTITION_TABLE_OFFSET + i * PARTITION_ENTRY_SIZE];
        s == 0x00 || s == 0x80
    }) {
        Layout::Mbr
    } else if looks_like_fat_bpb(buf) {
        Layout::Superfloppy
    } else {
        Layout::Unknown
    }
}

/// Best-effort FAT BPB sniff: jump instruction at offset 0, sane
/// `bytes_per_sector` (offset 11), `num_fats` (offset 16) of 1 or 2.
fn looks_like_fat_bpb(s: &[u8; MBR_SIZE]) -> bool {
    let jump_ok = s[0] == 0xEB || s[0] == 0xE9;
    let bps = u16::from_le_bytes([s[11], s[12]]);
    let bps_ok = matches!(bps, 512 | 1024 | 2048 | 4096);
    let nfats_ok = matches!(s[16], 1 | 2);
    jump_ok && bps_ok && nfats_ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_io_adapters::tokio_1::FromTokio;
    use embedded_io_async::ErrorType;

    /// `Read + Seek` wrapper that counts `seek(End(_))` calls.
    struct SeekCounting<IO> {
        inner: IO,
        end_seeks: u32,
    }

    impl<IO> SeekCounting<IO> {
        fn new(inner: IO) -> Self {
            Self {
                inner,
                end_seeks: 0,
            }
        }
    }

    impl<IO: ErrorType> ErrorType for SeekCounting<IO> {
        type Error = IO::Error;
    }

    impl<IO: Read> Read for SeekCounting<IO> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            self.inner.read(buf).await
        }
    }

    impl<IO: Seek> Seek for SeekCounting<IO> {
        async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
            if matches!(pos, SeekFrom::End(_)) {
                self.end_seeks += 1;
            }
            self.inner.seek(pos).await
        }
    }

    /// 1 MiB image carrying a real, valid MBR with two FAT partitions.
    fn make_mbr_disk() -> Vec<u8> {
        let mut disk = vec![0u8; 1024 * 1024];
        // Disk signature.
        disk[440..444].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        // Partition 1: bootable FAT16 at LBA 64, 256 sectors.
        disk[446] = 0x80;
        disk[450] = 0x06;
        disk[454..458].copy_from_slice(&64u32.to_le_bytes());
        disk[458..462].copy_from_slice(&256u32.to_le_bytes());
        // Partition 2: non-bootable FAT32-LBA at LBA 512, 1024 sectors.
        disk[462 + 4] = 0x0C;
        disk[462 + 8..462 + 12].copy_from_slice(&512u32.to_le_bytes());
        disk[462 + 12..462 + 16].copy_from_slice(&1024u32.to_le_bytes());
        // Boot signature.
        disk[510] = 0x55;
        disk[511] = 0xAA;
        disk
    }

    /// 1 MiB image whose first sector is a minimal FAT BPB (no MBR).
    fn make_superfloppy_disk() -> Vec<u8> {
        let mut disk = vec![0u8; 1024 * 1024];
        // Jump instruction.
        disk[0] = 0xEB;
        disk[1] = 0x3C;
        disk[2] = 0x90;
        // OEM ID.
        disk[3..11].copy_from_slice(b"MSDOS5.0");
        // bytes_per_sector = 512, sectors_per_cluster = 1.
        disk[11..13].copy_from_slice(&512u16.to_le_bytes());
        disk[13] = 1;
        // reserved_sectors = 1.
        disk[14..16].copy_from_slice(&1u16.to_le_bytes());
        // num_fats = 2.
        disk[16] = 2;
        // Boot signature shared with MBR.
        disk[510] = 0x55;
        disk[511] = 0xAA;
        // The bytes at offsets 446 etc. are still all-zero, so the MBR
        // status check would *pass* for this image. Plant a non-status
        // byte at offset 446 so the classifier has to fall through to
        // the FAT BPB heuristic.
        disk[446] = 0x42;
        disk
    }

    /// 1 MiB image with `0x55AA` but neither MBR nor FAT shape.
    fn make_unknown_disk() -> Vec<u8> {
        let mut disk = vec![0u8; 1024 * 1024];
        // Bogus byte where a partition status should live.
        disk[446] = 0x42;
        // No FAT-shaped header.
        disk[0] = 0xFA;
        disk[510] = 0x55;
        disk[511] = 0xAA;
        disk
    }

    #[tokio::test]
    async fn detect_mbr() {
        let mut io = FromTokio::new(std::io::Cursor::new(make_mbr_disk()));
        assert_eq!(Scheme::detect(&mut io).await.unwrap(), Layout::Mbr);
    }

    #[tokio::test]
    async fn detect_superfloppy() {
        let mut io = FromTokio::new(std::io::Cursor::new(make_superfloppy_disk()));
        assert_eq!(Scheme::detect(&mut io).await.unwrap(), Layout::Superfloppy);
    }

    #[tokio::test]
    async fn detect_unknown_with_signature() {
        let mut io = FromTokio::new(std::io::Cursor::new(make_unknown_disk()));
        assert_eq!(Scheme::detect(&mut io).await.unwrap(), Layout::Unknown);
    }

    #[tokio::test]
    async fn detect_unknown_no_signature() {
        let mut io = FromTokio::new(std::io::Cursor::new(vec![0u8; 1024 * 1024]));
        assert_eq!(Scheme::detect(&mut io).await.unwrap(), Layout::Unknown);
    }

    #[tokio::test]
    async fn detect_disk_too_small() {
        let mut io = FromTokio::new(std::io::Cursor::new(vec![0u8; 100]));
        let err = Scheme::detect(&mut io)
            .await
            .expect_err("expected DiskTooSmall");
        assert!(matches!(err, Error::DiskTooSmall));
    }

    #[tokio::test]
    async fn detect_does_not_query_disk_size() {
        let mut io = SeekCounting::new(FromTokio::new(std::io::Cursor::new(make_mbr_disk())));
        let _ = Scheme::detect(&mut io).await.unwrap();
        assert_eq!(io.end_seeks, 0);
    }

    #[tokio::test]
    async fn open_still_queries_disk_size() {
        let io = SeekCounting::new(FromTokio::new(std::io::Cursor::new(make_mbr_disk())));
        let scheme = Scheme::open(io).await.unwrap();
        let mbr = match scheme {
            Scheme::Mbr(m) => m,
            _ => panic!("expected Scheme::Mbr"),
        };
        assert_eq!(mbr.disk_size(), 1024 * 1024);
        let io = mbr.into_inner();
        assert_eq!(io.end_seeks, 1);
    }

    #[tokio::test]
    async fn detect_restores_cursor_on_disk_too_small() {
        let mut io = FromTokio::new(std::io::Cursor::new(vec![0u8; 100]));
        let err = Scheme::detect(&mut io)
            .await
            .expect_err("expected DiskTooSmall");
        assert!(matches!(err, Error::DiskTooSmall));
        let pos = io.seek(SeekFrom::Current(0)).await.unwrap();
        assert_eq!(pos, 0);
    }

    #[tokio::test]
    async fn detect_restores_cursor() {
        let mut io = FromTokio::new(std::io::Cursor::new(make_mbr_disk()));
        let _ = Scheme::detect(&mut io).await.unwrap();
        let pos = io.seek(SeekFrom::Current(0)).await.unwrap();
        assert_eq!(pos, 0);
    }

    #[tokio::test]
    async fn open_returns_mbr_handle() {
        let io = FromTokio::new(std::io::Cursor::new(make_mbr_disk()));
        let scheme = Scheme::open(io).await.unwrap();
        let mbr = match scheme {
            Scheme::Mbr(m) => m,
            _ => panic!("expected Scheme::Mbr"),
        };
        assert_eq!(mbr.disk_signature(), 0xDEAD_BEEF);
        assert_eq!(mbr.iter_used().count(), 2);
    }

    #[tokio::test]
    async fn open_returns_superfloppy_io() {
        let io = FromTokio::new(std::io::Cursor::new(make_superfloppy_disk()));
        let scheme = Scheme::open(io).await.unwrap();
        let mut io = match scheme {
            Scheme::Superfloppy(io) => io,
            _ => panic!("expected Scheme::Superfloppy"),
        };
        let pos = io.seek(SeekFrom::Current(0)).await.unwrap();
        assert_eq!(pos, 0);
    }

    #[tokio::test]
    async fn open_returns_unknown_io() {
        let io = FromTokio::new(std::io::Cursor::new(make_unknown_disk()));
        let scheme = Scheme::open(io).await.unwrap();
        let mut io = match scheme {
            Scheme::Unknown(io) => io,
            _ => panic!("expected Scheme::Unknown"),
        };
        let pos = io.seek(SeekFrom::Current(0)).await.unwrap();
        assert_eq!(pos, 0);
    }
}
