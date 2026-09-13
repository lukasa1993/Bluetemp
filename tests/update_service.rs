use bluetemp::application::{protocol::Error, update_service::UpdateService};
use std::{
    cell::Cell,
    future::{Future, pending},
    pin::pin,
    task::{Context, Poll, Waker},
};

#[tokio::test]
async fn concurrent_uploads_and_post_commit_uploads_are_rejected() {
    let service = UpdateService::from(Vec::<u8>::new());
    let rebooted = Cell::new(0);
    let mut first = pin!(service.update(
        async |flash| {
            flash.push(1);
            pending::<()>().await;
            Ok(())
        },
        || rebooted.set(rebooted.get() + 1)
    ));
    let mut cx = Context::from_waker(Waker::noop());
    assert_eq!(first.as_mut().poll(&mut cx), Poll::Pending);
    let second_called = Cell::new(false);
    assert_eq!(
        service
            .update(
                async |_| {
                    second_called.set(true);
                    Ok(())
                },
                || {}
            )
            .await,
        Err(Error::Busy)
    );
    assert!(!second_called.get());
    assert_eq!(rebooted.get(), 0);
}

#[tokio::test]
async fn cancellation_and_failure_release_the_lock_but_commit_is_final() {
    let service = UpdateService::from(Vec::<u8>::new());
    let rebooted = Cell::new(0);
    {
        let mut cancelled = pin!(service.update(
            async |flash| {
                flash.push(1);
                pending::<()>().await;
                Ok(())
            },
            || rebooted.set(rebooted.get() + 1)
        ));
        assert!(
            cancelled
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    assert_eq!(
        service
            .update(
                async |flash| {
                    assert_eq!(flash, &[1]);
                    Err(Error::Flash)
                },
                || rebooted.set(99)
            )
            .await,
        Err(Error::Flash)
    );
    assert_eq!(rebooted.get(), 0);
    service
        .update(
            async |flash| {
                flash.clear();
                flash.push(2);
                Ok(())
            },
            || rebooted.set(rebooted.get() + 1),
        )
        .await
        .unwrap();
    assert_eq!(rebooted.get(), 1);
    assert_eq!(
        service
            .update(
                async |_| panic!("committed image must not be overwritten"),
                || rebooted.set(99)
            )
            .await,
        Err(Error::Busy)
    );
    assert_eq!(rebooted.get(), 1);
}
