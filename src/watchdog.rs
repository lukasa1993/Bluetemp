//! RTC hardware reset plus task-watchdog's all-tasks-must-progress supervision.
use bluetemp::application::supervision::{self, EmbassyClock, Supervisor, Task};
use core::cell::RefCell;
use embassy_sync::blocking_mutex::{Mutex, raw::CriticalSectionRawMutex};
use embassy_time::Duration;
use esp_hal::rtc_cntl::{Rwdt, RwdtStage};
use task_watchdog::{HardwareWatchdog, ResetReason};

struct Hardware(Rwdt);
impl HardwareWatchdog<EmbassyClock> for Hardware {
    fn start(&mut self, timeout: Duration) {
        self.0.set_timeout(
            RwdtStage::Stage0,
            esp_hal::time::Duration::from_micros(timeout.as_micros()),
        );
        // HAL enable configures a full system/RTC reset; no interrupt handler is needed.
        self.0.enable();
        self.0.feed();
    }
    fn feed(&mut self) {
        self.0.feed();
    }
    fn trigger_reset(&mut self) -> ! {
        esp_hal::system::software_reset()
    }
    fn reset_reason(&self) -> Option<ResetReason> {
        None
    } // Log the more precise HAL reason at boot.
}

static SUPERVISOR: Mutex<
    CriticalSectionRawMutex,
    RefCell<Option<Supervisor<Hardware, EmbassyClock>>>,
> = Mutex::new(RefCell::new(None));

pub fn init(rwdt: Rwdt) {
    let supervisor = supervision::start(Hardware(rwdt), EmbassyClock);
    SUPERVISOR.lock(|slot| *slot.borrow_mut() = Some(supervisor));
}

/// Report a completed bounded operation, including a handled external-device error.
pub fn progress(task: Task) {
    SUPERVISOR.lock(|slot| {
        if let Some(supervisor) = slot.borrow_mut().as_mut() {
            supervisor.feed(&task);
        }
    });
}

#[embassy_executor::task]
pub async fn run() {
    supervision::run(check_tasks).await
}

fn check_tasks() -> bool {
    let starved = SUPERVISOR.lock(|slot| {
        slot.borrow_mut()
            .as_mut()
            .is_none_or(|supervisor| supervisor.check())
    });
    if starved {
        esp_println::println!("WATCHDOG: task progress deadline missed; waiting for RTC reset");
    }
    starved
}
