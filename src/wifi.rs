extern crate alloc;

use alloc::sync::Arc;

use embassy_net::{Stack, dns::DnsSocket, tcp::client::TcpClient};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use reqwless::client::HttpClient;

pub type HttpTcpClient<'a> = HttpClient<'a, TcpClient<'a, 2, 4096, 4096>, DnsSocket<'a>>;
pub type SharedHttpTcpClient = Arc<Mutex<CriticalSectionRawMutex, HttpTcpClient<'static>>>;
