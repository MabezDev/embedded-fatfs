//! Auto-detect the layout of a disk image and act on the verdict.
//!
//! Demonstrates both entry points of [`Scheme`]:
//!
//! * [`Scheme::detect`] — classify the layout without consuming the
//!   `IO`. Useful for diagnostics or for routing in code that wants
//!   to keep the handle around.
//! * [`Scheme::open`] — classify and take ownership, returning a
//!   ready-to-use parser handle (or the unmodified `IO` for non-MBR
//!   layouts) in a single read.
//!
//! Run `make_disk_image` first to produce the sample MBR-partitioned
//! image used by the other examples:
//!
//! ```sh
//! cargo run --example make_disk_image
//! cargo run --example detect
//! ```
//!
//! Pointing this at a raw FAT-formatted image (i.e. one with no
//! partition table — a "superfloppy") exercises the
//! [`Scheme::Superfloppy`] branch:
//!
//! ```sh
//! cargo run --example detect -- /path/to/raw-fat.img
//! ```
//!
//! Usage: `cargo run --example detect -- [path-to-image]`

use std::env;

use embedded_fatfs::{FileSystem, FsOptions};
use embedded_io_adapters::tokio_1::FromTokio;
use embedded_io_async::{Read, Seek, Write};
use embedded_partitions::mbr::Scheme;
use tokio::fs::OpenOptions;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "disk.img".to_string());

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .await?;
    let mut io = FromTokio::new(file);

    // Phase 1: classify-only. `detect` borrows `io` and leaves the
    // cursor at byte 0, so we can hand the same handle to `open`
    // afterwards without re-reading the sector externally.
    let layout = Scheme::detect(&mut io).await?;
    println!("{path}: {layout:?}");

    // Phase 2: take ownership and route on the verdict.
    match Scheme::open(io).await? {
        Scheme::Mbr(mut mbr) => {
            println!("  disk signature: {:#010X}", mbr.disk_signature());
            println!("  used entries:   {}", mbr.iter_used().count());
            for (idx, p) in mbr.iter_used() {
                println!(
                    "    [{idx}] {} at LBA {} ({} sectors)",
                    p.partition_type(),
                    p.start_lba(),
                    p.sector_count(),
                );
            }

            // Mount the first FAT partition, if any, and list its root.
            let fat = mbr.iter_used().find(|(_, p)| p.is_fat()).map(|(i, _)| i);
            if let Some(idx) = fat {
                println!("  mounting partition {idx} as FAT...");
                let mut slice = mbr.open_partition(idx).await?;
                list_fat_root(&mut slice).await?;
            } else {
                println!("  no FAT partition to mount.");
            }
        }
        Scheme::Superfloppy(io) => {
            println!("  no partition table; mounting directly as FAT.");
            list_fat_root(io).await?;
        }
        Scheme::Unknown(_) => {
            anyhow::bail!(
                "unrecognised layout — first sector is neither an MBR nor a FAT BPB",
            );
        }
    }

    Ok(())
}

/// Mount `io` as a FAT volume and print its root-directory listing.
async fn list_fat_root<IO>(io: IO) -> anyhow::Result<()>
where
    IO: Read + Write + Seek,
    IO::Error: core::fmt::Debug + Send + Sync + 'static,
{
    let fs = FileSystem::new(io, FsOptions::new()).await?;
    {
        // Scope the directory iterator so its borrow on `fs` ends
        // before we unmount.
        let mut iter = fs.root_dir().iter();
        while let Some(e) = iter.next().await {
            let e = e?;
            println!("    {:>10}  {}", e.len(), e.file_name());
        }
    }
    fs.unmount().await?;
    Ok(())
}
