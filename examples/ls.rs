use std::env;

use chrono::{DateTime, Local};
use embedded_fatfs::{FileSystem, FsOptions};

fn format_file_size(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if size < KB {
        format!("{}B", size)
    } else if size < MB {
        format!("{}KB", size / KB)
    } else if size < GB {
        format!("{}MB", size / MB)
    } else {
        format!("{}GB", size / GB)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let file = tokio::fs::File::open("resources/fat32.img").await?;
    let fs = FileSystem::new(file, FsOptions::new()).await?;
    let root_dir = fs.root_dir();
    let dir = match env::args().nth(1) {
        None => root_dir,
        Some(ref path) if path == "." => root_dir,
        Some(ref path) => root_dir.open_dir(&path).await?,
    };
    let mut iter = dir.iter();
    while let Some(r) = iter.next().await {
        let e = r?;
        let modified = DateTime::<Local>::from(e.modified())
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        println!("{:4}  {}  {}", format_file_size(e.len()), modified, e.file_name());
    }
    Ok(())
}
