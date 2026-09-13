# Embassy and community API audit

Checked against published crate sources on 2026-09-14. The policy is to use an Embassy API where it supports this board, then a compatible no-std community crate. Application behavior and necessary hardware integration remain in this repository.

| Function | Implementation now | Removed or retained |
| --- | --- | --- |
| Scheduling and time | `esp-rtos 0.4.0` Embassy integration, `embassy-executor 0.10.0`, `embassy-time 0.5.1` | Native tasks, `Ticker`, `Delay`, and `with_timeout`; no custom scheduler or delay loop. |
| Ethernet MAC/DMA | Official `esp-hal 1.2.1` async Ethernet implementing `embassy-net-driver 0.2.0` | No private MAC driver. |
| DHCP, TCP and reconnect | `embassy-net 0.9.1`, permanent `Runner::run()` | Stack owns DHCP recovery. Link/config up/down wait futures replace the application’s 500 ms status polling. The 60-second log includes the current address, including lease changes without a down transition. |
| HTTP | Community `picoserve 0.20.0`, with its `embassy` feature | Native `Server::serve`, router, services, body reader, responses and timeouts own HTTP. Two Embassy workers use a thin bounded TCP accept loop to expose real progress and enforce an absolute connection deadline. There is no application HTTP parser or response-header writer. |
| JSON | picoserve `Json` and `serde 1.0.229` | No manual JSON formatting. |
| SHT40 | Community `sht4x 0.2.0` async driver, `embedded-hal-async 1.0.0`, `embassy_time::Delay` | Driver owns commands, conversion timing, CRC checking, fixed-point conversion and soft reset. Application keeps humidity clamping, freshness and retry policy. |
| Latest reading | `embassy_sync::watch::Watch` | Replaces the application `Mutex<Cell<Reading>>` cache. No queued old readings. |
| OTA coordination | Embassy async `Mutex::try_lock`, `Signal` and an independent timer task | Reboot is scheduled two seconds after commit, independently of HTTP response success. |
| OTA flash and partitions | Official `esp-storage 0.10.0` and `esp-bootloader-esp-idf 0.6.0` | Flash operations are synchronous; the application yields between sectors. Wrapping synchronous flash in an async adapter would not make the hardware operations nonblocking. |
| Watchdog | Community `task-watchdog 0.1.2` generic core, official `esp_hal::rtc_cntl::Rwdt`, Embassy `Instant` / `Ticker` / critical-section mutex | Core performs per-task deadline checks; thin adapters supply the current Embassy clock and ESP32 hardware. No legacy platform features enabled. |
| Authentication and checksums | RustCrypto `hmac`/`sha2`, `crc 3.4.0`, `hex 0.4.3` | No custom cryptographic, CRC or hexadecimal decoder algorithms. |

Picoserve and sht4x are **community crates, not Embassy-owned drivers**. They integrate through the actual Embassy networking/runtime APIs and the standard embedded-hal async traits. Both are allocation-free in this firmware. The picoserve manifest explicitly depends on embassy-net 0.9.1, embassy-time 0.5.1 and embedded-io-async 0.7.0, matching our pins. It requires heapless 0.9.3, so that direct pin was updated too.

## Remaining small exceptions

**Watchdog integration.** The published [embassy-task-watchdog 0.1.0 manifest](https://docs.rs/crate/embassy-task-watchdog/0.1.0/source/Cargo.toml.orig) unconditionally uses Cortex-M dependencies and the Cortex-M executor platform; its platform integrations are RP and STM32. It is not a supported Xtensa ESP32 option. The inspected [task-watchdog 0.1.2 core](https://docs.rs/task-watchdog/0.1.2/task_watchdog/) works without its older HAL/Embassy platform features. We use `default-features = false`, implement its `Clock` and `HardwareWatchdog` traits with current APIs, and run checks using Embassy. Its core compares task discriminants, so the five IDs are unit enum variants; tests verify independent deadlines. Its `check()` can resume feeding after late progress, so our runner deliberately latches starvation by awaiting forever without another check. The RTC then resets even if the executor cannot run. The HAL itself disables watchdogs at initialization; we explicitly re-enable the RTC watchdog. See [the pinned HAL source](https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.2.1/esp-hal/src/rtc_cntl).

**HTTP worker supervision.** Picoserve’s `listen_and_serve` has no per-connection progress callback and can wait forever on an idle accept. Our small accept loop uses `embassy_time::with_timeout` (5-second idle accept, 190-second complete connection) and still hands the socket to its native `Server::serve`. A timeout on real work returns the worker to accepting requests. A separately scheduled heartbeat would conceal a stalled worker. Picoserve’s native body timeout override permits a 180-second accepted upload while rejected/unconsumed bodies retain the 10-second default. The shared flash mutex uses nonblocking acquisition and a committed flag so a second upload cannot queue behind the first or overwrite it before reboot.

**PHY adapter.** The official [esp-hal async Ethernet example](https://github.com/esp-rs/esp-hal/blob/esp-hal-v1.2.0/examples/async/embassy_ethernet/src/main.rs) itself wraps `GenericPhy` in an Embassy timer to avoid continuously waking the executor and to check idle link transitions. Our adapter follows that pattern. Version 1.2.1 exposes the generic PHY, not a dedicated LAN8720A implementation. The adapter also resolves the highest mutually advertised mode to correct its mixed-speed duplex selection. This is a HAL integration workaround, not a second Ethernet stack.

**Upload policy.** Picoserve parses the HTTP request. The application’s middleware rejects ambiguous/duplicate lengths, unsupported Transfer-Encoding/Expect headers and malformed/duplicate authentication fields. The inspected picoserve parser takes the first Content-Length and treats a missing/unparseable length as zero; it does not enforce our authenticated fixed-length upload contract. That policy must remain in an application layer. The application also owns the routes, text format, stale-reading status, image identity and authentication domain.

**ESP boot selection records.** This is the one substantial format-specific exception, implemented in `src/ota_record.rs` using the community CRC crate. The alternatives were checked directly:

- [embassy-boot 0.7.0](https://docs.rs/crate/embassy-boot/0.7.0/source/README.md) uses ACTIVE, DFU and BOOTLOADER STATE partitions and writes swap/boot magic with `FirmwareUpdater::mark_updated()` / `mark_booted()`. Its published platform integrations are NRF, RP and STM32. No supported ESP32 integration was found. Its state protocol is not understood by the standard ESP-IDF second-stage bootloader. Substituting only `FirmwareUpdater` would produce an update the installed bootloader cannot activate. See the [Embassy bootloader documentation](https://embassy.dev/book/#_bootloader).
- Official [esp-bootloader-esp-idf 0.6.0 OtaUpdater source](https://docs.rs/crate/esp-bootloader-esp-idf/0.6.0/source/src/ota_updater.rs): when erased otadata reports Factory but OTA0 actually booted, `next_ota_part()` reaches the collision branch and calls `Factory.ota_app_number()`, subtracting the OTA subtype base from zero. Its [record reader](https://docs.rs/crate/esp-bootloader-esp-idf/0.6.0/source/src/ota.rs) also errors on a torn record rather than recovering using the other valid record. The application still uses the official partition parser and actual booted-partition API.
- Community [esp-ota 0.2.2](https://docs.rs/crate/esp-ota/0.2.2/source/Cargo.toml.orig) calls `esp-idf-sys` OTA APIs, which violates the no-ESP-IDF-runtime requirement.
- Community [esp-ota-nostd 0.1.0](https://docs.rs/crate/esp-ota-nostd/0.1.0/source/src/lib.rs) uses embedded-io-async 0.6. More seriously, it sets a global `IS_UPDATING` flag with no cancellation/error cleanup, so an interrupted upload prevents retries until reboot. It activates after reading EOF without our independent readback-verification step; its record reader does not support initially erased otadata. An adapter would not repair these behavioral problems.

A supported ESP32 port of embassy-boot, or a corrected official ESP OTA helper, would allow revisiting this exception. A private bootloader port or a disguised local copy of a driver would add custom code rather than remove it.

## Primary API references

- [Embassy network API](https://docs.rs/embassy-net/0.9.1/embassy_net/struct.Stack.html)
- [Picoserve 0.20.0](https://docs.rs/picoserve/0.20.0/picoserve/), [published manifest](https://docs.rs/crate/picoserve/0.20.0/source/Cargo.toml.orig)
- [SHT4x async driver](https://docs.rs/sht4x/0.2.0/sht4x/struct.Sht4xAsync.html), [source](https://github.com/sirhcel/sht4x)
- [Embedded HAL async traits](https://github.com/rust-embedded/embedded-hal)
- [Embassy Watch](https://docs.rs/embassy-sync/0.8.0/embassy_sync/watch/struct.Watch.html)

Validation covers the actual shared picoserve router through host TCP sockets and the actual async SHT4x driver through embedded-hal-mock. It does not establish ESP32 hardware success. See `VALIDATION.md` for the build, quality gate and outstanding hardware checks.
