//! Official PHY driver with idle polling and LAN8720 negotiation policy.
use bluetemp::application::link::{Mode, PollCache, negotiated_mode};
use core::task::Context;
use esp_hal::ethernet::{
    mac::{Duplex, LinkState, Speed},
    phy::{MdioBus, Phy, PhyError, generic::GenericPhy},
};

pub struct TimedPhy {
    inner: GenericPhy,
    cache: PollCache<LinkState>,
}
impl TimedPhy {
    pub fn new() -> Self {
        Self {
            inner: GenericPhy::new(1),
            cache: PollCache::default(),
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
        self.cache.poll(cx, || {
            let mut state = self.inner.poll_link(mdio, None);
            if state.up {
                (state.speed, state.duplex) =
                    hardware_mode(negotiated_mode(mdio.read(1, 4), mdio.read(1, 5)));
            }
            // Only a completed actual PHY read counts as runner progress.
            crate::watchdog::progress(bluetemp::application::supervision::Task::Network);
            state
        })
    }
}
fn hardware_mode(mode: Mode) -> (Speed, Duplex) {
    const MODES: [(Speed, Duplex); 4] = [
        (Speed::_100M, Duplex::Full),
        (Speed::_100M, Duplex::Half),
        (Speed::_10M, Duplex::Full),
        (Speed::_10M, Duplex::Half),
    ];
    MODES[mode as usize]
}
