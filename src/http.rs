//! HTTP utility functions for making requests with the embassy HTTP client.
//!
//! Provides generic fetch functions with configurable buffer sizes.

use embassy_net::{dns::DnsSocket, tcp::client::TcpClient};
use log::debug;
use reqwless::{client::HttpClient, request::Method};

extern crate alloc;

pub type HttpTcpClient<'a> = HttpClient<'a, TcpClient<'a, 2, 4096, 4096>, DnsSocket<'a>>;

/// Generic HTTP fetch function with heap-allocated buffers.
///
/// Uses `Box<[u8]>` for heap allocation, suitable for large buffers.
///
/// # Arguments
/// - `http_client`: Mutable reference to the HTTP client (already locked)
/// - `method`: HTTP method (GET, POST, etc.)
/// - `url`: The URL to fetch
///
/// # Returns
/// A Vec containing the response body
///
/// # Example
/// ```no_run
/// let mut client = http_client.lock().await;
/// let data = fetch(
///     &mut client,
///     Method::GET,
///     "https://api.example.com/data",
/// ).await?;
/// ```
pub async fn fetch(
    client: &mut HttpTcpClient<'_>,
    method: Method,
    url: &str,
) -> anyhow::Result<alloc::vec::Vec<u8>> {
    debug!("Creating HTTP request for: {}", url);

    // Create request
    let mut request = client
        .request(method, url)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create request: {:?}", e))?;

    // Allocate buffers on the heap
    const SEND_SIZE: usize = 4 * 1024;
    const RECV_SIZE: usize = 4 * 1024;
    debug!("Allocating buffers: send={}, recv={}", SEND_SIZE, RECV_SIZE);
    let mut send_buffer = alloc::vec![0u8; SEND_SIZE].into_boxed_slice();
    let mut recv_buffer = alloc::vec![0u8; RECV_SIZE];

    // Send the request
    debug!("Sending HTTP request to {}", url);
    let response = request
        .send(&mut send_buffer)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to send request: {:?}", e))?;
    let status = response.status;
    debug!("Received HTTP response: status {}", status.0);

    let content_length = response
        .content_length
        .ok_or_else(|| anyhow::anyhow!("Unknown content length"))?;
    debug!("Expanding receive buffer to {} bytes", content_length);
    recv_buffer.resize(content_length, 0);

    debug!(
        "Starting to read response body (buffer size: {})",
        RECV_SIZE
    );
    let mut reader = response.body().reader();
    let bytes_read = reader.read_to_end(&mut recv_buffer).await.map_err(|e| {
        debug!("Failed during read_to_end: {:?}", e);
        anyhow::anyhow!("Failed to read response (status {}): {:?}", status.0, e)
    })?;

    debug!("Read {} bytes from response body", bytes_read);

    if status.0 != 200 {
        return Err(anyhow::anyhow!("HTTP error: status {}", status.0));
    }
    Ok(recv_buffer)
}
