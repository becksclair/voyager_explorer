use std::ops::Range;
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
    /// Generation of the most recently accepted request.
    newest_generation: u64,
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
            newest_generation: 0,
            last_response: Instant::now(),
        }
    }

    pub fn pending(&self) -> usize {
        self.pending
    }

    /// Generation of the newest request still outstanding, if any. The
    /// worker serves requests in order, so while anything is pending the
    /// newest request is part of it.
    pub fn newest_pending_generation(&self) -> Option<u64> {
        (self.pending > 0).then_some(self.newest_generation)
    }

    /// Enqueue a decode request (non-blocking). Returns false when the queue
    /// is full or the worker is gone.
    #[allow(clippy::too_many_arguments)]
    pub fn request(
        &mut self,
        samples: Arc<[f32]>,
        start_offset: usize,
        image_range: Option<Range<usize>>,
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
            image_range,
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
        self.newest_generation = generation;
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
    /// Call it after [`Self::poll`]: a finished result still waiting in the
    /// channel counts as unanswered work.
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
    /// Input-state generation. Results from an older generation are stale.
    pub generation: u64,
    /// Shared audio buffer (Arc enables zero-copy sharing with worker thread)
    pub samples: Arc<[f32]>,
    /// Starting sample position for the fallback decode window
    pub start_offset: usize,
    /// Detected image range, if the bounds scan found one for this position
    pub image_range: Option<Range<usize>>,
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

            let (start_offset, end_offset) = if let Some(range) = &request.image_range {
                // Keep the worker's 60-second limit on detected image ranges.
                let max_image_samples = (request.sample_rate as usize).saturating_mul(60);
                (range.start, range.end.min(range.start.saturating_add(max_image_samples)))
            } else {
                let window_samples = (request.params.decode_window_secs * request.sample_rate as f64) as usize;
                (request.start_offset, request.start_offset.saturating_add(window_samples))
            };
            let end_offset = end_offset.min(request.samples.len());
            let samples_slice = request.samples.get(start_offset..end_offset).unwrap_or(&[]);
            let preroll_len = (request.sample_rate as f64 * crate::sstv::DC_RESTORE_PREROLL_SECS) as usize;
            let preroll_start = start_offset.saturating_sub(preroll_len);
            let preroll = request.samples.get(preroll_start..start_offset).unwrap_or(&[]);
            let segmented_image = request.image_range.is_some_and(|range| range.end == end_offset);

            let result = match pipeline.process_with_preroll(preroll, samples_slice, &request.params, request.sample_rate) {
                Ok(mut pipeline_result) => {
                    pipeline_result.segmented_image = segmented_image;
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
                    tracing::error!("Decode failed for request {}: {:#}", request.id, e);
                    DecodeResult {
                        id: request.id,
                        generation: request.generation,
                        result: None,
                        decode_duration: start_time.elapsed(),
                        error: Some(format!("{e:#}")),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_decodes_the_requested_image_range() {
        let sample_rate = 48_000;
        let params = DecoderParams::default();
        let width = 64;
        let pixels: Vec<u8> = (0..width * 40).map(|i| (i % 256) as u8).collect();
        let image = crate::test_fixtures::encode_image_to_audio(&pixels, width, sample_rate, params.line_duration_ms);
        // Silence on both sides: only the image range should be decoded.
        let lead = sample_rate as usize;
        let mut samples = vec![0.0f32; lead];
        samples.extend_from_slice(&image);
        samples.extend(std::iter::repeat_n(0.0, lead));
        let samples: Arc<[f32]> = samples.into();
        let image_range = lead..lead + image.len();

        let (tx, rx, _handle) = spawn_decode_worker();
        let request = |id, image_range: Option<Range<usize>>| DecodeRequest {
            id,
            generation: 0,
            samples: Arc::clone(&samples),
            start_offset: 0,
            image_range,
            params,
            sample_rate,
        };
        tx.send(request(1, Some(image_range.clone()))).unwrap();
        // A range running past the buffer is truncated, so it is not a whole image.
        tx.send(request(2, Some(image_range.start..samples.len() + 1))).unwrap();

        let whole = rx.recv_timeout(Duration::from_secs(30)).unwrap();
        let whole = whole.result.expect("image range decodes");
        assert!(whole.segmented_image);
        let samples_per_line = (params.line_duration_ms / 1000.0 * sample_rate as f32).round() as usize;
        assert!(whole.height as usize <= image.len() / samples_per_line + 1);

        let truncated = rx.recv_timeout(Duration::from_secs(30)).unwrap();
        assert!(!truncated.result.expect("truncated range decodes").segmented_image);
    }

    #[test]
    fn newest_pending_generation_follows_outstanding_work() {
        let mut worker = DecodeOrchestrator::new();
        assert_eq!(worker.newest_pending_generation(), None);

        let samples: Arc<[f32]> = vec![0.0f32; 16].into();
        for generation in [3, 7] {
            assert!(worker.request(Arc::clone(&samples), 0, None, DecoderParams::default(), 48_000, 4, generation));
        }
        assert_eq!(worker.newest_pending_generation(), Some(7));

        let deadline = Instant::now() + Duration::from_secs(30);
        while worker.pending() > 0 && Instant::now() < deadline {
            worker.poll();
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(worker.newest_pending_generation(), None);
    }

    #[test]
    fn failed_decode_reports_its_cause() {
        let mut worker = DecodeOrchestrator::new();
        // Far too short for one scan line.
        let samples: Arc<[f32]> = vec![0.0f32; 16].into();
        assert!(worker.request(samples, 0, None, DecoderParams::default(), 48_000, 4, 0));

        let deadline = Instant::now() + Duration::from_secs(30);
        let mut results = Vec::new();
        while results.is_empty() && Instant::now() < deadline {
            results = worker.poll();
            thread::sleep(Duration::from_millis(5));
        }
        let error = results[0].error.as_deref().expect("decode fails");
        assert!(error.contains("Insufficient samples"), "{error}");
        assert!(error.contains("got 16"), "{error}");
    }

    #[test]
    fn uncollected_result_reads_as_unresponsive_until_polled() {
        let mut worker = DecodeOrchestrator::new();
        let samples: Arc<[f32]> = vec![0.0f32; 16].into();
        assert!(worker.request(samples, 0, None, DecoderParams::default(), 48_000, 4, 0));

        // The caller stalls past the timeout without polling.
        let timeout = Duration::from_millis(50);
        thread::sleep(timeout * 2);
        assert!(!worker.is_healthy(timeout));

        let deadline = Instant::now() + Duration::from_secs(30);
        while worker.pending() > 0 && Instant::now() < deadline {
            worker.poll();
            thread::sleep(Duration::from_millis(5));
        }
        assert!(worker.is_healthy(timeout));
    }
}
