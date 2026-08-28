//! Tests of FAT32 filesystem validity using the corpus
//!
//! These tests ensure:
//!
//! * Correctness of allocation and free cluster count handling when a FS has the dirty bit set
//! * `..` entries pointing at the root directory name cluster 0, as the spec requires, and resolve back to it
//! * Cluster numbers from outside the volume are rejected as `Error::CorruptedFileSystem`

mod corpus;

use corpus::{fsinfo, ls, mark_dirty, mount, write_a_file, Fat32Image};

use embedded_io_async::Read;

/// Offsets of the two fields of interest within the FSInfo sector.
const FREE_COUNT: u64 = 488;
const NEXT_FREE: u64 = 492;
/// What either field holds when its value is not known.
const UNKNOWN: u32 = 0xFFFF_FFFF;

// ############################################################################################################
//                                               ALLOCATTION HINT
// ############################################################################################################

/// The allocation hint must advance on a clean volume
#[tokio::test]
async fn the_allocation_hint_advances_on_a_clean_volume() {
    let pristine = corpus::formatted_fs::build().await;
    let before = fsinfo(&pristine, NEXT_FREE);

    let after = write_a_file(&pristine, "NEW.BIN", 4096).await;

    assert!(
        fsinfo(&after, NEXT_FREE) > before,
        "hint did not advance: {} -> {}",
        before,
        fsinfo(&after, NEXT_FREE)
    );
}

// ############################################################################################################
//                                           FREE CLUSTER COUNT
// ############################################################################################################

/// An untrusted free cluster count must not be written
///
/// Mounting dirty discards the free cluster count. Writing it would leave the card claiming free space that has since
/// been allocated.
#[tokio::test]
async fn a_discarded_free_count_is_written_back_as_unknown() {
    let pristine = corpus::formatted_fs::build().await;
    let mut dirty = pristine.clone();
    mark_dirty(&mut dirty);
    let before_count = fsinfo(&dirty, FREE_COUNT);
    let before_hint = fsinfo(&dirty, NEXT_FREE);
    assert_ne!(before_count, UNKNOWN, "the fixture should start with a known count");

    let after = write_a_file(&dirty, "NEW.BIN", 4096).await;
    // The write advances the next free cluster hint, but not the (untrusted) free cluster count
    assert!(
        fsinfo(&after, NEXT_FREE) > before_hint,
        "the FSInfo sector was not written at all: hint {} -> {}",
        before_hint,
        fsinfo(&after, NEXT_FREE)
    );
    assert_eq!(
        fsinfo(&after, FREE_COUNT),
        UNKNOWN,
        "a count discarded at mount was written back to the card anyway"
    );

    // ...and, dirty flag aside, the volume it produced is sound.
    let problems = corpus::verify::check(&after);
    assert!(problems.is_empty(), "dirty volume is not sound: {:#?}", problems);

    // ...and the volume is still dirty.
    let (fs, _buffer) = mount(&after).await;
    assert!(
        fs.read_status_flags().await.expect("read_status_flags").dirty(),
        "unmounting marked a volume clean that was mounted dirty"
    );
}

/// A count that cannot be true is not worth preserving, and writing UNKNOWN over it is an improvement.
#[tokio::test]
async fn an_impossible_free_count_is_replaced_with_unknown() {
    let pristine = corpus::formatted_fs::build().await;
    let mut broken = pristine.clone();
    mark_dirty(&mut broken);
    let impossible = broken.geom().total_clusters + 1000;
    broken.write_u32(broken.geom().fs_info_offset() + FREE_COUNT, impossible);

    let after = write_a_file(&broken, "NEW.BIN", 4096).await;

    assert_eq!(
        fsinfo(&after, FREE_COUNT),
        UNKNOWN,
        "an impossible count was written back to the card"
    );
}

/// The hint must continue advancing correctly across repeated dirty mount cycles
///
/// Mounting dirty discards the free-cluster count, so this also covers the case that writing the sector is driven by
/// the hint alone and is not conditional on the count being known.
#[tokio::test]
async fn the_hint_keeps_advancing_across_dirty_mount_cycles() {
    let pristine = corpus::formatted_fs::build().await;
    let mut img = pristine.clone();
    let mut prev_hint = fsinfo(&img, NEXT_FREE);

    for i in 0..4 {
        mark_dirty(&mut img);
        img = write_a_file(&img, &format!("LOG{}.BIN", i), 4096).await;
        let hint = fsinfo(&img, NEXT_FREE);
        assert!(
            hint > prev_hint,
            "after cycle {} the hint stalled: {} -> {}",
            i,
            prev_hint,
            hint
        );
        // Checked against the volume as it stands now, not at the end: a later cycle will quite properly allocate the
        // cluster this one pointed at.
        assert_eq!(
            img.fat_get(0, hint),
            0,
            "after cycle {} the hint names cluster {}, which is not free",
            i,
            hint
        );
        prev_hint = hint;
    }
}

/// Allocation must not depend on the hint being right.
#[tokio::test]
async fn a_stale_hint_still_allocates_correctly() {
    let pristine = corpus::formatted_fs::build().await;

    for hint in [0_u32, 2, UNKNOWN, pristine.geom().max_valid_cluster()] {
        let mut img = pristine.clone();
        img.write_u32(img.geom().fs_info_offset() + NEXT_FREE, hint);

        let after = write_a_file(&img, "NEW.BIN", 4096).await;

        let problems = corpus::verify::check(&after);
        assert!(
            problems.is_empty(),
            "hint {} produced an inconsistent volume: {:#?}",
            hint,
            problems
        );
    }
}

// ############################################################################################################
//                                             HANDLING OF ..
// ############################################################################################################

/// The cluster number stored in a `..` directory entry.
///
/// The spec is clear that any `..` ponting to the root should point to cluster 0. See Microsoft FAT Specification §6.5,
/// "Directory creation"
#[tokio::test]
async fn dotdot_names_cluster_zero_only_when_the_parent_is_the_root() {
    let img = corpus::formatted_fs::build().await;

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

/// The `..` entry of a child of the root holds cluster 0, which must resolve to the root without tripping cluster
/// validation: cluster 0 is not a data cluster, so reaching `offset_from_cluster` with it would be a false alarm.
///
/// The on-disk encoding is pinned by `dotdot_names_cluster_zero_only_when_the_parent_is_the_root`; the corruption
/// case is covered by `a_directory_with_an_invalid_first_cluster_is_corrupted`.
#[tokio::test]
async fn dotdot_walks_back_up_to_the_directory_it_names() {
    let img = corpus::formatted_fs::build().await;
    let (fs, _bytes) = mount(&img).await;

    let root = ls(&fs, "").await;
    let data = ls(&fs, "DATA").await;
    assert_ne!(root, data, "the fixture needs two distinguishable directories");

    assert_eq!(ls(&fs, "DATA/..").await, root);
    assert_eq!(ls(&fs, "DATA/SUB/..").await, data);
    assert_eq!(ls(&fs, "DATA/SUB/../..").await, root);
    assert_eq!(
        ls(
            &fs,
            "DATA/SUB/../../DATA/SUB/../../DATA/SUB/../../DATA/SUB/../../DATA/SUB/../.."
        )
        .await,
        root
    );
    // Round trips both ways, so `..` is not just landing somewhere plausible.
    assert_eq!(ls(&fs, "DATA/../DATA/SUB").await, ls(&fs, "DATA/SUB").await);
}

// ############################################################################################################
//                                           VALID CLUSTER CHECKING
// ############################################################################################################

/// Cluster numbers outside the valid data-cluster range, for poking into a directory entry's first-cluster field.
///
/// Cluster 0 is deliberately absent: a directory entry that names it reads as empty/root, which is not an error. The
/// `0x0FFF_FFFF` value is the flash-erase pattern a half-written entry leaves behind.
fn invalid_entry_clusters(img: &Fat32Image) -> Vec<u32> {
    vec![
        1,                                  // below the reserved entries
        img.geom().max_valid_cluster() + 1, // one past the top of the volume
        0x0FFF_FFFF,                        // flash-erase pattern
        0x0123_4567,                        // nonsense
    ]
}

/// Like [`invalid_entry_clusters`] but for FAT chain slots, so special markers are excluded: `0x0FFF_FFFF` means
/// end-of-chain and `0x0FFF_FFF7` means bad sector, so either would end the chain instead of producing an error.
fn invalid_link_clusters(img: &Fat32Image) -> Vec<u32> {
    vec![
        1,                                  // reserved, rejected by the chain walk itself
        img.geom().max_valid_cluster() + 1, // one past the top of the volume
        0x0123_4567,                        // nonsense
    ]
}

/// A file whose first cluster is out of range must report the volume as corrupted rather than seek to a bogus offset.
#[tokio::test]
async fn a_file_with_an_invalid_first_cluster_is_corrupted() {
    let pristine = corpus::formatted_fs::build().await;

    for bad in invalid_entry_clusters(&pristine) {
        let mut img = pristine.clone();
        let log = img.sfn(img.geom().root_cluster, "LOG.TXT");
        img.set_entry_first_cluster(log, bad);

        let (fs, _buffer) = mount(&img).await;
        let mut file = fs.root_dir().open_file("LOG.TXT").await.expect("open_file");
        let err = file.read_exact(&mut [0u8; 512]).await.expect_err("reading should fail");
        assert!(
            matches!(
                err,
                embedded_io_async::ReadExactError::Other(embedded_fatfs::Error::CorruptedFileSystem)
            ),
            "first cluster {:#x} gave {:?}",
            bad,
            err
        );
    }
}

/// A directory entry whose first cluster is out of range must be reported as corrupted when iterated.
///
/// Covers both an ordinary subdirectory and a `..` entry. Only cluster 0 is special-cased (a `..` holding it names the
/// root, see `dotdot_names_cluster_zero_only_when_the_parent_is_the_root`); any other out-of-range value must reach
/// cluster validation in either.
#[tokio::test]
async fn a_directory_with_an_invalid_first_cluster_is_corrupted() {
    let pristine = corpus::formatted_fs::build().await;

    // (entry whose first cluster we corrupt, path we then open and iterate)
    for (entry, open_path) in [("DATA", "DATA"), ("DATA/..", "DATA/..")] {
        for bad in invalid_entry_clusters(&pristine) {
            let mut img = pristine.clone();
            let data = img.dir_cluster("DATA");
            let sfn_offset = match entry {
                "DATA" => img.sfn(img.geom().root_cluster, "DATA"),
                "DATA/.." => img.sfn(data, ".."),
                _ => unreachable!("unknown target {:?}", entry),
            };
            img.set_entry_first_cluster(sfn_offset, bad);

            let (fs, _buffer) = mount(&img).await;
            let dir = fs.root_dir().open_dir(open_path).await.expect("open_dir");
            let err = dir
                .iter()
                .next()
                .await
                .expect("iterator")
                .expect_err("iterating should fail");
            assert!(
                matches!(err, embedded_fatfs::Error::CorruptedFileSystem),
                "{:?} first cluster {:#x} gave {:?}",
                entry,
                bad,
                err
            );
        }
    }
}

/// Following a chain through an out-of-range link must report the volume as corrupted.
///
/// `BOOT.BIN` spans three clusters, so the first read lands in the (valid) first cluster and the second read has to
/// follow the corrupt link to reach it.
#[tokio::test]
async fn an_out_of_range_link_in_a_chain_is_corrupted() {
    let pristine = corpus::formatted_fs::build().await;

    for bad in invalid_link_clusters(&pristine) {
        let mut img = pristine.clone();
        let boot = img.sfn(img.geom().root_cluster, "BOOT.BIN");
        let first = img.entry_first_cluster(boot);
        img.fat_set_all(first, bad);

        let (fs, _buffer) = mount(&img).await;
        let mut file = fs.root_dir().open_file("BOOT.BIN").await.expect("open_file");
        let err = file
            .read_exact(&mut [0u8; 1536])
            .await
            .expect_err("reading should fail");
        assert!(
            matches!(
                err,
                embedded_io_async::ReadExactError::Other(embedded_fatfs::Error::CorruptedFileSystem)
            ),
            "first cluster {:#x} gave {:?}",
            bad,
            err
        );
    }
}
