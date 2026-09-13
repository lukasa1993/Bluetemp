//! Application upload policy applied to headers parsed by picoserve.
use picoserve::request::{HeaderName, Headers};

pub use bluetemp_ota::{Error, Manifest};

pub use super::update::hex32;

#[derive(Default)]
struct Fields {
    length: Option<usize>,
    digest: Option<[u8; 32]>,
    signature: Option<[u8; 32]>,
}

impl Fields {
    fn unique<T>(slot: &mut Option<T>, value: Option<T>) -> Result<(), Error> {
        if slot.is_some() {
            return Err(Error::Request);
        }
        *slot = Some(value.ok_or(Error::Request)?);
        Ok(())
    }

    fn length(value: &[u8]) -> Result<usize, Error> {
        let text = core::str::from_utf8(value).map_err(|_| Error::Request)?;
        if !value.iter().all(u8::is_ascii_digit) {
            return Err(Error::Request);
        }
        text.parse().map_err(|_| Error::Request)
    }

    fn insert_length(&mut self, value: &[u8]) -> Result<(), Error> {
        Self::unique(&mut self.length, Some(Self::length(value)?))
    }

    fn insert(&mut self, name: HeaderName<'_>, value: &[u8]) -> Result<(), Error> {
        if name == "content-length" {
            self.insert_length(value)
        } else if name == "x-image-sha256" {
            Self::unique(&mut self.digest, hex32(value))
        } else if name == "x-image-signature" {
            Self::unique(&mut self.signature, hex32(value))
        } else if name == "transfer-encoding" || name == "expect" {
            Err(Error::Request)
        } else {
            Ok(())
        }
    }

    fn read(headers: Headers<'_>) -> Result<Self, Error> {
        let mut fields = Self::default();
        for (name, value) in headers.iter() {
            fields.insert(name, value.as_raw())?;
        }
        Ok(fields)
    }
}

pub fn validate_headers(headers: Headers<'_>) -> Result<(), Error> {
    Fields::read(headers).map(|_| ())
}

pub fn manifest(headers: Headers<'_>) -> Result<Manifest, Error> {
    let fields = Fields::read(headers)?;
    Ok(Manifest {
        content_length: fields.length.ok_or(Error::Request)?,
        digest: fields.digest.ok_or(Error::Authentication)?,
        signature: fields.signature.ok_or(Error::Authentication)?,
    })
}
