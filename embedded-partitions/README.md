## embedded-partitions

`no_std`-friendly partition-table parsing for use with
[`embedded-io-async`], [`block-device-adapters`] and [`embedded-fatfs`].

### Supported schemes

| Scheme | Module | Status |
| --- | --- | --- |
| Master Boot Record (MBR) | [`mbr`](src/mbr/) | supported |
| GUID Partition Table (GPT) | — | not implemented |

The crate name is intentionally generic to leave room for additional
schemes in the future without further renames.

### What it provides

* `mbr::Mbr` — wraps an [`embedded_io_async`] handle, parses the MBR,
  and validates the boot signature. `Read + Seek` is enough for
  parsing; `Read + Write + Seek` is needed to open partitions.
* `mbr::PartitionEntry` / `mbr::PartitionType` / `mbr::Chs` — inspect
  the parsed table (bootable flag, type, LBA range, CHS, byte offsets)
  or build entries from scratch and serialise them with `to_bytes`.
* `mbr::Mbr::open_partition` — borrow a partition as
  `StreamSlice<&mut IO>` (recommended).
* `mbr::Mbr::into_partition` — consume the `Mbr` and return an owned,
  `'static`-friendly `StreamSlice<IO>`.
* `mbr::Scheme` / `mbr::Layout` — auto-detect whether a device is
  partitioned (MBR), unpartitioned (FAT BPB at LBA 0, "superfloppy"),
  or neither. `Scheme::open(io)` returns the right handle in one
  step; `Scheme::detect(&mut io)` is the cheap classify-only variant.

### Choosing between `open_partition` and `into_partition`

Default to the **borrowing variant (`open_partition`)**. The `Mbr`
remains usable once the slice is dropped, and `&mut slice` plugs into
consumers that take their storage by value (such as
`embedded_fatfs::FileSystem::new`). It's also the only way to access
several partitions in sequence.

Reach for the **owning variant (`into_partition`)** only when the
slice has to outlive the current scope — spawning into a task,
returning from a function, storing in a `'static` field.

### Example

```rust,ignore
use embedded_partitions::mbr::Mbr;
use embedded_fatfs::{FileSystem, FsOptions};

let mut mbr = Mbr::new(io).await?;

for (idx, p) in mbr.iter_used() {
    println!("{idx}: type {:?} starts at LBA {}", p.partition_type(), p.start_lba());
}

let idx = mbr
    .iter_used()
    .find(|(_, p)| p.is_fat())
    .map(|(i, _)| i)
    .unwrap();

let mut slice = mbr.open_partition(idx).await?;
let fs = FileSystem::new(&mut slice, FsOptions::new()).await?;
```

For sequential access across multiple partitions, see
`examples/multi_partition.rs`.

### Trying it out

The runnable examples expect a `disk.img` in the current directory. A
generator example builds one:

```sh
cd embedded-partitions
cargo run --example make_disk_image     # writes ./disk.img (16 MiB)
cargo run --example inspect             # print the partition table
cargo run --example mount_partition     # mount the first FAT partition
cargo run --example multi_partition     # walk every used partition
cargo run --example detect              # auto-detect the layout (MBR vs. superfloppy)
```

The generated image contains one 8 MiB FAT16 partition (with a few
sample files) and one 7 MiB pseudo-Linux partition (with a magic
marker), so every example exercises a distinct code path.

### Limitations

* Only the four primary MBR entries are enumerated. Extended
  partitions (`PartitionType::ExtendedChs` / `ExtendedLba`) point to
  chained EBRs that this crate does not follow.
* MBR LBAs are 512-byte sectors. On 4Kn drives, use
  `PartitionEntry::start_byte_offset(sector_size)` and build a slice
  manually.
* Opening a partition needs `Read + Write + Seek` storage, because
  [`embedded-fatfs`]'s `FileSystem` consumes a `ReadWriteSeek`. The
  partition table itself parses from `Read + Seek`-only storage; only
  read-only *mounting* is unsupported.
* The table is otherwise accepted as-is — e.g. a partition with
  `start_lba == 0` (overlapping the MBR) is not rejected. Check this
  yourself if it matters.

[`embedded-io-async`]: https://crates.io/crates/embedded-io-async
[`block-device-adapters`]: https://crates.io/crates/block-device-adapters
[`embedded-fatfs`]: https://crates.io/crates/embedded-fatfs
