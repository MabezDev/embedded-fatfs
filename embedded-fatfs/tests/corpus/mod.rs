//! Building blocks for tests that need a real FAT32 volume to work on.
//!
//! [`image::Fat32Image`] is a FAT32 volume in memory, with a rudimentary set of accessors into the byte array, to make
//! tests easier to follow than a sea of corresponding byte offsets.
//!
//! [`formatted_fs::format_and_mount`] formats and mounts a volume from a caller-supplied buffer, and
//! [`formatted_fs::build`] uses it to produce one known-good volume with a small directory tree on it.
//! [`formatted_fs::MemDisk`] wraps a `Vec<u8>` as a block device whose buffer stays readable after `unmount` has
//! consumed the filesystem.
//!
//! [`verify::check`] is an independent reference checker: it reads a volume and reports what is wrong with it, deriving
//! nothing from the code under test. If available, it also cross-checks with `fsck.fat` to ensure the image is also
//! understood by other tools.

#![allow(dead_code, unused_imports)]

pub mod formatted_fs;
pub mod image;
pub mod util;
pub mod verify;

pub use image::Fat32Image;
pub use util::*;
