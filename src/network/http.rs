use embedded_svc::http::client::Client;
use embedded_svc::http::Headers;
use esp_idf_svc::http::client::{Configuration, EspHttpConnection};
use esp_idf_svc::http::Method;

use anyhow::{anyhow, bail, Result};
use std::time::Duration;

/// Refuse responses larger than this. MTA feeds are ~50-200KB; the cap stops
/// a misbehaving server from exhausting the 2MB of PSRAM.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// Per-operation network timeout (connect, each read)
const TIMEOUT: Duration = Duration::from_secs(10);

pub fn fetch_binary(url: &str) -> Result<Vec<u8>> {
    fetch(url, &[("accept", "application/x-protobuf")])
}

fn fetch(url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>> {
    use crate::diag::{self, Crumb, Stage};
    diag::stage(Crumb::WorkerStage, Stage::HttpConnect);

    let connection = EspHttpConnection::new(&Configuration {
        use_global_ca_store: true,
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        timeout: Some(TIMEOUT),
        ..Default::default()
    })
    .map_err(|e| anyhow!("Failed to create connection: {e:?}"))?;

    let mut client = Client::wrap(connection);

    let request = client
        .request(Method::Get, url, headers)
        .map_err(|e| anyhow!("Failed to create request: {e:?}"))?;

    let mut response = request
        .submit()
        .map_err(|e| anyhow!("Failed to submit request: {e:?}"))?;

    let status = response.status();
    if status != 200 {
        bail!("HTTP {status}: {url}");
    }

    // Reserve the whole body up front when the size is known, rather than
    // growing by doubling (which briefly needs ~3x the body size).
    let expected_len =
        response.content_len().and_then(|n| usize::try_from(n).ok());
    if let Some(len) = expected_len {
        if len > MAX_RESPONSE_BYTES {
            bail!("Response too large ({len} bytes): {url}");
        }
    }
    let mut result = Vec::new();
    result
        .try_reserve_exact(expected_len.unwrap_or(64 * 1024))
        .map_err(|e| anyhow!("Out of memory for response: {e}"))?;

    diag::stage(Crumb::WorkerStage, Stage::HttpRead);
    let mut buf = [0u8; 1024];
    loop {
        // A read error must not be mistaken for end-of-body: that would hand
        // a truncated feed to the parser as if it were complete.
        let n = response.read(&mut buf).map_err(|e| {
            anyhow!("Read failed after {} bytes: {e:?}", result.len())
        })?;
        if n == 0 {
            break;
        }
        if result.len() + n > MAX_RESPONSE_BYTES {
            bail!("Response exceeded {MAX_RESPONSE_BYTES} bytes: {url}");
        }
        result
            .try_reserve(n)
            .map_err(|e| anyhow!("Out of memory for response: {e}"))?;
        result.extend_from_slice(&buf[..n]);
    }

    if let Some(len) = expected_len {
        if result.len() != len {
            bail!("Truncated response: got {} of {len} bytes", result.len());
        }
    }

    Ok(result)
}
