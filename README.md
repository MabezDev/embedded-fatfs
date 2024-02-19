# embedded-fatfs

[![CI Status](https://github.com/mabezdev/embedded-fatfs/actions/workflows/ci.yml/badge.svg)](https://github.com/mabezdev/embedded-fatfs/actions/workflows/ci.yml)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE.txt)
[![crates.io](https://img.shields.io/crates/v/embedded-fatfs)](https://crates.io/crates/embedded-fatfs)
[![Documentation](https://docs.rs/embedded-fatfs/badge.svg)](https://docs.rs/embedded-fatfs)

![Minimum rustc version](https://img.shields.io/badge/rustc-1.75+-green.svg)

This repository contains various crates useful interacting with FAT filesystems and SD cards:

* [`embedded-fatfs`]: A FAT filesytem implementation.
* [`block-device-driver`]: A crate with a trait for handling block devices.
* [`block-device-adapters`]: Helpers for dealing with block devices and partitions.
* [`sdspi`]: A SPI SD card driver.

[ `embedded-fatfs` ]: https://crates.io/crates/embedded-fatfs
[ `block-device-driver` ]: https://crates.io/crates/block-device-driver
[ `block-device-adapters` ]: https://crates.io/crates/block-device-adapters
[ `sdspi` ]: https://crates.io/crates/sdspi

## Examples

Examples that can be run on your host machine are found in each crates `examples` folder. For full embedded examples, see the `examples` directory at the root of the repository.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the
work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
