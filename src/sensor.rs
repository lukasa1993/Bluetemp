use crate::board::SensorBus;
use bluetemp::application::measurement::{Reading, parse_uart_frame};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, watch::Watch};
use embassy_time::{Duration, Instant, with_timeout};

// Latest-value publication; no queued old readings and no application locking code.
static READING: Watch<CriticalSectionRawMutex, Reading, 0> = Watch::new();

pub fn snapshot() -> Reading {
    READING.try_get().unwrap_or_default()
}

#[embassy_executor::task]
pub async fn task(mut uart: SensorBus) {
    let mut buf = [0u8; 32];
    loop {
        let mut reading = snapshot();
        match read_line(&mut uart, &mut buf).await {
            Some(len) => match parse_uart_frame(&buf[..len]) {
                Some(value) => reading.success(value, Instant::now().as_millis()),
                None => reading.failure(),
            },
            None => reading.failure(),
        }
        log_failure(reading);
        READING.sender().send(reading);
        crate::watchdog::progress(bluetemp::application::supervision::Task::Sensor);
    }
}

/// Read one `\n`-terminated line within 3 seconds. The module transmits at
/// about 1 Hz, so a timeout means missed frames or a dead module.
async fn read_line(uart: &mut SensorBus, buf: &mut [u8; 32]) -> Option<usize> {
    let mut len = 0usize;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let mut byte = [0u8; 1];
        let received: usize = with_timeout(remaining, uart.read_async(&mut byte))
            .await
            .ok()?
            .ok()?;
        if received == 0 {
            continue;
        }
        if byte[0] == b'\n' {
            return Some(len);
        }
        if len < buf.len() {
            buf[len] = byte[0];
            len += 1;
        } else {
            // Overlong garbage: drop it and resync on the next newline.
            len = 0;
        }
    }
}

fn log_failure(reading: Reading) {
    if !reading.last_ok {
        esp_println::println!("sensor read failed; errors={}", reading.errors);
    }
}
