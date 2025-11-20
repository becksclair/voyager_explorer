//! Comprehensive metrics collection and reporting.
//!
//! Uses HDR histograms for accurate latency percentiles.

use hdrhistogram::Histogram;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Application-wide metrics
#[derive(Debug)]
pub struct AppMetrics {
    /// Decode operation latency histogram (milliseconds)
    decode_latency_ms: Histogram<u64>,

    /// UI frame time histogram (milliseconds)
    frame_time_ms: Histogram<u64>,

    /// Worker queue depth gauge
    worker_queue_depth: AtomicU64,

    /// Total decode requests
    total_decode_requests: AtomicU64,

    /// Total successful decodes
    total_decode_success: AtomicU64,

    /// Total decode errors
    total_decode_errors: AtomicU64,

    /// Total pixels decoded
    total_pixels_decoded: AtomicU64,

    /// Worker thread restarts (due to panic or timeout)
    worker_restarts: AtomicU64,

    /// Audio playback metrics
    #[cfg(feature = "audio_playback")]
    audio_metrics: AudioMetrics,

    /// Last update timestamp
    last_update: Instant,
}

/// Audio-specific metrics
#[cfg(feature = "audio_playback")]
#[derive(Debug, Clone)]
pub struct AudioMetrics {
    /// Total play/pause operations
    pub play_pause_count: u64,

    /// Total stop operations
    pub stop_count: u64,

    /// Total seek operations
    pub seek_count: u64,

    /// Audio device errors
    pub device_errors: u64,

    /// Total playback time in seconds
    pub total_playback_secs: f64,

    /// Last playback start time
    last_playback_start: Option<Instant>,
}

/// Summary of key metrics for display
#[derive(Debug, Clone)]
pub struct MetricsSummary {
    /// P50 decode latency (milliseconds)
    pub decode_p50_ms: f64,

    /// P95 decode latency (milliseconds)
    pub decode_p95_ms: f64,

    /// P99 decode latency (milliseconds)
    pub decode_p99_ms: f64,

    /// Average FPS
    pub avg_fps: f64,

    /// P99 frame time (milliseconds)
    pub frame_p99_ms: f64,

    /// Current worker queue depth
    pub worker_queue_depth: u64,

    /// Total decode requests
    pub total_requests: u64,

    /// Success rate (0.0-1.0)
    pub success_rate: f64,

    /// Total pixels decoded
    pub total_pixels: u64,

    /// Worker thread restarts
    pub worker_restarts: u64,

    /// Uptime in seconds
    pub uptime_secs: f64,
}

impl Default for AppMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl AppMetrics {
    /// Create new metrics collector
    pub fn new() -> Self {
        // Histogram for 1ns to 10 seconds with 2 significant digits
        let decode_latency_ms =
            Histogram::new_with_bounds(1, 10_000, 2).expect("Histogram creation should succeed");

        let frame_time_ms =
            Histogram::new_with_bounds(1, 1_000, 2).expect("Histogram creation should succeed");

        Self {
            decode_latency_ms,
            frame_time_ms,
            worker_queue_depth: AtomicU64::new(0),
            total_decode_requests: AtomicU64::new(0),
            total_decode_success: AtomicU64::new(0),
            total_decode_errors: AtomicU64::new(0),
            total_pixels_decoded: AtomicU64::new(0),
            worker_restarts: AtomicU64::new(0),
            #[cfg(feature = "audio_playback")]
            audio_metrics: AudioMetrics::default(),
            last_update: Instant::now(),
        }
    }

    /// Record decode operation latency
    pub fn record_decode(&mut self, duration: Duration, pixels: usize, success: bool) {
        let ms = duration.as_millis() as u64;

        if let Err(e) = self.decode_latency_ms.record(ms) {
            tracing::warn!("Failed to record decode latency: {}", e);
        }

        self.total_decode_requests.fetch_add(1, Ordering::Relaxed);

        if success {
            self.total_decode_success.fetch_add(1, Ordering::Relaxed);
            self.total_pixels_decoded
                .fetch_add(pixels as u64, Ordering::Relaxed);
        } else {
            self.total_decode_errors.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record UI frame time
    pub fn record_frame_time(&mut self, duration: Duration) {
        let ms = duration.as_millis() as u64;

        if let Err(e) = self.frame_time_ms.record(ms) {
            tracing::warn!("Failed to record frame time: {}", e);
        }
    }

    /// Record a worker thread restart
    pub fn record_worker_restart(&self) {
        self.worker_restarts.fetch_add(1, Ordering::Relaxed);
    }

    /// Update worker queue depth
    pub fn set_worker_queue_depth(&self, depth: usize) {
        self.worker_queue_depth
            .store(depth as u64, Ordering::Relaxed);
    }

    /// Get current metrics summary
    pub fn summary(&self) -> MetricsSummary {
        let total_requests = self.total_decode_requests.load(Ordering::Relaxed);
        let total_success = self.total_decode_success.load(Ordering::Relaxed);
        let success_rate = if total_requests > 0 {
            total_success as f64 / total_requests as f64
        } else {
            0.0
        };

        let decode_p50_ms = self.decode_latency_ms.value_at_quantile(0.5) as f64;
        let decode_p95_ms = self.decode_latency_ms.value_at_quantile(0.95) as f64;
        let decode_p99_ms = self.decode_latency_ms.value_at_quantile(0.99) as f64;

        let frame_p99_ms = self.frame_time_ms.value_at_quantile(0.99) as f64;
        let avg_frame_ms = self.frame_time_ms.mean();
        let avg_fps = if avg_frame_ms > 0.0 {
            1000.0 / avg_frame_ms
        } else {
            0.0
        };

        MetricsSummary {
            decode_p50_ms,
            decode_p95_ms,
            decode_p99_ms,
            avg_fps,
            frame_p99_ms,
            worker_queue_depth: self.worker_queue_depth.load(Ordering::Relaxed),
            total_requests,
            success_rate,
            total_pixels: self.total_pixels_decoded.load(Ordering::Relaxed),
            worker_restarts: self.worker_restarts.load(Ordering::Relaxed),
            uptime_secs: self.last_update.elapsed().as_secs_f64(),
        }
    }

    /// Reset all metrics
    pub fn reset(&mut self) {
        self.decode_latency_ms.clear();
        self.frame_time_ms.clear();
        self.total_decode_requests.store(0, Ordering::Relaxed);
        self.total_decode_success.store(0, Ordering::Relaxed);
        self.total_decode_errors.store(0, Ordering::Relaxed);
        self.total_pixels_decoded.store(0, Ordering::Relaxed);
        self.worker_restarts.store(0, Ordering::Relaxed);
        self.last_update = Instant::now();

        #[cfg(feature = "audio_playback")]
        {
            self.audio_metrics = AudioMetrics::default();
        }
    }

    /// Get audio metrics
    #[cfg(feature = "audio_playback")]
    pub fn audio(&self) -> &AudioMetrics {
        &self.audio_metrics
    }

    /// Get mutable audio metrics
    #[cfg(feature = "audio_playback")]
    pub fn audio_mut(&mut self) -> &mut AudioMetrics {
        &mut self.audio_metrics
    }
}

#[cfg(feature = "audio_playback")]
impl Default for AudioMetrics {
    fn default() -> Self {
        Self {
            play_pause_count: 0,
            stop_count: 0,
            seek_count: 0,
            device_errors: 0,
            total_playback_secs: 0.0,
            last_playback_start: None,
        }
    }
}

#[cfg(feature = "audio_playback")]
impl AudioMetrics {
    /// Record play/pause operation
    pub fn record_play_pause(&mut self) {
        self.play_pause_count += 1;
        self.last_playback_start = Some(Instant::now());
    }

    /// Record stop operation
    pub fn record_stop(&mut self) {
        self.stop_count += 1;

        if let Some(start) = self.last_playback_start.take() {
            self.total_playback_secs += start.elapsed().as_secs_f64();
        }
    }

    /// Record seek operation
    pub fn record_seek(&mut self) {
        self.seek_count += 1;
    }

    /// Record device error
    pub fn record_device_error(&mut self) {
        self.device_errors += 1;
    }
}

impl MetricsSummary {
    /// Render metrics as UI panel
    pub fn ui_panel(&self, ui: &mut egui::Ui) {
        ui.heading("📊 Performance Metrics");

        ui.separator();

        ui.label("⏱️ Decode Latency:");
        ui.indent("decode_latency", |ui| {
            ui.monospace(format!("  P50: {:.1}ms", self.decode_p50_ms));
            ui.monospace(format!("  P95: {:.1}ms", self.decode_p95_ms));
            ui.monospace(format!("  P99: {:.1}ms", self.decode_p99_ms));
        });

        ui.separator();

        ui.label("🖼️ UI Performance:");
        ui.indent("ui_perf", |ui| {
            ui.monospace(format!("  Avg FPS: {:.1}", self.avg_fps));
            ui.monospace(format!("  Frame P99: {:.1}ms", self.frame_p99_ms));
        });

        ui.separator();

        ui.label("📈 Statistics:");
        ui.indent("stats", |ui| {
            ui.monospace(format!("  Total Requests: {}", self.total_requests));
            ui.monospace(format!("  Success Rate: {:.1}%", self.success_rate * 100.0));
            ui.monospace(format!("  Total Pixels: {}", self.total_pixels));
            ui.monospace(format!("  Queue Depth: {}", self.worker_queue_depth));
        });

        ui.separator();

        ui.label(format!("⏲️ Uptime: {:.1}s", self.uptime_secs));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_creation() {
        let _metrics = AppMetrics::new();
    }

    #[test]
    fn test_decode_recording() {
        let mut metrics = AppMetrics::new();

        metrics.record_decode(Duration::from_millis(150), 512 * 100, true);
        metrics.record_decode(Duration::from_millis(200), 512 * 50, true);
        metrics.record_decode(Duration::from_millis(100), 0, false);

        let summary = metrics.summary();

        assert_eq!(summary.total_requests, 3);
        assert_eq!(summary.total_pixels, 512 * 150);
        assert!((summary.success_rate - 2.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn test_frame_time_recording() {
        let mut metrics = AppMetrics::new();

        for i in 1..=10 {
            metrics.record_frame_time(Duration::from_millis(i * 2));
        }

        let summary = metrics.summary();
        assert!(summary.avg_fps > 0.0);
    }

    #[test]
    fn test_queue_depth() {
        let metrics = AppMetrics::new();

        metrics.set_worker_queue_depth(5);
        assert_eq!(metrics.summary().worker_queue_depth, 5);

        metrics.set_worker_queue_depth(10);
        assert_eq!(metrics.summary().worker_queue_depth, 10);
    }

    #[test]
    fn test_metrics_reset() {
        let mut metrics = AppMetrics::new();

        metrics.record_decode(Duration::from_millis(100), 1000, true);
        metrics.set_worker_queue_depth(5);

        metrics.reset();

        let summary = metrics.summary();
        assert_eq!(summary.total_requests, 0);
        assert_eq!(summary.total_pixels, 0);
    }
}
