use bluetemp::application::{
    flash_update, ota_record,
    protocol::{Error, Manifest},
    update,
};
use embedded_hal_async::delay::DelayNs;
use embedded_io_async::{ErrorKind, ErrorType, Read};
use embedded_storage::{ReadStorage, Storage};
use esp_bootloader_esp_idf::partitions::{AppPartitionSubType, PartitionTable, PartitionType};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

struct Memory {
    bytes: Vec<u8>,
    fail_read: Option<u32>,
    fail_write: Option<u32>,
    corrupt: bool,
}
impl Memory {
    fn new(size: usize) -> Self {
        Self {
            bytes: vec![255; size],
            fail_read: None,
            fail_write: None,
            corrupt: false,
        }
    }
}
impl ReadStorage for Memory {
    type Error = ();
    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), ()> {
        if self.fail_read == Some(offset) {
            return Err(());
        }
        bytes.copy_from_slice(
            self.bytes
                .get(offset as usize..offset as usize + bytes.len())
                .ok_or(())?,
        );
        if self.corrupt {
            bytes[0] ^= 1;
        }
        Ok(())
    }
    fn capacity(&self) -> usize {
        self.bytes.len()
    }
}
impl Storage for Memory {
    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), ()> {
        if self.fail_write == Some(offset) {
            return Err(());
        }
        self.bytes
            .get_mut(offset as usize..offset as usize + bytes.len())
            .ok_or(())?
            .copy_from_slice(bytes);
        Ok(())
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    fragment: usize,
    fail: bool,
}
impl ErrorType for Reader<'_> {
    type Error = ErrorKind;
}
impl Read for Reader<'_> {
    async fn read(&mut self, bytes: &mut [u8]) -> Result<usize, ErrorKind> {
        if self.fail {
            return Err(ErrorKind::Other);
        }
        let count = bytes.len().min(self.bytes.len()).min(self.fragment);
        bytes[..count].copy_from_slice(&self.bytes[..count]);
        self.bytes = &self.bytes[count..];
        Ok(count)
    }
}
#[derive(Default)]
struct Delay(usize);
impl DelayNs for Delay {
    async fn delay_ns(&mut self, ns: u32) {
        assert_eq!(ns, 1_000_000);
        self.0 += 1;
    }
}

fn image(size: usize) -> Vec<u8> {
    let mut bytes = vec![0x42; size];
    bytes[0] = 0xe9;
    bytes[1] = 5;
    bytes[12..14].fill(0);
    bytes[32..36].copy_from_slice(&[0x32, 0x54, 0xcd, 0xab]);
    bytes[80..89].copy_from_slice(b"bluetemp\0");
    bytes
}
fn manifest(bytes: &[u8]) -> Manifest {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let mut mac = Hmac::<Sha256>::new_from_slice(&[7; 32]).unwrap();
    mac.update(update::DOMAIN);
    mac.update(&(bytes.len() as u32).to_be_bytes());
    mac.update(&digest);
    Manifest {
        content_length: bytes.len(),
        digest,
        signature: mac.finalize().into_bytes().into(),
    }
}
async fn transfer(
    bytes: &[u8],
    request: &Manifest,
    memory: &mut Memory,
    fragment: usize,
) -> Result<usize, Error> {
    let mut reader = Reader {
        bytes,
        fragment,
        fail: false,
    };
    let mut delay = Delay::default();
    flash_update::transfer(&mut reader, memory, request, request.digest, &mut delay).await?;
    assert!(reader.bytes.is_empty());
    Ok(delay.0)
}

#[tokio::test]
async fn transfer_preserves_exact_bytes_and_verifies_flash_readback() {
    for size in [288, 4096, 5000, 8192, 9001] {
        let bytes = image(size);
        let request = manifest(&bytes);
        assert_eq!(
            flash_update::authorize(Some(&"07".repeat(32)), &request),
            Ok(request.digest)
        );
        for fragment in [1, 7, 4096] {
            let mut memory = Memory::new(size + 32);
            assert_eq!(
                transfer(&bytes, &request, &mut memory, fragment)
                    .await
                    .unwrap(),
                2 * size.div_ceil(4096)
            );
            assert_eq!(&memory.bytes[..size], &bytes);
            assert_eq!(&memory.bytes[size..], &[255; 32]);
        }
        let mut corrupt = Memory::new(size);
        corrupt.corrupt = true;
        assert_eq!(
            transfer(&bytes, &request, &mut corrupt, 4096).await,
            Err(Error::Image)
        );
    }
}

#[tokio::test]
async fn transfer_rejects_bad_images_socket_errors_and_flash_failures() {
    let bytes = image(5000);
    let request = manifest(&bytes);
    let mut wrong = bytes.clone();
    wrong[0] = 0;
    let mut memory = Memory::new(5000);
    assert_eq!(
        transfer(&wrong, &request, &mut memory, 4096).await,
        Err(Error::Image)
    );
    assert!(memory.bytes.iter().all(|b| *b == 255));
    assert_eq!(
        transfer(&bytes[..4999], &request, &mut memory, 7).await,
        Err(Error::Socket)
    );
    for offset in [0, 4096] {
        let mut memory = Memory::new(5000);
        memory.fail_write = Some(offset);
        assert_eq!(
            transfer(&bytes, &request, &mut memory, 4096).await,
            Err(Error::Flash)
        );
        let mut memory = Memory::new(5000);
        memory.fail_read = Some(offset);
        assert_eq!(
            transfer(&bytes, &request, &mut memory, 4096).await,
            Err(Error::Flash)
        );
    }
    let mut reader = Reader {
        bytes: &bytes,
        fragment: 4096,
        fail: true,
    };
    assert_eq!(
        flash_update::transfer(
            &mut reader,
            &mut memory,
            &request,
            request.digest,
            &mut Delay::default()
        )
        .await,
        Err(Error::Socket)
    );
    for key in [None, Some("bad"), Some(&"00".repeat(32))] {
        assert_eq!(
            flash_update::authorize(key, &request),
            Err(Error::Authentication)
        );
    }
}

fn table(entries: &[(u8, u8, u32, u32)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for &(kind, subtype, offset, size) in entries {
        let mut entry = [0; 32];
        entry[..2].copy_from_slice(&[0xaa, 0x50]);
        entry[2] = kind;
        entry[3] = subtype;
        entry[4..8].copy_from_slice(&offset.to_le_bytes());
        entry[8..12].copy_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&entry);
    }
    let mut checksum = [255; 32];
    checksum[..2].copy_from_slice(&[0xeb, 0xeb]);
    checksum[16..].copy_from_slice(&md5::Md5::digest(&bytes));
    bytes.extend_from_slice(&checksum);
    bytes
}

fn parse_table(bytes: &mut [u8]) -> PartitionTable<'_> {
    // Only the table test uses this official single-threaded mock flash.
    let mut flash = esp_bootloader_esp_idf::partitions::FlashStorage::new();
    flash.write(0x8000, bytes).unwrap();
    esp_bootloader_esp_idf::partitions::read_partition_table(&mut flash, bytes).unwrap()
}

#[tokio::test]
async fn official_partition_parser_and_layout_guard_both_ota_directions() {
    let entries = [
        (1, 0, 0x9000, 8192),
        (0, 0x10, 0x10000, 0x1f0000),
        (0, 0x11, 0x200000, 0x1f0000),
    ];
    let mut binary = table(&entries);
    let parsed = parse_table(&mut binary);
    for (running, subtype, target) in [
        (0, AppPartitionSubType::Ota0, 0x200000),
        (1, AppPartitionSubType::Ota1, 0x10000),
    ] {
        let booted = parsed
            .find_partition(PartitionType::App(subtype))
            .unwrap()
            .unwrap();
        let layout = flash_update::layout(&parsed, booted).unwrap();
        assert_eq!(layout.running, running);
        assert_eq!(layout.app.offset(), target);
        assert_eq!(layout.metadata.offset(), 0x9000);
    }
    for index in 0..3 {
        for field in [2, 3] {
            let mut invalid = entries;
            if field == 2 {
                invalid[index].2 += 4096;
            } else {
                invalid[index].3 -= 4096;
            }
            let mut binary = table(&invalid);
            let parsed = parse_table(&mut binary);
            assert!(flash_update::layout(&parsed, parsed.get_partition(1).unwrap()).is_err());
        }
    }
    // Missing target, missing metadata, and a booted non-OTA partition.
    for missing in [0, 2] {
        let filtered: Vec<_> = entries
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != missing)
            .map(|(_, e)| *e)
            .collect();
        let mut binary = table(&filtered);
        let parsed = parse_table(&mut binary);
        let booted = parsed
            .find_partition(PartitionType::App(AppPartitionSubType::Ota0))
            .unwrap()
            .unwrap();
        assert!(flash_update::layout(&parsed, booted).is_err());
    }
    assert!(flash_update::layout(&parsed, parsed.get_partition(0).unwrap()).is_err());
    let mut flash = esp_bootloader_esp_idf::partitions::FlashStorage::new();
    flash.write(0x8000, &table(&entries)).unwrap();
    let bytes = image(5000);
    let request = manifest(&bytes);
    let booted = |table: &PartitionTable<'_>| {
        table.find_partition(PartitionType::App(AppPartitionSubType::Ota1))
    };
    let mut old = [255; 8192];
    flash.write(0x9000, &old).unwrap();
    let key = "07".repeat(32);
    let mut reader = Reader {
        bytes: &bytes[..4999],
        fragment: 7,
        fail: false,
    };
    assert_eq!(
        flash_update::upload(
            &mut reader,
            &mut flash,
            &request,
            Some(&key),
            booted,
            &mut Delay::default()
        )
        .await,
        Err(Error::Socket)
    );
    flash.read(0x9000, &mut old).unwrap();
    assert_eq!(old, [255; 8192], "failed transfer must not commit metadata");
    let mut reader = Reader {
        bytes: &bytes,
        fragment: 7,
        fail: false,
    };
    flash_update::upload(
        &mut reader,
        &mut flash,
        &request,
        Some(&key),
        booted,
        &mut Delay::default(),
    )
    .await
    .unwrap();
    let mut copied = vec![0; bytes.len()];
    flash.read(0x10000, &mut copied).unwrap();
    assert_eq!(copied, bytes);
    flash.read(0x9000, &mut old).unwrap();
    assert_eq!(ota_record::sequence(old[..32].try_into().unwrap()), Some(1));
    for missing in [false, true] {
        let mut reader = Reader {
            bytes: &bytes,
            fragment: 7,
            fail: false,
        };
        assert_eq!(
            flash_update::upload(
                &mut reader,
                &mut flash,
                &request,
                Some(&key),
                |_| {
                    if missing {
                        Ok(None)
                    } else {
                        Err(esp_bootloader_esp_idf::partitions::Error::Invalid)
                    }
                },
                &mut Delay::default()
            )
            .await,
            Err(Error::Flash)
        );
    }
    let mut reader = Reader {
        bytes: &bytes,
        fragment: 7,
        fail: false,
    };
    assert_eq!(
        flash_update::upload(
            &mut reader,
            &mut flash,
            &request,
            None,
            booted,
            &mut Delay::default()
        )
        .await,
        Err(Error::Authentication)
    );
    flash.write(0x8000, &[0; 32]).unwrap();
    assert_eq!(
        flash_update::upload(
            &mut reader,
            &mut flash,
            &request,
            Some(&key),
            booted,
            &mut Delay::default()
        )
        .await,
        Err(Error::Flash)
    );
}

#[test]
fn metadata_commit_preserves_running_record_and_propagates_storage_errors() {
    let mut memory = Memory::new(8192);
    flash_update::commit(&mut memory, 0).unwrap();
    let first = memory.bytes.clone();
    flash_update::commit(&mut memory, 1).unwrap();
    assert_eq!(&memory.bytes[..32], &first[..32]);
    let seq = ota_record::sequence(memory.bytes[4096..4128].try_into().unwrap());
    assert_eq!(seq, Some(3));
    for offset in [0, 4096] {
        let mut memory = Memory::new(8192);
        memory.fail_read = Some(offset);
        assert_eq!(flash_update::commit(&mut memory, 0), Err(Error::Flash));
        assert!(memory.bytes.iter().all(|b| *b == 255));
    }
    let mut memory = Memory::new(8192);
    memory.fail_write = Some(0);
    assert_eq!(flash_update::commit(&mut memory, 0), Err(Error::Flash));
    assert_eq!(flash_update::commit(&mut memory, 2), Err(Error::Flash));
}
