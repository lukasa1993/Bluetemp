use crate::board::SensorBus;
use bluetemp::measurement::{Reading, measure};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, watch::Watch};
use embassy_time::{Delay, Duration, Instant, Ticker, with_timeout};

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
        let result =
            with_timeout(Duration::from_millis(250), measure(&mut sensor, &mut delay)).await;
        let mut reading = snapshot();
        match result {
            Ok(Ok(value)) => {
                reading.value = Some(value);
                reading.sampled_ms = Instant::now().as_millis();
                reading.last_ok = true;
            }
            _ => {
                reading.errors = reading.errors.saturating_add(1);
                reading.last_ok = false;
                esp_println::println!("SHT40 read failed; errors={}", reading.errors);
                // Soft-reset the sensor if it still responds. Never wait indefinitely.
                let _ =
                    with_timeout(Duration::from_millis(100), sensor.soft_reset(&mut delay)).await;
            }
        }
        READING.sender().send(reading);
        crate::watchdog::progress(bluetemp::supervision::Task::Sensor);
        ticker.next().await;
    }
}
