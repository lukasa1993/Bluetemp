use bluetemp::supervision::{self, DEADLINES, HARDWARE_TIMEOUT_MS};
use std::{cell::Cell, rc::Rc};
use task_watchdog::{Clock, HardwareWatchdog, ResetReason};

#[derive(Clone, Default)]
struct TestClock(Rc<Cell<u64>>);
impl Clock for TestClock {
    type Instant = u64;
    type Duration = u64;
    fn now(&self) -> u64 {
        self.0.get()
    }
    fn elapsed_since(&self, instant: u64) -> u64 {
        self.now().saturating_sub(instant)
    }
    fn has_elapsed(&self, instant: u64, duration: &u64) -> bool {
        self.elapsed_since(instant) >= *duration
    }
    fn duration_from_millis(&self, millis: u64) -> u64 {
        millis
    }
}

#[derive(Clone, Default)]
struct Hardware {
    timeout: Rc<Cell<u64>>,
    feeds: Rc<Cell<u64>>,
}
impl HardwareWatchdog<TestClock> for Hardware {
    fn start(&mut self, timeout: u64) {
        self.timeout.set(timeout);
    }
    fn feed(&mut self) {
        self.feeds.set(self.feeds.get() + 1);
    }
    fn trigger_reset(&mut self) -> ! {
        panic!("unexpected software reset")
    }
    fn reset_reason(&self) -> Option<ResetReason> {
        None
    }
}

#[test]
fn every_task_independently_blocks_hardware_feeding_at_its_deadline() {
    for (missing, deadline) in DEADLINES {
        let clock = TestClock::default();
        let hardware = Hardware::default();
        let mut supervisor = supervision::start(hardware.clone(), clock.clone());
        assert_eq!(hardware.timeout.get(), HARDWARE_TIMEOUT_MS);
        assert!(!supervisor.check(), "startup grace: {missing:?}");
        clock.0.set(deadline - 1);
        for (task, _) in DEADLINES {
            if task != missing {
                supervisor.feed(&task);
            }
        }
        assert!(!supervisor.check(), "before deadline: {missing:?}");
        let feeds = hardware.feeds.get();
        clock.0.set(deadline);
        assert!(
            supervisor.check(),
            "missed task must stop feeding: {missing:?}"
        );
        assert_eq!(hardware.feeds.get(), feeds);
        clock.0.set(deadline + 1000);
        assert!(supervisor.check());
        assert_eq!(hardware.feeds.get(), feeds);
    }
}

#[test]
fn completed_work_keeps_all_tasks_alive_over_many_hardware_periods() {
    let clock = TestClock::default();
    let hardware = Hardware::default();
    let mut supervisor = supervision::start(hardware.clone(), clock.clone());
    for second in 1..=600 {
        clock.0.set(second * 1000);
        for (task, _) in DEADLINES {
            supervisor.feed(&task);
        }
        assert!(!supervisor.check());
    }
    assert_eq!(hardware.feeds.get(), 600);
    // A busy HTTP worker cannot hide a stopped network task, or vice versa.
    clock.0.set(610_000);
    supervisor.feed(&supervision::Task::Http0);
    supervisor.feed(&supervision::Task::Http1);
    assert!(supervisor.check());
    assert_eq!(hardware.feeds.get(), 600);
}
