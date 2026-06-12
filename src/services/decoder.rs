use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::pipeline::{DecodingPipeline, PipelineResult};
use crate::sstv::DecoderParams;

/// Owns the background decode worker: channels, request ids, queue depth
/// accounting, health monitoring, and restart. `VoyagerApp` talks to this
/// instead of juggling raw channel halves.
pub struct DecodeOrchestrator {
    tx: Option<Sender<DecodeRequest>>,
    rx: Option<Receiver<DecodeResult>>,
    handle: Option<JoinHandle<()>>,
    next_id: u64,
    pending: usize,
    last_response: Instant,
}

impl DecodeOrchestrator {
    pub fn new() -> Self {
        let (tx, rx, handle) = spawn_decode_worker();
        Self {
            tx: Some(tx),
            rx: Some(rx),
            handle: Some(handle),
            next_id: 0,
            pending: 0,
            last_response: Instant::now(),
        }
    }

    pub fn pending(&self) -> usize {
        self.pending
    }

    /// Enqueue a decode request (non-blocking). Returns false when the queue
    /// is full or the worker is gone.
    #[allow(clippy::too_many_arguments)]
    pub fn request(
        &mut self,
        samples: Arc<[f32]>,
        start_offset: usize,
        params: DecoderParams,
        sample_rate: u32,
        max_queue: usize,
        generation: u64,
    ) -> bool {
        if self.pending >= max_queue {
            tracing::debug!("Decode queue full, skipping request");
            return false;
        }
        let Some(tx) = &self.tx else {
            return false;
        };
        let request = DecodeRequest {
            id: self.next_id,
            generation,
            samples,
            start_offset,
            params,
            sample_rate,
        };
        self.next_id += 1;
        // Stamp activity when the queue transitions idle -> busy, otherwise a
        // long idle period before the first request reads as "unresponsive"
        // and triggers a spurious worker restart.
        if self.pending == 0 {
            self.last_response = Instant::now();
        }
        self.pending += 1;
        if tx.send(request).is_err() {
            tracing::warn!("Decode worker thread has terminated");
            self.pending = self.pending.saturating_sub(1);
            return false;
        }
        true
    }

    /// Drain all available results without blocking, updating queue depth and
    /// the responsiveness timestamp.
    pub fn poll(&mut self) -> Vec<DecodeResult> {
        let Some(rx) = &self.rx else {
            return Vec::new();
        };
        let results: Vec<DecodeResult> = rx.try_iter().collect();
        if !results.is_empty() {
            self.pending = self.pending.saturating_sub(results.len());
            self.last_response = Instant::now();
        }
        results
    }

    /// Health check: thread alive, and responsive while work is pending.
    pub fn is_healthy(&self, max_unresponsive: Duration) -> bool {
        let Some(handle) = &self.handle else {
            tracing::warn!("Worker handle is None, needs restart");
            return false;
        };
        if handle.is_finished() {
            tracing::error!("Worker thread has exited/panicked, needs restart");
            return false;
        }
        if self.pending > 0 {
            let elapsed = self.last_response.elapsed();
            if elapsed > max_unresponsive {
                tracing::warn!(
                    elapsed_ms = elapsed.as_millis(),
                    threshold_ms = max_unresponsive.as_millis(),
                    "Worker thread unresponsive, needs restart"
                );
                return false;
            }
        }
        true
    }

    /// Recreate channels and respawn the worker. Pending requests are lost.
    pub fn restart(&mut self) {
        tracing::warn!("Restarting worker thread");
        let discarded = self.rx.as_ref().map(|rx| rx.try_iter().count()).unwrap_or(0);
        if discarded > 0 {
            tracing::debug!("Discarding {} pending results on worker restart", discarded);
        }
        self.tx = None;
        self.rx = None;
        self.handle = None;

        let (tx, rx, handle) = spawn_decode_worker();
        self.tx = Some(tx);
        self.rx = Some(rx);
        self.handle = Some(handle);
        self.pending = 0;
        self.last_response = Instant::now();
        tracing::info!("Worker thread restarted successfully");
    }
}

impl Default for DecodeOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DecodeOrchestrator {
    fn drop(&mut self) {
        // Drop channels first to signal the worker to shut down, then give it
        // a short grace period; detach rather than block the UI thread.
        self.tx = None;
        self.rx = None;
        if let Some(handle) = self.handle.take() {
            if !handle.is_finished() {
                thread::sleep(Duration::from_millis(100));
            }
            if !handle.is_finished() {
                tracing::warn!("Worker thread still running after timeout, detaching");
            } else if let Err(e) = handle.join() {
                tracing::error!("Worker thread panicked on shutdown: {:?}", e);
            }
        }
    }
}

/// Request to decode audio samples in background thread.
#[derive(Debug)]
pub struct DecodeRequest {
    /// Unique request ID for matching results to requests
    pub id: u64,
    /// Input-state generation (bumped on file load / channel switch). Results
    /// from an older generation are stale and must not be displayed.
    pub generation: u64,
    /// Shared audio buffer (Arc enables zero-copy sharing with worker thread)
    pub samples: Arc<[f32]>,
    /// Starting sample position for decode window
    pub start_offset: usize,
    /// Decoder parameters (line duration, threshold)
    pub params: DecoderParams,
    /// Sample rate in Hz (needed for samples-per-line calculation)
    pub sample_rate: u32,
}

/// Result from background decoding operation.
#[derive(Debug)]
pub struct DecodeResult {
    /// Request ID (matches DecodeRequest.id)
    pub id: u64,
    /// Echo of the request's input-state generation
    pub generation: u64,
    /// Decoded pipeline result (pixels, dimensions, mode), or None on error
    pub result: Option<PipelineResult>,
    /// Time taken to decode (for performance monitoring)
    pub decode_duration: Duration,
    /// Error message if decode failed
    pub error: Option<String>,
}

/// Spawn a background worker thread for non-blocking SSTV decoding.
pub fn spawn_decode_worker() -> (Sender<DecodeRequest>, Receiver<DecodeResult>, JoinHandle<()>) {
    // Create bidirectional channels for request/response
    let (request_tx, request_rx) = channel::<DecodeRequest>();
    let (result_tx, result_rx) = channel::<DecodeResult>();

    let handle = thread::spawn(move || {
        let pipeline = DecodingPipeline::new();

        tracing::info!("Decode worker thread started");

        while let Ok(request) = request_rx.recv() {
            let start_time = Instant::now();
            tracing::debug!("Starting decode for request {}", request.id);

            // Handle slicing here since DecodingPipeline expects a slice
            // We need to calculate the slice based on start_offset and decode_window_secs
            let window_duration_secs = request.params.decode_window_secs;
            let window_samples = (window_duration_secs * request.sample_rate as f64) as usize;
            let end_offset = request.start_offset.saturating_add(window_samples).min(request.samples.len());

            let samples_slice = if request.start_offset < request.samples.len() {
                &request.samples[request.start_offset..end_offset]
            } else {
                &[]
            };

            let result = match pipeline.process(samples_slice, &request.params, request.sample_rate) {
                Ok(pipeline_result) => {
                    tracing::debug!("Decode successful for request {}", request.id);
                    DecodeResult {
                        id: request.id,
                        generation: request.generation,
                        result: Some(pipeline_result),
                        decode_duration: start_time.elapsed(),
                        error: None,
                    }
                }
                Err(e) => {
                    tracing::error!("Decode failed for request {}: {}", request.id, e);
                    DecodeResult {
                        id: request.id,
                        generation: request.generation,
                        result: None,
                        decode_duration: start_time.elapsed(),
                        error: Some(e.to_string()),
                    }
                }
            };

            if result_tx.send(result).is_err() {
                tracing::info!("Main thread closed result channel, worker shutting down");
                break;
            }
        }

        tracing::info!("Decode worker thread exiting");
    });

    (request_tx, result_rx, handle)
}
