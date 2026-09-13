use bluetemp::application::{
    link::{Mode, PollCache, negotiated_mode},
    measurement::{Measurement, Reading},
    supervision::EmbassyClock,
};
use embassy_time::{Duration, Instant, MockDriver};
use std::{
    cell::Cell,
    task::{Context, Waker},
};
use task_watchdog::Clock;

#[test]
fn link_polls_and_watchdog_clock_follow_embassy_time() {
    // One test owns Embassy's global mock clock in this integration-test process.
    let driver = MockDriver::get();
    driver.reset();
    let mut cx = Context::from_waker(Waker::noop());
    let calls = Cell::new(0);
    let mut cache = PollCache::default();
    let read = || {
        calls.set(calls.get() + 1);
        calls.get()
    };
    assert_eq!(cache.poll(Some(&mut cx), read), 1);
    assert_eq!(cache.poll(Some(&mut cx), read), 1);
    driver.advance(Duration::from_millis(499));
    assert_eq!(cache.poll(Some(&mut cx), read), 1);
    driver.advance(Duration::from_millis(1));
    assert_eq!(cache.poll(Some(&mut cx), read), 2);
    assert_eq!(cache.poll(None, read), 3); // synchronous HAL checks bypass the timer
    assert_eq!(cache.poll(Some(&mut cx), read), 3);
    driver.advance(Duration::from_millis(500));
    assert_eq!(cache.poll(Some(&mut cx), read), 4);
    let clock = EmbassyClock;
    assert_eq!(clock.now(), Instant::from_millis(1000));
    assert_eq!(
        clock.duration_from_millis(2000),
        Duration::from_millis(2000)
    );
    assert_eq!(
        clock.elapsed_since(Instant::from_millis(500)),
        Duration::from_millis(500)
    );
    assert!(!clock.has_elapsed(Instant::from_millis(501), &Duration::from_millis(500)));
    assert!(clock.has_elapsed(Instant::from_millis(500), &Duration::from_millis(500)));
    assert_eq!(
        clock.elapsed_since(Instant::from_millis(2000)),
        Duration::from_ticks(0)
    );
    use std::{future::Future, pin::pin};
    let checks = Cell::new(0);
    let failed = Cell::new(false);
    let mut supervisor = pin!(bluetemp::application::supervision::run(|| {
        checks.set(checks.get() + 1);
        failed.get()
    }));
    assert!(supervisor.as_mut().poll(&mut cx).is_pending());
    driver.advance(Duration::from_secs(1));
    assert!(supervisor.as_mut().poll(&mut cx).is_pending());
    assert_eq!(checks.get(), 1);
    failed.set(true);
    driver.advance(Duration::from_secs(1));
    assert!(supervisor.as_mut().poll(&mut cx).is_pending());
    assert_eq!(checks.get(), 2);
    failed.set(false);
    driver.advance(Duration::from_secs(120));
    assert!(supervisor.as_mut().poll(&mut cx).is_pending());
    assert_eq!(
        checks.get(),
        2,
        "a latched watchdog must not resume checking/feeding"
    );
    connection_deadlines(&mut cx, driver);
}

#[test]
fn link_selects_highest_mutual_technology() {
    let technologies = [
        (0x100, Mode::FastFull),
        (0x80, Mode::FastHalf),
        (0x40, Mode::SlowFull),
        (0x20, Mode::SlowHalf),
    ];
    for local in 0..16 {
        for partner in 0..16 {
            let advertisement = |set: u16| {
                technologies
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| set & (1 << i) != 0)
                    .map(|(_, (mask, _))| mask)
                    .sum::<u16>()
            };
            let expected = technologies
                .iter()
                .enumerate()
                .find(|(i, _)| local & (1 << i) != 0 && partner & (1 << i) != 0)
                .map(|(_, (_, mode))| *mode)
                .unwrap_or(Mode::SlowHalf);
            assert_eq!(
                negotiated_mode(advertisement(local), advertisement(partner)),
                expected
            );
        }
    }
}

#[test]
fn sensor_failure_keeps_last_sample_and_recovers_without_erasing_errors() {
    let value = Measurement {
        temperature_c: 21.5,
        humidity_percent: 48.0,
    };
    let mut reading = Reading::default();
    reading.failure();
    assert_eq!(reading.errors, 1);
    assert!(!reading.last_ok);
    assert_eq!(reading.value, None);
    reading.success(value, 123);
    assert_eq!(reading.sampled_ms, 123);
    assert_eq!(reading.value, Some(value));
    assert!(reading.is_fresh(124));
    reading.failure();
    assert_eq!(reading.value, Some(value));
    assert_eq!(reading.sampled_ms, 123);
    assert_eq!(reading.errors, 2);
    assert!(!reading.is_fresh(124));
    reading.success(value, 200);
    assert!(reading.is_fresh(200));
    assert_eq!(reading.errors, 2);
    reading.errors = u32::MAX;
    reading.failure();
    assert_eq!(reading.errors, u32::MAX);
}

fn connection_deadlines(cx: &mut Context<'_>, driver: &MockDriver) {
    use bluetemp::application::connection;
    use std::{
        future::{Future, pending},
        pin::pin,
        task::Poll,
    };
    let handled = Cell::new(0);
    let mut normal = pin!(connection::serve(
        (),
        async |_| Ok::<_, ()>(()),
        async |_| {
            handled.set(1);
            Ok::<_, ()>(())
        }
    ));
    assert_eq!(normal.as_mut().poll(cx), Poll::Ready(Ok(())));
    assert_eq!(handled.get(), 1);
    let mut failed = pin!(connection::serve(
        (),
        async |_| Ok::<_, ()>(()),
        async |_| Err::<(), _>(())
    ));
    assert_eq!(failed.as_mut().poll(cx), Poll::Ready(Err(())));
    let mut rejected = pin!(connection::serve(
        (),
        async |_| Err::<(), _>(()),
        async |_| {
            handled.set(99);
            Ok::<_, ()>(())
        }
    ));
    assert!(rejected.as_mut().poll(cx).is_pending());
    driver.advance(Duration::from_millis(100));
    assert_eq!(rejected.as_mut().poll(cx), Poll::Ready(Ok(())));
    assert_eq!(handled.get(), 1);
    let mut idle = pin!(connection::serve(
        (),
        async |_| pending::<Result<(), ()>>().await,
        async |_| {
            handled.set(99);
            Ok::<_, ()>(())
        }
    ));
    assert!(idle.as_mut().poll(cx).is_pending());
    driver.advance(Duration::from_secs(5));
    assert!(idle.as_mut().poll(cx).is_pending());
    driver.advance(Duration::from_millis(100));
    assert_eq!(idle.as_mut().poll(cx), Poll::Ready(Ok(())));
    assert_eq!(handled.get(), 1);
    let mut stalled = pin!(connection::serve(
        (),
        async |_| Ok::<_, ()>(()),
        async |_| pending::<Result<(), ()>>().await
    ));
    assert!(stalled.as_mut().poll(cx).is_pending());
    driver.advance(Duration::from_secs(189));
    assert!(stalled.as_mut().poll(cx).is_pending());
    driver.advance(Duration::from_secs(1));
    assert_eq!(stalled.as_mut().poll(cx), Poll::Ready(Err(())));
}
