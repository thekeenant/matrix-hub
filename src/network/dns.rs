//! Captive-portal DNS: answers every A query with the access point's own IP,
//! so phones/laptops joining the setup network land on the config page.

use anyhow::Result;
use log::{debug, error, info};
use std::net::UdpSocket;
use std::thread;

use crate::config::AP_IP;

const HEADER_LEN: usize = 12;
const TYPE_A: u16 = 1;
const TYPE_ANY: u16 = 255;

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

        let Some(response) = build_response(&buf[..amt]) else {
            debug!("DNS: ignoring malformed/unsupported packet from {src}");
            continue;
        };

        if let Err(e) = socket.send_to(&response, src) {
            error!("Failed to send DNS response: {:?}", e);
        }
    }
}

/// Builds the reply to a standard single-question query: the header and
/// question echoed back, plus one A record pointing at the AP for A/ANY
/// queries (other types, e.g. AAAA, get an empty NOERROR answer).
///
/// Anything after the question (EDNS OPT records etc.) is dropped, and
/// NSCOUNT/ARCOUNT are zeroed to match.
fn build_response(query: &[u8]) -> Option<Vec<u8>> {
    if query.len() < HEADER_LEN {
        return None;
    }
    let flags_hi = query[2];
    let is_response = flags_hi & 0x80 != 0;
    let opcode = (flags_hi >> 3) & 0x0F;
    let qdcount = u16::from_be_bytes([query[4], query[5]]);
    if is_response || opcode != 0 || qdcount != 1 {
        return None;
    }

    // Walk QNAME's labels to find where the question ends
    let mut pos = HEADER_LEN;
    loop {
        let len = *query.get(pos)? as usize;
        if len == 0 {
            pos += 1;
            break;
        }
        if len & 0xC0 != 0 {
            return None; // compression pointers aren't valid in a question
        }
        pos += 1 + len;
    }
    let qtype = u16::from_be_bytes([*query.get(pos)?, *query.get(pos + 1)?]);
    let question_end = pos + 4; // QTYPE + QCLASS
    if question_end > query.len() {
        return None;
    }

    let answer = matches!(qtype, TYPE_A | TYPE_ANY);

    let mut response = Vec::with_capacity(question_end + 16);
    response.extend_from_slice(&query[..question_end]);
    // QR=1, AA=1, keep RD from the query; RA=0, RCODE=0 (no error)
    response[2] = 0x84 | (flags_hi & 0x01);
    response[3] = 0x00;
    // QDCOUNT=1, ANCOUNT, NSCOUNT=0, ARCOUNT=0
    response[4..12].copy_from_slice(&[0, 1, 0, u8::from(answer), 0, 0, 0, 0]);

    if answer {
        response.extend_from_slice(&[
            0xC0, 0x0C, // name: pointer to the question's QNAME
            0x00, 0x01, // type A
            0x00, 0x01, // class IN
            0x00, 0x00, 0x00, 0x3C, // TTL 60s
            0x00, 0x04, // RDLENGTH
        ]);
        response.extend_from_slice(&AP_IP);
    }

    Some(response)
}
