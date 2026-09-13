//! Serialize OTA work without queuing, and protect committed images until reboot.
use super::protocol::Error;
use core::ops::AsyncFnOnce;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};

struct State<F> {
    flash: F,
    committed: bool,
}

pub struct UpdateService<F> {
    state: Mutex<CriticalSectionRawMutex, State<F>>,
}

impl<F> From<F> for UpdateService<F> {
    fn from(flash: F) -> Self {
        Self {
            state: Mutex::new(State {
                flash,
                committed: false,
            }),
        }
    }
}

impl<F> UpdateService<F> {
    pub async fn update(
        &self,
        operation: impl AsyncFnOnce(&mut F) -> Result<(), Error>,
        request_reboot: impl FnOnce(),
    ) -> Result<(), Error> {
        let mut state = self.state.try_lock().map_err(|_| Error::Busy)?;
        if state.committed {
            return Err(Error::Busy);
        }
        operation(&mut state.flash).await?;
        // Cancellation cannot occur between successful commit, exclusion and notification.
        state.committed = true;
        request_reboot();
        Ok(())
    }
}
