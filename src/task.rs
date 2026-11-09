use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, OnceLock};
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

/// Runs a job immediately and then every `interval` on the shared
/// background worker thread, until dropped.
///
/// All `TimerTask`s share one worker thread, so jobs run one at a time.
/// Dropping a task stops it at its next check (within ~100ms while sleeping),
/// but a job already in progress (e.g. an HTTP fetch) runs to completion
/// first, delaying any task queued behind it.
pub struct TimerTask {
    running: Arc<AtomicBool>,
}

impl TimerTask {
    pub fn spawn<F>(interval: Duration, mut f: F) -> Self
    where
        F: FnMut() + Send + 'static,
    {
        let running = Arc::new(AtomicBool::new(true));
        let r2 = running.clone();

        get_worker()
            .send(Box::new(move || {
                while r2.load(Ordering::Relaxed) {
                    f();

                    // Sleep in small increments to allow fast shutdown
                    for _ in 0..(interval.as_millis() / 100).max(1) {
                        if !r2.load(Ordering::Relaxed) {
                            break;
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            }))
            .unwrap_or_else(|e| panic!("Failed to send task to worker: {e:?}"));

        Self { running }
    }
}

impl Drop for TimerTask {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}
