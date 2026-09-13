//! Application upload policy applied to headers parsed by picoserve.
use picoserve::request::Headers;

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

pub fn hex32(input: &[u8]) -> Option<[u8; 32]> {
    let mut result = [0; 32];
    hex::decode_to_slice(input, &mut result).ok()?;
    Some(result)
}

#[derive(Default)]
struct Fields {
    length: Option<usize>,
    digest: Option<[u8; 32]>,
    signature: Option<[u8; 32]>,
}

impl Fields {
    fn read(headers: Headers<'_>) -> Result<Self, Error> {
        let mut fields = Self::default();
        for (name, value) in headers.iter() {
            let value = value.as_raw();
            if name == "content-length" {
                let text = core::str::from_utf8(value).map_err(|_| Error::Request)?;
                if !value.iter().all(u8::is_ascii_digit) {
                    return Err(Error::Request);
                }
                unique(&mut fields.length, text.parse().ok())?;
            } else if name == "x-image-sha256" {
                unique(&mut fields.digest, hex32(value))?;
            } else if name == "x-image-signature" {
                unique(&mut fields.signature, hex32(value))?;
            } else if name == "transfer-encoding" || name == "expect" {
                return Err(Error::Request);
            }
        }
        Ok(fields)
    }
}

fn unique<T>(slot: &mut Option<T>, value: Option<T>) -> Result<(), Error> {
    if slot.is_some() {
        return Err(Error::Request);
    }
    *slot = Some(value.ok_or(Error::Request)?);
    Ok(())
}

pub fn validate_headers(headers: Headers<'_>) -> Result<(), Error> {
    Fields::read(headers).map(|_| ())
}

impl Manifest {
    pub fn from_headers(headers: Headers<'_>) -> Result<Self, Error> {
        let fields = Fields::read(headers)?;
        Ok(Self {
            content_length: fields.length.ok_or(Error::Request)?,
            digest: fields.digest.ok_or(Error::Authentication)?,
            signature: fields.signature.ok_or(Error::Authentication)?,
        })
    }
}
