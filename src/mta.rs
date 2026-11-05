//! Platform side of the MTA app: downloads feeds over HTTPS on the
//! background worker, with diagnostics breadcrumbs.

use matrix_hub_core::apps::mta::{FeedSource, Fetcher, FETCH_INTERVAL};

use crate::diag::{self, Crumb, Stage};
use crate::network::http;
use crate::task::TimerTask;

/// [`matrix_hub_core::apps::mta::StartFetching`] for the device.
pub fn start_fetching(mut fetcher: Fetcher) -> Box<dyn Send> {
    Box::new(TimerTask::spawn(FETCH_INTERVAL, move || {
        fetcher.run(&mut HttpFeedSource);
        diag::WORKER.enter(Stage::WorkerSleep);
    }))
}

struct HttpFeedSource;

impl FeedSource for HttpFeedSource {
    fn fetch(&mut self, url: &str) -> anyhow::Result<Vec<u8>> {
        diag::inc(Crumb::FetchStarted);
        let started_ms = diag::uptime_ms();
        diag::WORKER.enter(Stage::HttpConnect);
        diag::maybe_test_hang_worker();

        let result = http::fetch_binary(url);

        match &result {
            Ok(data) => {
                diag::inc(Crumb::FetchOk);
                diag::set(Crumb::LastFetchBytes, data.len() as u32);
                diag::set_max(Crumb::MaxFetchBytes, data.len() as u32);
            }
            Err(_) => diag::inc(Crumb::FetchErr),
        }
        diag::set(Crumb::LastFetchMs, (diag::uptime_ms() - started_ms) as u32);
        diag::set(Crumb::LastFetchEndS, diag::uptime_s());
        result
    }

    fn min_minutes(&self) -> u32 {
        crate::storage::with_config(|c| c.min_minutes.unwrap_or(1))
    }

    fn on_parse(&mut self) {
        diag::WORKER.enter(Stage::Parse);
    }

    fn on_publish(&mut self) {
        diag::WORKER.enter(Stage::Publish);
    }
}
