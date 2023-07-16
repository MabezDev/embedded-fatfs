use anyhow::Context;
use embedded_io::Write;
use fatfs::{FileSystem, FsOptions};
use tokio::fs::OpenOptions;
use tokio::io::BufStream;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let img_file = OpenOptions::new().read(true).write(true).open("fat.img").await.context("Failed to open image!")?;
    let buf_stream = BufStream::new(img_file);
    let options = FsOptions::new().update_accessed_date(true);
    let fs = FileSystem::new(buf_stream, options).await?;
    let mut file = fs.root_dir().create_file("hello.txt").await?;
    file.write_all(b"Hello World!").await?;
    Ok(())
}
