//! Types describing a single MBR partition entry.

/// Size of the Master Boot Record sector, in bytes.
pub const MBR_SIZE: usize = 512;

/// Logical sector size for MBR LBAs, in bytes.
///
/// MBR LBAs are 512 bytes by convention, regardless of the device's
/// physical sector size.
pub const SECTOR_SIZE: u64 = 512;

/// Number of primary partition entries in an MBR.
pub const PARTITION_COUNT: usize = 4;

/// Size of a single partition entry, in bytes.
pub const PARTITION_ENTRY_SIZE: usize = 16;

/// Status byte value indicating an active (bootable) partition.
const PARTITION_STATUS_BOOTABLE: u8 = 0x80;

/// Well-known MBR partition type identifiers.
///
/// Only the most common types are listed explicitly. Any other value is
/// preserved as [`PartitionType::Unknown`].
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Copy, Clone, Eq, PartialEq, Debug, Default)]
#[non_exhaustive]
pub enum PartitionType {
    /// `0x00` — entry is unused / empty.
    #[default]
    Empty,
    /// `0x01` — FAT12.
    Fat12,
    /// `0x04` — FAT16, partition smaller than 32 MiB (CHS).
    Fat16Small,
    /// `0x05` — Extended partition (CHS addressing).
    ExtendedChs,
    /// `0x06` — FAT16 (CHS).
    Fat16,
    /// `0x07` — exFAT / NTFS / HPFS.
    ExFatOrNtfs,
    /// `0x0B` — FAT32 (CHS).
    Fat32Chs,
    /// `0x0C` — FAT32 (LBA).
    Fat32Lba,
    /// `0x0E` — FAT16 (LBA).
    Fat16Lba,
    /// `0x0F` — Extended partition (LBA).
    ExtendedLba,
    /// `0x82` — Linux swap.
    LinuxSwap,
    /// `0x83` — Linux native filesystem.
    Linux,
    /// `0xEE` — GPT protective MBR.
    GptProtective,
    /// `0xEF` — EFI System Partition.
    EfiSystem,
    /// Any value not covered by another variant.
    Unknown(u8),
}

impl PartitionType {
    /// Returns the raw 8-bit identifier byte for this partition type.
    pub const fn as_u8(self) -> u8 {
        match self {
            PartitionType::Empty => 0x00,
            PartitionType::Fat12 => 0x01,
            PartitionType::Fat16Small => 0x04,
            PartitionType::ExtendedChs => 0x05,
            PartitionType::Fat16 => 0x06,
            PartitionType::ExFatOrNtfs => 0x07,
            PartitionType::Fat32Chs => 0x0B,
            PartitionType::Fat32Lba => 0x0C,
            PartitionType::Fat16Lba => 0x0E,
            PartitionType::ExtendedLba => 0x0F,
            PartitionType::LinuxSwap => 0x82,
            PartitionType::Linux => 0x83,
            PartitionType::GptProtective => 0xEE,
            PartitionType::EfiSystem => 0xEF,
            PartitionType::Unknown(b) => b,
        }
    }

    /// Static identifier name (e.g. `"Fat16Lba"`, `"Linux"`).
    ///
    /// Returns `"Unknown"` for any unknown variant; for a rendering
    /// that includes the byte value, use the [`Display`](core::fmt::Display)
    /// impl.
    pub const fn name(self) -> &'static str {
        match self {
            PartitionType::Empty => "Empty",
            PartitionType::Fat12 => "Fat12",
            PartitionType::Fat16Small => "Fat16Small",
            PartitionType::ExtendedChs => "ExtendedChs",
            PartitionType::Fat16 => "Fat16",
            PartitionType::ExFatOrNtfs => "ExFatOrNtfs",
            PartitionType::Fat32Chs => "Fat32Chs",
            PartitionType::Fat32Lba => "Fat32Lba",
            PartitionType::Fat16Lba => "Fat16Lba",
            PartitionType::ExtendedLba => "ExtendedLba",
            PartitionType::LinuxSwap => "LinuxSwap",
            PartitionType::Linux => "Linux",
            PartitionType::GptProtective => "GptProtective",
            PartitionType::EfiSystem => "EfiSystem",
            PartitionType::Unknown(_) => "Unknown",
        }
    }

    /// Returns true for any FAT-family variant (FAT12/16/32).
    pub const fn is_fat(self) -> bool {
        matches!(
            self,
            PartitionType::Fat12
                | PartitionType::Fat16Small
                | PartitionType::Fat16
                | PartitionType::Fat16Lba
                | PartitionType::Fat32Chs
                | PartitionType::Fat32Lba
        )
    }

    /// Returns true for the extended-partition variants
    /// ([`ExtendedChs`](Self::ExtendedChs) / [`ExtendedLba`](Self::ExtendedLba)).
    ///
    /// This crate does not walk the EBR chain; logical partitions
    /// inside an extended partition must be enumerated manually.
    pub const fn is_extended(self) -> bool {
        matches!(self, PartitionType::ExtendedChs | PartitionType::ExtendedLba)
    }

    /// Returns true for the [`Empty`](Self::Empty) variant (`0x00`).
    pub const fn is_empty(self) -> bool {
        matches!(self, PartitionType::Empty)
    }
}

impl From<u8> for PartitionType {
    fn from(value: u8) -> Self {
        match value {
            0x00 => PartitionType::Empty,
            0x01 => PartitionType::Fat12,
            0x04 => PartitionType::Fat16Small,
            0x05 => PartitionType::ExtendedChs,
            0x06 => PartitionType::Fat16,
            0x07 => PartitionType::ExFatOrNtfs,
            0x0B => PartitionType::Fat32Chs,
            0x0C => PartitionType::Fat32Lba,
            0x0E => PartitionType::Fat16Lba,
            0x0F => PartitionType::ExtendedLba,
            0x82 => PartitionType::LinuxSwap,
            0x83 => PartitionType::Linux,
            0xEE => PartitionType::GptProtective,
            0xEF => PartitionType::EfiSystem,
            other => PartitionType::Unknown(other),
        }
    }
}

impl From<PartitionType> for u8 {
    fn from(value: PartitionType) -> Self {
        value.as_u8()
    }
}

impl core::fmt::Display for PartitionType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PartitionType::Unknown(b) => write!(f, "Unknown(0x{b:02X})"),
            other => f.write_str(other.name()),
        }
    }
}

/// CHS (Cylinder/Head/Sector) address as packed in an MBR partition entry.
///
/// CHS is a legacy IBM PC convention; prefer LBA on modern disks. The
/// on-disk encoding is:
///
/// ```text
/// byte 0:  H H H H H H H H        -> head    (0..=255)
/// byte 1:  C C s s s s s s        -> sector  = byte1 & 0x3F      (0..=63)
/// byte 2:  c c c c c c c c        -> cylinder = ((byte1 & 0xC0) << 2) | byte2
/// ```
///
/// The top two bits of byte 1 form the high bits of a 10-bit cylinder.
///
/// Valid CHS addresses use sector `1..=63`. This struct stores the raw
/// parsed values, so unused entries (encoded as `[0, 0, 0]`) decode to
/// `Chs { head: 0, sector: 0, cylinder: 0 }`.
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Copy, Clone, Eq, PartialEq, Debug, Default)]
pub struct Chs {
    /// Head number (0..=255).
    pub head: u8,
    /// Sector value extracted from the low six bits of byte 1
    /// (storage range 0..=63; valid CHS addresses use 1..=63).
    pub sector: u8,
    /// Cylinder number (0..=1023), already merged from the high two bits
    /// of byte 1 and all of byte 2.
    pub cylinder: u16,
}

impl Chs {
    /// All-zero CHS, matching the on-disk encoding of unused entries.
    pub const ZERO: Self = Self {
        head: 0,
        sector: 0,
        cylinder: 0,
    };

    /// The "use LBA, ignore CHS" sentinel `(0xFE, 0xFF, 0xFF)`.
    ///
    /// Decodes to head 254, sector 63, cylinder 1023 — the maximum
    /// representable CHS address, conventionally written into modern
    /// LBA-only partition entries.
    pub const LBA_SENTINEL: Self = Self {
        head: 0xFE,
        sector: 0x3F,
        cylinder: 0x3FF,
    };

    /// Parses a 3-byte on-disk CHS triplet.
    pub const fn from_bytes(b: [u8; 3]) -> Self {
        let head = b[0];
        let sector = b[1] & 0x3F;
        let cylinder = ((b[1] & 0xC0) as u16) << 2 | b[2] as u16;
        Self {
            head,
            sector,
            cylinder,
        }
    }

    /// Encodes the CHS address as 3 on-disk bytes.
    ///
    /// `sector > 63` and `cylinder > 1023` are silently masked to fit
    /// the 6-bit and 10-bit on-disk fields.
    pub const fn to_bytes(self) -> [u8; 3] {
        let sector = self.sector & 0x3F;
        let cylinder = self.cylinder & 0x03FF;
        let cyl_low = (cylinder & 0xFF) as u8;
        let cyl_high = ((cylinder >> 8) & 0x03) as u8;
        [self.head, (cyl_high << 6) | sector, cyl_low]
    }
}

/// A single partition entry parsed from a Master Boot Record.
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Copy, Clone, Eq, PartialEq, Debug, Default)]
pub struct PartitionEntry {
    bootable: bool,
    start_chs: Chs,
    partition_type: PartitionType,
    end_chs: Chs,
    start_lba: u32,
    sector_count: u32,
}

impl PartitionEntry {
    /// An all-zero (unused) partition entry.
    pub const ZERO: Self = Self {
        bootable: false,
        start_chs: Chs::ZERO,
        partition_type: PartitionType::Empty,
        end_chs: Chs::ZERO,
        start_lba: 0,
        sector_count: 0,
    };

    /// Constructs a partition entry from its fields.
    ///
    /// Use [`Chs::LBA_SENTINEL`] for `start_chs` / `end_chs` to mark
    /// the entry as LBA-only.
    pub const fn new(
        bootable: bool,
        start_chs: Chs,
        partition_type: PartitionType,
        end_chs: Chs,
        start_lba: u32,
        sector_count: u32,
    ) -> Self {
        Self {
            bootable,
            start_chs,
            partition_type,
            end_chs,
            start_lba,
            sector_count,
        }
    }

    /// Parses a 16-byte on-disk partition entry.
    pub const fn from_bytes(buf: &[u8; PARTITION_ENTRY_SIZE]) -> Self {
        let bootable = buf[0] == PARTITION_STATUS_BOOTABLE;
        let start_chs = Chs::from_bytes([buf[1], buf[2], buf[3]]);
        let partition_type = match buf[4] {
            0x00 => PartitionType::Empty,
            0x01 => PartitionType::Fat12,
            0x04 => PartitionType::Fat16Small,
            0x05 => PartitionType::ExtendedChs,
            0x06 => PartitionType::Fat16,
            0x07 => PartitionType::ExFatOrNtfs,
            0x0B => PartitionType::Fat32Chs,
            0x0C => PartitionType::Fat32Lba,
            0x0E => PartitionType::Fat16Lba,
            0x0F => PartitionType::ExtendedLba,
            0x82 => PartitionType::LinuxSwap,
            0x83 => PartitionType::Linux,
            0xEE => PartitionType::GptProtective,
            0xEF => PartitionType::EfiSystem,
            other => PartitionType::Unknown(other),
        };
        let end_chs = Chs::from_bytes([buf[5], buf[6], buf[7]]);
        let start_lba = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
        let sector_count = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
        Self {
            bootable,
            start_chs,
            partition_type,
            end_chs,
            start_lba,
            sector_count,
        }
    }

    /// Encodes the entry as 16 on-disk bytes.
    ///
    /// Round-trips through [`from_bytes`](Self::from_bytes) for any
    /// entry parsed from real on-disk data. Entries built via
    /// [`new`](Self::new) round-trip iff their CHS values fit the
    /// on-disk bit widths (see [`Chs::to_bytes`]).
    pub const fn to_bytes(&self) -> [u8; PARTITION_ENTRY_SIZE] {
        let start_chs = self.start_chs.to_bytes();
        let end_chs = self.end_chs.to_bytes();
        let lba = self.start_lba.to_le_bytes();
        let count = self.sector_count.to_le_bytes();
        [
            if self.bootable {
                PARTITION_STATUS_BOOTABLE
            } else {
                0x00
            },
            start_chs[0],
            start_chs[1],
            start_chs[2],
            self.partition_type.as_u8(),
            end_chs[0],
            end_chs[1],
            end_chs[2],
            lba[0],
            lba[1],
            lba[2],
            lba[3],
            count[0],
            count[1],
            count[2],
            count[3],
        ]
    }

    /// Returns true if the entry is marked active / bootable.
    pub const fn is_bootable(&self) -> bool {
        self.bootable
    }

    /// Returns the partition type.
    pub const fn partition_type(&self) -> PartitionType {
        self.partition_type
    }

    /// Returns true if the entry is unused: partition type
    /// [`Empty`](PartitionType::Empty) or `sector_count == 0`.
    pub const fn is_unused(&self) -> bool {
        self.partition_type.is_empty() || self.sector_count == 0
    }

    /// Returns true if the entry's type is a FAT-family variant.
    pub const fn is_fat(&self) -> bool {
        self.partition_type.is_fat()
    }

    /// LBA of the first sector (in [`SECTOR_SIZE`]-byte units).
    pub const fn start_lba(&self) -> u32 {
        self.start_lba
    }

    /// Number of sectors covered by the partition.
    pub const fn sector_count(&self) -> u32 {
        self.sector_count
    }

    /// Inclusive LBA of the last sector, or `None` if `sector_count`
    /// is zero or `start_lba + sector_count - 1` would overflow `u32`.
    pub const fn end_lba(&self) -> Option<u32> {
        match self.sector_count {
            0 => None,
            n => self.start_lba.checked_add(n - 1),
        }
    }

    /// First-sector CHS address (legacy).
    pub const fn start_chs(&self) -> Chs {
        self.start_chs
    }

    /// Last-sector CHS address (legacy).
    pub const fn end_chs(&self) -> Chs {
        self.end_chs
    }

    /// Byte offset of the partition's first byte (using `sector_size`).
    pub const fn start_byte_offset(&self, sector_size: u64) -> u64 {
        self.start_lba as u64 * sector_size
    }

    /// Total size in bytes (using `sector_size`).
    pub const fn size_in_bytes(&self, sector_size: u64) -> u64 {
        self.sector_count as u64 * sector_size
    }

    /// Byte offset just past the partition's last byte (using `sector_size`).
    pub const fn end_byte_offset(&self, sector_size: u64) -> u64 {
        self.start_byte_offset(sector_size) + self.size_in_bytes(sector_size)
    }
}
