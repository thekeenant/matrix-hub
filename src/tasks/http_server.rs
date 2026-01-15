//! HTTP server task using picoserve.
//!
//! This task provides a simple HTTP server for configuration and status endpoints.

extern crate alloc;

use embassy_net::tcp::TcpSocket;
use embassy_time::Duration;
use log::info;
use picoserve::{response::Content, routing::get};

use crate::{
    proto::app_state::MatrixHubState, state::SharedMatrixHubState, wifi::SharedNetworkStack,
};

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
    info!("HTTP server: Waiting for IP address...");

    // Wait for network stack to get an IP address
    let ip = loop {
        let stack = stack.lock().await;
        if let Some(config) = stack.config_v4() {
            break config.address.address();
        }
        drop(stack);
        embassy_time::Timer::after(Duration::from_millis(100)).await;
    };

    info!("HTTP server: Starting on http://{}:80", ip);

    let app = picoserve::Router::new()
        .route("/", get(index))
        .route("/state", get(state_handler))
        .route("/config", get(config).post(config_update))
        .route("/styles.css", get(styles_css))
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

async fn index(
    _state: picoserve::extract::State<SharedMatrixHubState>,
) -> ((&'static str, &'static str), &'static str) {
    (
        ("Content-Type", "text/html; charset=utf-8"),
        include_str!("index.html"),
    )
}

struct Css(&'static str);

impl Content for Css {
    fn content_type(&self) -> &'static str {
        "text/css; charset=utf-8"
    }

    async fn write_content<W: picoserve::io::Write>(self, mut writer: W) -> Result<(), W::Error> {
        writer.write_all(self.0.as_bytes()).await
    }

    fn content_length(&self) -> usize {
        self.0.len()
    }
}

async fn styles_css(_state: picoserve::extract::State<SharedMatrixHubState>) -> Css {
    Css(include_str!("styles.css"))
}

async fn state_handler(
    picoserve::extract::State(state): picoserve::extract::State<SharedMatrixHubState>,
) -> picoserve::response::Json<MatrixHubState> {
    let hub_state = state.lock().await.clone().clone();
    picoserve::response::Json(hub_state)
}

async fn config(
    picoserve::extract::State(state): picoserve::extract::State<SharedMatrixHubState>,
) -> ((&'static str, &'static str), impl Content) {
    let html_template = include_str!("config.html");
    let hub_state_json =
        serde_json::to_string_pretty(&state.lock().await.config).unwrap_or_default();
    let html_content: alloc::string::String = html_template
        .replace("{{config_json}}", &hub_state_json)
        .replace("{{post_message}}", "");
    (("Content-Type", "text/html; charset=utf-8"), html_content)
}

async fn config_update(
    picoserve::extract::State(state): picoserve::extract::State<SharedMatrixHubState>,
    picoserve::extract::Form(form): picoserve::extract::Form<ConfigUpdateForm>,
) -> ((&'static str, &'static str), impl Content) {
    let html_template = include_str!("config.html");

    let (message, config_json) = match serde_json::from_str(&form.config) {
        Ok(new_config) => {
            let mut hub_state = state.lock().await;
            hub_state.config = new_config;
            info!("Config updated successfully");
            let success_msg = alloc::string::String::from(
                "<div class=\"p-4 mb-4 rounded-lg font-semibold bg-green-900 text-green-400 border-2 border-green-700\">✓ Configuration updated successfully!</div>",
            );
            (success_msg, form.config.clone())
        }
        Err(e) => {
            log::error!("Failed to parse config: {:?}", e);
            let error_msg = alloc::format!(
                "<div class=\"p-4 mb-4 rounded-lg font-semibold bg-red-900 text-red-400 border-2 border-red-700\">✗ Configuration parsing failed: {}</div>",
                e
            );
            (error_msg, form.config.clone())
        }
    };

    let html_content: alloc::string::String = html_template
        .replace("{{config_json}}", &config_json)
        .replace("{{post_message}}", &message);

    (("Content-Type", "text/html; charset=utf-8"), html_content)
}

#[derive(serde::Deserialize)]
struct ConfigUpdateForm {
    config: alloc::string::String,
}
