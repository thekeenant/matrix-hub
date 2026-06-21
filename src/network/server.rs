use anyhow::Result;
use esp_idf_svc::http::server::{Configuration, EspHttpServer};
use esp_idf_svc::http::Method;
use esp_idf_svc::io::Write;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use log::info;

fn url_decode(encoded: &str) -> String {
    let mut decoded = String::new();
    let mut chars = encoded.chars();
    while let Some(c) = chars.next() {
        if c == '+' {
            decoded.push(' ');
        } else if c == '%' {
            if let (Some(a), Some(b)) = (chars.next(), chars.next()) {
                if let Ok(byte) = u8::from_str_radix(&format!("{}{}", a, b), 16)
                {
                    decoded.push(byte as char);
                }
            }
        } else {
            decoded.push(c);
        }
    }
    decoded
}

pub fn start_server(
    nvs_partition: EspDefaultNvsPartition,
    credentials_tx: std::sync::mpsc::Sender<(String, String)>,
) -> Result<EspHttpServer<'static>> {
    let config = Configuration {
        uri_match_wildcard: true,
        ..Default::default()
    };
    let mut server = EspHttpServer::new(&config)
        .map_err(|e| anyhow::anyhow!("Failed to create server: {:?}", e))?;

    server.fn_handler("/", Method::Get, |request| {
        let current_brightness = crate::display::GLOBAL_BRIGHTNESS.load(std::sync::atomic::Ordering::Relaxed);
        let html = format!(r#"<!DOCTYPE html>
<html>
<head>
    <title>Matrix-Hub Setup</title>
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <style>
        body {{ font-family: sans-serif; margin: 40px auto; max-width: 400px; padding: 20px; }}
        input[type="text"], input[type="password"], input[type="number"] {{ width: 100%; padding: 10px; margin: 10px 0; box-sizing: border-box; }}
        input[type="submit"] {{ background-color: #4CAF50; color: white; padding: 14px 20px; margin: 8px 0; border: none; cursor: pointer; width: 100%; }}
    </style>
</head>
<body>
    <h2>WiFi Setup</h2>
    <form action="/save" method="POST" style="margin-bottom: 20px;">
        <label>SSID:</label>
        <input type="text" name="ssid" required>
        <label>Password:</label>
        <input type="password" name="pass">
        <input type="submit" value="Save & Connect">
    </form>

    <h2>Display Settings</h2>
    <form action="/settings" method="POST">
        <label>Brightness (0-255):</label>
        <input type="number" name="brightness" min="0" max="255" value="{}" required>
        <!-- Add future settings here -->
        <input type="submit" value="Save Settings">
    </form>
</body>
</html>"#, current_brightness);
        request.into_ok_response()?.write_all(html.as_bytes())?;
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

    let nvs_clone = nvs_partition.clone();

    server.fn_handler("/save", Method::Post, move |mut request| {
        let mut buf = vec![0; 512];
        let bytes_read = request.read(&mut buf).unwrap_or(0);
        let body = String::from_utf8_lossy(&buf[..bytes_read]);

        let mut ssid = "";
        let mut pass = "";

        for pair in body.split('&') {
            let mut kv = pair.split('=');
            if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
                if k == "ssid" {
                    ssid = v;
                } else if k == "pass" {
                    pass = v;
                }
            }
        }

        let ssid = url_decode(ssid);
        let pass = url_decode(pass);

        if let Err(e) = crate::network::wifi::save_credentials(
            nvs_clone.clone(),
            &ssid,
            &pass,
        ) {
            let error_html = format!("Failed to save: {:?}", e);
            request
                .into_status_response(500)?
                .write_all(error_html.as_bytes())?;
            return Ok::<(), anyhow::Error>(());
        }

        info!("Credentials & Config saved. Sending update event...");
        let _ = credentials_tx.send((ssid, pass));

        // PRG Pattern: Redirect back to root
        let mut response = request.into_response(
            303,
            Some("See Other"),
            &[("Location", "http://192.168.71.1/")],
        )?;
        response.write_all(b"Redirecting...")?;

        Ok::<(), anyhow::Error>(())
    })?;

    let nvs_clone_2 = nvs_partition.clone();
    server.fn_handler("/settings", Method::Post, move |mut request| {
        let mut body = String::new();
        let mut buf = [0u8; 128];
        if let Ok(size) = request.read(&mut buf) {
            body = String::from_utf8_lossy(&buf[..size]).into_owned();
        }

        for pair in body.split('&') {
            let mut kv = pair.split('=');
            if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
                if k == "brightness" {
                    if let Ok(b) = v.parse::<u8>() {
                        crate::display::GLOBAL_BRIGHTNESS
                            .store(b, std::sync::atomic::Ordering::Relaxed);
                        if let Ok(store) = esp_idf_svc::nvs::EspNvs::new(
                            nvs_clone_2.clone(),
                            "matrix_config",
                            true,
                        ) {
                            let _ = store.set_u8("brightness", b);
                        }
                    }
                }
            }
        }

        // PRG Pattern: Redirect back to root
        let mut response = request.into_response(
            303,
            Some("See Other"),
            &[("Location", "http://192.168.71.1/")],
        )?;
        response.write_all(b"Redirecting...")?;

        Ok::<(), anyhow::Error>(())
    })?;

    info!("HTTP server listening on 192.168.71.1:80");
    Ok(server)
}
