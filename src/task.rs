#![allow(dead_code, reason = "unused task utilities")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

type Job = Box<dyn FnOnce() + Send + 'static>;

static WORKER: OnceLock<Sender<Job>> = OnceLock::new();

pub fn init() {
    let _ = get_worker();
}

fn get_worker() -> Sender<Job> {
    WORKER
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Job>();

            let config = esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration {
                priority: 5, // LOW priority for network tasks
                ..Default::default()
            };
            config
                .set()
                .unwrap_or_else(|e| panic!("Failed to set worker thread config: {e:?}"));

            thread::Builder::new()
                .stack_size(16384)
                .spawn(move || {
                    while let Ok(job) = rx.recv() {
                        job();
                    }
                })
                .unwrap_or_else(|e| panic!("Failed to spawn background worker thread: {e:?}"));

            let _ = esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration::default().set();

            tx
        })
        .clone()
}

pub struct TimerTask<T> {
    latest: Arc<Mutex<Option<T>>>,
    running: Arc<AtomicBool>,
}

impl<T> TimerTask<T> {
    pub fn spawn<F>(interval: Duration, mut f: F) -> Self
    where
        F: FnMut() -> T + Send + 'static,
        T: Send + 'static,
    {
        let latest = Arc::new(Mutex::new(None));
        let running = Arc::new(AtomicBool::new(true));

        let l2 = latest.clone();
        let r2 = running.clone();

        get_worker()
            .send(Box::new(move || {
                // Immediately execute once before sleeping
                if r2.load(Ordering::Relaxed) {
                    let res = f();
                    if let Ok(mut guard) = l2.lock() {
                        *guard = Some(res);
                    }
                }

                while r2.load(Ordering::Relaxed) {
                    // Sleep cleanly in small increments to allow fast shutdown
                    for _ in 0..(interval.as_millis() / 100).max(1) {
                        if !r2.load(Ordering::Relaxed) {
                            break;
                        }
                        thread::sleep(Duration::from_millis(100));
                    }

                    if r2.load(Ordering::Relaxed) {
                        let res = f();
                        if let Ok(mut guard) = l2.lock() {
                            *guard = Some(res);
                        }
                    }
                }
            }))
            .unwrap_or_else(|e| panic!("Failed to send task to worker: {e:?}"));

        Self { latest, running }
    }

    pub fn get_latest(&self) -> Option<T>
    where
        T: Clone,
    {
        self.latest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl<T> Drop for TimerTask<T> {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}
