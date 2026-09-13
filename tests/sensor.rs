use bluetemp::measurement::{self, Reading};
use embedded_hal_mock::eh1::{
    delay::{CheckedDelay, Transaction as Delay},
    i2c::{Mock, Transaction as I2c},
};

async fn sample(
    frame: [u8; 6],
) -> Result<measurement::Measurement, sht4x::Error<embedded_hal_async::i2c::ErrorKind>> {
    let mut bus = Mock::new(&[
        I2c::write(0x44, vec![0xfd]),
        I2c::read(0x44, frame.to_vec()),
    ]);
    let mut delay = CheckedDelay::new(&[Delay::async_delay_ms(9)]);
    let mut driver = sht4x::Sht4xAsync::new(&mut bus);
    let result = measurement::measure(&mut driver, &mut delay).await;
    bus.done();
    delay.done();
    result
}

#[tokio::test]
async fn async_driver_checks_sensor_crc_and_units() {
    let frame = [0xbe, 0xef, 0x92, 0xbe, 0xef, 0x92];
    let value = sample(frame).await.unwrap();
    // The community driver uses fixed-point arithmetic; allow its quantization.
    assert!((value.temperature_c - 85.523).abs() < 0.004);
    assert!((value.humidity_percent - 87.2307).abs() < 0.004);
    for bit in 0..48 {
        let mut corrupt = frame;
        corrupt[bit / 8] ^= 1 << (bit % 8);
        assert!(sample(corrupt).await.is_err());
    }
    let low = sample([0, 0, 0x81, 0, 0, 0x81]).await.unwrap();
    assert_eq!(low.temperature_c, -45.0);
    assert_eq!(low.humidity_percent, 0.0);
    let high = sample([255, 255, 0xac, 255, 255, 0xac]).await.unwrap();
    assert_eq!(high.temperature_c, 130.0);
    assert_eq!(high.humidity_percent, 100.0);
}

#[test]
fn freshness_rejects_initial_failed_and_old_samples() {
    let mut reading = Reading::default();
    assert!(!reading.is_fresh(0));
    reading.value = Some(measurement::Measurement {
        temperature_c: 20.0,
        humidity_percent: 50.0,
    });
    reading.sampled_ms = 100;
    reading.last_ok = true;
    assert!(reading.is_fresh(15_100));
    assert!(!reading.is_fresh(15_101));
    reading.last_ok = false;
    assert!(!reading.is_fresh(101));
}
