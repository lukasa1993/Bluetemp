//! ESP-IDF's two-sector OTA selection journal, for exactly two application slots.
//! Record layout and CRC match bootloader_common_ota_select_valid().

pub fn sequence_crc(sequence: u32) -> u32 {
    // ESP ROM crc32_le(0xffffffff, ...) uses zero internal initialization.
    const OTA_CRC: crc::Crc<u32> = crc::Crc::<u32>::new(&crc::Algorithm {
        init: 0,
        ..crc::CRC_32_ISO_HDLC
    });
    OTA_CRC.checksum(&sequence.to_le_bytes())
}

pub fn sequence(record: &[u8; 32]) -> Option<u32> {
    let seq = u32::from_le_bytes([record[0], record[1], record[2], record[3]]);
    let state = u32::from_le_bytes([record[24], record[25], record[26], record[27]]);
    let crc = u32::from_le_bytes([record[28], record[29], record[30], record[31]]);
    if !(1..u32::MAX).contains(&seq) || [3, 4].contains(&state) || crc != sequence_crc(seq) {
        return None;
    }
    Some(seq)
}

pub fn record(sequence: u32) -> [u8; 32] {
    let mut bytes = [0xff; 32];
    bytes[..4].copy_from_slice(&sequence.to_le_bytes());
    // Undefined: stock espflash bootloader has no application health rollback.
    bytes[28..].copy_from_slice(&sequence_crc(sequence).to_le_bytes());
    bytes
}

/// Choose the metadata sector that does not hold the running slot's newest record.
/// Exhausted sequence numbers fail closed, never wrap into an erased marker.
pub fn next(records: &[[u8; 32]; 2], running_slot: u8) -> Option<(u32, [u8; 32])> {
    if running_slot > 1 {
        return None;
    }
    let sequences = records.map(|entry| sequence(&entry));
    let newest = sequences.iter().flatten().copied().max().unwrap_or(0);
    let current = sequences
        .iter()
        .enumerate()
        .filter_map(|(index, seq)| {
            seq.filter(|n| app_slot(*n) == running_slot)
                .map(|n| (index, n))
        })
        .max_by_key(|(_, seq)| *seq)
        .map(|(index, _)| index);
    let sector = current.map_or(0, |index| 1 - index);
    let target_slot = 1 - running_slot;
    let mut next = newest.checked_add(1)?;
    if app_slot(next) != target_slot {
        next = next.checked_add(1)?;
    }
    if next == u32::MAX {
        return None;
    }
    Some(((sector * 4096) as u32, record(next)))
}

fn app_slot(sequence: u32) -> u8 {
    // Equivalent to (sequence - 1) % 2 for the nonzero sequence numbers used here.
    u8::from(sequence & 1 == 0)
}
