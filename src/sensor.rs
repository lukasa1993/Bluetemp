use crate::board::SensorBus;
use bluetemp::application::measurement::{Reading, sample};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, watch::Watch};
use embassy_time::{Delay, Duration, Ticker};

// Latest-value publication; no queued old readings and no application locking code.
static READING: Watch<CriticalSectionRawMutex, Reading, 0> = Watch::new();

pub fn snapshot() -> Reading {
    READING.try_get().unwrap_or_default()
}

#[embassy_executor::task]
pub async fn task(bus: SensorBus) {
    let mut sensor = sht4x::Sht4xAsync::new(bus);
    let mut delay = Delay;
    let mut ticker = Ticker::every(Duration::from_secs(5));
    loop {
        let mut reading = snapshot();
        sample(&mut sensor, &mut reading, &mut delay).await;
        log_failure(reading);
        READING.sender().send(reading);
        crate::watchdog::progress(bluetemp::application::supervision::Task::Sensor);
        ticker.next().await;
    }
}

fn log_failure(reading: Reading) {
    if !reading.last_ok {
        esp_println::println!("SHT40 read failed; errors={}", reading.errors);
    }
}
