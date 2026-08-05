//! Building blocks for tests that need a real FAT32 volume to work on.
//!
//! [`image::Fat32Image`] is a FAT32 volume in memory, with accessors that
//! address it by structure — FAT entries, directory entries, cluster contents —
//! rather than by byte offset, so a test says what it means and a failure names
//! the thing that is wrong.
//!
//! [`pristine::build`] produces one known-good volume with a small directory
//! tree on it, and [`pristine::MemDisk`] wraps a `Vec<u8>` as a block device
//! whose buffer stays readable after `unmount` has consumed the filesystem —
//! which is how a test inspects what was actually written to disk.
//!
//! [`verify::check`] is an independent reference checker: it reads a volume and
//! reports what is wrong with it, deriving nothing from the code under test, so
//! a test can assert that an operation left the volume sound without trusting
//! the implementation's own view of soundness.

#![allow(dead_code)]

pub mod image;
pub mod pristine;
pub mod verify;

pub use image::Fat32Image;
