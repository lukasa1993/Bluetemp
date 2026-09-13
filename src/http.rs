use crate::{ota, sensor, watchdog};
use bluetemp::application::{
    connection,
    protocol::{Error, Manifest},
    supervision::Task,
    update_service::UpdateService,
    web::{self, Device, Status},
};
use embassy_executor::Spawner;
use embassy_net::{Stack, tcp::TcpSocket};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Instant, Timer, with_timeout};
use embedded_io_async::Read;
use esp_storage::FlashStorage;
use static_cell::StaticCell;

struct Services {
    stack: Stack<'static>,
    flash: UpdateService<FlashStorage<'static>>,
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
        let result = self
            .flash
            .update(
                async |flash| {
                    with_timeout(Duration::from_secs(180), ota::upload(body, flash, manifest))
                        .await
                        .map_err(|_| Error::Socket)?
                },
                || REBOOT.signal(()),
            )
            .await;
        log_update(result);
        result
    }
}

pub fn start(spawner: &Spawner, stack: Stack<'static>, flash: FlashStorage<'static>) {
    let services = &*SERVICES.init(Services {
        stack,
        flash: UpdateService::from(flash),
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
        socket.set_timeout(Some(Duration::from_secs(15)));
        let result = connection::serve(
            socket,
            async |socket| socket.accept(80).await,
            async |socket| {
                picoserve::Server::new(&app, &config, &mut buffer)
                    .serve(socket)
                    .await
                    .map(|_| ())
            },
        )
        .await;
        log_connection(id, result);
    }
}

#[embassy_executor::task]
async fn reboot() {
    REBOOT.wait().await;
    // Independent of response/connection errors: committed updates always reboot.
    Timer::after(Duration::from_secs(2)).await;
    esp_hal::system::software_reset();
}

fn log_connection(id: Task, result: Result<(), ()>) {
    if result.is_err() {
        esp_println::println!("HTTP {:?}: connection failed or timed out", id);
    }
}

fn log_update(result: Result<(), Error>) {
    if let Err(error) = result {
        esp_println::println!("OTA failed: {:?}", error);
    }
}
