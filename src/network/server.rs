use anyhow::Result;
use esp_idf_svc::http::server::{Configuration, EspHttpServer};
use esp_idf_svc::http::Method;
use esp_idf_svc::io::Write;

use log::info;

pub fn start_server(
    credentials_tx: std::sync::mpsc::Sender<(String, String)>,
) -> Result<EspHttpServer<'static>> {
    let config = Configuration {
        uri_match_wildcard: true,
        ..Default::default()
    };
    let mut server = EspHttpServer::new(&config)
        .map_err(|e| anyhow::anyhow!("Failed to create server: {:?}", e))?;

    // Embed frontend assets
    const INDEX_HTML: &[u8] =
        include_bytes!("../../frontend/dist/index.html.gz");
    const BUNDLE_JS: &[u8] =
        include_bytes!("../../frontend/dist/assets/bundle.js.gz");
    const BUNDLE_CSS: &[u8] =
        include_bytes!("../../frontend/dist/assets/bundle.css.gz");

    server.fn_handler("/", Method::Get, |request| {
        let mut response = request.into_response(
            200,
            None,
            &[("Content-Encoding", "gzip"), ("Content-Type", "text/html")],
        )?;
        response.write_all(INDEX_HTML)?;
        Ok::<(), anyhow::Error>(())
    })?;

    server.fn_handler("/assets/bundle.js", Method::Get, |request| {
        let mut response = request.into_response(
            200,
            None,
            &[
                ("Content-Encoding", "gzip"),
                ("Content-Type", "application/javascript"),
            ],
        )?;
        response.write_all(BUNDLE_JS)?;
        Ok::<(), anyhow::Error>(())
    })?;

    server.fn_handler("/assets/bundle.css", Method::Get, |request| {
        let mut response = request.into_response(
            200,
            None,
            &[("Content-Encoding", "gzip"), ("Content-Type", "text/css")],
        )?;
        response.write_all(BUNDLE_CSS)?;
        Ok::<(), anyhow::Error>(())
    })?;

    server.fn_handler("/api/config", Method::Get, |request| {
        use buffa::Message;
        let config = crate::storage::get_config();
        let bytes = config.encode_to_vec();

        let mut response = request.into_response(
            200,
            None,
            &[("Content-Type", "application/octet-stream")],
        )?;
        response.write_all(&bytes)?;
        Ok::<(), anyhow::Error>(())
    })?;

    server.fn_handler("/api/config", Method::Post, move |mut request| {
        use buffa::Message;
        let mut buf = vec![0; 1024];
        let bytes_read = request.read(&mut buf).unwrap_or(0);
        let mut slice = &buf[..bytes_read];

        let Ok(update_req) =
            crate::proto::config::UpdateConfigRequest::decode(&mut slice)
        else {
            request
                .into_status_response(400)?
                .write_all(b"Bad Request")?;
            return Ok::<(), anyhow::Error>(());
        };

        let Some(new_config) = update_req.config.into_option() else {
            request.into_ok_response()?.write_all(b"OK")?;
            return Ok::<(), anyhow::Error>(());
        };

        let update_wifi = update_req.update_mask.contains(&"wifi".to_string());
        let update_brightness =
            update_req.update_mask.contains(&"brightness".to_string());

        if let Err(e) = crate::storage::update_config(|config| {
            if update_wifi || update_req.update_mask.is_empty() {
                config.wifi = new_config.wifi.clone();
            }
            if update_brightness || update_req.update_mask.is_empty() {
                config.brightness = new_config.brightness;
            }
        }) {
            request
                .into_status_response(500)?
                .write_all(format!("Failed to save: {:?}", e).as_bytes())?;
            return Ok::<(), anyhow::Error>(());
        }

        // If WiFi was updated, tell the background task to attempt reconnection
        if update_wifi || update_req.update_mask.is_empty() {
            info!("WiFi credentials updated. Sending update event...");
            let _ = credentials_tx.send((
                new_config.wifi.ssid.clone(),
                new_config.wifi.pass.clone(),
            ));
        }

        request.into_ok_response()?.write_all(b"OK")?;
        Ok::<(), anyhow::Error>(())
    })?;

    server.fn_handler("/*", Method::Get, |request| {
        info!("Captive Portal wildcard hit: {}", request.uri());
        // Redirect all other GET requests (Captive Portal checks) to the root URL
        let mut response = request.into_response(
            302,
            Some("Found"),
            &[("Location", "http://192.168.71.1/")],
        )?;
        response.write_all(b"Redirecting...")?;
        Ok::<(), anyhow::Error>(())
    })?;

    info!("HTTP server listening on 192.168.71.1:80");
    Ok(server)
}
