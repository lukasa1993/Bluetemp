# Hardware, setup and update reference

Bare-metal Rust firmware for **WT32-ETH01 / original ESP32 revision v3.0 or newer**, with a LAN8720A PHY and an SHT40 temperature/humidity sensor. Ethernet, DHCP, HTTP, sensor sampling, a 60-second ticker, authenticated LAN OTA, and task-supervised RTC watchdog resets are implemented. **No hardware has been tested: the board and sensor have not arrived.**

This build does not support ESP32 revisions below v3.0. Read the actual revision before flashing. Do not bypass the revision check or lower the HAL's configured minimum to make an old chip boot. See [validation status](VALIDATION.md).

## Wiring

The Ethernet wiring is already on the WT32-ETH01. Verified against the manufacturer's [v1.4 schematic](https://github.com/egnor/wt32-eth01/blob/main/WT32-ETH01_V1.4.schematic.pdf) and [datasheet](https://github.com/egnor/wt32-eth01/blob/main/WT32-ETH01-datasheet-v1.4-en.pdf), mirrored in this board-documentation repository:

| Signal | Configuration |
| --- | --- |
| PHY | LAN8720A, MDIO address **1** (PHYAD0 pulled high) |
| MDC / MDIO | GPIO23 / GPIO18 |
| REF_CLK | External **50 MHz input on GPIO0** |
| Ethernet enable | GPIO16 high enables the external oscillator; it is **not PHY reset** |
| RMII receive | RXD0 GPIO25, RXD1 GPIO26, CRS_DV GPIO27 |
| RMII transmit | TXD0 GPIO19, TXD1 GPIO22, TX_EN GPIO21 |
| PHY reset | Board RC reset circuit, not an ESP32 GPIO |

Wire the sensor directly, without extenders:

| WT32-ETH01 | SHT40 |
| --- | --- |
| GPIO32 (often labelled CFG) | SDA |
| GPIO33 (often labelled 485_EN) | SCL |
| 3V3 | VDD / 3.3 V input |
| GND | GND |

Use external **2.2 kΩ pull-ups from SDA and SCL to 3.3 V**, located at the board, and a **100 nF decoupling capacitor at the sensor**. Check the sensor breakout's existing pull-ups first; do not unknowingly stack multiple strong pull-ups. Never pull either signal up to 5 V. Supply the WT32-ETH01 through its specified 5 V input from a suitable supply, with common ground; do not rely on a USB-UART adapter's weak 3.3 V supply.

I²C is deliberately configured at **10 kHz**, address **0x44**. SHT40 variants with another address require changing the address in `src/sensor.rs`. The sensor supports I²C only, not native SPI or 1-Wire. See [Sensirion's electrical specifications and commands](https://sensirion.com/media/documents/33FD6951/6555C40E/Sensirion_Datasheet_SHT4x.pdf).

**A direct 2–3 m I²C cable is an unverified hardware constraint, not a guaranteed range.** Lower clock speed helps timing but does not eliminate capacitance, ringing, voltage drop, or noise. Keep the cable away from mains and switching power wiring. Twisting is optional: if practical, pair SDA with ground and SCL with ground, rather than SDA with SCL. Test at the full intended length. The firmware reports errors and stale values; it cannot repair unsuitable electrical signalling. No extenders are required by this design.

## Build

Prerequisites: a normal Rust/rustup installation, Python 3.10+, and network access for first-time tool/dependency downloads. On this Mac the pinned tools have already been installed.

```sh
cd /Users/l/_DEV/Bluetemp
./scripts/dev setup
./scripts/dev build
./scripts/dev test
```

`setup` installs espup **0.17.1**, espflash **4.6.0**, and Xtensa Rust **1.95.0.0**. It does not install or build the ESP-IDF application runtime. The Xtensa toolchain is registered globally as `esp`; build scripts check its exact version. Host tests/formatting use the host's `stable` toolchain (1.95 or newer).

The first scripted build creates a random OTA key in `.secrets/ota-key.hex` and embeds it in the firmware. Keep that file for subsequent builds and updates. It is ignored by Git and is never printed. Losing it requires serial reflashing to replace the board's key. A direct Cargo build without `BLUETEMP_OTA_KEY` still builds, but its OTA endpoint rejects all updates.

Build outputs:

- `target/xtensa-esp32-none-elf/release/bluetemp`: ELF, for serial flashing and symbols.
- `target/bluetemp.bin`: **application-only** image, for OTA; never a merged flash dump.
- `target/bluetemp.map`: linker map, including the native object audit.

Equivalent manual build (from the repository root):

```sh
export PATH="$HOME/.cargo/bin:$PWD/.tools/bin:$PATH"
. .tools/export-esp.sh
export BLUETEMP_OTA_KEY="$(cat .secrets/ota-key.hex)"
cargo +esp build --release --locked -Zbuild-std=core --target xtensa-esp32-none-elf
espflash save-image --chip esp32 --min-chip-rev 3.0 \
  --flash-mode dio --flash-freq 40mhz --flash-size 4mb \
  --partition-table partitions.csv \
  target/xtensa-esp32-none-elf/release/bluetemp target/bluetemp.bin
```

## Serial installation

Use a **3.3 V logic** USB-UART adapter: adapter TX to board RX0/GPIO3, adapter RX to TX0/GPIO1, and common ground. For manual download mode, hold GPIO0 low, then reset EN. GPIO0 must be released before normal boot so the Ethernet reference clock can operate.

```sh
./scripts/dev board-info --port /dev/cu.YOUR_ADAPTER --manual
./scripts/dev flash --port /dev/cu.YOUR_ADAPTER --manual
```

`board-info` runs `espflash board-info`, prints the chip/revision/flash information, and records it in `target/board-info.txt`. `flash` rebuilds, reads the board information again, requires original ESP32 revision ≥3.0 and at least 4 MB flash, then installs the standard bootloader, partition table and `ota_0` image. It erases **otadata** so an old OTA selection does not mask the serially installed image. This replaces existing firmware/layout. It does not erase eFuses.

Manual mode leaves the chip in download mode: remove GPIO0–GND, reset EN, then monitor at 115200 baud:

```sh
.tools/bin/espflash monitor --port /dev/cu.YOUR_ADAPTER \
  --non-interactive --no-reset --elf target/xtensa-esp32-none-elf/release/bluetemp
```

Omit `--manual` only when your adapter is wired for automatic EN/GPIO0 reset. The actual serial port must be specified; no unknown connected device is selected automatically.

## HTTP and LAN OTA

Plug into a LAN with DHCP. Serial logs show link transitions and the DHCP address. For a stable address, reserve the printed Ethernet MAC in your router. **192.168.19.129 is only the existing reference endpoint; it is not hardcoded or overwritten.**

| Request | Response |
| --- | --- |
| `GET /` | `device_mac - uptime_ms - temperature_c - humidity_percent - local_ipv4` |
| `GET /api/reading` | JSON: firmware version, uptime, temperature, humidity, sample age, freshness, error count |
| `GET /health` | `bluetemp alive` (application liveness, independent of sensor readiness) |
| `GET /ready` | HTTP 200 only with a fresh sensor sample and IPv4 configuration; otherwise 503 |
| `POST /ota` | Authenticated raw firmware upload; use the script below |

```sh
curl http://DEVICE_IP/
curl http://DEVICE_IP/api/reading
./scripts/dev ota http://DEVICE_IP
```

The root's five-field text format resembles the supplied reference service, whose undocumented first two fields are not assumed to have the same meaning. Temperature uses °C; humidity uses %RH. Measurements occur immediately at startup and every 5 seconds. Both sensor CRCs must pass. A failed read causes a bounded soft-reset attempt and another sample on the next tick. Old values remain available in JSON but have `fresh:false`; `/` and `/api/reading` return **503** when the last attempt failed or the sample is older than 15 seconds. Before any valid reading, JSON values are `null`.

`ota` rebuilds, validates the application image and its appended SHA256, then sends its exact byte length, SHA256 and HMAC-SHA256 signature. The firmware authenticates the manifest before flash writes, verifies the project/chip header, writes the **inactive** slot, hashes its flash contents, and changes boot selection only when the digest matches. It then reboots. An incomplete/invalid transfer leaves the current boot selection untouched. Metadata has two independent sectors; torn records are rejected by CRC.

HMAC covers `bluetemp-ota-v1\0 || length_u32_be || image_sha256`. The key never goes over the wire. HTTP is unencrypted; a previously valid signed image can be replayed. There is no secure boot, anti-rollback policy, or automatic rollback after an application hangs. The stock bootloader can reject an invalid image, but that is not a health check. Keep a serial recovery path. Two HTTP workers allow a normal upload and status requests concurrently. Initial/ordinary requests have a 10-second budget, accepted upload bodies use picoserve’s 180-second override, response writes have 2 seconds, and Embassy bounds an entire connection to 190 seconds. Unconsumed rejected bodies retain the short request deadline. TCP inactivity is limited to 15 seconds. Concurrent uploads return 503; a committed image cannot be overwritten while its reboot is pending. Other tasks continue between synchronous flash operations. Two occupied workers can temporarily exhaust HTTP capacity; this is not a denial-of-service guarantee.
