//! A raw, library-independent view of a FAT32 image held in memory.
//!
//! Everything here works on bytes and BPB-derived geometry only. It deliberately
//! does *not* use `embedded_fatfs` to locate structures, so a bug in the library
//! cannot hide a bug in a corpus image (and vice versa).

use std::fmt::Write as _;
use std::ops::Range;

/// Size of a directory entry, in bytes.
pub const DIR_ENTRY_SIZE: u64 = 32;

/// A FAT32 entry only carries 28 bits; the top nibble belongs to the volume and
/// must be preserved across writes.
pub const FAT32_MASK: u32 = 0x0FFF_FFFF;

/// Cluster is not allocated.
pub const FAT_FREE: u32 = 0x0000_0000;
/// End of a cluster chain. Any value >= 0x0FFF_FFF8 means the same thing; this
/// is the one the library writes.
pub const FAT_EOC: u32 = 0x0FFF_FFFF;
/// Cluster is marked unusable.
pub const FAT_BAD: u32 = 0x0FFF_FFF7;

/// Offsets of the fields of a 32-byte short-name directory entry.
pub mod dirent {
    pub const NAME: usize = 0;
    pub const ATTRS: usize = 11;
    pub const FIRST_CLUSTER_HI: usize = 20;
    pub const FIRST_CLUSTER_LO: usize = 26;
    pub const SIZE: usize = 28;

    pub const ATTR_READ_ONLY: u8 = 0x01;
    pub const ATTR_HIDDEN: u8 = 0x02;
    pub const ATTR_SYSTEM: u8 = 0x04;
    pub const ATTR_VOLUME_ID: u8 = 0x08;
    pub const ATTR_DIRECTORY: u8 = 0x10;
    pub const ATTR_ARCHIVE: u8 = 0x20;
    pub const ATTR_LFN: u8 = 0x0F;

    pub const DELETED: u8 = 0xE5;

    /// Offsets of the fields of a long-file-name entry.
    pub const LFN_ORDER: usize = 0;
    pub const LFN_CHECKSUM: usize = 13;
}

/// Offsets within the FSInfo sector.
pub mod fsinfo {
    pub const LEAD_SIG: usize = 0;
    pub const STRUC_SIG: usize = 484;
    pub const FREE_COUNT: usize = 488;
    pub const NEXT_FREE: usize = 492;
    pub const TRAIL_SIG: usize = 508;

    pub const UNKNOWN: u32 = 0xFFFF_FFFF;
}

/// FAT32 volume geometry, as decoded from the BPB in sector 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub bytes_per_sector: u32,
    pub sectors_per_cluster: u32,
    pub reserved_sectors: u32,
    pub num_fats: u32,
    pub sectors_per_fat: u32,
    pub total_sectors: u32,
    pub root_cluster: u32,
    pub fs_info_sector: u32,
    pub media: u8,
    /// Number of *data* clusters. Valid cluster numbers are `2..=total_clusters + 1`.
    pub total_clusters: u32,
}

impl Geometry {
    pub fn bytes_per_cluster(&self) -> u32 {
        self.bytes_per_sector * self.sectors_per_cluster
    }

    pub fn max_valid_cluster(&self) -> u32 {
        self.total_clusters + 1
    }

    /// Byte offset of the first sector of FAT copy `fat` (0-based).
    pub fn fat_offset(&self, fat: u32) -> u64 {
        assert!(fat < self.num_fats, "FAT copy {} does not exist", fat);
        u64::from(self.reserved_sectors + fat * self.sectors_per_fat) * u64::from(self.bytes_per_sector)
    }

    /// Byte offset of the FAT entry for `cluster` within FAT copy `fat`.
    pub fn fat_entry_offset(&self, fat: u32, cluster: u32) -> u64 {
        self.fat_offset(fat) + u64::from(cluster) * 4
    }

    /// The byte range spanned by FAT copy `fat`.
    pub fn fat_range(&self, fat: u32) -> Range<u64> {
        let start = self.fat_offset(fat);
        start..start + u64::from(self.sectors_per_fat) * u64::from(self.bytes_per_sector)
    }

    /// Byte offset of the first data sector (cluster 2).
    pub fn data_offset(&self) -> u64 {
        u64::from(self.reserved_sectors + self.num_fats * self.sectors_per_fat) * u64::from(self.bytes_per_sector)
    }

    /// Byte offset of the first byte of `cluster`.
    pub fn cluster_offset(&self, cluster: u32) -> u64 {
        assert!(
            (2..=self.max_valid_cluster()).contains(&cluster),
            "cluster {} out of range 2..={}",
            cluster,
            self.max_valid_cluster()
        );
        self.data_offset() + u64::from(cluster - 2) * u64::from(self.bytes_per_cluster())
    }

    pub fn fs_info_offset(&self) -> u64 {
        u64::from(self.fs_info_sector) * u64::from(self.bytes_per_sector)
    }
}

/// A FAT32 image plus the geometry needed to poke at its structures.
#[derive(Clone)]
pub struct Fat32Image {
    data: Vec<u8>,
    geom: Geometry,
}

impl Fat32Image {
    /// Decode the BPB of `data` and wrap it.
    ///
    /// # Panics
    ///
    /// Panics if `data` is not a FAT32 volume.
    pub fn parse(data: Vec<u8>) -> Self {
        let rd_u16 = |off: usize| u16::from_le_bytes([data[off], data[off + 1]]);
        let rd_u32 = |off: usize| u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]);

        assert_eq!(rd_u16(510), 0xAA55, "missing boot sector signature");
        let bytes_per_sector = u32::from(rd_u16(11));
        let sectors_per_cluster = u32::from(data[13]);
        let reserved_sectors = u32::from(rd_u16(14));
        let num_fats = u32::from(data[16]);
        let root_entries = rd_u16(17);
        let media = data[21];
        let sectors_per_fat_16 = u32::from(rd_u16(22));
        let total_sectors_16 = u32::from(rd_u16(19));
        let total_sectors_32 = rd_u32(32);
        let sectors_per_fat_32 = rd_u32(36);
        let root_cluster = rd_u32(44);
        let fs_info_sector = u32::from(rd_u16(48));

        assert_eq!(sectors_per_fat_16, 0, "not a FAT32 volume (sectors_per_fat_16 != 0)");
        assert_eq!(root_entries, 0, "not a FAT32 volume (root_entries != 0)");
        let total_sectors = if total_sectors_16 != 0 {
            total_sectors_16
        } else {
            total_sectors_32
        };

        let data_sectors = total_sectors - reserved_sectors - num_fats * sectors_per_fat_32;
        let total_clusters = data_sectors / sectors_per_cluster;
        assert!(
            total_clusters >= 65525,
            "not a FAT32 volume ({} clusters)",
            total_clusters
        );

        let geom = Geometry {
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors,
            num_fats,
            sectors_per_fat: sectors_per_fat_32,
            total_sectors,
            root_cluster,
            fs_info_sector,
            media,
            total_clusters,
        };
        Self { data, geom }
    }

    pub fn geom(&self) -> &Geometry {
        &self.geom
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }

    pub fn len(&self) -> u64 {
        self.data.len() as u64
    }

    // -- raw byte access ---------------------------------------------------

    pub fn read(&self, offset: u64, len: usize) -> &[u8] {
        let start = usize::try_from(offset).unwrap();
        &self.data[start..start + len]
    }

    pub fn write(&mut self, offset: u64, bytes: &[u8]) {
        let start = usize::try_from(offset).unwrap();
        self.data[start..start + bytes.len()].copy_from_slice(bytes);
    }

    pub fn read_u16(&self, offset: u64) -> u16 {
        let b = self.read(offset, 2);
        u16::from_le_bytes([b[0], b[1]])
    }

    pub fn write_u16(&mut self, offset: u64, value: u16) {
        self.write(offset, &value.to_le_bytes());
    }

    pub fn read_u32(&self, offset: u64) -> u32 {
        let b = self.read(offset, 4);
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }

    pub fn write_u32(&mut self, offset: u64, value: u32) {
        self.write(offset, &value.to_le_bytes());
    }

    // -- FAT ---------------------------------------------------------------

    /// Read the 28-bit FAT entry for `cluster` from FAT copy `fat`.
    pub fn fat_get(&self, fat: u32, cluster: u32) -> u32 {
        self.read_u32(self.geom.fat_entry_offset(fat, cluster)) & FAT32_MASK
    }

    /// Write the 28-bit FAT entry for `cluster` in FAT copy `fat`, preserving
    /// the reserved top nibble as a real driver would.
    pub fn fat_set(&mut self, fat: u32, cluster: u32, value: u32) {
        let off = self.geom.fat_entry_offset(fat, cluster);
        let old = self.read_u32(off);
        self.write_u32(off, (old & !FAT32_MASK) | (value & FAT32_MASK));
    }

    /// Write a FAT entry to *every* FAT copy. This is what a correct repair does.
    pub fn fat_set_all(&mut self, cluster: u32, value: u32) {
        for fat in 0..self.geom.num_fats {
            self.fat_set(fat, cluster, value);
        }
    }

    /// Follow the cluster chain starting at `first` in FAT copy 0.
    ///
    /// Stops at a free entry, an end-of-chain marker, an out-of-range link, or
    /// after `total_clusters` hops (so a loop cannot hang the generator).
    pub fn chain(&self, first: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut cluster = first;
        while (2..=self.geom.max_valid_cluster()).contains(&cluster) {
            out.push(cluster);
            if out.len() as u32 > self.geom.total_clusters {
                panic!("cluster chain from {} does not terminate", first);
            }
            let next = self.fat_get(0, cluster);
            if next < 2 || next > self.geom.max_valid_cluster() {
                break;
            }
            cluster = next;
        }
        out
    }

    /// The `n` lowest-numbered free clusters, searching upward from `from`.
    pub fn free_clusters(&self, from: u32, n: usize) -> Vec<u32> {
        let mut out = Vec::with_capacity(n);
        for cluster in from..=self.geom.max_valid_cluster() {
            if self.fat_get(0, cluster) == FAT_FREE {
                out.push(cluster);
                if out.len() == n {
                    return out;
                }
            }
        }
        panic!("volume has fewer than {} free clusters above {}", n, from);
    }

    /// Count clusters whose FAT entry is free, i.e. what FSInfo should report.
    pub fn count_free_clusters(&self) -> u32 {
        (2..=self.geom.max_valid_cluster())
            .filter(|&c| self.fat_get(0, c) == FAT_FREE)
            .count() as u32
    }

    // -- FSInfo ------------------------------------------------------------

    pub fn fsinfo_free_count(&self) -> u32 {
        self.read_u32(self.geom.fs_info_offset() + fsinfo::FREE_COUNT as u64)
    }

    pub fn set_fsinfo_free_count(&mut self, value: u32) {
        self.write_u32(self.geom.fs_info_offset() + fsinfo::FREE_COUNT as u64, value);
    }

    pub fn fsinfo_next_free(&self) -> u32 {
        self.read_u32(self.geom.fs_info_offset() + fsinfo::NEXT_FREE as u64)
    }

    pub fn set_fsinfo_next_free(&mut self, value: u32) {
        self.write_u32(self.geom.fs_info_offset() + fsinfo::NEXT_FREE as u64, value);
    }

    /// The whole FSInfo sector, for use as a don't-care range.
    pub fn fsinfo_range(&self) -> Range<u64> {
        let start = self.geom.fs_info_offset();
        start..start + u64::from(self.geom.bytes_per_sector)
    }

    // -- directories -------------------------------------------------------

    /// Byte offsets of every 32-byte slot in the directory starting at `cluster`,
    /// in on-disk order, across the whole cluster chain.
    pub fn dir_slots(&self, cluster: u32) -> Vec<u64> {
        let per_cluster = u64::from(self.geom.bytes_per_cluster()) / DIR_ENTRY_SIZE;
        let mut out = Vec::new();
        for c in self.chain(cluster) {
            let base = self.geom.cluster_offset(c);
            out.extend((0..per_cluster).map(|i| base + i * DIR_ENTRY_SIZE));
        }
        out
    }

    /// Read the 32 bytes of the directory entry at `offset`.
    pub fn entry(&self, offset: u64) -> [u8; 32] {
        self.read(offset, 32).try_into().unwrap()
    }

    /// Offset of the entry with short name `sfn` (an 11-byte padded name such as
    /// `"LOG     TXT"`) in the directory starting at `cluster`.
    pub fn find_sfn(&self, cluster: u32, sfn: &str) -> Option<u64> {
        let want = pad_sfn(sfn);
        self.dir_slots(cluster).into_iter().find(|&off| {
            let e = self.entry(off);
            e[dirent::ATTRS] != dirent::ATTR_LFN && e[..11] == want
        })
    }

    /// Like [`Self::find_sfn`] but panics with a directory listing if not found.
    pub fn sfn(&self, cluster: u32, sfn: &str) -> u64 {
        self.find_sfn(cluster, sfn).unwrap_or_else(|| {
            panic!(
                "no entry {:?} in dir at cluster {}:\n{}",
                sfn,
                cluster,
                self.ls(cluster)
            )
        })
    }

    /// Offsets of the run of LFN entries immediately preceding the short-name
    /// entry at `sfn_offset`, ordered as they appear on disk.
    pub fn lfn_run(&self, dir_cluster: u32, sfn_offset: u64) -> Vec<u64> {
        let slots = self.dir_slots(dir_cluster);
        let idx = slots.iter().position(|&o| o == sfn_offset).expect("offset not in dir");
        let mut out = Vec::new();
        for &off in slots[..idx].iter().rev() {
            let e = self.entry(off);
            if e[dirent::ATTRS] != dirent::ATTR_LFN || e[dirent::LFN_ORDER] == dirent::DELETED {
                break;
            }
            out.push(off);
        }
        out.reverse();
        out
    }

    /// Offset of the first never-used slot (`name[0] == 0`) in the directory,
    /// i.e. where a driver would append the next entry.
    pub fn first_unused_slot(&self, cluster: u32) -> u64 {
        self.dir_slots(cluster)
            .into_iter()
            .find(|&off| self.entry(off)[0] == 0)
            .expect("directory is full")
    }

    pub fn entry_first_cluster(&self, offset: u64) -> u32 {
        let hi = u32::from(self.read_u16(offset + dirent::FIRST_CLUSTER_HI as u64));
        let lo = u32::from(self.read_u16(offset + dirent::FIRST_CLUSTER_LO as u64));
        (hi << 16) | lo
    }

    pub fn set_entry_first_cluster(&mut self, offset: u64, cluster: u32) {
        self.write_u16(offset + dirent::FIRST_CLUSTER_HI as u64, (cluster >> 16) as u16);
        self.write_u16(offset + dirent::FIRST_CLUSTER_LO as u64, cluster as u16);
    }

    pub fn entry_size(&self, offset: u64) -> u32 {
        self.read_u32(offset + dirent::SIZE as u64)
    }

    pub fn set_entry_size(&mut self, offset: u64, size: u32) {
        self.write_u32(offset + dirent::SIZE as u64, size);
    }

    pub fn entry_attrs(&self, offset: u64) -> u8 {
        self.entry(offset)[dirent::ATTRS]
    }

    /// Mark the entry at `offset` deleted, the way a driver (and fsck) does it:
    /// stamp 0xE5 over the first name byte and change nothing else.
    pub fn delete_entry(&mut self, offset: u64) {
        self.write(offset, &[dirent::DELETED]);
    }

    /// Resolve a `/`-separated path of directories to its first cluster.
    /// The empty path is the root directory.
    pub fn dir_cluster(&self, path: &str) -> u32 {
        let mut cluster = self.geom.root_cluster;
        for component in path.split('/').filter(|c| !c.is_empty()) {
            let off = self.sfn(cluster, component);
            assert!(
                self.entry_attrs(off) & dirent::ATTR_DIRECTORY != 0,
                "{:?} is not a directory",
                component
            );
            cluster = self.entry_first_cluster(off);
        }
        cluster
    }

    /// A human-readable dump of a directory, used in assertion messages.
    pub fn ls(&self, cluster: u32) -> String {
        let mut out = String::new();
        for off in self.dir_slots(cluster) {
            let e = self.entry(off);
            if e[0] == 0 {
                let _ = writeln!(out, "  {:#010x}  <end of directory>", off);
                break;
            }
            if e[dirent::ATTRS] == dirent::ATTR_LFN {
                let _ = writeln!(
                    out,
                    "  {:#010x}  LFN  order={:#04x} checksum={:#04x}",
                    off,
                    e[dirent::LFN_ORDER],
                    e[dirent::LFN_CHECKSUM]
                );
            } else {
                let _ = writeln!(
                    out,
                    "  {:#010x}  {:?} attrs={:#04x} cluster={} size={}",
                    off,
                    String::from_utf8_lossy(&e[..11]),
                    e[dirent::ATTRS],
                    self.entry_first_cluster(off),
                    self.entry_size(off),
                );
            }
        }
        out
    }

    /// Which structure a byte offset falls in. Used to explain image diffs.
    pub fn describe_offset(&self, offset: u64) -> String {
        let g = &self.geom;
        if offset < u64::from(g.reserved_sectors) * u64::from(g.bytes_per_sector) {
            let sector = offset / u64::from(g.bytes_per_sector);
            let what = if sector == 0 {
                "boot sector"
            } else if sector == u64::from(g.fs_info_sector) {
                "FSInfo sector"
            } else {
                "reserved area"
            };
            return format!(
                "{} (sector {}, +{})",
                what,
                sector,
                offset % u64::from(g.bytes_per_sector)
            );
        }
        for fat in 0..g.num_fats {
            let range = g.fat_range(fat);
            if range.contains(&offset) {
                let cluster = (offset - range.start) / 4;
                return format!("FAT{} entry for cluster {}", fat, cluster);
            }
        }
        let cluster = 2 + (offset - g.data_offset()) / u64::from(g.bytes_per_cluster());
        let within = (offset - g.data_offset()) % u64::from(g.bytes_per_cluster());
        format!(
            "cluster {} +{} (dir slot {} +{} if a directory)",
            cluster,
            within,
            within / DIR_ENTRY_SIZE,
            within % DIR_ENTRY_SIZE
        )
    }
}

/// Pad a short name such as `"LOG.TXT"` or `"LOG     TXT"` to the 11-byte
/// on-disk form.
pub fn pad_sfn(name: &str) -> [u8; 11] {
    let mut out = [b' '; 11];
    // `.` and `..` are stored literally, not as a name/extension pair.
    if name == "." || name == ".." {
        out[..name.len()].copy_from_slice(name.as_bytes());
        return out;
    }
    if name.len() == 11 && !name.contains('.') {
        out.copy_from_slice(name.as_bytes());
        return out;
    }
    let (base, ext) = match name.split_once('.') {
        Some((b, e)) => (b, e),
        None => (name, ""),
    };
    assert!(base.len() <= 8 && ext.len() <= 3, "{:?} is not a short name", name);
    out[..base.len()].copy_from_slice(base.as_bytes());
    out[8..8 + ext.len()].copy_from_slice(ext.as_bytes());
    out
}

/// The checksum an LFN entry must carry to belong to the given short name.
pub fn lfn_checksum(sfn: &[u8; 11]) -> u8 {
    sfn.iter().fold(0_u8, |sum, &b| sum.rotate_right(1).wrapping_add(b))
}
