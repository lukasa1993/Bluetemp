//! WT32-ETH01 v1.4 schematic wiring. GPIO16 is oscillator enable, not PHY reset.
use embassy_time::{Duration, Timer};
use esp_hal::{
    clock::CpuClock,
    ethernet::{Ethernet, EthernetDmaStorage, RmiiPinBundle, clock::ExternalRefClock},
    gpio::{Level, Output, OutputConfig},
    i2c::master::{Config, I2c},
    time::Rate,
    timer::timg::TimerGroup,
};
use esp_storage::FlashStorage;
use static_cell::ConstStaticCell;

use crate::phy::TimedPhy;

pub type EthDriver = Ethernet<'static, esp_hal::Async, TimedPhy>;
pub type SensorBus = I2c<'static, esp_hal::Async>;
static DMA: ConstStaticCell<EthernetDmaStorage<8, 8>> =
    ConstStaticCell::new(EthernetDmaStorage::new());

pub struct Board {
    pub ethernet: EthDriver,
    pub ethernet_enable: Output<'static>,
    pub sensor: SensorBus,
    pub flash: FlashStorage<'static>,
}

pub async fn init() -> Board {
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::_160MHz));
    // Arm before board setup, logging, task spawning or any await can hang.
    crate::watchdog::init(esp_hal::rtc_cntl::Rtc::new(p.RTC_TIMER).rwdt);
    esp_println::println!(
        "Bluetemp {} ESP32 revision {:?}",
        env!("CARGO_PKG_VERSION"),
        esp_hal::efuse::chip_revision()
    );
    esp_println::println!(
        "Reset reason: {:?}; RTC watchdog: 30 seconds",
        esp_hal::system::reset_reason()
    );
    let timers = TimerGroup::new(p.TIMG0);
    esp_rtos::start(timers.timer0, p.FROM_CPU_INTR0);

    let enable = Output::new(p.GPIO16, Level::High, OutputConfig::default());
    Timer::after(Duration::from_millis(300)).await;
    let mut mac = [0; 6];
    mac.copy_from_slice(esp_hal::efuse::base_mac_address().as_bytes());
    // Derive a locally administered Ethernet address without enabling a radio.
    mac[0] = (mac[0] | 2) & 0xfe;
    esp_println::println!("Ethernet MAC {:02x?}", mac);
    let ethernet = Ethernet::new(
        p.ETH,
        DMA.take(),
        mac,
        TimedPhy::new(),
        RmiiPinBundle {
            clock: ExternalRefClock::new(p.GPIO0),
            rxd0: p.GPIO25,
            rxd1: p.GPIO26,
            rx_dv: p.GPIO27,
            txd0: p.GPIO19,
            txd1: p.GPIO22,
            tx_en: p.GPIO21,
            mdc: p.GPIO23,
            mdio: p.GPIO18,
        },
    )
    .expect("Ethernet initialization failed")
    .into_async();

    let sensor = I2c::new(
        p.I2C0,
        Config::default().with_frequency(Rate::from_hz(10_000)),
    )
    .expect("I2C configuration")
    .with_sda(p.GPIO32)
    .with_scl(p.GPIO33)
    .into_async();
    Board {
        ethernet,
        ethernet_enable: enable,
        sensor,
        flash: FlashStorage::new(p.FLASH),
    }
}
