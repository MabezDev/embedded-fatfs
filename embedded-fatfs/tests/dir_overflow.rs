//! Test that a directory with enough entries to overflow one cluster
//! survives the expansion and remains usable afterward.
//!
//! The storage is pre-filled with a deliberately adversarial pattern:
//! every 32-byte block looks like a valid, non-deleted SFN directory
//! entry.  Any failure to zero a newly-allocated directory cluster is
//! therefore caught — the iterator returns garbage entries alongside
//! the real ones.

mod formatted_fs;

use std::io::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

use embedded_fatfs::FormatVolumeOptions;
use embedded_io_async::{Read, Write};

// One sector per cluster, so a directory overflows after a handful of files and
// the whole volume still fits comfortably in memory.
//
// The volume has to be big enough that the cluster count clears FAT32's 65525
// minimum, or formatting quietly gives us FAT16 instead: `fat_type` is a hint
// that geometry overrides, not a demand. It matters because the FAT16 root
// directory is a fixed-size region rather than a cluster chain, and cannot
// overflow at all — the root test below would pass without testing anything. At
// 512-byte clusters, 40 MiB leaves about 81800 clusters, well clear of the line.
const CLUSTER: u32 = 512;
const VOLUME_BYTES: u64 = 40 * 1024 * 1024;
/// The 28-character names below need 3 LFN entries plus the short-name entry.
const SLOTS_PER_FILE: usize = 4;
const FILES_PER_CLUSTER: usize = CLUSTER as usize / (32 * SLOTS_PER_FILE);
/// Enough to fill three clusters, so the directory grows more than once.
const ROOT_FILE_COUNT: usize = FILES_PER_CLUSTER * 3;
/// `.` and `..` take a slot each out of a subdirectory's first cluster, which at
/// this cluster size is not enough to change the file count.
const SUBDIR_FILE_COUNT: usize = ROOT_FILE_COUNT;

/// Fill every 32-byte block with a plausible-looking SFN entry.
///
/// If a directory cluster is not zeroed, the iterator will see these as
/// extra files with names like `RUBBISH 001`, `RUBBISH 002`, …
fn adversarial_fill(buf: &mut [u8]) {
    let mut n: u32 = 1;
    for chunk in buf.chunks_mut(32) {
        if chunk.len() < 32 {
            break;
        }
        let _ = write!(&mut chunk[..8], "RUBBISH");
        let _ = write!(&mut chunk[8..11], "{n:03}");
        chunk[11] = 0x20; // archive attribute
        chunk[12..32].fill(0x00);
        n = n.wrapping_add(1);
    }
}

async fn count_entries(
    dir: &embedded_fatfs::Dir<
        '_,
        impl embedded_fatfs::ReadWriteSeek,
        impl embedded_fatfs::TimeProvider,
        impl embedded_fatfs::OemCpConverter,
    >,
) -> Vec<String> {
    let mut names: Vec<String> = dir
        .iter()
        .collect()
        .await
        .iter()
        .filter_map(|r| r.as_ref().ok().map(|e| e.file_name()))
        .filter(|n| n != "." && n != "..")
        .collect();
    names.sort();
    names
}

async fn check_files_readable(
    root: &embedded_fatfs::Dir<
        '_,
        impl embedded_fatfs::ReadWriteSeek,
        impl embedded_fatfs::TimeProvider,
        impl embedded_fatfs::OemCpConverter,
    >,
    dir_path: &str,
    names: &[String],
    ts: u64,
    file_count: usize,
) {
    for &i in &[0usize, file_count / 2, file_count - 1] {
        let path = format!("{dir_path}/{}", names[i]);
        let mut f = root.open_file(&path).await.unwrap();
        let mut buf = vec![0u8; 256];
        let n = f.read(&mut buf).await.unwrap();
        buf.truncate(n);
        let expected = format!("{ts} file_{i:05}\n");
        assert_eq!(
            std::str::from_utf8(&buf).unwrap(),
            &expected,
            "wrong content for {path}"
        );
    }
}

#[tokio::test]
async fn subdirectory_overflow_zeroes_second_cluster() {
    let _ = env_logger::builder().is_test(true).try_init();

    let opts = FormatVolumeOptions::new()
        .fat_type(embedded_fatfs::FatType::Fat32)
        .bytes_per_cluster(CLUSTER);

    let storage = {
        let mut v = vec![0u8; VOLUME_BYTES as usize];
        adversarial_fill(&mut v);
        v
    };
    let fs = formatted_fs::make_cursor_fs(storage, opts).await;

    assert_eq!(fs.cluster_size(), CLUSTER);
    assert_eq!(
        fs.fat_type(),
        embedded_fatfs::FatType::Fat32,
        "the root directory only grows on FAT32; on FAT16 this test proves nothing"
    );

    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

    let root = fs.root_dir();
    let dir = root.create_dir("overflow_dir").await.expect("create_dir");

    for i in 0..SUBDIR_FILE_COUNT {
        let name = format!("FILE_{i:05}_OVERFLOW_TEST.DAT");
        let mut f = dir.create_file(&name).await.expect("create_file");
        let content = format!("{ts} file_{i:05}\n");
        f.write_all(content.as_bytes()).await.unwrap();
        f.flush().await.unwrap();
    }

    let names = count_entries(&dir).await;
    assert_eq!(
        names.len(),
        SUBDIR_FILE_COUNT,
        "should have exactly {SUBDIR_FILE_COUNT} files (no adversarial garbage)"
    );
    for (i, name) in names.iter().enumerate() {
        let expected = format!("FILE_{i:05}_OVERFLOW_TEST.DAT");
        assert_eq!(name, &expected, "wrong name at index {i}");
    }
    check_files_readable(&root, "overflow_dir", &names, ts, SUBDIR_FILE_COUNT).await;

    dir.create_file("EXTRA_AFTER_OVERFLOW.DAT")
        .await
        .expect("create after overflow");
    let names2 = count_entries(&dir).await;
    assert_eq!(names2.len(), SUBDIR_FILE_COUNT + 1);
    assert!(names2.contains(&"EXTRA_AFTER_OVERFLOW.DAT".to_string()));
}

#[tokio::test]
async fn root_directory_overflow_zeroes_second_cluster() {
    let _ = env_logger::builder().is_test(true).try_init();

    let opts = FormatVolumeOptions::new()
        .fat_type(embedded_fatfs::FatType::Fat32)
        .bytes_per_cluster(CLUSTER);

    let storage = {
        let mut v = vec![0u8; VOLUME_BYTES as usize];
        adversarial_fill(&mut v);
        v
    };
    let fs = formatted_fs::make_cursor_fs(storage, opts).await;

    assert_eq!(fs.cluster_size(), CLUSTER);
    assert_eq!(
        fs.fat_type(),
        embedded_fatfs::FatType::Fat32,
        "the root directory only grows on FAT32; on FAT16 this test proves nothing"
    );

    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

    let root = fs.root_dir();

    for i in 0..ROOT_FILE_COUNT {
        let name = format!("ROOT_{i:05}_OVERFLOW_TEST.DAT");
        let mut f = root.create_file(&name).await.expect("create_file");
        let content = format!("{ts} file_{i:05}\n");
        f.write_all(content.as_bytes()).await.unwrap();
        f.flush().await.unwrap();
    }

    let names = count_entries(&root).await;
    assert_eq!(
        names.len(),
        ROOT_FILE_COUNT,
        "should have exactly {ROOT_FILE_COUNT} files (no adversarial garbage)"
    );
    for (i, name) in names.iter().enumerate() {
        let expected = format!("ROOT_{i:05}_OVERFLOW_TEST.DAT");
        assert_eq!(name, &expected, "wrong name at index {i}");
    }
    check_files_readable(&root, "", &names, ts, ROOT_FILE_COUNT).await;

    root.create_file("ROOT_EXTRA_AFTER_OVERFLOW.DAT")
        .await
        .expect("create after overflow");
    let names2 = count_entries(&root).await;
    assert_eq!(names2.len(), ROOT_FILE_COUNT + 1);
    assert!(names2.contains(&"ROOT_EXTRA_AFTER_OVERFLOW.DAT".to_string()));
}
