//! Walk every used partition. FAT partitions are mounted transiently
//! with `embedded-fatfs` and their root directory is listed; other
//! partitions get a 32-byte raw peek.
//!
//! Demonstrates the borrowing `open_partition` flow that
//! `into_partition` can't replicate: each `slice` is dropped before
//! the next iteration, so `mbr` stays usable across the loop.
//!
//! Usage: `cargo run --example multi_partition -- <path-to-image>`

use std::env;

use embedded_fatfs::{FileSystem, FsOptions};
use embedded_io_adapters::tokio_1::FromTokio;
use embedded_io_async::Read;
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

    // Snapshot used partitions up-front: the iterator yields by value,
    // so this Vec doesn't keep `mbr` borrowed, leaving `open_partition`
    // free to take `&mut self` inside the loop.
    let used: Vec<(usize, _)> = mbr.iter_used().collect();

    for (idx, entry) in used {
        let kind = entry.partition_type();
        println!("--- partition {idx} ({kind}) ---");

        let mut slice = mbr.open_partition(idx).await?;

        if kind.is_fat() {
            let fs = FileSystem::new(&mut slice, FsOptions::new()).await?;
            {
                let mut iter = fs.root_dir().iter();
                while let Some(e) = iter.next().await {
                    let e = e?;
                    println!("  {:>10}  {}", e.len(), e.file_name());
                }
            }
            fs.unmount().await?;
        } else {
            let mut buf = [0u8; 32];
            let n = slice.read(&mut buf).await?;
            println!("  first {n} bytes: {:02X?}", &buf[..n]);
        }
    }

    Ok(())
}
