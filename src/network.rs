use crate::board::EthDriver;
use embassy_executor::Spawner;
use embassy_futures::select::select;
use embassy_net::{Runner, Stack, StackResources};
use static_cell::StaticCell;

static RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();

pub fn start(spawner: &Spawner, ethernet: EthDriver) -> Stack<'static> {
    let rng = esp_hal::rng::Rng::new();
    let seed = (u64::from(rng.random()) << 32) | u64::from(rng.random());
    let (stack, runner) = embassy_net::new(
        ethernet,
        embassy_net::Config::dhcpv4(Default::default()),
        RESOURCES.init(StackResources::new()),
        seed,
    );
    spawner.spawn(run(runner).expect("network runner slot"));
    stack
}

#[embassy_executor::task]
async fn run(mut runner: Runner<'static, EthDriver>) {
    runner.run().await
}

pub async fn monitor(stack: Stack<'static>) -> ! {
    esp_println::println!("Ethernet started; waiting for link and DHCP");
    loop {
        let status = (stack.is_link_up(), stack.config_v4());
        esp_println::println!("Network link={} IPv4={:?}", status.0, status.1);
        select(
            wait_link_change(stack, status.0),
            wait_config_change(stack, status.1.is_some()),
        )
        .await;
    }
}

async fn wait_link_change(stack: Stack<'_>, up: bool) {
    if up {
        stack.wait_link_down().await
    } else {
        stack.wait_link_up().await
    }
}

async fn wait_config_change(stack: Stack<'_>, configured: bool) {
    if configured {
        stack.wait_config_down().await
    } else {
        stack.wait_config_up().await
    }
}
