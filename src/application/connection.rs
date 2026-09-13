//! Bound both accepting and serving a connection with Embassy timeouts.
use core::ops::AsyncFnOnce;
use embassy_time::{Duration, Timer, with_timeout};

pub async fn serve<S, A, E>(
    mut socket: S,
    accept: impl AsyncFnOnce(&mut S) -> Result<(), A>,
    handle: impl AsyncFnOnce(S) -> Result<(), E>,
) -> Result<(), ()> {
    if !matches!(
        with_timeout(Duration::from_secs(5), accept(&mut socket)).await,
        Ok(Ok(()))
    ) {
        Timer::after(Duration::from_millis(100)).await;
        // No request was accepted. Idle/no-link retries are normal operation.
        return Ok(());
    }
    match with_timeout(Duration::from_secs(190), handle(socket)).await {
        Ok(Ok(())) => Ok(()),
        _ => Err(()),
    }
}
