//! Timer-driven link checks also wake the runner when the cable is unplugged.
use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use embassy_time::{Duration, Timer};
use esp_hal::ethernet::{
    mac::{Duplex, LinkState, Speed},
    phy::{MdioBus, Phy, PhyError, generic::GenericPhy},
};

pub struct TimedPhy {
    inner: GenericPhy,
    timer: Timer,
    cached: LinkState,
}

impl TimedPhy {
    pub fn new() -> Self {
        Self {
            inner: GenericPhy::new(1),
            timer: Timer::after_ticks(0),
            cached: LinkState {
                up: false,
                speed: Speed::_100M,
                duplex: Duplex::Full,
            },
        }
    }
}

impl Phy for TimedPhy {
    fn address(&self) -> u8 {
        self.inner.address()
    }
    fn init<M: MdioBus>(&mut self, mdio: &mut M) -> Result<(), PhyError> {
        self.inner.init(mdio)
    }
    fn poll_link<M: MdioBus>(&mut self, mdio: &mut M, cx: Option<&mut Context<'_>>) -> LinkState {
        if let Some(cx) = cx {
            if Pin::new(&mut self.timer).poll(cx) == Poll::Pending {
                return self.cached;
            }
            self.timer = Timer::after(Duration::from_millis(500));
            let _ = Pin::new(&mut self.timer).poll(cx);
        }
        self.cached = self.inner.poll_link(mdio, None);
        if self.cached.up {
            // Resolve the highest common mode (100-full, 100-half, 10-full, 10-half).
            // GenericPhy 1.2.1 can misreport duplex for mixed partner advertisements.
            let common = mdio.read(1, 4) & mdio.read(1, 5);
            let fast = common & 0x180 != 0;
            let full = if fast {
                common & 0x100 != 0
            } else {
                common & 0x40 != 0
            };
            self.cached.speed = if fast { Speed::_100M } else { Speed::_10M };
            self.cached.duplex = if full { Duplex::Full } else { Duplex::Half };
        }
        // Only the runner reaches this after an actual bounded PHY status check.
        crate::watchdog::progress(bluetemp::supervision::Task::Network);
        self.cached
    }
}
