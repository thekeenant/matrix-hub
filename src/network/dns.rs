//! Captive-portal DNS server (replies are built in `matrix_hub_core`).

use anyhow::Result;
use log::{debug, error, info};
use std::net::UdpSocket;
use std::thread;

use matrix_hub_core::config::AP_IP;
use matrix_hub_core::net::dns::build_response;

pub fn start_dns_server() {
    thread::Builder::new()
        .stack_size(4096)
        .spawn(|| {
            if let Err(e) = run_dns_server() {
                error!("DNS Server error: {:?}", e);
            }
        })
        .unwrap_or_else(|e| panic!("Failed to spawn DNS thread: {:?}", e));
}

fn run_dns_server() -> Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:53")?;
    info!("DNS Server listening on port 53");

    let mut buf = [0u8; 512];

    loop {
        let (amt, src) = match socket.recv_from(&mut buf) {
            Ok(res) => res,
            Err(e) => {
                error!("DNS recv_from error: {:?}", e);
                continue;
            }
        };

        let Some(response) = build_response(&buf[..amt], AP_IP) else {
            debug!("DNS: ignoring malformed/unsupported packet from {src}");
            continue;
        };

        if let Err(e) = socket.send_to(&response, src) {
            error!("Failed to send DNS response: {:?}", e);
        }
    }
}
