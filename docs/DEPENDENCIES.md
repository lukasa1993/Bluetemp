# Native and binary dependency audit

The application is `no_std`, `no_main`, with static task, TCP and DMA buffers. It does not allocate a heap. Ethernet uses the official pure-Rust HAL driver; TCP/IP and DHCP use Rust `embassy-net` / `smoltcp`.

## Device code

| Component | Origin and role |
| --- | --- |
| First-stage bootloader and mask-ROM routines | Already inside the ESP32. Startup, timing, UART, reset, flash and ROM cryptographic/checksum support are called by the HAL ecosystem. Not rebuilt or flashed by this project. |
| Second-stage bootloader | Precompiled ESP-IDF bootloader bundled in **espflash 4.6.0**. This is a separate startup binary, not an ESP-IDF application runtime. The release's [generation manifest](https://github.com/esp-rs/espflash/blob/v4.6.0/espflash/resources/bootloaders/manifest.yaml) identifies the `release/v6.1` branch and default configuration. OTA partition selection is present in the bundled ESP32 binary; automatic application-health rollback is not enabled. |
| `esp-rom-sys 0.1.5`, `libs/esp32/libesp_rom.a` | Precompiled C/assembly ROM support patches. Its [README](https://github.com/esp-rs/esp-hal/blob/esp-hal-v1.2.0/esp-rom-sys/README.md) identifies **ESP-IDF v5.3.1** ROM patch sources. The linked application pulls in `esp_rom_spiflash.c.obj` for flash support. |
| Rust target runtime | `xtensa-lx`, `xtensa-lx-rt`, generated interrupt/vector code, inline/assembly startup and context switching, Rust `core` and `compiler_builtins`. No C standard library is linked. |

The ROM patch archive contains `esp_rom_crc.c.obj`, `esp_rom_sys.c.obj`, `esp_rom_uart.c.obj`, `esp_rom_spiflash.c.obj`, `esp_rom_efuse.c.obj`, and `esp_rom_longjmp.S.obj`. The linker discards unneeded members/sections. The linker map is the authority for which objects are retained in a particular build.

The target linker uses `-nostartfiles` and Rust's `-nodefaultlibs`: GCC's CRT startup objects and libc/libgcc are not application dependencies. Required arithmetic helpers come from Rust compiler_builtins. Espressif's Xtensa GCC/binutils **esp-15.2.0_20250920** are build tools. LLVM/Clang **esp-20.1.1_20250829** are installed by espup; they are not firmware blobs.

Reference SHA256 values inspected during implementation:

```text
espflash v4.6.0 resources/bootloaders/esp32-bootloader.bin
136f160379c2d78b50b51431bffb8e8471e896fca8bc692ffd3980e7c0372a9e

esp-rom-sys 0.1.5 libs/esp32/libesp_rom.a
3162bebdf925974eaaef5b7d99c027394e7cf2735cda49857deda5e188a0dc42
```

The bootloader's image header/checksum can be adjusted by espflash for flash mode/frequency/size. The hash above identifies the **original bundled resource**, not a promise that a flash-readback bootloader has an identical header.

## Host-only components

espflash is a compiled host executable and normally uploads an additional precompiled **flasher stub to RAM** for flash operations. That stub does not persist as the application runtime. The `board-info` script uses `--no-stub`. The flash operation uses the normal espflash stub. See the pinned [ESP32 stub resource](https://github.com/esp-rs/espflash/blob/v4.6.0/espflash/resources/stubs/esp32.toml).

Python and its standard-library TLS/HTTP/crypto facilities run only on the development computer. RepoRigor is also a host executable and includes compiled Tree-sitter language grammars; it never enters the firmware. rustfmt, Clippy, cargo-llvm-cov and LLVM coverage tools are host development dependencies.

Cargo.lock contains optional/other-target packages such as `cc` and `esp-alloc` because Cargo locks more than the active target graph. They are not evidence that this ESP32 firmware compiles C source or enables allocation. Inspect the selected graph:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cargo +esp tree --locked --target xtensa-esp32-none-elf -e normal
```

No `esp-idf-sys`, `esp-idf-hal`, `esp-idf-svc`, `esp-radio`, `esp-wifi`, Wi-Fi/Bluetooth radio blob, FreeRTOS or lwIP appears in the active normal dependency graph. The name `esp-bootloader-esp-idf` denotes the Rust bootloader-format/partition support crate, not the IDF runtime.

## Embassy and community crates

HTTP routing, parsing, streaming bodies, responses and JSON use pure-Rust picoserve 0.20.0 with its Embassy feature. Sensor communication, timing and CRC use sht4x 0.2.0 / sensirion-i2c 0.4.0 over embedded-hal-async; fixed 1.31.0 provides conversion arithmetic. The crc and hex crates replace local checksum/decoder algorithms. Task supervision adds task-watchdog 0.1.2 with all platform/default features disabled; only its generic core and portable-atomic enter the target graph. These add no C or binary firmware dependency. Tokio, socket2, mio and embedded-hal-mock are host-test dependencies only. The selected firmware graph must be inspected separately from dev dependencies.

See [EMBASSY.md](EMBASSY.md) for the API-by-API audit and the rejected OTA alternatives.

## OTA API choice

Official flash storage and partition APIs are used. The 0.6.0 high-level `OtaUpdater` is not used: its `next_ota_part()` can call `Factory.ota_app_number()` when erased OTA data selects `Factory` but OTA0 actually booted. That subtracts the OTA subtype base from zero. Its record handling also returns an error for a torn record instead of recovering from the other valid record.

`src/ota_record.rs` implements only the two-slot selection record, CRC, and next-sector choice. It uses the actual booted partition from the official API, rejects invalid CRC/aborted records, preserves the running slot's newest record, and refuses sequence overflow. Record format and CRC were checked against Espressif's [bootloader selection code](https://github.com/espressif/esp-idf/blob/v5.5.1/components/bootloader_support/src/bootloader_common_loader.c). Host tests cover erased data, repeated A/B updates, CRC corruption, torn records and sequence exhaustion. Power-failure behavior on real flash remains untested.
