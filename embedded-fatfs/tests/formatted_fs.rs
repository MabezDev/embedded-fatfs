// A fixture shared between test binaries, each of which uses only the part it
// needs — so anything unused here is unused by that binary, not dead.
#![allow(dead_code)]

use std::io;

use embedded_fatfs::{ChronoTimeProvider, FormatVolumeOptions, FsOptions, LossyOemCpConverter};
use embedded_io_adapters::tokio_1::FromTokio;

pub type CursorFs = embedded_fatfs::FileSystem<
    FromTokio<tokio::io::BufStream<io::Cursor<Vec<u8>>>>,
    ChronoTimeProvider,
    LossyOemCpConverter,
>;

/// Fill `buf` with deterministic pseudo-random bytes (xorshift64*).
pub fn pseudo_random_fill(buf: &mut [u8]) {
    let mut state: u64 = 0xDEAD_BEEF_CAFE_BABE;
    for b in buf.iter_mut() {
        state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        *b = ((state >> 32) ^ (state & 0xFFFF_FFFF)) as u8;
    }
}

/// Pre-allocate a zeroed vector of `total_bytes` and fill it with random bytes.
pub fn random_storage(total_bytes: u64) -> Vec<u8> {
    let mut bytes = vec![0u8; total_bytes as usize];
    pseudo_random_fill(&mut bytes);
    bytes
}

/// Create a formatted [`CursorFs`] from an already-filled `Vec<u8>`.
///
/// The storage is consumed: `format_volume` writes the filesystem structures
/// in place, then `FileSystem::new` takes ownership of the stream.
pub async fn make_cursor_fs(storage: Vec<u8>, format_opts: FormatVolumeOptions) -> CursorFs {
    let mut buf = FromTokio::new(tokio::io::BufStream::new(io::Cursor::new(storage)));
    embedded_fatfs::format_volume(&mut buf, format_opts)
        .await
        .expect("format_volume");
    let options = FsOptions::new()
        .time_provider(ChronoTimeProvider::new())
        .oem_cp_converter(LossyOemCpConverter::new());
    embedded_fatfs::FileSystem::new(buf, options)
        .await
        .expect("mount")
}
