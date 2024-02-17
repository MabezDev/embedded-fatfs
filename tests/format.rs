use std::io;

use embedded_fatfs::{ChronoTimeProvider, LossyOemCpConverter};
use embedded_io_async::Write;

const KB: u64 = 1024;
const MB: u64 = KB * 1024;
const TEST_STR: &str = "Hi there Rust programmer!\n";

type FileSystem = embedded_fatfs::FileSystem<
    embedded_io_adapters::tokio_1::FromTokio<tokio::io::BufStream<std::io::Cursor<Vec<u8>>>>,
    ChronoTimeProvider,
    LossyOemCpConverter,
>;

async fn basic_fs_test(fs: &FileSystem) {
    let stats = fs.stats().await.expect("stats");
    if fs.fat_type() == embedded_fatfs::FatType::Fat32 {
        // On FAT32 one cluster is allocated for root directory
        assert_eq!(stats.total_clusters(), stats.free_clusters() + 1);
    } else {
        assert_eq!(stats.total_clusters(), stats.free_clusters());
    }

    let root_dir = fs.root_dir();
    let entries = root_dir.iter().collect().await;
    let entries = entries.iter().map(|r| r.as_ref().unwrap()).collect::<Vec<_>>();
    assert_eq!(entries.len(), 0);

    let subdir1 = root_dir.create_dir("subdir1").await.expect("create_dir subdir1");
    let subdir2 = root_dir
        .create_dir("subdir1/subdir2 with long name")
        .await
        .expect("create_dir subdir2");

    let test_str = TEST_STR.repeat(1000);
    {
        let mut file = subdir2.create_file("test file name.txt").await.expect("create file");
        file.truncate().await.expect("truncate file");
        file.write_all(test_str.as_bytes()).await.expect("write file");
        file.flush().await.unwrap(); // important, no more flush on drop :(
    }

    let mut file = root_dir
        .open_file("subdir1/subdir2 with long name/test file name.txt")
        .await
        .unwrap();
    let content = read_to_end(&mut file).await.unwrap();
    assert_eq!(core::str::from_utf8(&content).unwrap(), test_str);

    let filenames = root_dir
        .iter()
        .collect()
        .await
        .iter()
        .map(|r| r.as_ref().unwrap().file_name())
        .collect::<Vec<String>>();
    assert_eq!(filenames, ["subdir1"]);

    let filenames = subdir2
        .iter()
        .collect()
        .await
        .iter()
        .map(|r| r.as_ref().unwrap().file_name())
        .collect::<Vec<String>>();
    assert_eq!(filenames, [".", "..", "test file name.txt"]);

    subdir1
        .rename("subdir2 with long name/test file name.txt", &root_dir, "new-name.txt")
        .await
        .expect("rename");

    let filenames = subdir2
        .iter()
        .collect()
        .await
        .iter()
        .map(|r| r.as_ref().unwrap().file_name())
        .collect::<Vec<String>>();
    assert_eq!(filenames, [".", ".."]);

    let filenames = root_dir
        .iter()
        .collect()
        .await
        .iter()
        .map(|r| r.as_ref().unwrap().file_name())
        .collect::<Vec<String>>();
    assert_eq!(filenames, ["subdir1", "new-name.txt"]);
}

async fn test_format_fs(opts: embedded_fatfs::FormatVolumeOptions, total_bytes: u64) -> FileSystem {
    let _ = env_logger::builder().is_test(true).try_init();
    // Init storage to 0xD1 bytes (value has been choosen to be parsed as normal file)
    let storage_vec: Vec<u8> = vec![0xD1_u8; total_bytes as usize];
    let storage_cur = io::Cursor::new(storage_vec);
    let mut buffered_stream = embedded_io_adapters::tokio_1::FromTokio::new(tokio::io::BufStream::new(storage_cur));
    embedded_fatfs::format_volume(&mut buffered_stream, opts)
        .await
        .expect("format volume");

    let fs = embedded_fatfs::FileSystem::new(buffered_stream, embedded_fatfs::FsOptions::new())
        .await
        .expect("open fs");
    basic_fs_test(&fs).await;
    fs
}

#[tokio::test]
async fn test_format_1mb() {
    let total_bytes = MB;
    let opts = embedded_fatfs::FormatVolumeOptions::new();
    let fs = test_format_fs(opts, total_bytes).await;
    assert_eq!(fs.fat_type(), embedded_fatfs::FatType::Fat12);
}

#[tokio::test]
async fn test_format_8mb_1fat() {
    let total_bytes = 8 * MB;
    let opts = embedded_fatfs::FormatVolumeOptions::new().fats(1);
    let fs = test_format_fs(opts, total_bytes).await;
    assert_eq!(fs.fat_type(), embedded_fatfs::FatType::Fat16);
}

#[tokio::test]
async fn test_format_50mb() {
    let total_bytes = 50 * MB;
    let opts = embedded_fatfs::FormatVolumeOptions::new();
    let fs = test_format_fs(opts, total_bytes).await;
    assert_eq!(fs.fat_type(), embedded_fatfs::FatType::Fat16);
}

#[tokio::test]
async fn test_format_2gb_512sec() {
    let total_bytes = 2 * 1024 * MB;
    let opts = embedded_fatfs::FormatVolumeOptions::new();
    let fs = test_format_fs(opts, total_bytes).await;
    assert_eq!(fs.fat_type(), embedded_fatfs::FatType::Fat32);
}

#[tokio::test]
async fn test_format_1gb_4096sec() {
    let total_bytes = 1024 * MB;
    let opts = embedded_fatfs::FormatVolumeOptions::new().bytes_per_sector(4096);
    let fs = test_format_fs(opts, total_bytes).await;
    assert_eq!(fs.fat_type(), embedded_fatfs::FatType::Fat32);
}

#[tokio::test]
async fn test_format_empty_volume_label() {
    let total_bytes = 2 * 1024 * MB;
    let opts = embedded_fatfs::FormatVolumeOptions::new();
    let fs = test_format_fs(opts, total_bytes).await;
    assert_eq!(fs.volume_label(), "NO NAME");
    assert_eq!(fs.read_volume_label_from_root_dir().await.unwrap(), None);
}

#[tokio::test]
async fn test_format_volume_label_and_id() {
    let total_bytes = 2 * 1024 * MB;
    let opts = embedded_fatfs::FormatVolumeOptions::new()
        .volume_id(1234)
        .volume_label(*b"VOLUMELABEL");
    let fs = test_format_fs(opts, total_bytes).await;
    assert_eq!(fs.volume_label(), "VOLUMELABEL");
    assert_eq!(
        fs.read_volume_label_from_root_dir().await.unwrap(),
        Some("VOLUMELABEL".to_string())
    );
    assert_eq!(fs.volume_id(), 1234);
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
