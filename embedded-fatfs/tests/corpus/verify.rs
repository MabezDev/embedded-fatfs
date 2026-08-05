//! A reference consistency checker for FAT32 images.
//!
//! This is the *detection* half of an fsck, written for clarity rather than for
//! an embedded budget — it keeps a `HashSet` of every allocated cluster, which
//! is exactly the cost a real lightweight checker has to avoid. It exists to
//! keep the corpus honest: every `fixed` image must come back clean, and every
//! `corrupt` image must not. It also serves as a precise statement of what the
//! repaired volume is required to satisfy.

use std::collections::HashSet;
use std::fmt;

use super::image::{dirent, lfn_checksum, Fat32Image, FAT_BAD, FAT_FREE};

/// End-of-chain markers, as a range.
const EOC_MIN: u32 = 0x0FFF_FFF8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// FAT copy `fat` disagrees with FAT #1 about `cluster`.
    FatMirrorMismatch {
        fat: u32,
        cluster: u32,
        fat0: u32,
        other: u32,
    },
    /// FAT entry 0 or 1 does not hold its fixed signature value.
    ReservedFatEntry { index: u32, found: u32, expected: u32 },
    /// A chain link is neither a valid cluster number nor an end-of-chain mark.
    BadLink { path: String, cluster: u32, value: u32 },
    /// The directory entry's first cluster is not a valid cluster number.
    BadFirstCluster { path: String, value: u32 },
    /// Size is non-zero but the entry names no first cluster.
    SizeWithoutCluster { path: String, size: u32 },
    /// The chain is too short to hold the file the size field describes.
    SizeExceedsChain { path: String, size: u32, clusters: u32 },
    /// The chain holds more clusters than the size field accounts for.
    ChainExceedsSize { path: String, size: u32, clusters: u32 },
    /// A cluster is reachable from more than one place.
    CrossLinked { path: String, cluster: u32 },
    /// A cluster is allocated but nothing references it.
    LostCluster { cluster: u32 },
    /// `.` or `..` points somewhere other than this directory or its parent.
    WrongDotEntry { path: String, found: u32, expected: u32 },
    /// A long-name run is not followed by the short name it belongs to.
    OrphanLfn { path: String, offset: u64 },
    /// A long-name entry's checksum does not match the short name behind it.
    LfnChecksumMismatch {
        path: String,
        offset: u64,
        found: u8,
        expected: u8,
    },
    /// FSInfo's free-cluster count disagrees with the FAT.
    FsInfoFreeCount { found: u32, expected: u32 },
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// Check an image and return everything wrong with it.
pub fn check(img: &Fat32Image) -> Vec<Problem> {
    let mut problems = Vec::new();
    let geom = *img.geom();

    // Reserved FAT entries.
    let expect0 = 0x0FFF_FF00 | u32::from(geom.media);
    let found0 = img.fat_get(0, 0);
    if found0 != expect0 {
        problems.push(Problem::ReservedFatEntry {
            index: 0,
            found: found0,
            expected: expect0,
        });
    }
    let found1 = img.fat_get(0, 1);
    if found1 < EOC_MIN {
        problems.push(Problem::ReservedFatEntry {
            index: 1,
            found: found1,
            expected: 0x0FFF_FFFF,
        });
    }

    // FAT mirrors.
    for fat in 1..geom.num_fats {
        for cluster in 0..=geom.max_valid_cluster() {
            let a = img.fat_get(0, cluster);
            let b = img.fat_get(fat, cluster);
            if a != b {
                problems.push(Problem::FatMirrorMismatch {
                    fat,
                    cluster,
                    fat0: a,
                    other: b,
                });
            }
        }
    }

    // Walk the tree, recording every cluster it reaches.
    let mut used = HashSet::new();
    let mut walker = Walker {
        img,
        used: &mut used,
        problems: &mut problems,
    };
    walker.claim_chain("/", geom.root_cluster);
    walker.visit_dir("", geom.root_cluster, 0);

    // Anything allocated that the walk never reached is lost.
    for cluster in 2..=geom.max_valid_cluster() {
        let value = img.fat_get(0, cluster);
        if value != FAT_FREE && value != FAT_BAD && !used.contains(&cluster) {
            problems.push(Problem::LostCluster { cluster });
        }
    }

    // FSInfo. A count of 0xFFFFFFFF means "unknown", which is always honest.
    let found = img.fsinfo_free_count();
    let expected = img.count_free_clusters();
    if found != super::image::fsinfo::UNKNOWN && found != expected {
        problems.push(Problem::FsInfoFreeCount { found, expected });
    }

    problems
}

struct Walker<'a> {
    img: &'a Fat32Image,
    used: &'a mut HashSet<u32>,
    problems: &'a mut Vec<Problem>,
}

impl Walker<'_> {
    /// Follow a chain, recording its clusters. Returns how many clusters it has.
    fn claim_chain(&mut self, path: &str, first: u32) -> u32 {
        let max = self.img.geom().max_valid_cluster();
        let mut cluster = first;
        let mut count = 0;
        loop {
            if !self.used.insert(cluster) {
                self.problems.push(Problem::CrossLinked {
                    path: path.to_string(),
                    cluster,
                });
                return count;
            }
            count += 1;
            let value = self.img.fat_get(0, cluster);
            if value >= EOC_MIN {
                return count;
            }
            if value < 2 || value > max {
                self.problems.push(Problem::BadLink {
                    path: path.to_string(),
                    cluster,
                    value,
                });
                return count;
            }
            cluster = value;
        }
    }

    fn visit_dir(&mut self, path: &str, cluster: u32, parent: u32) {
        let geom = *self.img.geom();
        let mut pending_lfn: Vec<u64> = Vec::new();

        for slot in self.img.dir_slots(cluster) {
            let entry = self.img.entry(slot);
            if entry[0] == 0 {
                break;
            }
            if entry[0] == dirent::DELETED {
                pending_lfn.clear();
                continue;
            }
            if entry[dirent::ATTRS] == dirent::ATTR_LFN {
                pending_lfn.push(slot);
                continue;
            }

            let raw_name: [u8; 11] = entry[..11].try_into().unwrap();
            let name = String::from_utf8_lossy(&raw_name).trim_end().to_string();
            let child = if path.is_empty() {
                format!("/{}", name)
            } else {
                format!("{}/{}", path, name)
            };

            let expected_checksum = lfn_checksum(&raw_name);
            for off in pending_lfn.drain(..) {
                let found = self.img.read(off + dirent::LFN_CHECKSUM as u64, 1)[0];
                if found != expected_checksum {
                    self.problems.push(Problem::LfnChecksumMismatch {
                        path: child.clone(),
                        offset: off,
                        found,
                        expected: expected_checksum,
                    });
                }
            }

            // The volume label lives in the root directory and owns no clusters.
            if entry[dirent::ATTRS] & dirent::ATTR_VOLUME_ID != 0 {
                continue;
            }

            let first = self.img.entry_first_cluster(slot);
            let size = self.img.entry_size(slot);
            let is_dir = entry[dirent::ATTRS] & dirent::ATTR_DIRECTORY != 0;

            if raw_name[..2] == *b". " || raw_name[..2] == *b".." {
                let is_dotdot = raw_name[1] == b'.';
                let expected = if is_dotdot { parent } else { cluster };
                // The spec says a `..` whose parent is the root holds 0, and
                // that is what this library writes, but plenty of other
                // implementations store the root's cluster number instead.
                // Both are accepted so the corpus can be pointed at volumes
                // this library did not create.
                let root_alias = is_dotdot && expected == 0 && first == geom.root_cluster;
                if first != expected && !root_alias {
                    self.problems.push(Problem::WrongDotEntry {
                        path: child,
                        found: first,
                        expected,
                    });
                }
                continue;
            }

            if first == 0 {
                if size != 0 {
                    self.problems.push(Problem::SizeWithoutCluster { path: child, size });
                }
                continue;
            }
            if first < 2 || first > geom.max_valid_cluster() {
                self.problems.push(Problem::BadFirstCluster {
                    path: child,
                    value: first,
                });
                continue;
            }

            let clusters = self.claim_chain(&child, first);
            if is_dir {
                // A `..` whose parent is the root holds 0, not the root's
                // cluster number.
                let as_parent = if cluster == geom.root_cluster { 0 } else { cluster };
                self.visit_dir(&child, first, as_parent);
            } else {
                let needed = size.div_ceil(geom.bytes_per_cluster());
                if needed > clusters {
                    self.problems.push(Problem::SizeExceedsChain {
                        path: child,
                        size,
                        clusters,
                    });
                } else if needed < clusters {
                    self.problems.push(Problem::ChainExceedsSize {
                        path: child,
                        size,
                        clusters,
                    });
                }
            }
        }

        for off in pending_lfn {
            self.problems.push(Problem::OrphanLfn {
                path: path.to_string(),
                offset: off,
            });
        }
    }
}
