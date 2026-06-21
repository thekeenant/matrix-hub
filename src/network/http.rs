#![allow(dead_code, reason = "unused network utilities")]

use embedded_svc::http::client::Client;
use esp_idf_svc::http::client::{Configuration, EspHttpConnection};
use esp_idf_svc::http::Method;

use anyhow::{anyhow, Result};

pub fn fetch_text(url: &str) -> Result<String> {
    fetch_with_headers(url, &[("accept", "text/plain")])
}

pub fn fetch_binary(url: &str) -> Result<Vec<u8>> {
    fetch(url, &[("accept", "application/x-protobuf")])
}

fn fetch_with_headers(url: &str, headers: &[(&str, &str)]) -> Result<String> {
    let bytes = fetch(url, headers)?;
    String::from_utf8(bytes).map_err(|e| anyhow!("Invalid UTF-8: {e:?}"))
}

fn fetch(url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>> {
    let connection = EspHttpConnection::new(&Configuration {
        use_global_ca_store: true,
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
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
        return Err(anyhow!("HTTP {status}: {url}"));
    }

    let mut buf = [0u8; 1024];
    let mut result = Vec::new();
    loop {
        let bytes_read = response.read(&mut buf).unwrap_or(0);
        if bytes_read == 0 {
            break;
        }
        result.extend_from_slice(&buf[0..bytes_read]);
    }

    Ok(result)
}
