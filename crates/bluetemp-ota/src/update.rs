//! Authenticated OTA manifest. The payload is hashed again after writing flash.
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub const SLOT_SIZE: usize = 0x1f0000;
pub const DOMAIN: &[u8] = b"bluetemp-ota-v1\0";

pub fn authenticate(
    key: &[u8; 32],
    length: usize,
    digest: &[u8; 32],
    signature: &[u8; 32],
) -> bool {
    if !(288..=SLOT_SIZE).contains(&length) {
        return false;
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts a 32-byte key");
    mac.update(DOMAIN);
    mac.update(&(length as u32).to_be_bytes());
    mac.update(digest);
    mac.verify_slice(signature).is_ok()
}

/// espflash application image: ESP32 chip ID 0, descriptor at 0x20.
/// A signed manifest authenticates the full image; this guards operator mistakes.
pub fn compatible_image(bytes: &[u8]) -> bool {
    bytes.len() >= 288
        && bytes[0] == 0xe9
        && (1..=16).contains(&bytes[1])
        && bytes[12..14] == [0, 0]
        && bytes[32..36] == [0x32, 0x54, 0xcd, 0xab]
        && bytes[80..89] == *b"bluetemp\0"
}

pub fn hex32(input: &[u8]) -> Option<[u8; 32]> {
    let mut result = [0; 32];
    hex::decode_to_slice(input, &mut result).ok()?;
    Some(result)
}
