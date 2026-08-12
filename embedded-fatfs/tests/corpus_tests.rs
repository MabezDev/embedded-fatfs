//! Tests of FAT32 filesystem validity using the corpus
//!
//! These tests ensure:
//!
//! * Correctness of allocation and free cluster count handling when a FS has the dirty bit set
//! *

mod corpus;

use corpus::{fsinfo, ls, mark_dirty, mount, write_a_file};

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

/// The allocation hint must still advance on a dirty volume
///
/// The hint shares the FSInfo sector with the free-cluster count, and mounting
/// dirty discards that count. This pins that writing the sector is driven by the
/// hint alone and is not conditional on the free cluster count being known
#[tokio::test]
async fn the_allocation_hint_advances_on_a_dirty_volume() {
    let pristine = corpus::formatted_fs::build().await;
    let mut dirty = pristine.clone();
    mark_dirty(&mut dirty);
    let before = fsinfo(&dirty, NEXT_FREE);

    let after = write_a_file(&dirty, "NEW.BIN", 4096).await;

    let hint = fsinfo(&after, NEXT_FREE);
    assert!(
        hint > before,
        "the hint was not written back after a dirty mount: {} -> {}",
        before,
        hint
    );

    // And it has to be *useful*: pointing at a cluster that is actually free,
    // so the next allocation finds one immediately rather than searching.
    assert_eq!(
        after.fat_get(0, hint),
        0,
        "hint points at cluster {}, which is not free",
        hint
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
#[tokio::test]
async fn the_hint_keeps_advancing_across_dirty_mount_cycles() {
    let pristine = corpus::formatted_fs::build().await;
    let mut img = pristine.clone();
    let mut hints = Vec::new();

    for i in 0..4 {
        mark_dirty(&mut img);
        img = write_a_file(&img, &format!("LOG{}.BIN", i), 4096).await;
        let hint = fsinfo(&img, NEXT_FREE);
        // Checked against the volume as it stands now, not at the end: a later cycle will quite properly allocate the
        // cluster this one pointed at.
        assert_eq!(
            img.fat_get(0, hint),
            0,
            "after cycle {} the hint names cluster {}, which is not free",
            i,
            hint
        );
        hints.push(hint);
    }

    assert!(
        hints.windows(2).all(|w| w[1] > w[0]),
        "the hint stalled across mount cycles: {:?}",
        hints
    );
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

/// Ensure we correctly resolve ../ pointing at 0
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
