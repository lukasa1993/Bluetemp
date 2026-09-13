use bluetemp::protocol::Error;
use bluetemp::{
    ota_record,
    protocol::{Manifest, hex32},
    update,
};
use embassy_time::{Duration, Timer};
use embedded_io_async::Read;
use esp_bootloader_esp_idf::partitions::{
    self, AppPartitionSubType as App, DataPartitionSubType as Data, FlashRegion,
    PARTITION_TABLE_MAX_LEN, PartitionEntry, PartitionType as Part,
};
use esp_storage::FlashStorage;
use sha2::{Digest, Sha256};

fn authorize(request: &Manifest) -> Result<[u8; 32], Error> {
    // Missing key disables OTA. No default credential is shipped in source.
    let key = option_env!("BLUETEMP_OTA_KEY")
        .and_then(|text| hex32(text.as_bytes()))
        .ok_or(Error::Authentication)?;
    let digest = request.digest;
    let signature = request.signature;
    if !update::authenticate(&key, request.content_length, &digest, &signature) {
        return Err(Error::Authentication);
    }
    Ok(digest)
}

struct Layout {
    running: u8,
    app: PartitionEntry,
    metadata: PartitionEntry,
}

fn layout(flash: &mut FlashStorage<'_>) -> Result<Layout, Error> {
    let mut buffer = [0; PARTITION_TABLE_MAX_LEN];
    let table = partitions::read_partition_table(flash, &mut buffer).map_err(|_| Error::Flash)?;
    let booted = table
        .booted_partition()
        .map_err(|_| Error::Flash)?
        .ok_or(Error::Flash)?;
    let (running, kind, expected_offsets) = match booted.partition_type() {
        Part::App(App::Ota0) => (0, App::Ota1, (0x10000, 0x200000, 0x9000)),
        Part::App(App::Ota1) => (1, App::Ota0, (0x200000, 0x10000, 0x9000)),
        _ => return Err(Error::Flash),
    };
    let app = table
        .find_partition(Part::App(kind))
        .map_err(|_| Error::Flash)?
        .ok_or(Error::Flash)?;
    let metadata = table
        .find_partition(Part::Data(Data::Ota))
        .map_err(|_| Error::Flash)?
        .ok_or(Error::Flash)?;
    let actual = (
        (booted.offset(), app.offset(), metadata.offset()),
        (booted.len(), app.len(), metadata.len()),
    );
    let expected = (
        expected_offsets,
        (update::SLOT_SIZE as u32, update::SLOT_SIZE as u32, 8192),
    );
    if actual != expected {
        return Err(Error::Flash);
    }
    Ok(Layout {
        running,
        app,
        metadata,
    })
}

pub async fn upload<R: Read>(
    body: &mut R,
    flash: &mut FlashStorage<'_>,
    request: &Manifest,
) -> Result<(), Error> {
    let expected = authorize(request)?;
    let layout = layout(flash)?;
    esp_println::println!(
        "OTA: {} bytes to {:?}",
        request.content_length,
        layout.app.partition_type()
    );
    let mut partition = layout.app.as_flash_region(flash);
    receive(body, &mut partition, request.content_length).await?;
    if hash_partition(&mut partition, request.content_length).await? != expected {
        return Err(Error::Image);
    }
    // No awaits after commit: return the reboot decision even if the client vanishes.
    commit(&mut layout.metadata.as_flash_region(flash), layout.running)
}

async fn receive<R: Read>(
    body: &mut R,
    partition: &mut FlashRegion<'_, '_>,
    length: usize,
) -> Result<(), Error> {
    let mut block = [0; 4096];
    let mut offset = 0;
    while offset < length {
        let size = block.len().min(length - offset);
        body.read_exact(&mut block[..size])
            .await
            .map_err(|_| Error::Socket)?;
        if offset == 0 && !update::compatible_image(&block[..size]) {
            return Err(Error::Image);
        }
        partition
            .write(offset as u32, &block[..size])
            .map_err(|_| Error::Flash)?;
        offset += size;
        // Flash operations block briefly. Yield between sectors for Ethernet and timers.
        Timer::after(Duration::from_millis(1)).await;
    }
    Ok(())
}

async fn hash_partition(
    partition: &mut FlashRegion<'_, '_>,
    length: usize,
) -> Result<[u8; 32], Error> {
    let mut block = [0; 4096];
    let mut hash = Sha256::new();
    let mut offset = 0;
    while offset < length {
        let size = block.len().min(length - offset);
        partition
            .read(offset as u32, &mut block[..size])
            .map_err(|_| Error::Flash)?;
        hash.update(&block[..size]);
        offset += size;
        Timer::after(Duration::from_millis(1)).await;
    }
    Ok(hash.finalize().into())
}

fn commit(metadata: &mut FlashRegion<'_, '_>, running: u8) -> Result<(), Error> {
    let mut records = [[0; 32]; 2];
    metadata
        .read(0, &mut records[0])
        .map_err(|_| Error::Flash)?;
    metadata
        .read(4096, &mut records[1])
        .map_err(|_| Error::Flash)?;
    let (offset, record) = ota_record::next(&records, running).ok_or(Error::Flash)?;
    metadata.write(offset, &record).map_err(|_| Error::Flash)
}
