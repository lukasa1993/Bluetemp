use bluetemp::application::measurement::{self, Reading};
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
fn uart_frame_parses_live_module_output() {
    // Captured from the module at 9600 baud: `R:053.8RH 024.9C\r\n`.
    let value = measurement::parse_uart_frame(b"R:053.8RH 024.9C\r").unwrap();
    assert!((value.temperature_c - 24.9).abs() < 0.001);
    assert!((value.humidity_percent - 53.8).abs() < 0.001);
    let bare = measurement::parse_uart_frame(b"R:100.0RH -04.5C").unwrap();
    assert!((bare.humidity_percent - 100.0).abs() < 0.001);
    assert!((bare.temperature_c + 4.5).abs() < 0.001);
    for bad in [
        &b""[..],
        b"\n",
        b"R:053.8RH 024.9C\r\n",
        b"X:053.8RH 024.9C",
        b"R-053.8RH 024.9C",
        b"R:053.8RX 024.9C",
        b"R:053.8RH 024.9",
        b"R:053,8RH 024.9C",
        b"R:05.38RH 024.9C",
        b"R:05A.8RH 024.9C",
        b"R:053.8RH 024.9F",
        b"R:053.8RH024.9C",
        b"R:053.8Rh 024.9C",
        b"R:101.0RH 024.9C",
        b"R:053.8RH 200.0C",
        b"R:053.8RH -50.0C",
        b"R:053.8RH -A4.5C",
        b"R:053.8RH -045.C",
    ] {
        assert!(measurement::parse_uart_frame(bad).is_none(), "{bad:?}");
    }
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

#[tokio::test]
async fn sampling_recovers_from_bus_errors_and_bounds_stalled_operations() {
    use embassy_time::{Duration, MockDriver};
    use embedded_hal_async::i2c::ErrorKind;
    let clock = MockDriver::get();
    clock.reset();
    let mut reading = Reading::default();
    let mut bus = Mock::new(&[
        I2c::write(0x44, vec![0xfd]).with_error(ErrorKind::Other),
        I2c::write(0x44, vec![0x94]),
        I2c::write(0x44, vec![0xfd]),
        I2c::read(0x44, vec![0xbe, 0xef, 0x92, 0xbe, 0xef, 0x92]),
    ]);
    let mut delay = CheckedDelay::new(&[Delay::async_delay_ms(1), Delay::async_delay_ms(9)]);
    let mut sensor = sht4x::Sht4xAsync::new(&mut bus);
    measurement::sample(&mut sensor, &mut reading, &mut delay).await;
    assert_eq!(reading.errors, 1);
    assert!(!reading.last_ok);
    assert_eq!(reading.value, None);
    clock.advance(Duration::from_millis(5000));
    measurement::sample(&mut sensor, &mut reading, &mut delay).await;
    assert!(reading.is_fresh(5000));
    assert_eq!(reading.sampled_ms, 5000);
    assert_eq!(reading.errors, 1);
    bus.done();
    delay.done();

    struct StalledDelay;
    impl embedded_hal_async::delay::DelayNs for StalledDelay {
        async fn delay_ns(&mut self, _: u32) {
            std::future::pending::<()>().await;
        }
    }
    use std::{
        future::Future,
        pin::pin,
        task::{Context, Poll, Waker},
    };
    let mut bus = Mock::new(&[I2c::write(0x44, vec![0xfd]), I2c::write(0x44, vec![0x94])]);
    let mut sensor = sht4x::Sht4xAsync::new(&mut bus);
    let previous = reading.value;
    {
        let mut delay = StalledDelay;
        let mut attempt = pin!(measurement::sample(&mut sensor, &mut reading, &mut delay));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(attempt.as_mut().poll(&mut cx).is_pending());
        clock.advance(Duration::from_millis(249));
        assert!(attempt.as_mut().poll(&mut cx).is_pending());
        clock.advance(Duration::from_millis(1));
        assert!(attempt.as_mut().poll(&mut cx).is_pending());
        clock.advance(Duration::from_millis(99));
        assert!(attempt.as_mut().poll(&mut cx).is_pending());
        clock.advance(Duration::from_millis(1));
        assert_eq!(attempt.as_mut().poll(&mut cx), Poll::Ready(()));
    }
    assert_eq!(reading.errors, 2);
    assert!(!reading.last_ok);
    assert_eq!(reading.value, previous);
    bus.done();
}
