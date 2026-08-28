//! A reference consistency checker for FAT32 images.
//!

use std::collections::HashSet;
use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

use regex::Regex;
use tempfile::NamedTempFile;

use super::image::{dirent, lfn_checksum, Fat32Image, FAT_BAD, FAT_FREE};

/// End-of-chain markers, as a range.
const EOC_MIN: u32 = 0x0FFF_FFF8;
/// The two flag bits a driver may borrow from FAT[1]: clear means dirty / hard error respectively.
const CLN_SHUT_BIT: u32 = 0x0800_0000;
const HRD_ERR_BIT: u32 = 0x0400_0000;

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
    /// `fsck.fat` found something the checks above did not.
    FsckError { status: Option<i32>, output: String },
    /// `fsck.fat` could not be run, and `EMBEDDED_FATFS_REQUIRE_FSCK` said it had to be.
    FsckUnavailable { reason: String },
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// Check an image and return everything wrong with it.
///
/// This is the detection half of fsck/repair. It walks the filesystem, recording all used clusters (in a simple hash).
///
/// Finally, it hands the image to `fsck.fat -n` via a tempfile, on unix hosts with dosfstools installed. It then
/// parses the fsck result, and ensures that fsck agrees the volume is valid, catching any issues we may have missed (or
/// bugs (either here, or in the corpus image creation code).
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
    // FAT[1] is an EOC mark, except that a driver may use its top two bits to record that the volume is dirty or has
    // had a hard error (MS FAT spec pg. 19). Those are mount state, not corruption, so put them back before comparing.
    let found1 = img.fat_get(0, 1);
    if found1 | CLN_SHUT_BIT | HRD_ERR_BIT < EOC_MIN {
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

    problems.extend(fsck_fat(img));

    problems
}

// ####################################################################################################################
//                                               FSCK.fat HARNESS
// ####################################################################################################################

/// Where `fsck.fat` might be, in the order worth trying.
///
/// Many distros keep fsck in `/sbin` or `/usr/sbin`, which are typically not on `$PATH` for an ordinary user.
const FSCK_CANDIDATES: &[&str] = &[
    "fsck.fat",
    "/sbin/fsck.fat",
    "/usr/sbin/fsck.fat",
    "fsck.vfat",
    "/sbin/fsck.vfat",
    "/usr/sbin/fsck.vfat",
];

/// If this env var is set, we fail if we don't find an fsck.fat binary. For CI
const REQUIRE_ENV: &str = "EMBEDDED_FATFS_REQUIRE_FSCK";

/// The first binary that runs, resolved once per test binary.
fn fsck_path() -> Option<&'static Path> {
    static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| {
        FSCK_CANDIDATES
            .iter()
            .find(|candidate| Command::new(candidate).arg("-V").output().is_ok())
            .map(PathBuf::from)
    })
    .as_deref()
}

/// Write `img` to temp file, and run `fsck.fat -n`.
///
/// Returns `Result` to simplify error handling with ?
fn run_fsck(fsck: &Path, img: &Fat32Image) -> Result<Output, String> {
    let mut image = NamedTempFile::new().map_err(|e| format!("could not create a temporary image: {e}"))?;
    image
        .write_all(img.as_bytes())
        .map_err(|e| format!("could not write {}: {e}", image.path().display()))?;
    Command::new(fsck)
        .arg("-n")
        .arg(image.path())
        .output()
        .map_err(|e| format!("{} did not run: {e}", fsck.display()))
}

/// Check `img` with an external call to `fsck.fat -n`
pub fn fsck_fat(img: &Fat32Image) -> Option<Problem> {
    let Some(fsck) = fsck_path() else {
        return std::env::var_os(REQUIRE_ENV)
            .is_some()
            .then(|| Problem::FsckUnavailable {
                reason: format!("none of {:?} could be run; is dosfstools installed?", FSCK_CANDIDATES),
            });
    };

    let out = match run_fsck(fsck, img) {
        Ok(out) => out,
        Err(reason) => return Some(Problem::FsckUnavailable { reason }),
    };

    // exit 2 means fsck rejected our invocation and never looked at the filesystem
    match out.status.code() {
        Some(0) => return None,
        Some(2) => {
            return Some(Problem::FsckUnavailable {
                reason: format!("{} usage error: {}", fsck.display(), get_cmd_output(&out)),
            })
        }
        _ => {} // Error found, parse below
    }

    let report = get_cmd_output(&out);
    // len > 0 forces an empty report to be an error, to catch e.g. killed fsck.vfat
    if report.len() > 0 && only_expected_fsck_lines(&report) {
        return None;
    }
    Some(Problem::FsckError {
        status: out.status.code(),
        output: report,
    })
}

/// Ensure no line `fsck.fat` printed indidcates a broken volume.
///
/// There are several expected message or "error" lines printed by fsck that do no indicate an error, or at least not
/// one we care about, so we can't just check for an empty log. We a regex allowlist to eliminate known-irrelevant
/// erros, and assume any additional lines are due to a broken FS image.
fn only_expected_fsck_lines(report: &str) -> bool {
    // Adding to this list overrides fsck's objection, so only add lines verified to be harmless or deliberate. Also,
    // expect this to break if they change their error messages (sorry!). All lines are trimmed before matching and
    // blank lines after trimming are automatically accepted so no need for those in this list.
    const EXPECTED_LINES: &[&str] = &[
        r"^fsck\.fat .*$",                           // Version banner
        r"^.+: \d+ files, \d+/\d+ clusters$",        // Closing summary line
        r"^Leaving filesystem unchanged\.$",         // `-n` confirming it wrote nothing
        r"^Dirty bit is set\..*$",                   // The dirty flag is set, which these tests set on purpose.
        r"^Automatically removing dirty bit\.$",     // fsck's follow-on to the line above
        r"^Free cluster summary uninitialized\b.*$", // An unknown free-cluster count, which we set for dirty FSes
    ];

    // Cache compiled RE in a static
    static EXPECTED: OnceLock<Vec<Regex>> = OnceLock::new();
    let expected = EXPECTED.get_or_init(|| {
        EXPECTED_LINES
            .iter()
            .map(|pattern| Regex::new(pattern).expect("EXPECTED_LINES pattern does not compile"))
            .collect()
    });
    report
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .all(|line| expected.iter().any(|re| re.is_match(line)))
}

fn get_cmd_output(out: &std::process::Output) -> String {
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    s
}

// ####################################################################################################################
//                                           Internal walker-based FSCK
// ####################################################################################################################

struct Walker<'a> {
    img: &'a Fat32Image,
    used: &'a mut HashSet<u32>,
    problems: &'a mut Vec<Problem>,
}

impl Walker<'_> {
    /// Follow a chain, recording its clusters. Returns how many clusters it has.
    fn claim_chain(&mut self, path: &str, first: u32) -> u32 {
        let mut cluster = first;
        let mut count = 0;
        loop {
            let already_used = !self.used.insert(cluster);
            if already_used {
                self.problems.push(Problem::CrossLinked {
                    path: path.to_string(),
                    cluster,
                });
                return count;
            }
            count += 1;
            let value = self.img.fat_get(0, cluster);
            if value >= EOC_MIN {
                // End of chain found, return length
                return count;
            }
            // 2 is first data cluster
            if value < 2 || value > self.img.geom().max_valid_cluster() {
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

            let first_cluster = self.img.entry_first_cluster(slot);
            let size = self.img.entry_size(slot);
            let is_dir = entry[dirent::ATTRS] & dirent::ATTR_DIRECTORY != 0;

            // . and ..
            if raw_name[..2] == *b". " || raw_name[..2] == *b".." {
                let is_dotdot = raw_name[1] == b'.';
                let expected = if is_dotdot { parent } else { cluster };
                // The spec says a `..` pointing at the root should have cluster 0, which we below pass as parent when
                // descending from the root. We enforce this, but images from other implementations might not pass this
                // test and be perfectly valid. But, we're only here to check our own homework...
                if first_cluster != expected {
                    self.problems.push(Problem::WrongDotEntry {
                        path: child,
                        found: first_cluster,
                        expected,
                    });
                }
                continue;
            }

            if first_cluster == 0 {
                if size != 0 {
                    self.problems.push(Problem::SizeWithoutCluster { path: child, size });
                }
                continue;
            }
            if first_cluster < 2 || first_cluster > geom.max_valid_cluster() {
                self.problems.push(Problem::BadFirstCluster {
                    path: child,
                    value: first_cluster,
                });
                continue;
            }

            // mark entries clusters used & get true chain length
            let clusters = self.claim_chain(&child, first_cluster);
            if is_dir {
                // Ensure we use as parent 0 for root cluster
                let this_dir_cluster = if cluster == geom.root_cluster { 0 } else { cluster };
                self.visit_dir(&child, first_cluster, this_dir_cluster);
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
