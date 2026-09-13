use crate::{ota, sensor, watchdog};
use bluetemp::{
    protocol::{Error, Manifest},
    supervision::Task,
    web::{self, Device, Status},
};
use embassy_executor::Spawner;
use embassy_net::{Stack, tcp::TcpSocket};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{Duration, Instant, Timer, with_timeout};
use embedded_io_async::Read;
use esp_storage::FlashStorage;
use static_cell::StaticCell;

struct FlashState {
    storage: FlashStorage<'static>,
    committed: bool,
}

struct Services {
    stack: Stack<'static>,
    flash: Mutex<CriticalSectionRawMutex, FlashState>,
}

static SERVICES: StaticCell<Services> = StaticCell::new();
static REBOOT: Signal<CriticalSectionRawMutex, ()> = Signal::new();

impl Device for &Services {
    fn status(&self) -> Status {
        let mut mac = [0; 6];
        mac.copy_from_slice(esp_hal::efuse::base_mac_address().as_bytes());
        Status {
            reading: sensor::snapshot(),
            uptime_ms: Instant::now().as_millis(),
            mac,
            ipv4: self.stack.config_v4().map(|c| c.address.address()),
        }
    }

    async fn update<R: Read>(&self, manifest: &Manifest, body: &mut R) -> Result<(), Error> {
        // Never queue another upload behind flash work or overwrite a committed image.
        let mut flash = self.flash.try_lock().map_err(|_| Error::Busy)?;
        if flash.committed {
            return Err(Error::Busy);
        }
        let result = with_timeout(
            Duration::from_secs(180),
            ota::upload(body, &mut flash.storage, manifest),
        )
        .await
        .map_err(|_| Error::Socket)?;
        if result.is_ok() {
            // No await between successful metadata commit, exclusion and reboot signal.
            flash.committed = true;
            REBOOT.signal(());
        } else {
            esp_println::println!("OTA failed: {:?}", result);
        }
        result
    }
}

pub fn start(spawner: &Spawner, stack: Stack<'static>, flash: FlashStorage<'static>) {
    let services = &*SERVICES.init(Services {
        stack,
        flash: Mutex::new(FlashState {
            storage: flash,
            committed: false,
        }),
    });
    spawner.spawn(worker(services, Task::Http0).expect("HTTP worker 0 slot"));
    spawner.spawn(worker(services, Task::Http1).expect("HTTP worker 1 slot"));
    spawner.spawn(reboot().expect("OTA reboot task slot"));
}

#[embassy_executor::task(pool_size = 2)]
async fn worker(services: &'static Services, id: Task) {
    let mut rx = [0; 4096];
    let mut tx = [0; 2048];
    let mut buffer = [0; 1536];
    let app = web::router(services);
    let config = picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Duration::from_secs(10),
        persistent_start_read_request: Duration::from_secs(10),
        read_request: Duration::from_secs(10),
        write: Duration::from_secs(2),
    });
    loop {
        watchdog::progress(id);
        let mut socket = TcpSocket::new(services.stack, &mut rx, &mut tx);
        // Bounded accept makes idle progress observable even with no cable or DHCP.
        if !matches!(
            with_timeout(Duration::from_secs(5), socket.accept(80)).await,
            Ok(Ok(()))
        ) {
            Timer::after(Duration::from_millis(100)).await;
            continue;
        }
        socket.set_timeout(Some(Duration::from_secs(15)));
        // Picoserve owns HTTP; Embassy bounds the entire connection, including shutdown.
        let result = with_timeout(
            Duration::from_secs(190),
            picoserve::Server::new(&app, &config, &mut buffer).serve(socket),
        )
        .await;
        if !matches!(result, Ok(Ok(_))) {
            esp_println::println!("HTTP {:?}: connection failed or timed out", id);
        }
    }
}

#[embassy_executor::task]
async fn reboot() {
    REBOOT.wait().await;
    // Independent of response/connection errors: committed updates always reboot.
    Timer::after(Duration::from_secs(2)).await;
    esp_hal::system::software_reset();
}
