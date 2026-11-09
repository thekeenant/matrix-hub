//! HTTPS GET client (body handling lives in `matrix_hub_core::net::body`).

use anyhow::{anyhow, bail, Context, Result};
use embedded_svc::http::client::Client;
use embedded_svc::http::Headers;
use esp_idf_svc::http::client::{Configuration, EspHttpConnection};
use esp_idf_svc::http::Method;
use matrix_hub_core::net::body::BodyCollector;
use std::time::Duration;

use crate::diag::{self, Stage};

/// Refuse responses larger than this. MTA feeds are ~50-200KB; the cap stops
/// a misbehaving server from exhausting the 2MB of PSRAM.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// Per-operation network timeout (connect, each read)
const TIMEOUT: Duration = Duration::from_secs(10);

pub fn fetch_binary(url: &str) -> Result<Vec<u8>> {
    fetch(url, &[("accept", "application/x-protobuf")])
}

fn fetch(url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>> {
    diag::WORKER.enter(Stage::HttpConnect);

    let connection = EspHttpConnection::new(&Configuration {
        use_global_ca_store: true,
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        timeout: Some(TIMEOUT),
        ..Default::default()
    })
    .context("Failed to create connection")?;

    let mut client = Client::wrap(connection);
    let request = client
        .request(Method::Get, url, headers)
        .context("Failed to create request")?;
    let mut response = request.submit().context("Failed to submit request")?;

    let status = response.status();
    if status != 200 {
        bail!("HTTP {status}: {url}");
    }

    let mut body =
        BodyCollector::new(response.content_len(), MAX_RESPONSE_BYTES)?;
    diag::WORKER.enter(Stage::HttpRead);
    let mut buf = [0u8; 1024];
    loop {
        // A read error must not be mistaken for end-of-body: that would hand
        // a truncated feed to the parser as if it were complete.
        let n = response.read(&mut buf).map_err(|e| {
            anyhow!("Read failed after {} bytes: {e:?}", body.len())
        })?;
        if n == 0 {
            break;
        }
        body.push(&buf[..n])?;
    }
    body.finish()
}
