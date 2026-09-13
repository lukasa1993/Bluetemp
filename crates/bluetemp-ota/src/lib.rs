#![no_std]

pub mod flash_update;
pub mod ota_record;
pub mod update;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Request,
    Authentication,
    Socket,
    Flash,
    Image,
    Busy,
}

#[derive(Clone, Copy, Debug)]
pub struct Manifest {
    pub content_length: usize,
    pub digest: [u8; 32],
    pub signature: [u8; 32],
}
