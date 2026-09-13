//! Portable application services shared by the firmware and host tests.
pub mod connection;
pub use bluetemp_ota::flash_update;
pub mod link;
pub mod measurement;
pub use bluetemp_ota::ota_record;
pub mod protocol;
pub mod supervision;
pub use bluetemp_ota::update;
pub mod update_service;
pub mod web;
