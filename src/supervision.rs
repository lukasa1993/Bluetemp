//! Task deadlines implemented by the community watchdog core, without legacy HALs.
use task_watchdog::{Clock, HardwareWatchdog, Watchdog, WatchdogConfig};

pub const HARDWARE_TIMEOUT_MS: u64 = 30_000;
pub const CHECK_INTERVAL_MS: u64 = 1_000;

// Unit enum variants are required: task-watchdog 0.1.2 identifies discriminants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    Sensor,
    Network,
    Http0,
    Http1,
    Periodic,
}
impl task_watchdog::Id for Task {}

pub const DEADLINES: [(Task, u64); 5] = [
    (Task::Sensor, 20_000),
    (Task::Network, 10_000),
    (Task::Http0, 210_000),
    (Task::Http1, 210_000),
    (Task::Periodic, 90_000),
];

pub type Supervisor<W, C> = Watchdog<Task, 5, W, C>;

pub fn start<W: HardwareWatchdog<C>, C: Clock>(hardware: W, clock: C) -> Supervisor<W, C> {
    let config = WatchdogConfig::new(HARDWARE_TIMEOUT_MS, CHECK_INTERVAL_MS, &clock);
    let durations = DEADLINES.map(|(task, ms)| (task, clock.duration_from_millis(ms)));
    let mut supervisor = Watchdog::new(hardware, config, clock);
    for (task, duration) in durations {
        assert!(supervisor.register_task(&task, duration).is_ok());
    }
    supervisor.start();
    supervisor
}
