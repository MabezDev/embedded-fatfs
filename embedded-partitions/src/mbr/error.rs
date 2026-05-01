//! Error type for MBR parsing.

use core::fmt::Debug;

/// Errors that can be returned while parsing or operating on a Master
/// Boot Record.
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error<E> {
    /// The boot signature in the last two bytes of the MBR was not the
    /// expected `0x55AA`.
    InvalidBootSignature {
        /// The actual two bytes that were read.
        found: [u8; 2],
    },
    /// The underlying storage returned EOF before the full 512-byte MBR
    /// could be read.
    DiskTooSmall,
    /// The requested partition index is outside the range `0..4`.
    InvalidPartitionIndex(usize),
    /// The requested partition entry is unused (empty / zero-sized).
    EmptyPartition(usize),
    /// The partition entry refers to a byte range that extends past the
    /// end of the underlying device.
    PartitionOutOfBounds {
        /// Index of the offending partition entry.
        index: usize,
        /// Byte offset of the first byte past the end of the partition.
        end_byte: u64,
        /// Total size, in bytes, of the underlying device.
        disk_size: u64,
    },
    /// I/O error from the underlying storage.
    Io(E),
}

impl<E> From<E> for Error<E> {
    fn from(value: E) -> Self {
        Error::Io(value)
    }
}

impl<E: Debug> core::fmt::Display for Error<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::InvalidBootSignature { found } => write!(
                f,
                "Invalid MBR boot signature: 0x{:02X}{:02X}, expected 0x55AA",
                found[0], found[1]
            ),
            Error::DiskTooSmall => {
                write!(f, "Disk is smaller than 512 bytes; cannot read MBR")
            }
            Error::InvalidPartitionIndex(idx) => {
                write!(f, "Invalid MBR partition index: {idx} (must be < 4)")
            }
            Error::EmptyPartition(idx) => {
                write!(f, "MBR partition entry {idx} is unused / empty")
            }
            Error::PartitionOutOfBounds {
                index,
                end_byte,
                disk_size,
            } => write!(
                f,
                "MBR partition {index} extends to byte {end_byte} but disk is only {disk_size} bytes",
            ),
            Error::Io(e) => write!(f, "I/O error: {e:?}"),
        }
    }
}

impl<E: Debug> core::error::Error for Error<E> {}
