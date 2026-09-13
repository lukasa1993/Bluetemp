//! Application reading state; sensor protocol and conversion belong to sht4x.
use embedded_hal_async::{delay::DelayNs, i2c::I2c};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measurement {
    pub temperature_c: f32,
    pub humidity_percent: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Reading {
    pub value: Option<Measurement>,
    pub sampled_ms: u64,
    pub errors: u32,
    pub last_ok: bool,
}

impl Reading {
    pub fn success(&mut self, value: Measurement, now_ms: u64) {
        self.value = Some(value);
        self.sampled_ms = now_ms;
        self.last_ok = true;
    }
    pub fn failure(&mut self) {
        self.errors = self.errors.saturating_add(1);
        self.last_ok = false;
    }

    pub fn is_fresh(&self, now_ms: u64) -> bool {
        self.last_ok && self.value.is_some() && now_ms.saturating_sub(self.sampled_ms) <= 15_000
    }
}

pub async fn measure<I: I2c, D: DelayNs>(
    sensor: &mut sht4x::Sht4xAsync<I, D>,
    delay: &mut D,
) -> Result<Measurement, sht4x::Error<I::Error>> {
    let value = sensor.measure(sht4x::Precision::High, delay).await?;
    Ok(Measurement {
        temperature_c: value.temperature_celsius().to_num(),
        humidity_percent: value.humidity_percent().to_num::<f32>().clamp(0.0, 100.0),
    })
}

/// Complete one bounded attempt, including state publication data and sensor recovery.
pub async fn sample<I: I2c, D: DelayNs>(
    sensor: &mut sht4x::Sht4xAsync<I, D>,
    reading: &mut Reading,
    delay: &mut D,
) {
    use embassy_time::{Duration, Instant, with_timeout};
    match with_timeout(Duration::from_millis(250), measure(sensor, delay)).await {
        Ok(Ok(value)) => reading.success(value, Instant::now().as_millis()),
        _ => {
            reading.failure();
            let _ = with_timeout(Duration::from_millis(100), sensor.soft_reset(delay)).await;
        }
    }
}
