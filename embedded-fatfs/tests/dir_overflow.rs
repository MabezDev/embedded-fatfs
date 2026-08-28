//! Test that large directories are handled correctly.
//!
//! The storage is pre-filled with a deliberately adversarial pattern: every 32-byte block looks like a valid,
//! non-deleted SFN directory entry. Any failure to zero a newly-allocated directory cluster is therefore caught as
//! erroneous additonal entries.
//!
//! Each case runs the same procedure over a directory at a different depth. The root directory is the one without an
//! entry of its own recording that it is a directory, so it has to be recognised by its cluster number instead.
//!
//! Every case ends with a full `corpus::verify::check`, which also hands the volume to `fsck.fat`.

mod corpus;

use std::io::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

use corpus::formatted_fs::{format_and_mount, BYTES_PER_CLUSTER, TOTAL_BYTES};
use corpus::Fat32Image;
use embedded_io_async::{Read, Write};

/// The 28-character names below need 3 LFN entries plus the short-name entry.
const SLOTS_PER_FILE: usize = 4;
const FILES_PER_CLUSTER: usize = BYTES_PER_CLUSTER as usize / (32 * SLOTS_PER_FILE);
/// Enough to fill three clusters, so every directory under test grows more than once. A subdirectory also spends two
/// slots on `.` and `..`, which only makes it overflow sooner.
const FILE_COUNT: usize = FILES_PER_CLUSTER * 3;

/// Fill every 32-byte block with a plausible-looking SFN entry.
///
/// If a directory cluster is not zeroed, the iterator will see these as
/// extra files with names like `RUBBISH 001`, `RUBBISH 002`,
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

/// Overflow the directory at `from` and check that nothing but the files we wrote comes back.
///
/// `from` is an absolute path; `/` is the root directory. Every component is created first, so `/a/b` is a directory
/// inside a directory rather than one with a slash in its name.
async fn test_directory_is_zeroed(from: &str) {
    let _ = env_logger::builder().is_test(true).try_init();

    let storage = {
        let mut v = vec![0u8; TOTAL_BYTES as usize];
        adversarial_fill(&mut v);
        v
    };
    let (fs, buffer) = format_and_mount(storage).await;

    assert_eq!(fs.cluster_size(), BYTES_PER_CLUSTER);
    assert_eq!(
        fs.fat_type(),
        embedded_fatfs::FatType::Fat32,
        "a FAT16 root is a fixed-size region that cannot overflow at all, so the root case would prove nothing"
    );

    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let root = fs.root_dir();

    // `create_dir` resolves a path but does not create the parents along it, so walk them.
    let mut built = String::new();
    for component in from.split('/').filter(|c| !c.is_empty()) {
        built.push('/');
        built.push_str(component);
        root.create_dir(&built).await.expect("create_dir");
    }
    let dir = if built.is_empty() {
        root.clone()
    } else {
        root.open_dir(&built).await.expect("open_dir")
    };

    for i in 0..FILE_COUNT {
        let name = format!("FILE_{i:05}_OVERFLOW_TEST.DAT");
        let mut f = dir.create_file(&name).await.expect("create_file");
        let content = format!("{ts} file_{i:05}\n");
        f.write_all(content.as_bytes()).await.unwrap();
        f.flush().await.unwrap();
    }

    let names = count_entries(&dir).await;
    assert_eq!(
        names.len(),
        FILE_COUNT,
        "{from} should hold exactly {FILE_COUNT} files (no adversarial garbage)"
    );
    for (i, name) in names.iter().enumerate() {
        let expected = format!("FILE_{i:05}_OVERFLOW_TEST.DAT");
        assert_eq!(name, &expected, "wrong name at index {i} in {from}");
    }
    check_files_readable(&root, &built, &names, ts, FILE_COUNT).await;

    // The directory is still usable once it has grown.
    dir.create_file("EXTRA_AFTER_OVERFLOW.DAT")
        .await
        .expect("create after overflow");
    let after = count_entries(&dir).await;
    assert_eq!(after.len(), FILE_COUNT + 1);
    assert!(after.contains(&"EXTRA_AFTER_OVERFLOW.DAT".to_string()));

    drop(dir);
    drop(root);
    fs.unmount().await.expect("unmount");
    let img = Fat32Image::parse(buffer.borrow().clone());
    let problems = corpus::verify::check(&img);
    assert!(problems.is_empty(), "{from} left the volume unsound: {problems:#?}");
}

#[tokio::test]
async fn the_root_directory_is_zeroed() {
    test_directory_is_zeroed("/").await;
}

#[tokio::test]
async fn a_subdirectory_is_zeroed() {
    test_directory_is_zeroed("/subdir").await;
}

#[tokio::test]
async fn a_nested_subdirectory_is_zeroed() {
    test_directory_is_zeroed("/sub/subdir").await;
}

#[tokio::test]
async fn a_deeply_nested_subdirectory_is_zeroed() {
    test_directory_is_zeroed("/sub/sub/sub/subdir").await;
}
