//! HTTP server task using picoserve.
//!
//! This task provides a simple HTTP server for configuration and status endpoints.

extern crate alloc;

use embassy_net::Stack;
use embassy_time::Duration;
use log::info;
use picoserve::{
    response::Content,
    routing::{get, post},
};

use crate::{
    app_rotation::{AppRotationDirection, AppRotationSignal},
    proto::app_state::MatrixHubState,
    state::SharedMatrixHubState,
};

/// HTTP server task - serves configuration and status endpoints
#[embassy_executor::task]
pub async fn http_server_task(
    stack: Stack<'static>,
    state: SharedMatrixHubState,
    app_rotation_signal: &'static AppRotationSignal,
) {
    http_server_task_impl(stack, state, app_rotation_signal)
        .await
        .expect("HTTP server task failed");
}

async fn http_server_task_impl(
    stack: Stack<'static>,
    state: SharedMatrixHubState,
    app_rotation_signal: &'static AppRotationSignal,
) -> anyhow::Result<()> {
    info!("HTTP server: Waiting for IP address...");

    // Wait for network stack to get an IP address
    let ip = loop {
        if let Some(config) = stack.config_v4() {
            break config.address.address();
        }
        embassy_time::Timer::after(Duration::from_millis(100)).await;
    };

    info!("HTTP server: Starting on http://{}:80", ip);

    let app = picoserve::Router::new()
        .route("/", get(index))
        .route("/state", get(state_handler))
        .route("/config", get(config).post(config_update))
        .route("/next", post(next_app))
        .route("/prev", post(prev_app))
        .route("/styles.css", get(styles_css))
        .with_state((state, app_rotation_signal));

    let config = picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Some(Duration::from_secs(5)),
        read_request: Some(Duration::from_secs(1)),
        write: Some(Duration::from_secs(10)),
        persistent_start_read_request: None,
    })
    .keep_connection_alive();

    let mut rx_buffer = [0; 2048];
    let mut tx_buffer = [0; 4096];
    let mut http_buffer = [0; 8192];

    let _ = picoserve::Server::new(&app, &config, &mut http_buffer)
        .listen_and_serve("http", stack, 80, &mut rx_buffer, &mut tx_buffer)
        .await;
    Ok(())
}

async fn index(
    _state: picoserve::extract::State<(SharedMatrixHubState, &'static AppRotationSignal)>,
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

async fn styles_css(
    _state: picoserve::extract::State<(SharedMatrixHubState, &'static AppRotationSignal)>,
) -> Css {
    Css(include_str!("styles.css"))
}

async fn state_handler(
    picoserve::extract::State((state, _)): picoserve::extract::State<(
        SharedMatrixHubState,
        &'static AppRotationSignal,
    )>,
) -> picoserve::response::Json<MatrixHubState> {
    let hub_state = state.lock().await.clone();
    picoserve::response::Json(hub_state)
}

async fn next_app(
    picoserve::extract::State((_, rotation_signal)): picoserve::extract::State<(
        SharedMatrixHubState,
        &'static AppRotationSignal,
    )>,
) -> &'static str {
    rotation_signal.signal(AppRotationDirection::Next);
    "OK"
}

async fn prev_app(
    picoserve::extract::State((_, rotation_signal)): picoserve::extract::State<(
        SharedMatrixHubState,
        &'static AppRotationSignal,
    )>,
) -> &'static str {
    rotation_signal.signal(AppRotationDirection::Prev);
    "OK"
}

async fn config(
    picoserve::extract::State((state, _)): picoserve::extract::State<(
        SharedMatrixHubState,
        &'static AppRotationSignal,
    )>,
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
    picoserve::extract::State((state, _)): picoserve::extract::State<(
        SharedMatrixHubState,
        &'static AppRotationSignal,
    )>,
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
