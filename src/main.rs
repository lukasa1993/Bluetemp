#![no_std]
#![no_main]
#![recursion_limit = "256"]

mod board;
mod http;
mod network;
mod ota;
mod phy;
mod sensor;
mod watchdog;

use embassy_executor::Spawner;
use embassy_time::{Duration, Ticker};
use esp_backtrace as _;

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    esp_println::logger::init_logger_from_env();
    let board = board::init().await;
    spawner.spawn(watchdog::run().expect("watchdog task slot"));
    let stack = network::start(&spawner, board.ethernet);
    spawner.spawn(sensor::task(board.sensor).expect("sensor task slot"));
    http::start(&spawner, stack, board.flash);
    spawner.spawn(periodic(stack).expect("periodic task slot"));
    // This task owns the oscillator enable for the entire application lifetime.
    let _ethernet_enable = board.ethernet_enable;
    network::monitor(stack).await;
}

#[embassy_executor::task]
async fn periodic(stack: embassy_net::Stack<'static>) {
    let mut ticker = Ticker::every(Duration::from_secs(60));
    loop {
        ticker.next().await;
        esp_println::println!(
            "60-second tick: {:?}; IPv4={:?}",
            sensor::snapshot(),
            stack.config_v4()
        );
        watchdog::progress(bluetemp::application::supervision::Task::Periodic);
    }
}
