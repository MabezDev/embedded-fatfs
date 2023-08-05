use std::env;

use embedded_fatfs::{FileSystem, FsOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let file = tokio::fs::File::open("resources/fat32.img").await?;
    let fs = FileSystem::new(file, FsOptions::new()).await?;
    let root_dir = fs.root_dir();
    let mut file = root_dir
        .open_file(&env::args().nth(1).expect("filename expected"))
        .await?;
    let buf = read_to_end(&mut file).await?;
    print!("{}", String::from_utf8_lossy(&buf));
    Ok(())
}

async fn read_to_end<IO: embedded_io_async::Read>(io: &mut IO) -> Result<Vec<u8>, IO::Error> {
    let mut buf = Vec::new();
    loop {
        let mut tmp = [0; 256];
        match io.read(&mut tmp).await {
            Ok(0) => break,
            Ok(n) => buf.extend(&tmp[..n]),
            Err(e) => return Err(e),
        }
    }

    Ok(buf)
}
