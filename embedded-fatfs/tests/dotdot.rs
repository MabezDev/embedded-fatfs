//! The cluster number stored in a `..` directory entry.
//!
//! The spec is specific about one case. Microsoft FAT Specification §6.5,
//! "Directory creation":
//!
//! > The contents of the DIR_FstClusLO and DIR_FstClusHI fields must be the same
//! > as that of the parent of the current directory. If the parent of the
//! > current directory is the root directory […] the DIR_FstClusLO and
//! > DIR_FstClusHI contents must be set to 0.
//!
//! On FAT32 the root is an ordinary cluster chain with a real cluster number, so
//! writing "the parent's first cluster" there is the natural thing to do and is
//! wrong. `fsck.vfat` reports it.
//!
//! Both halves of the rule are tested, because a fix that simply always wrote 0
//! would satisfy the first half and corrupt every deeper directory.

mod corpus;

use corpus::pristine::MemDisk;
use corpus::Fat32Image;
use embedded_fatfs::{FsOptions, LossyOemCpConverter, NullTimeProvider};

type FileSystem = embedded_fatfs::FileSystem<MemDisk, NullTimeProvider, LossyOemCpConverter>;

async fn mount(img: &Fat32Image) -> FileSystem {
    let _ = env_logger::builder().is_test(true).try_init();
    let disk = MemDisk::from_bytes(img.as_bytes().to_vec());
    let options = FsOptions::new()
        .time_provider(NullTimeProvider::new())
        .oem_cp_converter(LossyOemCpConverter::new());
    embedded_fatfs::FileSystem::new(disk, options).await.expect("mount")
}

/// Sorted names in `path`, so two directories can be compared for identity.
async fn ls(fs: &FileSystem, path: &str) -> Vec<String> {
    let dir = if path.is_empty() {
        fs.root_dir()
    } else {
        fs.root_dir().open_dir(path).await.expect("open_dir")
    };
    let mut names: Vec<String> = dir
        .iter()
        .collect()
        .await
        .iter()
        .map(|r| r.as_ref().expect("read entry").file_name())
        .collect();
    names.sort();
    names
}

/// What is actually on the card, read without going through the library.
#[tokio::test]
async fn dotdot_names_cluster_zero_only_when_the_parent_is_the_root() {
    let img = corpus::pristine::build().await;

    // `DATA` sits in the root, so its `..` must read as 0 rather than as the
    // root's own cluster number.
    let data = img.dir_cluster("DATA");
    assert_eq!(
        img.entry_first_cluster(img.sfn(data, "..")),
        0,
        "`DATA/..` should be cluster 0, the root's cluster is {}",
        img.geom().root_cluster
    );
    // Its `.` still names itself; only `..` is special-cased.
    assert_eq!(img.entry_first_cluster(img.sfn(data, ".")), data);

    // `DATA/SUB`'s parent is an ordinary directory, so its `..` names that
    // directory. This is the half a blanket "always write 0" would break.
    let sub = img.dir_cluster("DATA/SUB");
    assert_eq!(
        img.entry_first_cluster(img.sfn(sub, "..")),
        data,
        "`DATA/SUB/..` should name DATA (cluster {})",
        data
    );
    assert_eq!(img.entry_first_cluster(img.sfn(sub, ".")), sub);
}

/// And cluster 0 still resolves: `..` has to be usable, not merely spec-shaped.
#[tokio::test]
async fn dotdot_walks_back_up_to_the_directory_it_names() {
    let img = corpus::pristine::build().await;
    let fs = mount(&img).await;

    let root = ls(&fs, "").await;
    let data = ls(&fs, "DATA").await;
    assert_ne!(root, data, "the fixture needs two distinguishable directories");

    assert_eq!(ls(&fs, "DATA/..").await, root);
    assert_eq!(ls(&fs, "DATA/SUB/..").await, data);
    assert_eq!(ls(&fs, "DATA/SUB/../..").await, root);
    // Round trips both ways, so `..` is not just landing somewhere plausible.
    assert_eq!(ls(&fs, "DATA/../DATA/SUB").await, ls(&fs, "DATA/SUB").await);
}
