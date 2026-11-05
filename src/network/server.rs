use anyhow::Result;
use esp_idf_svc::http::server::{Configuration, EspHttpServer};
use esp_idf_svc::http::Method;
use esp_idf_svc::io::Write;

use log::info;
use matrix_hub_core::device_config::{ConfigUpdate, MAX_UPDATE_BYTES};
use matrix_hub_core::net::body::BodyCollector;

pub fn start_server(
    credentials_tx: std::sync::mpsc::Sender<()>,
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
        // Read the whole body: a single read() can return only part of it
        let mut body = BodyCollector::new(None, MAX_UPDATE_BYTES)?;
        let mut buf = [0u8; 256];
        loop {
            let n = request.read(&mut buf)?;
            if n == 0 {
                break;
            }
            if body.push(&buf[..n]).is_err() {
                request
                    .into_status_response(413)?
                    .write_all(b"Request too large")?;
                return Ok::<(), anyhow::Error>(());
            }
        }

        let update = match ConfigUpdate::parse(&body.finish()?) {
            Ok(Some(update)) => update,
            Ok(None) => {
                request.into_ok_response()?.write_all(b"OK")?;
                return Ok(());
            }
            Err(e) => {
                request
                    .into_status_response(400)?
                    .write_all(e.to_string().as_bytes())?;
                return Ok(());
            }
        };

        if let Err(e) =
            crate::storage::update_config(|config| update.apply(config))
        {
            request
                .into_status_response(500)?
                .write_all(format!("Failed to save: {e:?}").as_bytes())?;
            return Ok(());
        }

        if update.changes_wifi() {
            info!(
                "WiFi credentials updated; asking the main loop to reconnect"
            );
            let _ = credentials_tx.send(());
        }

        request.into_ok_response()?.write_all(b"OK")?;
        Ok(())
    })?;

    // Diagnostics: live health snapshot (JSON)
    server.fn_handler("/api/debug", Method::Get, |request| {
        let body = crate::diag::report_json();
        let mut response = request.into_response(
            200,
            None,
            &[
                ("Content-Type", "application/json"),
                ("Cache-Control", "no-store"),
            ],
        )?;
        response.write_all(body.as_bytes())?;
        Ok::<(), anyhow::Error>(())
    })?;

    // Diagnostics: raw coredump image from flash (decode with
    // scripts/decode-coredump.sh). 404 if there is none.
    server.fn_handler("/api/coredump", Method::Get, |request| {
        let Some((addr, size)) = crate::platform::coredump::location() else {
            request
                .into_status_response(404)?
                .write_all(b"No coredump")?;
            return Ok::<(), anyhow::Error>(());
        };

        let len = size.to_string();
        let mut response = request.into_response(
            200,
            None,
            &[
                ("Content-Type", "application/octet-stream"),
                ("Content-Length", &len),
            ],
        )?;
        let mut buf = vec![0u8; 1024];
        let mut offset = 0;
        while offset < size {
            let n = buf.len().min(size - offset);
            crate::platform::coredump::read(addr + offset, &mut buf[..n])?;
            response.write_all(&buf[..n])?;
            offset += n;
        }
        Ok::<(), anyhow::Error>(())
    })?;

    server.fn_handler("/api/coredump", Method::Delete, |request| {
        crate::platform::coredump::erase()?;
        request.into_ok_response()?.write_all(b"Erased")?;
        Ok::<(), anyhow::Error>(())
    })?;

    // Diagnostics: deliberately hang the main loop to test the watchdog,
    // coredump, and reboot path. Expect a reboot ~10s later.
    server.fn_handler("/api/debug/test-hang", Method::Post, |request| {
        crate::diag::TEST_HANG
            .store(true, std::sync::atomic::Ordering::Relaxed);
        request
            .into_ok_response()?
            .write_all(b"Main loop will hang; expect a watchdog reboot")?;
        Ok::<(), anyhow::Error>(())
    })?;

    // Diagnostics: deliberately hang the background worker mid-fetch to
    // test the worker hang guard. Expect a reboot ~5 minutes later.
    server.fn_handler(
        "/api/debug/test-hang-worker",
        Method::Post,
        |request| {
            crate::diag::TEST_HANG_WORKER
                .store(true, std::sync::atomic::Ordering::Relaxed);
            request.into_ok_response()?.write_all(
                b"Worker will hang on its next fetch; expect a reboot ~5 min later",
            )?;
            Ok::<(), anyhow::Error>(())
        },
    )?;

    server.fn_handler("/*", Method::Get, |request| {
        info!("Captive Portal wildcard hit: {}", request.uri());
        // Redirect all other GET requests (Captive Portal checks) to the root URL
        let [a, b, c, d] = crate::config::AP_IP;
        let location = format!("http://{a}.{b}.{c}.{d}/");
        let mut response = request.into_response(
            302,
            Some("Found"),
            &[("Location", &location)],
        )?;
        response.write_all(b"Redirecting...")?;
        Ok::<(), anyhow::Error>(())
    })?;

    info!("HTTP server listening on port 80");
    Ok(server)
}
