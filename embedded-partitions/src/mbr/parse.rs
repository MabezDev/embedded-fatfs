//! [`Mbr`] — parses and operates on a Master Boot Record.

use block_device_adapters::{StreamSlice, StreamSliceError};
use embedded_io_async::{Read, Seek, Write};

use crate::mbr::entry::{
    PartitionEntry, MBR_SIZE, PARTITION_COUNT, PARTITION_ENTRY_SIZE, SECTOR_SIZE,
};
use crate::mbr::error::Error;

/// Offset (in bytes) of the first partition entry inside the MBR.
pub(super) const PARTITION_TABLE_OFFSET: usize = 446;

/// Offset (in bytes) of the 4-byte little-endian disk signature.
const DISK_SIGNATURE_OFFSET: usize = 440;

/// Offset (in bytes) of the boot signature `0x55AA`.
pub(super) const BOOT_SIGNATURE_OFFSET: usize = 510;

/// Expected boot signature bytes.
pub(super) const BOOT_SIGNATURE: [u8; 2] = [0x55, 0xAA];

/// A parsed Master Boot Record plus the underlying storage handle.
///
/// The MBR is read from the first 512 bytes of `io`. LBAs are
/// translated to byte offsets using
/// [`SECTOR_SIZE`](crate::mbr::SECTOR_SIZE) (512 bytes per sector, the
/// MBR convention).
///
/// # Validation
///
/// [`Mbr::new`] verifies the `0x55AA` boot signature.
/// [`open_partition`](Self::open_partition) /
/// [`into_partition`](Self::into_partition) bound-check that the
/// partition fits within the device. The table is otherwise accepted
/// as-is: in particular, partitions with `start_lba == 0` (overlapping
/// the MBR) are not rejected — check this yourself if it matters.
///
/// # Choosing how to open a partition
///
/// * [`open_partition`](Self::open_partition) borrows from the `Mbr`.
///   The slice's lifetime is tied to `&mut self`, but the `Mbr` and
///   the underlying device are usable again as soon as the slice is
///   dropped. **Recommended default**, especially for working with
///   multiple partitions.
/// * [`into_partition`](Self::into_partition) consumes the `Mbr` and
///   returns an owned, `'static`-friendly slice — for spawning into a
///   task, returning from a function, or storing in a `'static` field.
///
/// # Example
///
/// ```ignore
/// use embedded_partitions::mbr::Mbr;
/// use embedded_fatfs::{FileSystem, FsOptions};
///
/// let mut mbr = Mbr::new(io).await?;
/// for (index, entry) in mbr.iter_used() {
///     println!("partition {index}: {}", entry.partition_type());
/// }
/// let mut slice = mbr.open_partition(0).await?;
/// let fs = FileSystem::new(&mut slice, FsOptions::new()).await?;
/// ```
pub struct Mbr<IO> {
    io: IO,
    partitions: [PartitionEntry; PARTITION_COUNT],
    disk_signature: u32,
    disk_size: u64,
}

impl<IO> core::fmt::Debug for Mbr<IO> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Mbr")
            .field(
                "disk_signature",
                &format_args!("{:#010X}", self.disk_signature),
            )
            .field("disk_size", &self.disk_size)
            .field("partitions", &self.partitions)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "defmt")]
impl<IO> defmt::Format for Mbr<IO> {
    fn format(&self, fmt: defmt::Formatter) {
        defmt::write!(
            fmt,
            "Mbr {{ disk_signature: {=u32:#010X}, disk_size: {=u64}, partitions: {} }}",
            self.disk_signature,
            self.disk_size,
            self.partitions,
        );
    }
}

impl<IO> Mbr<IO> {
    /// Returns the four primary partition entries.
    ///
    /// Unused entries report [`PartitionEntry::is_unused`] as `true`.
    pub fn partitions(&self) -> &[PartitionEntry; PARTITION_COUNT] {
        &self.partitions
    }

    /// Returns the 4-byte little-endian disk signature at MBR offset 440.
    ///
    /// Pre-Windows-NT MBRs predate this field — those bytes are part
    /// of the bootstrap code, so the returned value is meaningless on
    /// such images. Modern MBRs use a non-zero signature.
    pub const fn disk_signature(&self) -> u32 {
        self.disk_signature
    }

    /// Returns the underlying device size in bytes (queried at parse time).
    pub const fn disk_size(&self) -> u64 {
        self.disk_size
    }

    /// Iterates over all four partition entries, including unused ones.
    ///
    /// Entries are yielded by value, so callers can `.collect()` and
    /// then call [`open_partition`](Self::open_partition) (which needs
    /// `&mut self`) without borrow-checker friction. Use
    /// `.enumerate()` if you also need the entry index.
    pub fn iter(&self) -> impl Iterator<Item = PartitionEntry> + '_ {
        self.partitions.iter().copied()
    }

    /// Iterates `(index, entry)` over partition entries that are in
    /// use. The index is preserved so callers can pass it straight to
    /// [`open_partition`](Self::open_partition) /
    /// [`into_partition`](Self::into_partition).
    pub fn iter_used(&self) -> impl Iterator<Item = (usize, PartitionEntry)> + '_ {
        self.iter().enumerate().filter(|(_, p)| !p.is_unused())
    }

    /// Returns the partition entry at `index`, or `None` if `index >= 4`.
    pub fn partition(&self, index: usize) -> Option<PartitionEntry> {
        self.partitions.get(index).copied()
    }

    /// Consumes the `Mbr` and returns the wrapped storage handle, with
    /// the cursor at offset 0 (as restored by [`Mbr::new`]).
    pub fn into_inner(self) -> IO {
        self.io
    }
}

impl<IO: Read + Seek> Mbr<IO> {
    /// Reads and parses the MBR from the start of `io`.
    ///
    /// On success the storage cursor is left at byte 0.
    ///
    /// # Errors
    ///
    /// * [`Error::Io`] — underlying storage I/O error.
    /// * [`Error::DiskTooSmall`] — fewer than 512 bytes could be read.
    /// * [`Error::InvalidBootSignature`] — the final two bytes were
    ///   not `0x55 0xAA`.
    pub async fn new(mut io: IO) -> Result<Self, Error<IO::Error>> {
        trace!("Mbr::new: querying disk size");
        let mut buf = [0u8; MBR_SIZE];
        let disk_size = super::read_first_sector_with_size(&mut io, &mut buf).await?;

        if buf[BOOT_SIGNATURE_OFFSET..BOOT_SIGNATURE_OFFSET + 2] != BOOT_SIGNATURE {
            warn!(
                "Invalid MBR boot signature: 0x{:02X}{:02X}",
                buf[BOOT_SIGNATURE_OFFSET],
                buf[BOOT_SIGNATURE_OFFSET + 1],
            );
            return Err(Error::InvalidBootSignature {
                found: [buf[BOOT_SIGNATURE_OFFSET], buf[BOOT_SIGNATURE_OFFSET + 1]],
            });
        }

        Ok(Self::from_sector(io, &buf, disk_size))
    }
}

impl<IO> Mbr<IO> {
    /// Constructs an `Mbr` from an already-read 512-byte first sector.
    ///
    /// The caller is responsible for having validated the boot
    /// signature and for leaving the cursor in `io` at byte 0.
    pub(super) fn from_sector(io: IO, buf: &[u8; MBR_SIZE], disk_size: u64) -> Self {
        let disk_signature = u32::from_le_bytes([
            buf[DISK_SIGNATURE_OFFSET],
            buf[DISK_SIGNATURE_OFFSET + 1],
            buf[DISK_SIGNATURE_OFFSET + 2],
            buf[DISK_SIGNATURE_OFFSET + 3],
        ]);

        let mut partitions = [PartitionEntry::ZERO; PARTITION_COUNT];
        for (i, slot) in partitions.iter_mut().enumerate() {
            let off = PARTITION_TABLE_OFFSET + i * PARTITION_ENTRY_SIZE;
            let mut entry = [0u8; PARTITION_ENTRY_SIZE];
            entry.copy_from_slice(&buf[off..off + PARTITION_ENTRY_SIZE]);
            *slot = PartitionEntry::from_bytes(&entry);
            debug!(
                "Mbr partition {}: type=0x{:02X} start_lba={} sectors={} bootable={}",
                i,
                slot.partition_type().as_u8(),
                slot.start_lba(),
                slot.sector_count(),
                slot.is_bootable(),
            );
        }

        Self {
            io,
            partitions,
            disk_signature,
            disk_size,
        }
    }
}

impl<IO: Read + Write + Seek> Mbr<IO> {
    /// Returns a [`StreamSlice`] over partition `index`, borrowing the
    /// underlying I/O.
    ///
    /// The slice keeps `&mut self` alive; once dropped, the `Mbr` is
    /// usable again. To plug the slice into a consumer that takes its
    /// storage by value (such as `embedded_fatfs::FileSystem::new`),
    /// pass `&mut slice` rather than `slice` itself.
    ///
    /// For an owned, `'static`-friendly slice, see
    /// [`into_partition`](Self::into_partition).
    ///
    /// # Errors
    ///
    /// * [`Error::InvalidPartitionIndex`] — `index >= 4`.
    /// * [`Error::EmptyPartition`] — entry at `index` is unused.
    /// * [`Error::PartitionOutOfBounds`] — partition extends past the
    ///   end of the device.
    /// * [`Error::Io`] — underlying storage I/O error.
    pub async fn open_partition(
        &mut self,
        index: usize,
    ) -> Result<StreamSlice<&mut IO>, Error<IO::Error>> {
        let (start, end) = compute_byte_range(&self.partitions, index, self.disk_size)?;
        StreamSlice::new(&mut self.io, start, end)
            .await
            .map_err(map_stream_slice_error)
    }

    /// Consumes the `Mbr` and returns an owned [`StreamSlice`] over
    /// partition `index`.
    ///
    /// Use this when the slice needs to outlive the current scope —
    /// e.g. to spawn it into a task, return it from a function, or
    /// store it in a `'static` field. After this call the `Mbr` is
    /// gone; if you still need the table or other partitions, use
    /// [`open_partition`](Self::open_partition).
    ///
    /// # Errors
    ///
    /// See [`open_partition`](Self::open_partition).
    pub async fn into_partition(self, index: usize) -> Result<StreamSlice<IO>, Error<IO::Error>> {
        let (start, end) = compute_byte_range(&self.partitions, index, self.disk_size)?;
        StreamSlice::new(self.io, start, end)
            .await
            .map_err(map_stream_slice_error)
    }
}

fn compute_byte_range<E>(
    partitions: &[PartitionEntry; PARTITION_COUNT],
    index: usize,
    disk_size: u64,
) -> Result<(u64, u64), Error<E>> {
    let entry = partitions
        .get(index)
        .ok_or(Error::InvalidPartitionIndex(index))?;
    if entry.is_unused() {
        return Err(Error::EmptyPartition(index));
    }
    let start = entry.start_byte_offset(SECTOR_SIZE);
    let end = entry.end_byte_offset(SECTOR_SIZE);
    if end > disk_size {
        return Err(Error::PartitionOutOfBounds {
            index,
            end_byte: end,
            disk_size,
        });
    }
    Ok((start, end))
}

fn map_stream_slice_error<E: core::fmt::Debug>(e: StreamSliceError<E>) -> Error<E> {
    match e {
        StreamSliceError::Other(inner) => Error::Io(inner),
        // `StreamSlice::new` only seeks; only `Other` should reach us.
        // Treat anything else as a bug rather than dropping it.
        _ => panic!("unexpected StreamSlice error during MBR partition open"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mbr::PartitionType;
    use embedded_io_adapters::tokio_1::FromTokio;
    use embedded_io_async::SeekFrom;

    /// Build a 1 MiB synthetic disk image with two partitions in its MBR
    /// table.
    fn make_disk() -> Vec<u8> {
        let mut disk = vec![0u8; 1024 * 1024];

        // Disk signature.
        disk[440..444].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());

        // Partition 1: bootable, FAT16 (0x06), starts at LBA 64, 256 sectors.
        let mut p1 = [0u8; 16];
        p1[0] = 0x80;
        p1[4] = 0x06;
        p1[8..12].copy_from_slice(&64u32.to_le_bytes());
        p1[12..16].copy_from_slice(&256u32.to_le_bytes());
        disk[446..462].copy_from_slice(&p1);

        // Partition 2: non-bootable, FAT32-LBA (0x0C), starts at LBA 512,
        // 1024 sectors.
        let mut p2 = [0u8; 16];
        p2[4] = 0x0C;
        p2[8..12].copy_from_slice(&512u32.to_le_bytes());
        p2[12..16].copy_from_slice(&1024u32.to_le_bytes());
        disk[462..478].copy_from_slice(&p2);

        // Boot signature.
        disk[510] = 0x55;
        disk[511] = 0xAA;

        // Markers inside each partition.
        disk[64 * 512..64 * 512 + 5].copy_from_slice(b"FAT16");
        disk[512 * 512..512 * 512 + 5].copy_from_slice(b"FAT32");

        disk
    }

    #[tokio::test]
    async fn parses_partition_table() {
        let _ = env_logger::builder().is_test(true).try_init();
        let cur = std::io::Cursor::new(make_disk());
        let mbr = Mbr::new(FromTokio::new(cur)).await.unwrap();

        assert_eq!(mbr.disk_signature(), 0xDEAD_BEEF);
        assert_eq!(mbr.disk_size(), 1024 * 1024);

        let parts = mbr.partitions();
        assert!(parts[0].is_bootable());
        assert_eq!(parts[0].partition_type(), PartitionType::Fat16);
        assert_eq!(parts[0].start_lba(), 64);
        assert_eq!(parts[0].sector_count(), 256);
        assert_eq!(parts[0].size_in_bytes(SECTOR_SIZE), 256 * 512);
        assert_eq!(parts[0].end_lba(), Some(64 + 256 - 1));
        assert!(parts[0].is_fat());

        assert!(!parts[1].is_bootable());
        assert_eq!(parts[1].partition_type(), PartitionType::Fat32Lba);
        assert!(parts[1].is_fat());

        assert!(parts[2].is_unused());
        assert!(parts[3].is_unused());

        let used: Vec<_> = mbr.iter_used().map(|(i, _)| i).collect();
        assert_eq!(used, vec![0, 1]);
    }

    #[tokio::test]
    async fn type_inference_does_not_require_turbofish() {
        // Smoke test: this used to fail before dropping the const generic.
        let cur = std::io::Cursor::new(make_disk());
        let _mbr = Mbr::new(FromTokio::new(cur)).await.unwrap();
    }

    #[tokio::test]
    async fn restores_io_position_to_zero() {
        let cur = std::io::Cursor::new(make_disk());
        let mbr = Mbr::new(FromTokio::new(cur)).await.unwrap();
        let mut io = mbr.into_inner();
        // `seek(Current(0))` returns the absolute position; assert
        // directly that the cursor is at 0.
        let pos = io.seek(SeekFrom::Current(0)).await.unwrap();
        assert_eq!(pos, 0);
    }

    #[tokio::test]
    async fn rejects_disk_too_small() {
        let _ = env_logger::builder().is_test(true).try_init();
        let cur = std::io::Cursor::new(vec![0u8; 100]);
        let err = Mbr::new(FromTokio::new(cur))
            .await
            .expect_err("expected DiskTooSmall");
        assert!(matches!(err, Error::DiskTooSmall));
    }

    #[tokio::test]
    async fn rejects_invalid_signature() {
        let _ = env_logger::builder().is_test(true).try_init();
        let mut disk = make_disk();
        disk[510] = 0x00;
        let cur = std::io::Cursor::new(disk);
        let err = Mbr::new(FromTokio::new(cur))
            .await
            .expect_err("expected InvalidBootSignature error");
        assert!(matches!(
            err,
            Error::InvalidBootSignature {
                found: [0x00, 0xAA]
            }
        ));
    }

    #[tokio::test]
    async fn into_partition_returns_correct_slice() {
        let _ = env_logger::builder().is_test(true).try_init();
        let cur = std::io::Cursor::new(make_disk());
        let mbr = Mbr::new(FromTokio::new(cur)).await.unwrap();

        let mut slice = mbr.into_partition(1).await.unwrap();
        slice.seek(SeekFrom::Start(0)).await.unwrap();
        let mut buf = [0u8; 5];
        slice.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"FAT32");
    }

    #[tokio::test]
    async fn open_partition_borrowing_round_trip() {
        let _ = env_logger::builder().is_test(true).try_init();
        let cur = std::io::Cursor::new(make_disk());
        let mut mbr = Mbr::new(FromTokio::new(cur)).await.unwrap();

        {
            let mut slice = mbr.open_partition(0).await.unwrap();
            slice.seek(SeekFrom::Start(0)).await.unwrap();
            let mut buf = [0u8; 5];
            slice.read_exact(&mut buf).await.unwrap();
            assert_eq!(&buf, b"FAT16");
        }
        let mut slice = mbr.open_partition(1).await.unwrap();
        slice.seek(SeekFrom::Start(0)).await.unwrap();
        let mut buf = [0u8; 5];
        slice.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"FAT32");
    }

    #[tokio::test]
    async fn rejects_invalid_indexes() {
        let _ = env_logger::builder().is_test(true).try_init();
        let cur = std::io::Cursor::new(make_disk());
        let mut mbr = Mbr::new(FromTokio::new(cur)).await.unwrap();

        assert!(matches!(
            mbr.open_partition(2).await.err().unwrap(),
            Error::EmptyPartition(2)
        ));
        assert!(matches!(
            mbr.open_partition(99).await.err().unwrap(),
            Error::InvalidPartitionIndex(99)
        ));
    }

    #[tokio::test]
    async fn rejects_partition_out_of_bounds() {
        let _ = env_logger::builder().is_test(true).try_init();
        let mut disk = make_disk();
        // Rewrite partition 0 to claim 1 GiB worth of sectors on a 1 MiB
        // disk: 2_097_152 sectors * 512 = 1 GiB.
        disk[446 + 12..446 + 16].copy_from_slice(&2_097_152u32.to_le_bytes());

        let cur = std::io::Cursor::new(disk);
        let mut mbr = Mbr::new(FromTokio::new(cur)).await.unwrap();

        let err = mbr.open_partition(0).await.err().unwrap();
        assert!(matches!(err, Error::PartitionOutOfBounds { index: 0, .. }));
    }

    #[test]
    fn chs_decodes_lba_sentinel() {
        use crate::mbr::Chs;
        let chs = Chs::from_bytes([0xFE, 0xFF, 0xFF]);
        assert_eq!(chs, Chs::LBA_SENTINEL);
        assert_eq!(chs.head, 0xFE);
        assert_eq!(chs.sector, 63);
        assert_eq!(chs.cylinder, 1023);
    }

    #[test]
    fn chs_round_trip() {
        use crate::mbr::Chs;
        for chs in [
            Chs::ZERO,
            Chs::LBA_SENTINEL,
            Chs {
                head: 12,
                sector: 34,
                cylinder: 567,
            },
        ] {
            assert_eq!(Chs::from_bytes(chs.to_bytes()), chs);
        }
    }

    #[test]
    fn partition_entry_round_trip() {
        use crate::mbr::Chs;
        let original = PartitionEntry::new(
            true,
            Chs::LBA_SENTINEL,
            PartitionType::Fat32Lba,
            Chs::LBA_SENTINEL,
            2048,
            16384,
        );
        let parsed = PartitionEntry::from_bytes(&original.to_bytes());
        assert_eq!(original, parsed);
    }

    #[test]
    fn end_lba_handles_overflow() {
        use crate::mbr::Chs;
        let entry = PartitionEntry::new(
            false,
            Chs::ZERO,
            PartitionType::Linux,
            Chs::ZERO,
            u32::MAX,
            2,
        );
        // start_lba + (sector_count - 1) = u32::MAX + 1 -> overflow
        assert_eq!(entry.end_lba(), None);
    }

    #[test]
    fn partition_type_display_uses_hex_for_unknown() {
        use crate::mbr::PartitionType;
        let s = std::format!("{}", PartitionType::Unknown(0xAB));
        assert_eq!(s, "Unknown(0xAB)");
        assert_eq!(std::format!("{}", PartitionType::Fat16Lba), "Fat16Lba");
    }
}
