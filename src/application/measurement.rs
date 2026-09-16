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

/// Parse one ASCII frame from the UART sensor module, e.g. `R:053.8RH 024.9C`
/// with an optional trailing `\r` (the caller strips the `\n` terminator).
/// Returns `None` for malformed or implausible frames.
pub fn parse_uart_frame(line: &[u8]) -> Option<Measurement> {
    let line = match line.last() {
        Some(b'\r') => &line[..line.len() - 1],
        _ => line,
    };
    if line.len() != 16
        || line[0] != b'R'
        || line[1] != b':'
        || line[7] != b'R'
        || line[8] != b'H'
        || line[9] != b' '
        || line[15] != b'C'
    {
        return None;
    }
    let humidity_percent = parse_ddd_dot_d(&line[2..7])?;
    let temperature_c = parse_temp_field(&line[10..15])?;
    if !(0.0..=100.0).contains(&humidity_percent) || !(-45.0..=130.0).contains(&temperature_c) {
        return None;
    }
    Some(Measurement {
        temperature_c,
        humidity_percent,
    })
}

/// Parse a `DDD.D` field such as `053.8`.
fn parse_ddd_dot_d(field: &[u8]) -> Option<f32> {
    if field.len() != 5 || field[3] != b'.' {
        return None;
    }
    let mut value: u32 = 0;
    for &digit in &[field[0], field[1], field[2], field[4]] {
        if !digit.is_ascii_digit() {
            return None;
        }
        value = value * 10 + u32::from(digit - b'0');
    }
    Some(value as f32 / 10.0)
}

/// Parse a temperature field: `DDD.D` or `-DD.D` for sub-zero readings.
fn parse_temp_field(field: &[u8]) -> Option<f32> {
    if field.len() != 5 {
        return None;
    }
    if field[0] == b'-' {
        if field[3] != b'.' {
            return None;
        }
        let mut value: u32 = 0;
        for &digit in &[field[1], field[2], field[4]] {
            if !digit.is_ascii_digit() {
                return None;
            }
            value = value * 10 + u32::from(digit - b'0');
        }
        Some(-(value as f32) / 10.0)
    } else {
        parse_ddd_dot_d(field)
    }
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
