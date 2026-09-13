# Validation and hardware acceptance

Implementation checked on 2026-09-14 on an Apple Silicon Mac. The WT32-ETH01 and sensor had **not arrived**. No serial device was reset, no firmware was flashed, and no hardware success is claimed.

## Completed software checks

| Check | Result |
| --- | --- |
| `./scripts/dev build` | Passed. Full release build and link for `xtensa-esp32-none-elf`, Xtensa Rust 1.95.0.0, locked dependencies. |
| `espflash save-image` via build script | Passed. Application-only image **279,968 bytes**, fits each **2,031,616-byte** slot. Header/project/chip and appended SHA256 validated by the script. |
| `cargo +stable fmt --check` | Passed. |
| `cargo +esp clippy --release --locked -Zbuild-std=core -- -D warnings` | Passed. |
| `./scripts/dev test` | Passed: **13 Rust integration tests and 3 Python tests**. |
| Sensor integration | Actual sht4x async driver over embedded-hal-mock: 0x44 address, 0xfd command, 9 ms async delay, CRC vector, conversion endpoints/clamping and all 48 single-bit sample corruptions; separate freshness boundary tests. |
| HTTP integration | Actual picoserve router/server on localhost TCP: text/JSON/health/ready/404, initial null values, stale/missing-address 503 responses, busy upload 503, authenticated 5,000-byte body streamed in 1/7/1,024-byte fragments or coalesced, truncated/corrupt payload rejection, ambiguous lengths and malformed authentication headers. The update service is a host test double; actual flash remains untested. |
| Watchdog core | Actual task-watchdog core with fake clock/hardware: startup grace, each of five tasks independently blocks feeding at its deadline, and all-task progress over ten simulated minutes. RTC timing/reset and executor failure remain untested. |
| OTA logic | Independent Python HMAC vector; all signature-bit corruptions; wrong digest/key/length; wrong chip/project/truncated image; CRC vectors cross-checked against Python zlib with ESP ROM seed. |
| Boot selection | Erased metadata, repeated alternating slots, preservation of previous record, torn writes, invalid/aborted records, exhausted sequence numbers. These are host simulations, not flash power-loss tests. |
| Python upload client | Real localhost HTTP server received exact image bytes and verified the generated length/digest/HMAC headers. No ESP32 was involved. |
| Host LLVM coverage | Shared library, including the actual HTTP application handlers: **246/248 lines (99.19%)**. Firmware-only modules are absent from that measurement. |
| Native dependency audit | Active Cargo graph and link map inspected. Only the required ROM flash patch object is linked from the precompiled C archive; GCC CRT startup objects are excluded. No prohibited application runtime or radio library linked. |

The supplied reference `http://192.168.19.129` was read without modification. It returned five dash-separated text fields, including apparent temperature/humidity values. This informed the text endpoint shape, not a compatibility claim about undocumented fields.

## RepoRigor result: NOT PASSED

Used the checkout requested by the user: `/Users/l/_DEV/clean-code/reporigor`, commit `281cde7fd752630f4c218f4817f8c4b6021a1b8a`. Its existing executable was older than its source/documentation, so the current executable was rebuilt from that checkout.

Exact final gate command, invoked by `./scripts/dev quality`:

```sh
/Users/l/_DEV/clean-code/reporigor/target/release/reporigor check . \
  --language rust --backend native --allow-project-exec \
  --cargo /Users/l/.rustup/toolchains/esp/bin/cargo \
  --coverage /Users/l/_DEV/Bluetemp/target/quality/lcov.info \
  --run-mutations --test-command './scripts/dev test' --format json
```

Backend: **rust-native 0.1.0**. Exit: **2**. Report: `target/quality/reporigor.json`.

- 15 Rust files, 58 functions, zero parse errors or diagnostics.
- No CRAP threshold violations in measured functions; KISS, YAGNI and coupling rules passed.
- The default cohesion rule flags `src/protocol.rs` (0.0 versus a 0.1 minimum). Its module-level helpers are not connected by the tool's qualified direct-reference graph. This finding remains unwaived.
- Two informational duplicate groups; the default DRY rule does not fail on them.
- **54 mutations executed: 34 killed, 20 survived**, score **62.96%**, below the default 80% minimum. All survivors are in firmware-only HTTP flash coordination, Ethernet, PHY, sensor-task, watchdog clock adapter or flash-I/O paths. The shared text-response freshness/IP condition is now covered and its mutation is killed. No survivor has been waived as equivalent.
- **41 functions lack an unambiguous coverage match**: 34 unmatched and 7 ambiguous. One `crap.maximum` check is explicitly omitted because of missing coverage. These figures differ from LLVM's line coverage because the native adapter needs a unique function match.


No thresholds were weakened, source files hidden, or baselines introduced. Mutation runs use fresh temporary Cargo target directories so timestamp restoration cannot reuse a previously compiled mutant. This report supersedes the previous refactor's 64% score; adding firmware-only supervision/HTTP coordination increased the untested mutation surface. The sources were restored by the mutation runner, then the final release image was rebuilt successfully.

The complete sources build and pass normal tests. The RepoRigor gate is still **not passed**. A complete pass needs meaningful coverage of device-only paths and resolution of the cohesion finding. No production or hardware validation is claimed.

Final application-only image SHA256:

```text
fd652bd3c046089d6f7eb0431eb15662cd7a0d6f182a16d4c084e5cca1f20edf
```

The detailed [Embassy/community audit](EMBASSY.md) records the official and community replacements and the small hardware-specific exceptions.

## Hardware tests still required

1. **Identify the board.** Photograph module/PCB markings and check the schematic variant. Connect a 3.3 V logic UART, enter download mode, run `./scripts/dev board-info --port PORT --manual`. Record chip revision, MAC, crystal and flash size. This build requires original ESP32 revision ≥v3.0 and at least 4 MB flash. An older revision needs a separately researched supported stack; do not force this image onto it.
2. **First serial boot.** Run `./scripts/dev flash --port PORT --manual`, remove GPIO0–GND, reset EN and monitor at 115200 baud. Verify the bootloader loads OTA0 and the application reports its version and chip revision. Verify GPIO16 goes high and GPIO0 receives a 50 MHz input if measurement equipment is available.
3. **Short sensor connection first.** Connect the SHT40 with 3.3 V supply, common ground, SDA32/SCL33, pull-ups and decoupling. Verify the initial reading, subsequent readings every five seconds, plausible units and no CRC/I²C errors. Compare with a reference instrument. Check supply voltage at the sensor.
4. **Final sensor cable.** Use the actual cable length, routing and supply, including under 1 m if that is the final choice. Run at least 24 hours under expected environmental/electrical conditions. Inspect `errors`, freshness, temperature/humidity continuity, and remote supply voltage. If possible, measure SDA/SCL rise time, ringing and logic levels at the far end. Require zero unexplained bus/CRC failures before treating the cable as reliable. Slower firmware timing alone does not guarantee this result.
5. **DHCP and HTTP.** Boot with the cable inserted and DHCP available. Verify link and address logs, then `GET /`, `/api/reading`, `/ready`, and `/health` from another machine. Ensure the address is leased, not assumed to equal the reference device's address. Verify wrong paths return 404.
6. **Cable recovery.** Boot with no Ethernet cable; insert it later. Then repeatedly remove it for at least five seconds and reconnect. Verify link-down/address removal logs, fresh DHCP configuration and resumed HTTP responses without resetting the board. Repeat removal during a TCP request. The 60-second ticker must continue.
7. **DHCP outage.** Boot with no DHCP server and later enable it. Verify continued sensor/ticker operation and eventual DHCP recovery. Test lease renewal and address changes with a short test lease.
8. **Sensor faults.** Power down before altering wiring. Disconnect/reconnect the sensor and repeat tests. While unavailable, `/api/reading` must return 503 with `fresh:false` and an error count; unavailable initial values must be null. After recovery, values must refresh. A physically stuck-low bus may require correcting wiring or power cycling the sensor.
9. **Normal OTA twice.** Keep the generated key. Change the package version, update Cargo.lock, run `./scripts/dev ota http://DEVICE_IP`. Verify the response and reboot, new version and alternate boot partition. Repeat to exercise both directions; recheck sensor and Ethernet. Use concurrent HTTP reads to exercise the second worker; the runner/sensor/ticker should remain scheduled between flash operations. Attempt a concurrent OTA and confirm 503.
10. **Rejected/interrupted OTA.** Send a missing/wrong signature, invalid length, wrong chip/project, changed image bytes, truncated body and stalled request. Disconnect Ethernet during upload. Confirm the prior application still boots and that a later valid upload works. Test interruption while writing each OTA metadata sector only with a serial recovery path available. Actual power-failure behavior has not been tested.
11. **Serial recovery.** Reinstall with the flash script, which resets otadata and writes OTA0. Verify this works after OTA updates and after an intentionally unusable test application. The stock bootloader does not roll back merely because the application hangs.

12. **Watchdog and load faults.** Perform the detailed [reliability fault-injection plan](RELIABILITY.md): each task stopped independently, CPU/interrupt stalls, latched starvation, slow clients, simultaneous OTA and reads, and a 72-hour assembly soak. Verify reset reasons and measured deadlines, not only that the board eventually returns.

Not performed: RTC watchdog/reset timing, executor/interrupt failure, watchdog recovery, concurrent MCU HTTP/OTA, brownout characterization, board-info on the target, electrical measurements, direct-cable reliability, actual SHT40 accuracy, Ethernet link negotiation, DHCP, HTTP on the MCU, cable reconnection, OTA flash writes/reboot, power interruption, flash endurance, long-duration load testing, or firmware task coverage.
