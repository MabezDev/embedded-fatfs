//! Print the partition table of a disk image.
//!
//! Usage: `cargo run --example inspect -- <path-to-image>`

use std::env;

use embedded_io_adapters::tokio_1::FromTokio;
use embedded_partitions::mbr::{Mbr, PartitionType, SECTOR_SIZE};
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

    let mbr = Mbr::new(io).await?;

    println!("Disk signature: {:#010X}", mbr.disk_signature());
    println!("Disk size:      {} bytes", mbr.disk_size());
    println!();
    println!(
        "{:<5} {:<8} {:<22} {:<12} {:<12} {:<12}",
        "#", "Boot", "Type", "Start LBA", "Sectors", "Size"
    );
    println!("{}", "-".repeat(75));

    for (idx, p) in mbr.iter().enumerate() {
        let bootable = if p.is_bootable() { "*" } else { " " };
        let type_str = match p.partition_type() {
            PartitionType::Empty => "<empty>".to_string(),
            other => format!("{other} (0x{:02X})", other.as_u8()),
        };
        let extra = if p.partition_type().is_extended() {
            "  (contains nested logical partitions, not enumerated)"
        } else {
            ""
        };
        println!(
            "{:<5} {:<8} {:<22} {:<12} {:<12} {:<12}{}",
            idx,
            bootable,
            type_str,
            p.start_lba(),
            p.sector_count(),
            format_size(p.size_in_bytes(SECTOR_SIZE)),
            extra,
        );
    }

    Ok(())
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes == 0 {
        "0".to_string()
    } else if bytes < KB {
        format!("{bytes}B")
    } else if bytes < MB {
        format!("{}KB", bytes / KB)
    } else if bytes < GB {
        format!("{}MB", bytes / MB)
    } else {
        format!("{}GB", bytes / GB)
    }
}
