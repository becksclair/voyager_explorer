use crate::pipeline::{DecodingPipeline, PipelineResult};
use crate::sstv::DecoderParams;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Request to decode audio samples in background thread.
#[derive(Debug)]
pub struct DecodeRequest {
    /// Unique request ID for matching results to requests
    pub id: u64,
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
    /// Decoded pipeline result (pixels, dimensions, mode), or None on error
    pub result: Option<PipelineResult>,
    /// Time taken to decode (for performance monitoring)
    pub decode_duration: Duration,
    /// Error message if decode failed
    pub error: Option<String>,
}

/// Spawn a background worker thread for non-blocking SSTV decoding.
pub fn spawn_decode_worker() -> (
    Sender<DecodeRequest>,
    Receiver<DecodeResult>,
    JoinHandle<()>,
) {
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
            let end_offset = request
                .start_offset
                .saturating_add(window_samples)
                .min(request.samples.len());

            let samples_slice = if request.start_offset < request.samples.len() {
                &request.samples[request.start_offset..end_offset]
            } else {
                &[]
            };

            let result = match pipeline.process(samples_slice, &request.params, request.sample_rate)
            {
                Ok(pipeline_result) => {
                    tracing::debug!("Decode successful for request {}", request.id);
                    DecodeResult {
                        id: request.id,
                        result: Some(pipeline_result),
                        decode_duration: start_time.elapsed(),
                        error: None,
                    }
                }
                Err(e) => {
                    tracing::error!("Decode failed for request {}: {}", request.id, e);
                    DecodeResult {
                        id: request.id,
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
