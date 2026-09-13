# Bluetemp

Bare-metal Rust firmware for a **WT32-ETH01** with an **SHT40 temperature/humidity sensor**. Uses Embassy and esp-hal for Ethernet/DHCP, HTTP readings, LAN OTA, and task-supervised watchdog recovery. No Wi-Fi, Bluetooth, FreeRTOS, lwIP, or ESP-IDF application runtime.

**Hardware is untested.** This build requires an original ESP32 **revision ≥3.0** and **at least 4 MB flash**. Automatic rollback from a broken OTA application is not implemented.

## Sensor wiring

| Board | SHT40 |
| --- | --- |
| GPIO32 | SDA |
| GPIO33 | SCL |
| 3V3 | VDD |
| GND | GND |

I²C runs at 10 kHz, address `0x44`. Use 2.2 kΩ pull-ups to 3.3 V and 100 nF decoupling at the sensor; check existing breakout pull-ups first. Keep the cable short and test the actual assembly—even under 1 m is not a reliability guarantee. See [full wiring and board documentation](docs/SETUP.md#wiring).

## Build and flash

Requires Rust/rustup and Python 3.10+. Run from the project directory:

```sh
./scripts/dev setup
./scripts/dev build
./scripts/dev test
```

Connect a 3.3 V logic USB-UART adapter. Hold GPIO0 low and reset EN to enter download mode, then run:

```sh
./scripts/dev board-info --port /dev/cu.YOUR_ADAPTER --manual
./scripts/dev flash --port /dev/cu.YOUR_ADAPTER --manual
```

Release GPIO0 and reset EN to boot. Flashing replaces the existing firmware and partition layout. See [serial wiring and monitoring](docs/SETUP.md#serial-installation).

## Readings and updates

Connect Ethernet to a LAN with DHCP. The assigned IP appears in serial logs.

| Endpoint | Purpose |
| --- | --- |
| `/` | Plain-text temperature and humidity |
| `/api/reading` | JSON readings, age, freshness and error count |
| `/ready` | 200 with fresh data and IPv4; otherwise 503 |
| `/health` | HTTP application liveness |

Samples are taken every 5 seconds. Failed or stale readings return **503** from the reading endpoints; JSON retains the previous sample with `fresh:false`.

```sh
curl http://DEVICE_IP/api/reading
./scripts/dev ota http://DEVICE_IP
```

OTA rebuilds, uploads an authenticated image to the inactive slot, verifies it, and reboots. Keep `.secrets/ota-key.hex`, generated on the first build; it is excluded from Git. HTTP is unencrypted, and a bad signed update may require serial recovery.

## Validation and details

The ESP32 build, Clippy, 13 Rust tests and 3 Python tests pass. **RepoRigor currently fails**; hardware recovery and OTA remain untested.

```sh
./scripts/dev quality
```

This uses `/Users/l/_DEV/clean-code/reporigor` (override with `REPORIGOR_ROOT`). Dependencies are pinned in `Cargo.toml` and `Cargo.lock`.

- [Setup, board references and build outputs](docs/SETUP.md)
- [Embassy and community API choices](docs/EMBASSY.md)
- [Watchdog behavior and hardware test plan](docs/RELIABILITY.md)
- [Validation results and open quality findings](docs/VALIDATION.md)
- [C and binary dependency audit](docs/DEPENDENCIES.md)
