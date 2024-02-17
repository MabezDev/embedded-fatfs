# Changelog

All notable changes to this project will be documented in this file. For changes prior to [v0.1.0] please review the [rust-fatfs CHANGELOG](https://github.com/rafalh/rust-fatfs/blob/master/CHANGELOG.md).

## [Unreleased]

## [v0.1.0]

- Initial release of embedded-fatfs
- Add `async` support
- Remove `Drop` implementation for `File`, as there is no `async` `Drop` yet.
- Change the `Dir::*` methods from recursive to iterative.
- Add `StreamSlice` and `BufStream` helpers behind the `device` feature.
- Add a `BlockDevice` trait.
- Add restoring a `FileContext` to avoid costly seek operations.

[Unreleased]: https://github.com/mabezdev/embedded-fatfs/compare/v0.1.0...HEAD
[v0.1.0]: https://github.com/mabezdev/embedded-fatfs/releases/tag/v0.1.0