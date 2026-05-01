//! Parse the MBR of a disk image, find the first FAT partition, and list
//! its root directory using `embedded-fatfs`.
//!
//! Demonstrates the recommended `open_partition` + `&mut slice` pattern.
//! After `fs` is dropped the `Mbr` is fully usable again, so additional
//! partitions could be inspected; see `multi_partition.rs` for that flow.
//!
//! Usage: `cargo run --example mount_partition -- <path-to-image>`

use std::env;

use embedded_fatfs::{FileSystem, FsOptions};
use embedded_io_adapters::tokio_1::FromTokio;
use embedded_partitions::mbr::Mbr;
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
    let io = FromTokio::new(file);

    let mut mbr = Mbr::new(io).await?;

    let (idx, entry) = mbr
        .iter_used()
        .find(|(_, p)| p.is_fat())
        .ok_or_else(|| anyhow::anyhow!("no FAT partition found in MBR"))?;

    println!(
        "Mounting partition {idx} ({}) at LBA {} ({} sectors)",
        entry.partition_type(),
        entry.start_lba(),
        entry.sector_count(),
    );

    let mut slice = mbr.open_partition(idx).await?;
    let fs = FileSystem::new(&mut slice, FsOptions::new()).await?;

    {
        // The directory iterator borrows `fs`; scope it so the borrow
        // ends before `unmount` consumes `fs`.
        let root = fs.root_dir();
        let mut iter = root.iter();
        while let Some(r) = iter.next().await {
            let entry = r?;
            println!("{:>10}  {}", entry.len(), entry.file_name());
        }
    }

    fs.unmount().await?;
    Ok(())
}
