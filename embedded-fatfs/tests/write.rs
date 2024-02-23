use std::future::Future;
use std::str;
use tokio::fs;

use embedded_fatfs::{ChronoTimeProvider, FsOptions, LossyOemCpConverter};
use embedded_io_async::{Seek, SeekFrom, Write};

const FAT12_IMG: &str = "fat12.img";
const FAT16_IMG: &str = "fat16.img";
const FAT32_IMG: &str = "fat32.img";
const IMG_DIR: &str = "resources";
const TMP_DIR: &str = "tmp";
const TEST_STR: &str = "Hi there Rust programmer!\n";
const TEST_STR2: &str = "Rust is cool!\n";

type FileSystem = embedded_fatfs::FileSystem<
    embedded_io_adapters::tokio_1::FromTokio<tokio::fs::File>,
    ChronoTimeProvider,
    LossyOemCpConverter,
>;

async fn call_with_tmp_img<Fut: Future, F: Fn(String) -> Fut>(f: F, filename: &str, test_seq: u32) {
    let _ = env_logger::builder().is_test(true).try_init();
    let img_path = format!("{}/{}", IMG_DIR, filename);
    let tmp_path = format!("{}/{}-{}", TMP_DIR, test_seq, filename);
    fs::create_dir(TMP_DIR).await.ok();
    fs::copy(&img_path, &tmp_path).await.unwrap();
    f(tmp_path.clone()).await;
    fs::remove_file(tmp_path).await.unwrap();
}

async fn open_filesystem_rw(tmp_path: String) -> FileSystem {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&tmp_path)
        .await
        .unwrap();
    let options = FsOptions::new().update_accessed_date(true);
    FileSystem::new(file, options).await.unwrap()
}

async fn call_with_fs<Fut: Future, F: Fn(FileSystem) -> Fut>(f: F, filename: &str, test_seq: u32) {
    let callback = |tmp_path: String| async {
        let fs = open_filesystem_rw(tmp_path).await;
        f(fs).await;
    };
    call_with_tmp_img(&callback, filename, test_seq).await;
}

async fn test_write_short_file(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let mut file = root_dir.open_file("short.txt").await.expect("open file");
    file.truncate().await.unwrap();
    file.write_all(&TEST_STR.as_bytes()).await.unwrap();
    file.flush().await.unwrap();
    file.seek(SeekFrom::Start(0)).await.unwrap();
    let buf = read_to_end(&mut file).await.unwrap();
    file.flush().await.unwrap(); // update access time
    assert_eq!(TEST_STR, str::from_utf8(&buf).unwrap());
}

#[tokio::test]
async fn test_write_file_fat12() {
    call_with_fs(test_write_short_file, FAT12_IMG, 1).await
}

#[tokio::test]
async fn test_write_file_fat16() {
    call_with_fs(test_write_short_file, FAT16_IMG, 1).await
}

#[tokio::test]
async fn test_write_file_fat32() {
    call_with_fs(test_write_short_file, FAT32_IMG, 1).await
}

async fn test_write_long_file(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let mut file = root_dir.open_file("long.txt").await.expect("open file");
    file.truncate().await.unwrap();
    let test_str = TEST_STR.repeat(1000);
    file.write_all(&test_str.as_bytes()).await.unwrap();
    file.seek(SeekFrom::Start(0)).await.unwrap();
    file.flush().await.unwrap();
    let mut buf = read_to_end(&mut file).await.unwrap();
    assert_eq!(test_str, str::from_utf8(&buf).unwrap());
    file.seek(SeekFrom::Start(1234)).await.unwrap();
    file.truncate().await.unwrap();
    file.seek(SeekFrom::Start(0)).await.unwrap();
    buf.clear();
    buf = read_to_end(&mut file).await.unwrap();
    file.flush().await.unwrap(); // update access time
    assert_eq!(&test_str[..1234], str::from_utf8(&buf).unwrap());
}

#[tokio::test]
async fn test_write_long_file_fat12() {
    call_with_fs(test_write_long_file, FAT12_IMG, 2).await
}

#[tokio::test]
async fn test_write_long_file_fat16() {
    call_with_fs(test_write_long_file, FAT16_IMG, 2).await
}

#[tokio::test]
async fn test_write_long_file_fat32() {
    call_with_fs(test_write_long_file, FAT32_IMG, 2).await
}

async fn test_remove(fs: FileSystem) {
    let root_dir = fs.root_dir();
    assert!(root_dir.remove("very/long/path").await.is_err());
    let dir = root_dir.open_dir("very/long/path").await.unwrap();
    let mut names = dir
        .iter()
        .collect()
        .await
        .iter()
        .map(|r| r.as_ref().unwrap().file_name())
        .collect::<Vec<String>>();
    assert_eq!(names, [".", "..", "test.txt"]);
    root_dir.remove("very/long/path/test.txt").await.unwrap();
    names = dir
        .iter()
        .collect()
        .await
        .iter()
        .map(|r| r.as_ref().unwrap().file_name())
        .collect::<Vec<String>>();
    assert_eq!(names, [".", ".."]);
    assert!(root_dir.remove("very/long/path").await.is_ok());

    names = root_dir
        .iter()
        .collect()
        .await
        .iter()
        .map(|r| r.as_ref().unwrap().file_name())
        .collect::<Vec<String>>();
    assert_eq!(names, ["long.txt", "short.txt", "very", "very-long-dir-name"]);
    root_dir.remove("long.txt").await.unwrap();
    names = root_dir
        .iter()
        .collect()
        .await
        .iter()
        .map(|r| r.as_ref().unwrap().file_name())
        .collect::<Vec<String>>();
    assert_eq!(names, ["short.txt", "very", "very-long-dir-name"]);
}

#[tokio::test]
async fn test_remove_fat12() {
    call_with_fs(test_remove, FAT12_IMG, 3).await
}

#[tokio::test]
async fn test_remove_fat16() {
    call_with_fs(test_remove, FAT16_IMG, 3).await
}

#[tokio::test]
async fn test_remove_fat32() {
    call_with_fs(test_remove, FAT32_IMG, 3).await
}

async fn test_create_file(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let dir = root_dir.open_dir("very/long/path").await.unwrap();
    let mut names = dir
        .iter()
        .collect()
        .await
        .iter()
        .map(|r| r.as_ref().unwrap().file_name())
        .collect::<Vec<String>>();
    assert_eq!(names, [".", "..", "test.txt"]);
    {
        // test some invalid names
        assert!(root_dir.create_file("very/long/path/:").await.is_err());
        assert!(root_dir.create_file("very/long/path/\0").await.is_err());
        // create file
        let mut file = root_dir
            .create_file("very/long/path/new-file-with-long-name.txt")
            .await
            .unwrap();
        file.write_all(&TEST_STR.as_bytes()).await.unwrap();
        file.flush().await.unwrap();
    }
    // check for dir entry
    names = dir
        .iter()
        .collect()
        .await
        .iter()
        .map(|r| r.as_ref().unwrap().file_name())
        .collect::<Vec<String>>();
    assert_eq!(names, [".", "..", "test.txt", "new-file-with-long-name.txt"]);
    names = dir
        .iter()
        .collect()
        .await
        .iter()
        .map(|r| r.as_ref().unwrap().short_file_name())
        .collect::<Vec<String>>();
    assert_eq!(names, [".", "..", "TEST.TXT", "NEW-FI~1.TXT"]);
    {
        // check contents
        let mut file = root_dir
            .open_file("very/long/path/new-file-with-long-name.txt")
            .await
            .unwrap();
        let buf = read_to_end(&mut file).await.unwrap();
        assert_eq!(&core::str::from_utf8(&buf).unwrap(), &TEST_STR);
    }
    // Create enough entries to allocate next cluster
    for i in 0..512 / 32 {
        let name = format!("test{}", i);
        dir.create_file(&name).await.unwrap();
    }
    names = dir
        .iter()
        .collect()
        .await
        .iter()
        .map(|r| r.as_ref().unwrap().file_name())
        .collect::<Vec<String>>();
    assert_eq!(names.len(), 4 + 512 / 32);
    // check creating existing file opens it
    {
        let mut file = root_dir
            .create_file("very/long/path/new-file-with-long-name.txt")
            .await
            .unwrap();
        let buf = read_to_end(&mut file).await.unwrap();
        assert_eq!(&core::str::from_utf8(&buf).unwrap(), &TEST_STR);
    }
    // check using create_file with existing directory fails
    assert!(root_dir.create_file("very").await.is_err());
}

#[tokio::test]
async fn test_create_file_fat12() {
    call_with_fs(test_create_file, FAT12_IMG, 4).await
}

#[tokio::test]
async fn test_create_file_fat16() {
    call_with_fs(test_create_file, FAT16_IMG, 4).await
}

#[tokio::test]
async fn test_create_file_fat32() {
    call_with_fs(test_create_file, FAT32_IMG, 4).await
}

async fn test_create_dir(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let parent_dir = root_dir.open_dir("very/long/path").await.unwrap();
    let mut names = parent_dir
        .iter()
        .collect()
        .await
        .iter()
        .map(|r| r.as_ref().unwrap().file_name())
        .collect::<Vec<String>>();
    assert_eq!(names, [".", "..", "test.txt"]);
    {
        let subdir = root_dir
            .create_dir("very/long/path/new-dir-with-long-name")
            .await
            .unwrap();
        names = subdir
            .iter()
            .collect()
            .await
            .iter()
            .map(|r| r.as_ref().unwrap().file_name())
            .collect::<Vec<String>>();
        assert_eq!(names, [".", ".."]);
    }
    // check if new entry is visible in parent
    names = parent_dir
        .iter()
        .collect()
        .await
        .iter()
        .map(|r| r.as_ref().unwrap().file_name())
        .collect::<Vec<String>>();
    assert_eq!(names, [".", "..", "test.txt", "new-dir-with-long-name"]);
    {
        // Check if new directory can be opened and read
        let subdir = root_dir
            .open_dir("very/long/path/new-dir-with-long-name")
            .await
            .unwrap();
        names = subdir
            .iter()
            .collect()
            .await
            .iter()
            .map(|r| r.as_ref().unwrap().file_name())
            .collect::<Vec<String>>();
        assert_eq!(names, [".", ".."]);
    }
    // Check if '.' is alias for new directory
    {
        let subdir = root_dir
            .open_dir("very/long/path/new-dir-with-long-name/.")
            .await
            .unwrap();
        names = subdir
            .iter()
            .collect()
            .await
            .iter()
            .map(|r| r.as_ref().unwrap().file_name())
            .collect::<Vec<String>>();
        assert_eq!(names, [".", ".."]);
    }
    // Check if '..' is alias for parent directory
    {
        let subdir = root_dir
            .open_dir("very/long/path/new-dir-with-long-name/..")
            .await
            .unwrap();
        names = subdir
            .iter()
            .collect()
            .await
            .iter()
            .map(|r| r.as_ref().unwrap().file_name())
            .collect::<Vec<String>>();
        assert_eq!(names, [".", "..", "test.txt", "new-dir-with-long-name"]);
    }
    // check if creating existing directory returns it
    {
        let subdir = root_dir.create_dir("very").await.unwrap();
        names = subdir
            .iter()
            .collect()
            .await
            .iter()
            .map(|r| r.as_ref().unwrap().file_name())
            .collect::<Vec<String>>();
        assert_eq!(names, [".", "..", "long"]);
    }
    // check short names validity after create_dir
    {
        let subdir = root_dir.create_dir("test").await.unwrap();
        names = subdir
            .iter()
            .collect()
            .await
            .iter()
            .map(|r| r.as_ref().unwrap().short_file_name())
            .collect::<Vec<String>>();
        assert_eq!(names, [".", ".."]);
    }

    // check using create_dir with existing file fails
    assert!(root_dir.create_dir("very/long/path/test.txt").await.is_err());
}

#[tokio::test]
async fn test_create_dir_fat12() {
    call_with_fs(test_create_dir, FAT12_IMG, 5).await
}

#[tokio::test]
async fn test_create_dir_fat16() {
    call_with_fs(test_create_dir, FAT16_IMG, 5).await
}

#[tokio::test]
async fn test_create_dir_fat32() {
    call_with_fs(test_create_dir, FAT32_IMG, 5).await
}

async fn test_rename_file(fs: FileSystem) {
    let root_dir = fs.root_dir();
    let parent_dir = root_dir.open_dir("very/long/path").await.unwrap();
    let entries = parent_dir.iter().collect().await;
    let entries = entries.iter().map(|r| r.as_ref().unwrap()).collect::<Vec<_>>();
    let names = entries.iter().map(|r| r.file_name()).collect::<Vec<_>>();
    assert_eq!(names, [".", "..", "test.txt"]);
    assert_eq!(entries[2].len(), 14);
    let stats = fs.stats().await.unwrap();

    parent_dir
        .rename("test.txt", &parent_dir, "new-long-name.txt")
        .await
        .unwrap();
    let entries = parent_dir.iter().collect().await;
    let entries = entries.iter().map(|r| r.as_ref().unwrap()).collect::<Vec<_>>();
    let names = entries.iter().map(|r| r.file_name()).collect::<Vec<_>>();
    assert_eq!(names, [".", "..", "new-long-name.txt"]);
    assert_eq!(entries[2].len(), TEST_STR2.len() as u64);
    let mut file = parent_dir.open_file("new-long-name.txt").await.unwrap();
    let buf = read_to_end(&mut file).await.unwrap();
    file.flush().await.unwrap();
    assert_eq!(str::from_utf8(&buf).unwrap(), TEST_STR2);

    parent_dir
        .rename("new-long-name.txt", &root_dir, "moved-file.txt")
        .await
        .unwrap();
    let entries = root_dir.iter().collect().await;
    let entries = entries.iter().map(|r| r.as_ref().unwrap()).collect::<Vec<_>>();
    let names = entries.iter().map(|r| r.file_name()).collect::<Vec<_>>();
    assert_eq!(
        names,
        ["long.txt", "short.txt", "very", "very-long-dir-name", "moved-file.txt"]
    );
    assert_eq!(entries[4].len(), TEST_STR2.len() as u64);
    let mut file = root_dir.open_file("moved-file.txt").await.unwrap();
    let buf = read_to_end(&mut file).await.unwrap();
    file.flush().await.unwrap();
    assert_eq!(str::from_utf8(&buf).unwrap(), TEST_STR2);

    assert!(root_dir.rename("moved-file.txt", &root_dir, "short.txt").await.is_err());
    let entries = root_dir.iter().collect().await;
    let entries = entries.iter().map(|r| r.as_ref().unwrap()).collect::<Vec<_>>();
    let names = entries.iter().map(|r| r.file_name()).collect::<Vec<_>>();
    assert_eq!(
        names,
        ["long.txt", "short.txt", "very", "very-long-dir-name", "moved-file.txt"]
    );

    assert!(root_dir
        .rename("moved-file.txt", &root_dir, "moved-file.txt")
        .await
        .is_ok());

    let new_stats = fs.stats().await.unwrap();
    assert_eq!(new_stats.free_clusters(), stats.free_clusters());
}

#[tokio::test]
async fn test_rename_file_fat12() {
    call_with_fs(test_rename_file, FAT12_IMG, 6).await
}

#[tokio::test]
async fn test_rename_file_fat16() {
    call_with_fs(test_rename_file, FAT16_IMG, 6).await
}

#[tokio::test]
async fn test_rename_file_fat32() {
    call_with_fs(test_rename_file, FAT32_IMG, 6).await
}

async fn test_dirty_flag(tmp_path: String) {
    // Open filesystem, make change, and forget it - should become dirty
    let fs = open_filesystem_rw(tmp_path.clone()).await;
    let status_flags = fs.read_status_flags().await.unwrap();
    assert_eq!(status_flags.dirty(), false);
    assert_eq!(status_flags.io_error(), false);
    fs.root_dir().create_file("abc.txt").await.unwrap();
    core::mem::forget(fs);
    // Check if volume is dirty now
    let fs = open_filesystem_rw(tmp_path.clone()).await;
    let status_flags = fs.read_status_flags().await.unwrap();
    assert_eq!(status_flags.dirty(), true);
    assert_eq!(status_flags.io_error(), false);
    fs.unmount().await.unwrap();
    // Make sure remounting does not clear the dirty flag
    let fs = open_filesystem_rw(tmp_path).await;
    let status_flags = fs.read_status_flags().await.unwrap();
    assert_eq!(status_flags.dirty(), true);
    assert_eq!(status_flags.io_error(), false);
}

#[tokio::test]
async fn test_dirty_flag_fat12() {
    call_with_tmp_img(test_dirty_flag, FAT12_IMG, 7).await
}

#[tokio::test]
async fn test_dirty_flag_fat16() {
    call_with_tmp_img(test_dirty_flag, FAT16_IMG, 7).await
}

#[tokio::test]
async fn test_dirty_flag_fat32() {
    call_with_tmp_img(test_dirty_flag, FAT32_IMG, 7).await
}

async fn test_multiple_files_in_directory(fs: FileSystem) {
    let dir = fs.root_dir().create_dir("/TMP").await.unwrap();
    for i in 0..8 {
        let name = format!("T{}.TXT", i);
        let mut file = dir.create_file(&name).await.unwrap();
        file.write_all(TEST_STR.as_bytes()).await.unwrap();
        file.flush().await.unwrap();

        let files = dir.iter().collect().await;
        let files = files.iter().map(|r| r.as_ref().unwrap()).collect::<Vec<_>>();
        let file = files.iter().find(|e| e.file_name() == name).unwrap();
        assert_eq!(TEST_STR.len() as u64, file.len(), "Wrong file len on iteration {}", i);
    }
}

#[tokio::test]
async fn test_multiple_files_in_directory_fat12() {
    call_with_fs(&test_multiple_files_in_directory, FAT12_IMG, 8).await
}

#[tokio::test]
async fn test_multiple_files_in_directory_fat16() {
    call_with_fs(&test_multiple_files_in_directory, FAT16_IMG, 8).await
}

#[tokio::test]
async fn test_multiple_files_in_directory_fat32() {
    call_with_fs(&test_multiple_files_in_directory, FAT32_IMG, 8).await
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
