use bluetemp::application::{
    flash_update,
    protocol::{Error, Manifest},
};
use embassy_time::Delay;
use embedded_io_async::Read;
use esp_storage::FlashStorage;

pub async fn upload<R: Read>(
    body: &mut R,
    flash: &mut FlashStorage<'_>,
    request: &Manifest,
) -> Result<(), Error> {
    esp_println::println!("OTA: {} bytes requested", request.content_length);
    flash_update::upload(
        body,
        flash,
        request,
        option_env!("BLUETEMP_OTA_KEY"),
        |table| table.booted_partition(),
        &mut Delay,
    )
    .await
}
