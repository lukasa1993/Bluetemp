use bluetemp::{
    measurement::{Measurement, Reading},
    protocol::{Error, Manifest},
    update,
    web::{self, Device, Status},
};
use embedded_io_async::Read;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::{cell::Cell, net::Ipv4Addr, rc::Rc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Default)]
struct TestDevice {
    reading: Reading,
    now: u64,
    no_address: bool,
    busy: bool,
    committed: Rc<Cell<bool>>,
}

impl Device for TestDevice {
    fn status(&self) -> Status {
        Status {
            reading: self.reading,
            uptime_ms: self.now,
            mac: [2, 1, 2, 3, 4, 5],
            ipv4: (!self.no_address).then_some(Ipv4Addr::new(192, 168, 19, 150)),
        }
    }
    async fn update<R: Read>(&self, manifest: &Manifest, body: &mut R) -> Result<(), Error> {
        if self.busy {
            return Err(Error::Busy);
        }
        if !update::authenticate(
            &[7; 32],
            manifest.content_length,
            &manifest.digest,
            &manifest.signature,
        ) {
            return Err(Error::Authentication);
        }
        let mut bytes = vec![0; manifest.content_length];
        body.read_exact(&mut bytes)
            .await
            .map_err(|_| Error::Socket)?;
        if Sha256::digest(&bytes)[..] != manifest.digest {
            return Err(Error::Image);
        }
        self.committed.set(true);
        Ok(())
    }
}

async fn exchange(device: TestDevice, request: &[u8], fragment: usize) -> String {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = async {
        let (socket, _) = listener.accept().await.unwrap();
        let app = web::router(device);
        let mut buffer = [0; 1536];
        let config = picoserve::Config::new(picoserve::Timeouts {
            start_read_request: picoserve::time::Duration::from_secs(1),
            persistent_start_read_request: picoserve::time::Duration::from_secs(1),
            read_request: picoserve::time::Duration::from_secs(1),
            write: picoserve::time::Duration::from_secs(1),
        });
        picoserve::Server::new_tokio(&app, &config, &mut buffer)
            .serve(socket)
            .await
    };
    let client = async {
        let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
        for chunk in request.chunks(fragment) {
            socket.write_all(chunk).await.unwrap();
            tokio::task::yield_now().await;
        }
        socket.shutdown().await.unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).await.unwrap();
        response
    };
    let (_, response) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(server, client)
    })
    .await
    .unwrap();
    response
}

fn body(response: &str) -> &str {
    response.split_once("\r\n\r\n").unwrap().1
}

#[tokio::test]
async fn real_http_routes_and_stale_json() {
    let mut device = TestDevice::default();
    let initial = exchange(
        device.clone(),
        b"GET /api/reading HTTP/1.1\r\nHost: sensor\r\n\r\n",
        1,
    )
    .await;
    assert!(initial.starts_with("HTTP/1.1 503"), "{initial}");
    let value: serde_json::Value = serde_json::from_str(body(&initial)).unwrap();
    assert_eq!(value["fresh"], false);
    assert!(value["temperature_c"].is_null());
    assert!(value["age_ms"].is_null());
    device.reading = Reading {
        value: Some(Measurement {
            temperature_c: 25.5,
            humidity_percent: 61.25,
        }),
        sampled_ms: 100,
        errors: 2,
        last_ok: true,
    };
    device.now = 200;
    let text = exchange(device.clone(), b"GET / HTTP/1.1\r\n\r\n", 4096).await;
    assert!(text.starts_with("HTTP/1.1 200"));
    assert_eq!(
        body(&text),
        "02:01:02:03:04:05 - 200 - 25.50 - 61.25 - 192.168.19.150\n"
    );
    for (path, code) in [
        ("/health", 200),
        ("/ready", 200),
        ("/missing", 404),
        ("/api/reading", 200),
    ] {
        let raw = format!("GET {path} HTTP/1.1\r\n\r\n");
        let response = exchange(device.clone(), raw.as_bytes(), 2).await;
        assert!(
            response.starts_with(&format!("HTTP/1.1 {code}")),
            "{response}"
        );
    }
    device.no_address = true;
    for path in ["/", "/ready"] {
        let raw = format!("GET {path} HTTP/1.1\r\n\r\n");
        let response = exchange(device.clone(), raw.as_bytes(), 4096).await;
        assert!(response.starts_with("HTTP/1.1 503"), "{response}");
    }
    device.no_address = false;
    device.now = 15_101;
    for (path, code) in [("/", 503), ("/ready", 503), ("/health", 200)] {
        let raw = format!("GET {path} HTTP/1.1\r\n\r\n");
        let response = exchange(device.clone(), raw.as_bytes(), 4096).await;
        assert!(
            response.starts_with(&format!("HTTP/1.1 {code}")),
            "{response}"
        );
    }
    let stale = exchange(device, b"GET /api/reading HTTP/1.1\r\n\r\n", 4096).await;
    assert!(stale.starts_with("HTTP/1.1 503"));
    let value: serde_json::Value = serde_json::from_str(body(&stale)).unwrap();
    assert_eq!(value["fresh"], false);
    assert_eq!(value["temperature_c"], 25.5);
    assert_eq!(value["errors"], 2);
}

fn signed_request(bytes: &[u8]) -> Vec<u8> {
    let digest = Sha256::digest(bytes);
    let mut hmac = Hmac::<Sha256>::new_from_slice(&[7; 32]).unwrap();
    hmac.update(b"bluetemp-ota-v1\0");
    hmac.update(&(bytes.len() as u32).to_be_bytes());
    hmac.update(&digest);
    let mut request = format!("POST /ota HTTP/1.1\r\nContent-Length: {}\r\nX-Image-SHA256: {:x}\r\nX-Image-Signature: {:x}\r\n\r\n", bytes.len(), digest, hmac.finalize().into_bytes()).into_bytes();
    request.extend_from_slice(bytes);
    request
}

#[tokio::test]
async fn streamed_upload_survives_fragmentation_and_rejects_truncation() {
    let bytes = vec![0x42; 5000];
    let request = signed_request(&bytes);
    for fragment in [1, 7, 1024, request.len()] {
        let device = TestDevice::default();
        let response = exchange(device.clone(), &request, fragment).await;
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(device.committed.get());
    }
    let device = TestDevice::default();
    let response = exchange(device.clone(), &request[..request.len() - 1], 4096).await;
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    assert!(!device.committed.get());
    let mut corrupt = request;
    *corrupt.last_mut().unwrap() ^= 1;
    let device = TestDevice::default();
    let response = exchange(device.clone(), &corrupt, 4096).await;
    assert!(response.starts_with("HTTP/1.1 400"));
    assert!(!device.committed.get());
}

#[tokio::test]
async fn rejects_ambiguous_lengths_and_bad_authentication() {
    for headers in [
        "",
        "Content-Length: -1\r\n",
        "Content-Length: +1\r\n",
        "Content-Length: \r\n",
        "Content-Length: 0\r\ncontent-length: 0\r\n",
        "Transfer-Encoding: chunked\r\n",
        "Content-Length: 0\r\nExpect: 100-continue\r\n",
        "Content-Length: 999999999999999999999999999\r\n",
        "Content-Length: 0\r\nX-Image-SHA256: bad\r\n",
        "Content-Length: 0\r\nX-Image-Signature: bad\r\n",
    ] {
        let raw = format!("POST /ota HTTP/1.1\r\n{headers}\r\n");
        let device = TestDevice::default();
        let response = exchange(device.clone(), raw.as_bytes(), 1).await;
        assert!(
            response.starts_with("HTTP/1.1 400"),
            "{headers}: {response}"
        );
        assert!(!device.committed.get());
    }
    let device = TestDevice::default();
    let response = exchange(
        device.clone(),
        b"POST /ota HTTP/1.1\r\nContent-Length: 0\r\n\r\n",
        4096,
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 403"));
    assert!(!device.committed.get());
}

#[tokio::test]
async fn busy_upload_is_reported_without_committing() {
    let device = TestDevice {
        busy: true,
        ..Default::default()
    };
    let response = exchange(device.clone(), &signed_request(&[1, 2, 3]), 4096).await;
    assert!(response.starts_with("HTTP/1.1 503"), "{response}");
    assert!(!device.committed.get());
}
