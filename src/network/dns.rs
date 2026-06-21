use anyhow::Result;
use log::{error, info};
use std::net::UdpSocket;
use std::thread;

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

        info!("DNS query from {}, length: {}", src, amt);
        if amt < 12 {
            continue;
        }

        // Simple DNS Hijacking:
        // We only modify the flags, answer count, and append the answer.
        let mut response = buf[..amt].to_vec();

        // Flags: QR = 1, AA = 1, RD = 1, RA = 0, Z = 0, RCODE = 0
        response[2] = 0x84; // Standard response
        response[3] = 0x00; // No error

        // Answer Count: 1
        response[6] = 0;
        response[7] = 1;

        // Append Answer (Pointer to query name)
        response.push(0xC0);
        response.push(0x0C);

        // Type A (1)
        response.push(0x00);
        response.push(0x01);

        // Class IN (1)
        response.push(0x00);
        response.push(0x01);

        // TTL (60s)
        response.push(0x00);
        response.push(0x00);
        response.push(0x00);
        response.push(0x3C);

        // RDLENGTH (4 bytes for IP)
        response.push(0x00);
        response.push(0x04);

        // IP Address: 192.168.71.1
        response.push(192);
        response.push(168);
        response.push(71);
        response.push(1);

        if let Err(e) = socket.send_to(&response, src) {
            error!("Failed to send DNS response: {:?}", e);
        } else {
            info!("DNS response sent to {}", src);
        }
    }
}
