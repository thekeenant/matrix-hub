//! HTTP server task using picoserve.
//!
//! This task provides a simple HTTP server for configuration and status endpoints.

use embassy_net::tcp::TcpSocket;
use embassy_time::Duration;
use log::info;
use picoserve::routing::get;

use crate::{state::SharedMatrixHubState, wifi::SharedNetworkStack};

/// HTTP server task - serves configuration and status endpoints
#[embassy_executor::task]
pub async fn http_server_task(stack: SharedNetworkStack, state: SharedMatrixHubState) {
    http_server_task_impl(stack, state)
        .await
        .expect("HTTP server task failed");
}

async fn http_server_task_impl(
    stack: SharedNetworkStack,
    state: SharedMatrixHubState,
) -> anyhow::Result<()> {
    info!("HTTP server: Starting");

    let app = picoserve::Router::new()
        .route("/", get(index))
        .route("/status", get(status))
        .with_state(state);

    let config = picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Some(Duration::from_secs(5)),
        read_request: Some(Duration::from_secs(1)),
        write: Some(Duration::from_secs(1)),
        persistent_start_read_request: None,
    })
    .keep_connection_alive();

    let mut rx_buffer = [0; 1024];
    let mut tx_buffer = [0; 1024];
    let mut http_buffer = [0; 2048];

    loop {
        let mut socket = {
            let stack = stack.lock().await;
            TcpSocket::new(*stack, &mut rx_buffer, &mut tx_buffer)
        };
        socket.set_timeout(Some(Duration::from_secs(10)));

        info!("HTTP server: Listening on port 80");
        if let Err(e) = socket.accept(80).await {
            log::warn!("HTTP server: Accept error: {:?}", e);
            continue;
        }

        info!("HTTP server: Connection accepted");

        match picoserve::Server::new(&app, &config, &mut http_buffer)
            .serve(socket)
            .await
        {
            Ok(picoserve::DisconnectionInfo {
                handled_requests_count,
                ..
            }) => {
                info!(
                    "HTTP server: Connection closed, handled {} requests",
                    handled_requests_count
                );
            }
            Err(err) => log::warn!("HTTP server: Error: {:?}", err),
        }
    }
}

async fn index(_state: picoserve::extract::State<SharedMatrixHubState>) -> &'static str {
    "Matrix Hub API"
}

async fn status(
    picoserve::extract::State(state): picoserve::extract::State<SharedMatrixHubState>,
) -> &'static str {
    let hub_state = state.lock().await;

    let wifi_configured = hub_state
        .config
        .as_ref()
        .and_then(|c| c.wifi.as_ref())
        .map(|w| !w.ssid.is_empty())
        .unwrap_or(false);

    if wifi_configured {
        "Status: WiFi configured"
    } else {
        "Status: WiFi not configured"
    }
}
