# esp-spi

An `embedded-fatfs` example using SPI SD card (`sdspi`) on any supported ESP chip.

## Quick start

Cargo aliases are configured for each chip. From this directory, just run:

```sh
cargo esp32c6
```

This will build, flash, and monitor. Replace `esp32c6` with your target chip:

| Alias | Target | Toolchain |
|-------|--------|-----------|
| `cargo esp32` | `xtensa-esp32-none-elf` | `esp` |
| `cargo esp32c2` | `riscv32imc-unknown-none-elf` | `stable` |
| `cargo esp32c3` | `riscv32imc-unknown-none-elf` | `stable` |
| `cargo esp32c5` | `riscv32imac-unknown-none-elf` | `stable` |
| `cargo esp32c6` | `riscv32imac-unknown-none-elf` | `stable` |
| `cargo esp32c61` | `riscv32imac-unknown-none-elf` | `stable` |
| `cargo esp32h2` | `riscv32imac-unknown-none-elf` | `stable` |
| `cargo esp32s2` | `xtensa-esp32s2-none-elf` | `esp` |
| `cargo esp32s3` | `xtensa-esp32s3-none-elf` | `esp` |

## Build only (no flash)

If you just want to compile without a device connected:

```sh
cargo build --features esp32c6 --target riscv32imac-unknown-none-elf
```

## Toolchain

RISC-V chips (`esp32c*`, `esp32h2`) work with standard stable Rust.

Xtensa chips (`esp32`, `esp32s2`, `esp32s3`) require the [`esp` Rust toolchain](https://github.com/esp-rs/rust-build) installed via [`espup`](https://github.com/esp-rs/espup). Use `+esp` to select it:

```sh
cargo +esp esp32s3
```
