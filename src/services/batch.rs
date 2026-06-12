//! Background batch-processing runner: owns the worker thread, progress
//! channel, and cancellation flag so the app only starts, polls, and reaps.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::sstv::{DecoderMode, DecoderParams};

/// Processing state of a single batch queue entry.
#[derive(Debug, Clone, PartialEq)]
pub enum BatchStatus {
    Pending,
    Processing,
    Done,
    Error(String),
}

/// One file in the batch queue, paired with its current status.
#[derive(Debug, Clone)]
pub struct BatchItem {
    pub path: PathBuf,
    pub status: BatchStatus,
}

/// Progress messages from the batch worker to the UI.
pub enum BatchProgressMsg {
    ItemStatus(usize, BatchStatus),
    Progress(f32),
    Error(String),
}

#[derive(Default)]
pub struct BatchRunner {
    worker: Option<JoinHandle<()>>,
    rx: Option<Receiver<BatchProgressMsg>>,
    cancel: Option<Arc<AtomicBool>>,
}

impl BatchRunner {
    pub fn is_running(&self) -> bool {
        self.worker.is_some()
    }

    /// Spawn the batch worker over `queue`. Returns the cancellation flag so
    /// the panel's Stop button can share it.
    pub fn start(&mut self, queue: Vec<BatchItem>, output_dir: PathBuf, mode: DecoderMode) -> Arc<AtomicBool> {
        let cancel_flag = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel_flag.clone());

        let (tx, rx) = std::sync::mpsc::channel();
        self.rx = Some(rx);

        let worker_cancel = cancel_flag.clone();
        self.worker = Some(std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let params = DecoderParams {
                    mode,
                    ..Default::default()
                };

                let total = queue.len();

                for (index, item) in queue.iter().enumerate() {
                    if worker_cancel.load(Ordering::Acquire) {
                        tracing::info!("Batch processing cancelled by user");
                        break;
                    }

                    let _ = tx.send(BatchProgressMsg::ItemStatus(index, BatchStatus::Processing));

                    let result = crate::batch::process_single_file(&item.path, &output_dir, &params);

                    let status = match result {
                        Ok(_) => BatchStatus::Done,
                        Err(e) => BatchStatus::Error(e.to_string()),
                    };
                    let _ = tx.send(BatchProgressMsg::ItemStatus(index, status));

                    let progress = (index + 1) as f32 / total.max(1) as f32;
                    let _ = tx.send(BatchProgressMsg::Progress(progress));
                }

                tracing::info!("Batch processing thread completed");
            }));

            if let Err(e) = result {
                let panic_msg = if let Some(s) = e.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = e.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "Unknown panic".to_string()
                };
                let _ = tx.send(BatchProgressMsg::Error(format!("Worker panicked: {}", panic_msg)));
                tracing::error!("Batch worker panicked: {}", panic_msg);
            }
        }));

        cancel_flag
    }

    /// Drain all pending progress messages without blocking.
    pub fn poll(&mut self) -> Vec<BatchProgressMsg> {
        self.rx.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default()
    }

    /// Join and clean up the worker if it has finished. Returns the progress
    /// messages still queued at reap time (the worker's final status/progress
    /// sends race the finish check, and dropping the receiver unread would
    /// leave the last item stuck at "Processing").
    pub fn reap_if_finished(&mut self) -> Option<Vec<BatchProgressMsg>> {
        let finished = self.worker.as_ref().map(|h| h.is_finished()).unwrap_or(false);
        if !finished {
            return None;
        }
        if let Some(handle) = self.worker.take() {
            if let Err(e) = handle.join() {
                tracing::error!("Batch worker thread panicked: {:?}", e);
            }
        }
        let remaining = self.rx.take().map(|rx| rx.try_iter().collect()).unwrap_or_default();
        self.cancel = None;
        Some(remaining)
    }
}

impl Drop for BatchRunner {
    fn drop(&mut self) {
        if let Some(handle) = self.worker.take() {
            if let Some(flag) = &self.cancel {
                flag.store(true, Ordering::Release);
            }
            self.rx = None;
            if let Err(e) = handle.join() {
                tracing::error!("Batch worker thread panicked on shutdown: {:?}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_domain_types_construct_without_ui() {
        // The service layer must own these types outright — no UI dependency.
        let item = BatchItem {
            path: PathBuf::from("in.wav"),
            status: BatchStatus::Pending,
        };
        assert_eq!(item.status, BatchStatus::Pending);
        assert_eq!(BatchStatus::Error("x".to_string()), BatchStatus::Error("x".to_string()));
    }
}
