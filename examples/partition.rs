// use fatfs::{FileSystem, FsOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    todo!("Unimplemented")
    // let file = tokio::fs::File::open("resources/fat32.img").await?;
    // // Provide sample partition localization. In real application it should be read from MBR/GPT.
    // let first_lba = 0;
    // let last_lba = 10000;
    // // Create partition using provided start address and size in bytes
    // let partition = StreamSlice::new(file, first_lba, last_lba + 1)?;
    // // Create buffered stream to optimize file access
    // let buf_rdr = BufStream::new(partition);
    // // Finally initialize filesystem struct using provided partition
    // let fs = FileSystem::new(buf_rdr, FsOptions::new()).await?;
    // // Read and display volume label
    // println!("Volume Label: {}", fs.volume_label());
    // // other operations...
    // Ok(())
}
