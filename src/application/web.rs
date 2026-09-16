//! Shared picoserve application, used by the MCU and host HTTP integration tests.
use super::{
    measurement::Reading,
    protocol::{self, Error, Manifest},
};
use core::{fmt::Write, net::Ipv4Addr};
use heapless::String;
use picoserve::{
    ResponseSent, Router,
    extract::{FromRef, State},
    io::Read,
    request::{Request, RequestParts},
    response::{IntoResponse, Response, ResponseWriter, StatusCode},
    routing::{self, Layer, Next, PathRouter, RequestHandlerService},
};

pub struct Status {
    pub reading: Reading,
    pub uptime_ms: u64,
    pub mac: [u8; 6],
    pub ipv4: Option<Ipv4Addr>,
}

/// Device services keep platform I/O out of HTTP parsing and routing.
#[allow(async_fn_in_trait)]
pub trait Device {
    fn status(&self) -> Status;
    /// Must bound the entire upload, and request reboot immediately after committing it.
    async fn update<R: Read>(&self, manifest: &Manifest, body: &mut R) -> Result<(), Error>;
}

impl<D: Device> FromRef<D> for Status {
    fn from_ref(device: &D) -> Self {
        device.status()
    }
}

pub fn router<D: Device>(device: D) -> Router<impl PathRouter> {
    Router::<_, D>::new()
        .route("/", routing::get(text))
        .route("/api/reading", routing::get(json))
        .route("/health", routing::get(async || "bluetemp alive\n"))
        .route("/ready", routing::get(ready))
        .route("/ota", routing::post_service(Upload))
        .layer(StrictHeaders)
        .with_state(device)
}

async fn ready(State(status): State<Status>) -> impl IntoResponse {
    Response::new(
        reading_status(status.reading.is_fresh(status.uptime_ms) && status.ipv4.is_some()),
        "Readiness requires a fresh sensor sample and IPv4 configuration\n",
    )
    .with_header("Cache-Control", "no-store")
}

fn reading_status(fresh: bool) -> StatusCode {
    if fresh {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn text(State(status): State<Status>) -> impl IntoResponse {
    let fresh = status.reading.is_fresh(status.uptime_ms);
    let mut body = String::<256>::new();
    let formatted = match (fresh, status.reading.value, status.ipv4) {
        (true, Some(value), Some(ip)) => {
            let [a, b, c, d, e, f] = status.mac;
            writeln!(
                body,
                "{a:02x}:{b:02x}:{c:02x}:{d:02x}:{e:02x}:{f:02x} - {} - {:.2} - {:.2} - {}",
                status.uptime_ms, value.temperature_c, value.humidity_percent, ip
            )
        }
        _ => writeln!(
            body,
            "sensor unavailable or stale; errors={}",
            status.reading.errors
        ),
    };
    match formatted {
        Ok(()) => Ok(
            Response::new(reading_status(fresh && status.ipv4.is_some()), body)
                .with_header("Cache-Control", "no-store"),
        ),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn json(State(status): State<Status>) -> impl IntoResponse {
    let reading = status.reading;
    let fresh = reading.is_fresh(status.uptime_ms);
    // Floats are rendered with core::fmt: picoserve's JSON float path widens
    // f32 to f64, which misprints on this Xtensa soft-float target, while
    // integers and core::fmt output are exact.
    let mut body = String::<256>::new();
    let formatted = write!(
        body,
        "{{\"firmware\":\"{}\",\"uptime_ms\":{},\"fresh\":{},\"errors\":{},\"temperature_c\":{},\"humidity_percent\":{},\"age_ms\":{}}}",
        env!("CARGO_PKG_VERSION"),
        status.uptime_ms,
        fresh,
        reading.errors,
        JsonFloat(reading.value.map(|v| v.temperature_c)),
        JsonFloat(reading.value.map(|v| v.humidity_percent)),
        JsonAge(
            reading
                .value
                .map(|_| status.uptime_ms.saturating_sub(reading.sampled_ms))
        ),
    );
    match formatted {
        Ok(()) => Ok(Response::new(reading_status(fresh), body)
            .with_header("Content-Type", "application/json")
            .with_header("Cache-Control", "no-store")),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// One-decimal JSON number (the module's own resolution) or `null`.
struct JsonFloat(Option<f32>);

impl core::fmt::Display for JsonFloat {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            Some(value) => write!(f, "{value:.1}"),
            None => write!(f, "null"),
        }
    }
}

struct JsonAge(Option<u64>);

impl core::fmt::Display for JsonAge {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            Some(age) => write!(f, "{age}"),
            None => write!(f, "null"),
        }
    }
}

struct Failure(Error);

impl IntoResponse for Failure {
    async fn write_to<R: Read, W: ResponseWriter<Error = R::Error>>(
        self,
        connection: picoserve::response::Connection<'_, R>,
        writer: W,
    ) -> Result<ResponseSent, W::Error> {
        let status = match self.0 {
            Error::Authentication => StatusCode::FORBIDDEN,
            Error::Flash => StatusCode::INTERNAL_SERVER_ERROR,
            Error::Busy => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::BAD_REQUEST,
        };
        Response::new(status, "Request failed\n")
            .write_to(connection, writer)
            .await
    }
}

struct Upload;
impl<D: Device> RequestHandlerService<D> for Upload {
    async fn call_request_handler_service<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &D,
        (): (),
        mut request: Request<'_, R>,
        writer: W,
    ) -> Result<ResponseSent, W::Error> {
        let result = match protocol::manifest(request.parts.headers()) {
            Ok(manifest) => {
                let body = request.body_connection.body().reader();
                // Picoserve's native override gives accepted uploads a longer body budget.
                // The short default still bounds draining an unconsumed rejected body.
                #[cfg(target_arch = "xtensa")]
                let body = body.with_different_timeout(embassy_time::Duration::from_secs(180));
                let mut body = body;
                state.update(&manifest, &mut body).await
            }
            Err(error) => Err(error),
        };
        result
            .map(|()| "Update verified; rebooting\n")
            .map_err(Failure)
            .write_to(request.body_connection.finalize().await?, writer)
            .await
    }
}

// picoserve parses HTTP. This layer enforces our fixed-length upload contract;
// its parser intentionally does not reject duplicate Content-Length itself.
struct StrictHeaders;
impl<D> Layer<D, ()> for StrictHeaders {
    type NextState = D;
    type NextPathParameters = ();
    async fn call_layer<
        'a,
        R: Read + 'a,
        N: Next<'a, R, D, ()>,
        W: ResponseWriter<Error = R::Error>,
    >(
        &self,
        next: N,
        state: &D,
        (): (),
        parts: RequestParts<'_>,
        writer: W,
    ) -> Result<ResponseSent, W::Error> {
        match protocol::validate_headers(parts.headers()) {
            Ok(()) => next.run(state, (), writer).await,
            Err(error) => {
                Failure(error)
                    .write_to(next.into_connection().await?, writer)
                    .await
            }
        }
    }
}
