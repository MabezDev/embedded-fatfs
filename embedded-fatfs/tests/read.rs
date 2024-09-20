use std::str;

use embedded_fatfs::{ChronoTimeProvider, FatType, FsOptions, LossyOemCpConverter};
use embedded_io_async::{Read, Seek, SeekFrom};

const TEST_TEXT: &str = "Rust is cool!\n";
const FAT12_IMG: &str = "resources/fat12.img";
const FAT16_IMG: &str = "resources/fat16.img";
const FAT32_IMG: &str = "resources/fat32.img";

type FileSystem = embedded_fatfs::FileSystem<
    embedded_io_adapters::tokio_1::FromTokio<tokio::fs::File>,
    ChronoTimeProvider,
    LossyOemCpConverter,
>;

async fn create_fs(name: &str) -> FileSystem {
    let _ = env_logger::builder().is_test(true).try_init();
    let file = tokio::fs::File::open(name).await.unwrap();
    embedded_fatfs::FileSystem::new(file, FsOptions::new()).await.unwrap()
}

async fn test_root_dir(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let entries = root_dir.iter().collect().await;
    let entries = entries.iter().map(|r| r.as_ref().unwrap()).collect::<Vec<_>>();
    let short_names = entries.iter().map(|e| e.short_file_name()).collect::<Vec<String>>();
    assert_eq!(short_names, ["LONG.TXT", "SHORT.TXT", "VERY", "VERY-L~1"]);
    let names = entries.iter().map(|e| e.file_name()).collect::<Vec<String>>();
    assert_eq!(names, ["long.txt", "short.txt", "very", "very-long-dir-name"]);
    // Try read again
    let names2 = root_dir
        .iter()
        .collect()
        .await
        .iter()
        .map(|r| r.as_ref().unwrap().file_name())
        .collect::<Vec<String>>();
    assert_eq!(names2, names);
}

#[tokio::test]
async fn test_root_dir_fat12() {
    test_root_dir(create_fs(FAT12_IMG).await).await
}

#[tokio::test]
async fn test_root_dir_fat16() {
    test_root_dir(create_fs(FAT16_IMG).await).await
}

#[tokio::test]
async fn test_root_dir_fat32() {
    test_root_dir(create_fs(FAT32_IMG).await).await
}

async fn test_read_seek_short_file(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let mut short_file = root_dir.open_file("short.txt").await.unwrap();
    let buf = read_to_end(&mut short_file).await.unwrap();
    assert_eq!(str::from_utf8(&buf).unwrap(), TEST_TEXT);

    assert_eq!(short_file.seek(SeekFrom::Start(5)).await.unwrap(), 5);
    let mut buf2 = [0; 5];
    short_file.read_exact(&mut buf2).await.unwrap();
    assert_eq!(str::from_utf8(&buf2).unwrap(), &TEST_TEXT[5..10]);

    assert_eq!(
        short_file.seek(SeekFrom::Start(1000)).await.unwrap(),
        TEST_TEXT.len() as u64
    );
    let mut buf2 = [0; 5];
    assert_eq!(short_file.read(&mut buf2).await.unwrap(), 0);
}

async fn test_read_seek_context_resume(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let mut short_file = root_dir.open_file("short.txt").await.unwrap();
    let pos = short_file.seek(SeekFrom::End(-5)).await.unwrap();
    let context = short_file.close().await.unwrap();

    let short_file = root_dir.open_meta("short.txt").await.unwrap();
    let mut short_file = short_file.to_file_with_context(context.clone());
    let current = short_file.seek(SeekFrom::Current(0)).await.unwrap();
    assert_eq!(current, pos);

    let content = read_to_end(&mut short_file).await.unwrap();
    assert_eq!(&content[..], TEST_TEXT[pos as usize..].as_bytes());

    // test resuming on wrong file
    let long_file = root_dir.open_meta("long.txt").await.unwrap();
    let r = long_file.try_to_file_with_context(context);
    assert!(r.is_err());
}

#[tokio::test]
async fn test_read_seek_context_resume_fat12() {
    test_read_seek_context_resume(create_fs(FAT12_IMG).await).await
}

#[tokio::test]
async fn test_read_seek_context_resume_fat16() {
    test_read_seek_context_resume(create_fs(FAT16_IMG).await).await
}

#[tokio::test]
async fn test_read_seek_context_resume_fat32() {
    test_read_seek_context_resume(create_fs(FAT32_IMG).await).await
}

#[tokio::test]
async fn test_read_seek_short_file_fat12() {
    test_read_seek_short_file(create_fs(FAT12_IMG).await).await
}

#[tokio::test]
async fn test_read_seek_short_file_fat16() {
    test_read_seek_short_file(create_fs(FAT16_IMG).await).await
}

#[tokio::test]
async fn test_read_seek_short_file_fat32() {
    test_read_seek_short_file(create_fs(FAT32_IMG).await).await
}

async fn test_read_long_file(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let mut long_file = root_dir.open_file("long.txt").await.unwrap();
    let mut buf = read_to_end(&mut long_file).await.unwrap();
    assert_eq!(str::from_utf8(&buf).unwrap(), TEST_TEXT.repeat(1000));

    assert_eq!(long_file.seek(SeekFrom::Start(2017)).await.unwrap(), 2017);
    buf.clear();
    let mut buf2 = [0; 10];
    long_file.read_exact(&mut buf2).await.unwrap();
    assert_eq!(str::from_utf8(&buf2).unwrap(), &TEST_TEXT.repeat(1000)[2017..2027]);
}

#[tokio::test]
async fn test_read_long_file_fat12() {
    test_read_long_file(create_fs(FAT12_IMG).await).await
}

#[tokio::test]
async fn test_read_long_file_fat16() {
    test_read_long_file(create_fs(FAT16_IMG).await).await
}

#[tokio::test]
async fn test_read_long_file_fat32() {
    test_read_long_file(create_fs(FAT32_IMG).await).await
}

async fn test_get_dir_by_path(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let dir = root_dir.open_dir("very/long/path/").await.unwrap();
    let names = dir
        .iter()
        .collect()
        .await
        .iter()
        .map(|r| r.as_ref().unwrap().file_name())
        .collect::<Vec<String>>();
    assert_eq!(names, [".", "..", "test.txt"]);

    let dir2 = root_dir.open_dir("very/long/path/././.").await.unwrap();
    let names2 = dir2
        .iter()
        .collect()
        .await
        .iter()
        .map(|r| r.as_ref().unwrap().file_name())
        .collect::<Vec<String>>();
    assert_eq!(names2, [".", "..", "test.txt"]);

    let root_dir2 = root_dir.open_dir("very/long/path/../../..").await.unwrap();
    let root_names = root_dir2
        .iter()
        .collect()
        .await
        .iter()
        .map(|r| r.as_ref().unwrap().file_name())
        .collect::<Vec<String>>();
    let root_names2 = root_dir
        .iter()
        .collect()
        .await
        .iter()
        .map(|r| r.as_ref().unwrap().file_name())
        .collect::<Vec<String>>();
    assert_eq!(root_names, root_names2);

    root_dir.open_dir("VERY-L~1").await.unwrap();
}

#[tokio::test]
async fn test_get_dir_by_path_fat12() {
    test_get_dir_by_path(create_fs(FAT12_IMG).await).await
}

#[tokio::test]
async fn test_get_dir_by_path_fat16() {
    test_get_dir_by_path(create_fs(FAT16_IMG).await).await
}

#[tokio::test]
async fn test_get_dir_by_path_fat32() {
    test_get_dir_by_path(create_fs(FAT32_IMG).await).await
}

async fn test_get_file_by_path(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let mut file = root_dir.open_file("very/long/path/test.txt").await.unwrap();
    let buf = read_to_end(&mut file).await.unwrap();
    assert_eq!(str::from_utf8(&buf).unwrap(), TEST_TEXT);

    let mut file = root_dir
        .open_file("very-long-dir-name/very-long-file-name.txt")
        .await
        .unwrap();
    let buf = read_to_end(&mut file).await.unwrap();
    assert_eq!(str::from_utf8(&buf).unwrap(), TEST_TEXT);

    root_dir.open_file("VERY-L~1/VERY-L~1.TXT").await.unwrap();

    // try opening dir as file
    assert!(root_dir.open_file("very/long/path").await.is_err());
    // try opening file as dir
    assert!(root_dir.open_dir("very/long/path/test.txt").await.is_err());
    // try using invalid path containing file as non-last component
    assert!(root_dir.open_file("very/long/path/test.txt/abc").await.is_err());
    assert!(root_dir.open_dir("very/long/path/test.txt/abc").await.is_err());
}

#[tokio::test]
async fn test_get_file_by_path_fat12() {
    test_get_file_by_path(create_fs(FAT12_IMG).await).await
}

#[tokio::test]
async fn test_get_file_by_path_fat16() {
    test_get_file_by_path(create_fs(FAT16_IMG).await).await
}

#[tokio::test]
async fn test_get_file_by_path_fat32() {
    test_get_file_by_path(create_fs(FAT32_IMG).await).await
}

async fn test_exists(fs: FileSystem) {
    let root_dir = fs.root_dir();

    // check for existence of existing dir
    assert_eq!(root_dir.exists("very/long/path").await.unwrap(), true);
    assert_eq!(root_dir.file_exists("very/long/path").await.unwrap(), false);
    assert_eq!(root_dir.dir_exists("very/long/path").await.unwrap(), true);
    // check for existence of existing file
    assert_eq!(root_dir.exists("very/long/path/test.txt").await.unwrap(), true);
    assert_eq!(root_dir.file_exists("very/long/path/test.txt").await.unwrap(), true);
    assert_eq!(root_dir.dir_exists("very/long/path/test.txt").await.unwrap(), false);
    // check for existence of non existing file
    assert_eq!(root_dir.exists("very/long/path/missing.txt").await.unwrap(), false);
    assert_eq!(root_dir.file_exists("very/long/path/missing.txt").await.unwrap(), false);
    assert_eq!(root_dir.dir_exists("very/long/path/missing.txt").await.unwrap(), false);
    // check for existence of invalid path
    assert!(root_dir.exists("very/missing/path/test.txt").await.is_err());
    assert!(root_dir.file_exists("very/missing/path/test.txt").await.is_err());
    assert!(root_dir.dir_exists("very/missing/path/test.txt").await.is_err());
    // check for existence of invalid path containing file as non-last component
    assert!(root_dir.exists("very/missing/path/test.txt/abc").await.is_err());
    assert!(root_dir.file_exists("very/missing/path/test.txt/abc").await.is_err());
    assert!(root_dir.dir_exists("very/missing/path/test.txt/abc").await.is_err());
}

#[tokio::test]
async fn test_exists_fat12() {
    test_exists(create_fs(FAT12_IMG).await).await
}

#[tokio::test]
async fn test_exists_fat16() {
    test_exists(create_fs(FAT16_IMG).await).await
}

#[tokio::test]
async fn test_exists_fat32() {
    test_exists(create_fs(FAT32_IMG).await).await
}

async fn test_volume_metadata(fs: FileSystem, fat_type: FatType) {
    assert_eq!(fs.volume_id(), 0x1234_5678);
    assert_eq!(fs.volume_label(), "Test!");
    assert_eq!(&fs.read_volume_label_from_root_dir().await.unwrap().unwrap(), "Test!");
    assert_eq!(fs.fat_type(), fat_type);
}

#[tokio::test]
async fn test_volume_metadata_fat12() {
    test_volume_metadata(create_fs(FAT12_IMG).await, FatType::Fat12).await
}

#[tokio::test]
async fn test_volume_metadata_fat16() {
    test_volume_metadata(create_fs(FAT16_IMG).await, FatType::Fat16).await
}

#[tokio::test]
async fn test_volume_metadata_fat32() {
    test_volume_metadata(create_fs(FAT32_IMG).await, FatType::Fat32).await
}

async fn test_status_flags(fs: FileSystem) {
    let status_flags = fs.read_status_flags().await.unwrap();
    assert_eq!(status_flags.dirty(), false);
    assert_eq!(status_flags.io_error(), false);
}

#[tokio::test]
async fn test_status_flags_fat12() {
    test_status_flags(create_fs(FAT12_IMG).await).await
}

#[tokio::test]
async fn test_status_flags_fat16() {
    test_status_flags(create_fs(FAT16_IMG).await).await
}

#[tokio::test]
async fn test_status_flags_fat32() {
    test_status_flags(create_fs(FAT32_IMG).await).await
}

#[tokio::test]
async fn test_stats_fat12() {
    let fs = create_fs(FAT12_IMG).await;
    let stats = fs.stats().await.unwrap();
    assert_eq!(stats.cluster_size(), 512);
    assert_eq!(stats.total_clusters(), 1955); // 1000 * 1024 / 512 = 2000
    assert_eq!(stats.free_clusters(), 1920);
}

#[tokio::test]
async fn test_stats_fat16() {
    let fs = create_fs(FAT16_IMG).await;
    let stats = fs.stats().await.unwrap();
    assert_eq!(stats.cluster_size(), 512);
    assert_eq!(stats.total_clusters(), 4927); // 2500 * 1024 / 512 = 5000
    assert_eq!(stats.free_clusters(), 4892);
}

#[tokio::test]
async fn test_stats_fat32() {
    let fs = create_fs(FAT32_IMG).await;
    let stats = fs.stats().await.unwrap();
    assert_eq!(stats.cluster_size(), 512);
    assert_eq!(stats.total_clusters(), 66922); // 34000 * 1024 / 512 = 68000
    assert_eq!(stats.free_clusters(), 66886);
}

#[tokio::test]
async fn test_multi_thread() {
    use std::sync::{Arc, Mutex};
    use std::thread;
    let fs = create_fs(FAT32_IMG).await;
    let shared_fs = Arc::new(Mutex::new(fs));
    let mut handles = vec![];
    for _ in 0..2 {
        let shared_fs_cloned = Arc::clone(&shared_fs);
        let handle = thread::spawn(move || {
            let fs2 = shared_fs_cloned.lock().unwrap();
            assert_eq!(fs2.fat_type(), FatType::Fat32);
        });
        handles.push(handle);
    }
    for handle in handles {
        handle.join().unwrap();
    }
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
