//! Cluster allocation, and the FSInfo bookkeeping that keeps it cheap.
//!
//! Allocation walks the FAT forward from the "next free cluster" hint in the
//! FSInfo sector, so the cost of allocating a cluster is one FAT entry read —
//! provided that hint is accurate. If it is lost or stale the search restarts
//! from the beginning of the volume and walks every cluster already in use,
//! which on a large, full card means reading the entire FAT. These tests are
//! about the hint surviving.

mod corpus;

use corpus::pristine::MemDisk;
use corpus::Fat32Image;
use embedded_fatfs::{FsOptions, LossyOemCpConverter, NullTimeProvider};
use embedded_io_async::Write;

/// Offsets of the two fields of interest within the FSInfo sector.
const FREE_COUNT: u64 = 488;
const NEXT_FREE: u64 = 492;
/// What either field holds when its value is not known.
const UNKNOWN: u32 = 0xFFFF_FFFF;

type FileSystem = embedded_fatfs::FileSystem<MemDisk, NullTimeProvider, LossyOemCpConverter>;

fn fsinfo(img: &Fat32Image, field: u64) -> u32 {
    img.read_u32(img.geom().fs_info_offset() + field)
}

/// Leave the volume looking like an interrupted writer did, in both the places
/// FAT records it.
fn mark_dirty(img: &mut Fat32Image) {
    img.write(0x41, &[0b01]);
    let offset = img.geom().fat_entry_offset(0, 1);
    let raw = img.read_u32(offset);
    img.write_u32(offset, raw & !(1 << 27));
}

async fn mount(img: &Fat32Image) -> (FileSystem, std::rc::Rc<std::cell::RefCell<Vec<u8>>>) {
    let _ = env_logger::builder().is_test(true).try_init();
    let disk = MemDisk::from_bytes(img.as_bytes().to_vec());
    let buffer = disk.buffer();
    let options = FsOptions::new()
        .time_provider(NullTimeProvider::new())
        .oem_cp_converter(LossyOemCpConverter::new());
    let fs = embedded_fatfs::FileSystem::new(disk, options).await.expect("mount");
    (fs, buffer)
}

/// Mount `img`, write a file of `bytes`, unmount, and return the resulting
/// image.
async fn write_a_file(img: &Fat32Image, name: &str, bytes: usize) -> Fat32Image {
    let (fs, buffer) = mount(img).await;
    {
        let root = fs.root_dir();
        let mut file = root.create_file(name).await.expect("create");
        file.truncate().await.expect("truncate");
        file.write_all(&vec![0xAB; bytes]).await.expect("write");
        file.flush().await.expect("flush");
    }
    fs.unmount().await.expect("unmount");
    let data = buffer.borrow().clone();
    Fat32Image::parse(data)
}

/// The ordinary case, as a baseline for the dirty one.
#[tokio::test]
async fn the_allocation_hint_advances_on_a_clean_volume() {
    let pristine = corpus::pristine::build().await;
    let before = fsinfo(&pristine, NEXT_FREE);

    let after = write_a_file(&pristine, "NEW.BIN", 4096).await;

    assert!(
        fsinfo(&after, NEXT_FREE) > before,
        "hint did not advance: {} -> {}",
        before,
        fsinfo(&after, NEXT_FREE)
    );
}

/// A volume mounted dirty must still record where allocation got to.
///
/// It used to not: the hint shares the FSInfo sector with the free-cluster
/// count, mounting dirty discards that count, and the sector was only written
/// when the count was known. So a device that had lost power once would allocate
/// its way through every subsequent session without the hint ever reaching the
/// card, and every mount would restart the search from cluster 2.
#[tokio::test]
async fn the_allocation_hint_survives_a_dirty_mount() {
    let pristine = corpus::pristine::build().await;
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

/// Writing the hint must not cost the free-cluster count.
///
/// Mounting dirty discards the count in memory, but the value on the card may
/// well be right, and replacing it with "unknown" would force a full FAT scan
/// later to recover something we never actually knew to be wrong.
#[tokio::test]
async fn a_discarded_free_count_is_left_as_it_was_on_the_card() {
    let pristine = corpus::pristine::build().await;
    let mut dirty = pristine.clone();
    mark_dirty(&mut dirty);
    let before = fsinfo(&dirty, FREE_COUNT);
    assert_ne!(before, UNKNOWN, "the fixture should start with a known count");

    let after = write_a_file(&dirty, "NEW.BIN", 4096).await;

    assert_eq!(
        fsinfo(&after, FREE_COUNT),
        before,
        "the count on the card was replaced rather than preserved"
    );
}

/// A count that cannot be true is not worth preserving, and writing "unknown"
/// over it is an improvement.
#[tokio::test]
async fn an_impossible_free_count_is_replaced_with_unknown() {
    let pristine = corpus::pristine::build().await;
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

/// The hint has to keep working across repeated mount cycles, which is the
/// shape of a device that power-cycles unexpectedly: mount, write, lose power,
/// mount again.
#[tokio::test]
async fn the_hint_keeps_advancing_across_dirty_mount_cycles() {
    let pristine = corpus::pristine::build().await;
    let mut img = pristine.clone();
    let mut hints = Vec::new();

    for i in 0..4 {
        mark_dirty(&mut img);
        img = write_a_file(&img, &format!("LOG{}.BIN", i), 4096).await;
        let hint = fsinfo(&img, NEXT_FREE);
        // Checked against the volume as it stands now, not at the end: a later
        // cycle will quite properly allocate the cluster this one pointed at.
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

/// Allocation must not depend on the hint being right — a stale one costs a
/// search, never correctness.
#[tokio::test]
async fn a_stale_hint_still_allocates_correctly() {
    let pristine = corpus::pristine::build().await;

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
