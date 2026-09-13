use crate::Error;
use crate::{Manifest, ota_record, update, update::hex32};
use embedded_hal_async::delay::DelayNs;
use embedded_io_async::Read;
use embedded_storage::{ReadStorage, Storage};
use esp_bootloader_esp_idf::partitions::{
    self, AppPartitionSubType as App, DataPartitionSubType as Data, FlashStorage,
    PARTITION_TABLE_MAX_LEN, PartitionEntry, PartitionTable, PartitionType as Part,
};
use sha2::{Digest, Sha256};

pub fn authorize(key: Option<&str>, request: &Manifest) -> Result<[u8; 32], Error> {
    let key = key
        .and_then(|text| hex32(text.as_bytes()))
        .ok_or(Error::Authentication)?;
    let digest = request.digest;
    let signature = request.signature;
    if !update::authenticate(&key, request.content_length, &digest, &signature) {
        return Err(Error::Authentication);
    }
    Ok(digest)
}

pub struct Layout {
    pub running: u8,
    pub app: PartitionEntry,
    pub metadata: PartitionEntry,
}

pub fn layout(table: &PartitionTable<'_>, booted: PartitionEntry) -> Result<Layout, Error> {
    let (running, kind, expected_offsets) = match booted.partition_type() {
        Part::App(App::Ota0) => (0, App::Ota1, (0x10000, 0x200000, 0x9000)),
        Part::App(App::Ota1) => (1, App::Ota0, (0x200000, 0x10000, 0x9000)),
        _ => return Err(Error::Flash),
    };
    let app = required_partition(table, Part::App(kind))?;
    let metadata = required_partition(table, Part::Data(Data::Ota))?;
    validate_layout(booted, app, metadata, expected_offsets)?;
    Ok(Layout {
        running,
        app,
        metadata,
    })
}

fn validate_layout(
    booted: PartitionEntry,
    app: PartitionEntry,
    metadata: PartitionEntry,
    expected_offsets: (u32, u32, u32),
) -> Result<(), Error> {
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
    Ok(())
}

pub async fn transfer<R: Read, F: Storage, D: DelayNs>(
    body: &mut R,
    partition: &mut F,
    request: &Manifest,
    expected: [u8; 32],
    delay: &mut D,
) -> Result<(), Error> {
    receive(body, partition, request.content_length, delay).await?;
    if hash_partition(partition, request.content_length, delay).await? != expected {
        return Err(Error::Image);
    }
    Ok(())
}

async fn receive<R: Read, F: Storage, D: DelayNs>(
    body: &mut R,
    partition: &mut F,
    length: usize,
    delay: &mut D,
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
        delay.delay_ms(1).await;
    }
    Ok(())
}

async fn hash_partition<F: ReadStorage, D: DelayNs>(
    partition: &mut F,
    length: usize,
    delay: &mut D,
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
        delay.delay_ms(1).await;
    }
    Ok(hash.finalize().into())
}

pub fn commit<F: Storage>(metadata: &mut F, running: u8) -> Result<(), Error> {
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

fn required_partition(table: &PartitionTable<'_>, kind: Part) -> Result<PartitionEntry, Error> {
    table
        .find_partition(kind)
        .map_err(|_| Error::Flash)?
        .ok_or(Error::Flash)
}

fn prepare(
    flash: &mut FlashStorage<'_>,
    request: &Manifest,
    key: Option<&str>,
    booted: impl FnOnce(&PartitionTable<'_>) -> Result<Option<PartitionEntry>, partitions::Error>,
) -> Result<([u8; 32], Layout), Error> {
    let digest = authorize(key, request)?;
    let mut buffer = [0; PARTITION_TABLE_MAX_LEN];
    let table = partitions::read_partition_table(flash, &mut buffer).map_err(|_| Error::Flash)?;
    let booted = booted(&table)
        .map_err(|_| Error::Flash)?
        .ok_or(Error::Flash)?;
    layout(&table, booted).map(|layout| (digest, layout))
}

pub async fn upload<R: Read, D: DelayNs>(
    body: &mut R,
    flash: &mut FlashStorage<'_>,
    request: &Manifest,
    key: Option<&str>,
    booted: impl FnOnce(&PartitionTable<'_>) -> Result<Option<PartitionEntry>, partitions::Error>,
    delay: &mut D,
) -> Result<(), Error> {
    let (digest, layout) = prepare(flash, request, key, booted)?;
    transfer(
        body,
        &mut layout.app.as_flash_region(flash),
        request,
        digest,
        delay,
    )
    .await?;
    // No await after committing boot selection.
    commit(&mut layout.metadata.as_flash_region(flash), layout.running)
}
