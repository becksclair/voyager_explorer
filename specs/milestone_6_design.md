# Milestone 6 Design: Advanced Signal Analysis & Batch Processing UI

## 1. Overview

This document outlines the design for the advanced features in Milestone 6:
1. **Spectrum Panel Enhancements:** Improving the existing spectrum view with professional analysis tools.
2. **Batch Processing UI:** Adding a graphical interface for the existing batch processing logic.

## 2. Spectrum Panel Enhancements

### 2.1. Current State

- Basic linear magnitude vs linear frequency plot.
- Uses `realfft` and `egui_plot`.
- Located in a collapsible right side panel.

### 2.2. Proposed Improvements

#### 2.2.1. Logarithmic Frequency Scale

- **Why:** Audio signals cover a wide range (20Hz - 20kHz). A linear scale crowds the important low-mid frequencies.
- **Implementation:**
  - Add a checkbox "Log Scale" in the panel header.
  - **Important:** `egui_plot` does NOT have native log coordinate transforms. Two approaches:
        1. **Manual Transform (Used):** Transform x-values with `log10(freq)` before plotting, filter out zero/negative frequencies, and optionally use `x_axis_formatter` to display original frequency values on axis labels.
        2. **Visual Only:** Keep data in linear coordinates but use custom grid/tick helpers for log-like appearance (more complex).
  - Handle edge cases: filter `freq <= 0.0` before applying `log10()` to avoid NaN/infinity.

#### 2.2.2. Decibel (dB) Magnitude Scale

- **Why:** Audio dynamic range is best represented logarithmically.
- **Implementation:**
  - Formula: `dB = 20 * log10(magnitude)`.
  - Add a checkbox "dB Scale".
  - Clamp low values to a noise floor (e.g., -100 dB) to avoid `-inf`.

#### 2.2.3. Peak Detection

- **Why:** To easily identify sync tones (e.g., 1200Hz) or other artifacts.
- **Implementation:**
  - Find the bin with the maximum magnitude.
  - Display a text label or marker at that peak.
  - Show "Peak: X Hz" in the panel header.

### 2.3. UI Mockup

```text
┌────────────────────────────────────────┐
│ 📈 Signal Analysis (Spectrum)          │
│ [x] Log Freq  [x] dB Scale             │
│ Peak: 1200.5 Hz                        │
│ ┌────────────────────────────────────┐ │
│ │              |                     │ │
│ │              |                     │ │
│ │      /\      |                     │ │
│ │ ____/  \_____|____________________ │ │
│ │              ^ 1200Hz              │ │
│ └────────────────────────────────────┘ │
└────────────────────────────────────────┘
```

## 3. Batch Processing UI

### 3.1. Current State

- CLI command `voyager_explorer batch --input "*.wav" --output "out/"`.
- Logic exists in `src/batch.rs`.

### 3.2. Proposed Improvements

#### 3.2.1. New UI Panel

- Add a "Batch" button in the top panel (next to "Decode").
- Opens a modal window or a new central panel tab.

#### 3.2.2. Workflow

1. **Select Files:** Button to open file dialog (multi-select).
2. **Select Output Dir:** Button to select destination.
3. **Configure:** Select Decoder Mode (B/W vs Color).
4. **Queue:** List selected files with status (Pending, Processing, Done, Error).
5. **Run:** "Start Batch" button.
6. **Progress:** Progress bar showing overall completion.

#### 3.2.3. Implementation Details

- **State:**
    ```rust
    struct BatchState {
        queue: Vec<BatchItem>,
        output_dir: Option<PathBuf>,
        decoder_mode: DecoderMode, // Added for mode selection
        is_processing: bool,
        current_index: usize,
    }

    struct BatchItem {
        path: PathBuf,
        status: BatchStatus,
        error: Option<String>,
    }

    enum BatchStatus { Pending, Processing, Done, Error }
    ```
- **Concurrency & Robustness Contracts:**
  - **Cancellation:**
    - Mechanism: `Arc<AtomicBool>` `cancel_flag` shared with worker + `Cancel` channel message.
    - Check: Worker checks `cancel_flag` between items.
  - **Thread Safety:**
    - State: `Arc<Mutex<BatchState>>` (or `RwLock`) for queue/data access.
    - Flags: `is_processing` as `AtomicBool` for low-latency UI reads.
  - **Error Recovery:**
    - **Per-File:** Mark item `Error`, log error, emit progress, continue to next (default).
    - **Environmental (Fatal):** On write error to `output_dir`, mark item `Error`, emit `Fatal` notification, pause batch (`is_processing=false`), await user (Retry/New Dir/Cancel).
  - **Progress Messaging:**
    - Type:
            ```rust
            struct BatchProgress {
                total: usize,
                completed: usize,
                current_index: usize,
                item_statuses: Vec<(PathBuf, BatchStatus)>, // Or delta updates
                last_error: Option<String>,
            }
            ```
    - Frequency: Item start, Item complete, Error, Periodic heartbeat (N seconds).
  - **Partial Results:**
    - Policy: **Preserve by default**.
    - Option: UI toggle to cleanup completed outputs on Cancel/Fatal.

#### 3.2.4. Output Handling

- **Naming Convention:**
  - Preserve original filename: `input_name.wav` -> `input_name.png`.
  - Output format: Always PNG for decoded images.
- **Conflict Resolution:**
  - Strategy: **Smart Rename** (Auto-increment).
  - If `file.png` exists, try `file_1.png`, `file_2.png`, etc.
  - Prevents accidental overwrites without interrupting the batch.

### 3.3. UI Mockup

```text
┌────────────────────────────────────────┐
│ 📦 Batch Processing                    │
│                                        │
│ [ Add Files... ] [ Select Output... ]  │
│ Output: C:\Users\...\Desktop\out       │
│ Mode: [ Binary (B/W) |v]               │
│                                        │
│ Queue:                                 │
│ 1. golden_record_1.wav  ✅ Done        │
│ 2. golden_record_2.wav  🔄 Processing  │
│ 3. golden_record_3.wav  ⏳ Pending     │
│                                        │
│ [ Start Batch ]  [ Cancel ]            │
│ ▓▓▓▓▓▓░░░░░░░░░░░░░░░  33%             │
└────────────────────────────────────────┘
```

## 4. Execution Plan

1. **Refactor Spectrum Panel:** Implement log/dB scales and peak detection.
2. **Implement Batch State:** Create structs and state management in `VoyagerApp`.
3. **Implement Batch UI:** Draw the panel/window.
4. **Wire up Batch Logic:** Connect UI to background processing.
