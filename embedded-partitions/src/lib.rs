//! Partition-table parsing for `no_std` / embedded targets.
//!
//! This crate parses on-disk partition tables from any storage that
//! implements the [`embedded_io_async`] traits. Currently supported
//! schemes:
//!
//! * Master Boot Record (MBR) — see the [`mbr`] module.
//!
//! Partitions are exposed as [`block_device_adapters::StreamSlice`]
//! views over the underlying read/write/seek storage, ready to be
//! handed to a filesystem layer such as `embedded-fatfs`.
//!
//! # Example
//!
//! ```ignore
//! use embedded_partitions::mbr::Mbr;
//! use embedded_fatfs::{FileSystem, FsOptions};
//!
//! // `io` implements embedded_io_async::{Read, Write, Seek}
//! let mut mbr = Mbr::new(io).await?;
//!
//! for (idx, p) in mbr.iter_used() {
//!     println!(
//!         "{idx}: {} at LBA {} ({} sectors)",
//!         p.partition_type(),
//!         p.start_lba(),
//!         p.sector_count(),
//!     );
//! }
//!
//! let idx = mbr
//!     .iter_used()
//!     .find(|(_, p)| p.is_fat())
//!     .map(|(i, _)| i)
//!     .expect("no FAT partition");
//!
//! // Borrow `mbr` for the partition; pass `&mut slice` so the file-
//! // system layer doesn't take ownership and `mbr` stays usable for
//! // other partitions. For an owned, `'static`-friendly slice (e.g.
//! // to spawn into a task) use `mbr.into_partition(idx)`.
//! let mut slice = mbr.open_partition(idx).await?;
//! let fs = FileSystem::new(&mut slice, FsOptions::new()).await?;
//! ```
//!
//! See the [`mbr`] module for full documentation and the `examples/`
//! directory for runnable programs.

#![cfg_attr(not(test), no_std)]
#![warn(missing_docs)]
#![allow(async_fn_in_trait)]

// MUST be the first module listed
mod fmt;

pub mod mbr;
