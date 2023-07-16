use std::env;

use embedded_fatfs::{format_volume, FormatVolumeOptions};
use embedded_io_adapters::tokio_1::FromTokio;
use tokio::fs;
use tokio::io::BufStream;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let filename = env::args().nth(1).expect("image path expected");
    let file = fs::OpenOptions::new().read(true).write(true).open(&filename).await?;
    let buf_file = BufStream::new(file);
    format_volume(&mut FromTokio::new(buf_file), FormatVolumeOptions::new()).await?;
    Ok(())
}
