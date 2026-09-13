use bluetemp::application::{ota_record, protocol, update};

#[test]
fn hex_and_authenticated_manifest_match_python_vector() {
    let key = core::array::from_fn(|i| i as u8);
    let digest =
        protocol::hex32(b"c3bf47ea1f4a4a605470313cacb3a44f4a461f68c6faeab07e737610cb5ac835")
            .unwrap();
    let signature =
        protocol::hex32(b"625877214facd6092de768e25c6cec474ad14cbf85403c3bb3ccd01c5733ae01")
            .unwrap();
    assert!(update::authenticate(&key, 4096, &digest, &signature));
    assert!(!update::authenticate(&key, 4095, &digest, &signature));
    assert!(!update::authenticate(&[0; 32], 4096, &digest, &signature));
    assert!(!update::authenticate(&key, 4096, &[0; 32], &signature));
    assert!(!update::authenticate(&key, 0, &digest, &signature));
    assert!(!update::authenticate(
        &key,
        update::SLOT_SIZE + 1,
        &digest,
        &signature
    ));
    for bit in 0..256 {
        let mut invalid = signature;
        invalid[bit / 8] ^= 1 << (bit % 8);
        assert!(!update::authenticate(&key, 4096, &digest, &invalid));
    }
    assert!(protocol::hex32(b"00").is_none());
    assert!(protocol::hex32(&[b'g'; 64]).is_none());
}

#[test]
fn rejects_wrong_chip_project_and_truncated_images() {
    let mut bytes = [0; 288];
    bytes[0] = 0xe9;
    bytes[1] = 5;
    bytes[32..36].copy_from_slice(&[0x32, 0x54, 0xcd, 0xab]);
    bytes[80..89].copy_from_slice(b"bluetemp\0");
    assert!(update::compatible_image(&bytes));
    assert!(!update::compatible_image(&bytes[..287]));
    for index in [0, 12, 13, 32, 33, 34, 35, 80, 88] {
        let mut bad = bytes;
        bad[index] ^= 1;
        assert!(!update::compatible_image(&bad));
    }
    for count in [0, 17, 255] {
        bytes[1] = count;
        assert!(!update::compatible_image(&bytes));
    }
}

#[test]
fn ota_crc_matches_esp_rom_and_python_zlib_seed() {
    for (seq, expected) in [
        (1, 0x4743989a),
        (2, 0x55f63774),
        (3, 0xed4a5011),
        (0x12345678, 0x71d6a731),
    ] {
        assert_eq!(ota_record::sequence_crc(seq), expected);
        assert_eq!(ota_record::sequence(&ota_record::record(seq)), Some(seq));
    }
    assert_eq!(ota_record::sequence(&[255; 32]), None);
    assert_eq!(ota_record::sequence(&ota_record::record(0)), None);
    for state in [3u32, 4] {
        let mut entry = ota_record::record(1);
        entry[24..28].copy_from_slice(&state.to_le_bytes());
        assert_eq!(ota_record::sequence(&entry), None);
    }
}

#[test]
fn ota_alternates_slots_and_preserves_previous_record() {
    let mut records = [[255; 32]; 2];
    let mut running = 0;
    for expected in 2..20 {
        let (offset, entry) = ota_record::next(&records, running).unwrap();
        let seq = ota_record::sequence(&entry).unwrap();
        assert_eq!(seq, expected);
        assert_eq!((seq - 1) % 2, u32::from(1 - running));
        if expected > 2 {
            assert_eq!(
                ota_record::sequence(&records[1 - offset as usize / 4096]),
                Some(expected - 1)
            );
        }
        records[offset as usize / 4096] = entry;
        running = 1 - running;
    }
}

#[test]
fn ota_torn_metadata_and_sequence_exhaustion() {
    let previous = ota_record::record(7); // ota_0
    let next = ota_record::record(8);
    for cut in 0..32 {
        let mut torn = [255; 32];
        torn[..cut].copy_from_slice(&next[..cut]);
        if ota_record::sequence(&torn).is_some() {
            continue;
        }
        let (offset, entry) = ota_record::next(&[previous, torn], 0).unwrap();
        assert_eq!(offset, 4096);
        assert_eq!(ota_record::sequence(&entry), Some(8));
    }
    assert!(ota_record::next(&[ota_record::record(u32::MAX - 1), [255; 32]], 1).is_none());
    assert!(ota_record::next(&[[255; 32]; 2], 2).is_none());
}
