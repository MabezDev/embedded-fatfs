use anyhow::Context;
use embedded_fatfs::{FileSystem, FsOptions};
use embedded_io_async::Write;
use tokio::fs::OpenOptions;
use tokio::io::BufStream;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tokio::fs::copy("resources/fat32.img", "tmp/fat.img").await?;
    let img_file = OpenOptions::new()
        .read(true)
        .write(true)
        .open("tmp/fat.img")
        .await
        .context("Failed to open image!")?;
    let buf_stream = BufStream::new(img_file);
    let options = FsOptions::new().update_accessed_date(true);
    let fs = FileSystem::new(buf_stream, options).await?;
    {
        // create a dir
        fs.root_dir().create_dir("foo").await?;
        // Write a file
        let mut file = fs.root_dir().create_file("hello.txt").await?;
        file.write_all(b"Hello World!").await?;
        file.flush().await?;
    }
    fs.unmount().await?;

    Ok(())
}
