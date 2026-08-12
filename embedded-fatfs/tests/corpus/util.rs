//! Commonly-reused testing functions for the corpus tests

use embedded_io_async::Write;

use super::formatted_fs::MemDisk;
use super::Fat32Image;

use embedded_fatfs::{FsOptions, LossyOemCpConverter, NullTimeProvider};

pub type FileSystem = embedded_fatfs::FileSystem<MemDisk, NullTimeProvider, LossyOemCpConverter>;

// Read `field` from the FSInfo sector, where `field` is given as a byte offset.
pub fn fsinfo(img: &Fat32Image, field: u64) -> u32 {
    img.read_u32(img.geom().fs_info_offset() + field)
}

/// Leave the volume looking like an interrupted writer did, in both the canonical reserved FAT table and its backup, as
/// well as the BPB BS_Reserved1 bits.
pub fn mark_dirty(img: &mut Fat32Image) {
    // ClnShutBitMask in FAT[1]
    //
    // (From MS FAT spec PDF, pg.19) [...] The second reserved cluster, FAT[1], is set by FORMAT to the EOC mark. On
    // FAT12 volumes, it is not used and is simply always contains an EOC mark. For FAT16 and FAT32, the file system
    // driver may use the high two bits of the FAT[1] entry for dirty volume flags (all other bits, are always left set
    // to 1). Note that the bit location is different for FAT16 and FAT32, because they are the high 2 bits of the
    // entry.
    //
    // For FAT16: ClnShutBitMask = 0x8000; HrdErrBitMask = 0x4000;
    // For FAT32: ClnShutBitMask = 0x08000000; HrdErrBitMask = 0x04000000;
    //
    // Bit ClnShutBitMask:  This indicates that the file system driver did not Dismount the volume properly the last
    // time it had the volume mounted. If bit is 1, volume is clean. If bit is 0, volume is dirty.
    //
    // Every copy of the FAT, or the volume is left with mismatched mirrors as well as a dirty flag, which is not what
    // an interrupted writer produces and is its own kind of corruption.
    for fat in 0..img.geom().num_fats {
        let offset = img.geom().fat_entry_offset(fat, 1);
        let raw = img.read_u32(offset);
        img.write_u32(offset, raw & !(0x08000000));
    }

    // Dirty bit in the boot sector: see MS fat spec Section 3.3, Extended BPB structure for FAT32 volumes. This is the
    // BS_Reserved1 field, used (out of spec) as a dirty bit very widely. See
    // https://github.com/dosfstools/dosfstools/issues/38
    //
    // In the backup boot sector as well as the first, for the same reason as the FAT copies above: leaving the two
    // disagreeing is a second fault on top of the dirty flag, and not one an interrupted writer leaves behind.
    // BPB_BkBootSec is at offset 50, and is 0 on a volume with no backup.
    let backup_sector = u64::from(img.read_u16(50));
    img.write(0x41, &[0b01]);
    if backup_sector != 0 {
        img.write(backup_sector * u64::from(img.geom().bytes_per_sector) + 0x41, &[0b01]);
    }
}

// Mount a filesystem image, and return both it and its underlying byte buffer.
pub async fn mount(img: &Fat32Image) -> (FileSystem, std::rc::Rc<std::cell::RefCell<Vec<u8>>>) {
    let _ = env_logger::builder().is_test(true).try_init();
    let disk = MemDisk::from_bytes(img.as_bytes().to_vec());
    let buffer = disk.buffer();
    let options = FsOptions::new()
        .time_provider(NullTimeProvider::new())
        .oem_cp_converter(LossyOemCpConverter::new());
    let fs = embedded_fatfs::FileSystem::new(disk, options).await.expect("mount");
    (fs, buffer)
}

/// Mount `img`, write a file of `bytes`, unmount, and return the resulting image.
pub async fn write_a_file(img: &Fat32Image, name: &str, bytes: usize) -> Fat32Image {
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

/// Sorted names in `path`, so two directories can be compared for identity.
pub async fn ls(fs: &FileSystem, path: &str) -> Vec<String> {
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
