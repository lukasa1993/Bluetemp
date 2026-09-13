//! LAN8720 negotiation policy and Embassy-driven idle link polling.
use core::{future::Future, pin::Pin, task::Context};
use embassy_time::{Duration, Timer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    FastFull,
    FastHalf,
    SlowFull,
    SlowHalf,
}

pub fn negotiated_mode(local: u16, partner: u16) -> Mode {
    let common = local & partner;
    if common & 0x100 != 0 {
        Mode::FastFull
    } else if common & 0x80 != 0 {
        Mode::FastHalf
    } else if common & 0x40 != 0 {
        Mode::SlowFull
    } else {
        Mode::SlowHalf
    }
}

pub struct PollCache<T> {
    value: Option<T>,
    timer: Timer,
}

impl<T: Copy> Default for PollCache<T> {
    fn default() -> Self {
        Self {
            value: None,
            timer: Timer::after_ticks(0),
        }
    }
}

impl<T: Copy> PollCache<T> {
    pub fn poll(&mut self, cx: Option<&mut Context<'_>>, read: impl FnOnce() -> T) -> T {
        if let Some(cx) = cx {
            match self.value {
                Some(value) if Pin::new(&mut self.timer).poll(cx).is_pending() => return value,
                _ => {}
            }
            self.timer = Timer::after(Duration::from_millis(500));
            let _ = Pin::new(&mut self.timer).poll(cx);
        }
        let value = read();
        self.value = Some(value);
        value
    }
}
