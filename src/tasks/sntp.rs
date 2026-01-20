//! SNTP (Simple Network Time Protocol) task for time synchronization.
//!
//! This task waits for WiFi connection and then periodically syncs the system time
//! with an NTP server.

use embassy_net::{
    Stack,
    udp::{PacketMetadata, UdpSocket},
};
use embassy_time::{Duration, Instant, Timer};
use log::{info, warn};

use crate::{time::current_time_blocking, wifi::SharedHttpTcpClient};

const NTP_SERVER: &str = "pool.ntp.org";
const NTP_PORT: u16 = 123;
const SYNC_INTERVAL: Duration = Duration::from_secs(3600); // Sync every hour
const RETRY_DELAY: Duration = Duration::from_secs(10);

/// SNTP client task - syncs time with NTP server periodically
#[embassy_executor::task]
pub async fn sntp_task(stack: Stack<'static>, http_client: SharedHttpTcpClient) {
    sntp_task_impl(stack, http_client)
        .await
        .expect("SNTP task failed");
}

async fn sntp_task_impl(
    stack: Stack<'static>,
    http_client: SharedHttpTcpClient,
) -> anyhow::Result<()> {
    loop {
        match sync_time(http_client.clone(), stack.clone()).await {
            Ok(()) => {
                let time = current_time_blocking().await;
                info!(
                    "SNTP: Time sync successful - {}",
                    time.format("%Y-%m-%d %H:%M:%S UTC")
                );
                Timer::after(SYNC_INTERVAL).await;
            }
            Err(e) => {
                warn!("SNTP: Sync failed: {:?}, retrying...", e);
                Timer::after(RETRY_DELAY).await;
            }
        }
    }
}

async fn sync_time(http_client: SharedHttpTcpClient, stack: Stack<'static>) -> anyhow::Result<()> {
    info!("SNTP: Starting time sync");
    let _http_client_guard = http_client.lock().await;
    let stack = stack;

    // Resolve NTP server address
    let addrs = stack
        .dns_query(NTP_SERVER, embassy_net::dns::DnsQueryType::A)
        .await
        .map_err(|e| anyhow::anyhow!("DNS query failed: {:?}", e))?;

    let addr = *addrs
        .first()
        .ok_or_else(|| anyhow::anyhow!("DNS resolution failed"))?;

    info!("SNTP: Resolved {} to {}", NTP_SERVER, addr);

    // Create UDP socket
    let mut rx_meta = [PacketMetadata::EMPTY; 2];
    let mut rx_buffer = [0u8; 256];
    let mut tx_meta = [PacketMetadata::EMPTY; 2];
    let mut tx_buffer = [0u8; 256];

    let mut socket = UdpSocket::new(
        stack,
        &mut rx_meta,
        &mut rx_buffer,
        &mut tx_meta,
        &mut tx_buffer,
    );

    socket
        .bind(0)
        .map_err(|e| anyhow::anyhow!("Bind error: {:?}", e))?;

    // Build NTP request packet (48 bytes)
    let mut ntp_packet = [0u8; 48];
    ntp_packet[0] = 0b00_011_011; // LI=0, VN=3, Mode=3 (client)

    // Send request
    socket
        .send_to(&ntp_packet, (addr, NTP_PORT))
        .await
        .map_err(|e| anyhow::anyhow!("Send error: {:?}", e))?;

    // Receive response with timeout
    let mut response = [0u8; 256];
    let (len, _) =
        embassy_time::with_timeout(Duration::from_secs(5), socket.recv_from(&mut response))
            .await
            .map_err(|_| anyhow::anyhow!("NTP response timeout"))?
            .map_err(|e| anyhow::anyhow!("Receive error: {:?}", e))?;

    if len < 48 {
        return Err(anyhow::anyhow!("Invalid NTP response length: {}", len));
    }

    // Extract timestamp from response (bytes 40-43: transmit timestamp seconds)
    let ntp_seconds = u32::from_be_bytes([response[40], response[41], response[42], response[43]]);

    // Convert NTP timestamp (since 1900) to Unix timestamp (since 1970)
    const NTP_UNIX_OFFSET: u64 = 2_208_988_800; // Seconds between 1900 and 1970
    let unix_timestamp = (ntp_seconds as u64).saturating_sub(NTP_UNIX_OFFSET);

    // Store boot time for offset calculations
    let now_us = Instant::now().as_micros();
    let unix_us = unix_timestamp * 1_000_000;

    crate::time::set_boot_time(unix_us, now_us);
    Ok(())
}
