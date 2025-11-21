//! Decoder parameter presets for common use cases.
//!
//! This module provides predefined parameter sets for different SSTV decoding scenarios,
//! making it easier to quickly switch between configurations optimized for different
//! types of Voyager Golden Record images.

use crate::sstv::{DecoderMode, DecoderParams};

/// A named preset containing decoder parameters.
#[derive(Debug, Clone, Copy)]
pub struct DecoderPreset {
    /// Human-readable preset name
    pub name: &'static str,
    /// Associated decoder parameters
    pub params: DecoderParams,
}

/// Standard presets for common decoding scenarios
pub const PRESETS: &[DecoderPreset] = &[
    DecoderPreset {
        name: "Voyager Default",
        params: DecoderParams {
            line_duration_ms: 8.3,
            threshold: 0.2,
            decode_window_secs: 2.0,
            mode: DecoderMode::BinaryGrayscale,
        },
    },
    DecoderPreset {
        name: "High Resolution",
        params: DecoderParams {
            line_duration_ms: 12.0,
            threshold: 0.15,
            decode_window_secs: 3.0,
            mode: DecoderMode::BinaryGrayscale,
        },
    },
    DecoderPreset {
        name: "Fast Scan",
        params: DecoderParams {
            line_duration_ms: 5.0,
            threshold: 0.25,
            decode_window_secs: 1.5,
            mode: DecoderMode::BinaryGrayscale,
        },
    },
    DecoderPreset {
        name: "Color (Pseudocolor)",
        params: DecoderParams {
            line_duration_ms: 8.3,
            threshold: 0.2,
            decode_window_secs: 2.0,
            mode: DecoderMode::PseudoColor,
        },
    },
    DecoderPreset {
        name: "Sensitive (Low Threshold)",
        params: DecoderParams {
            line_duration_ms: 8.3,
            threshold: 0.1,
            decode_window_secs: 2.0,
            mode: DecoderMode::BinaryGrayscale,
        },
    },
    DecoderPreset {
        name: "Test Pattern",
        params: DecoderParams {
            line_duration_ms: 10.0,
            threshold: 0.3,
            decode_window_secs: 2.5,
            mode: DecoderMode::BinaryGrayscale,
        },
    },
];

/// Helper to find a preset by name
pub fn find_preset(name: &str) -> Option<&'static DecoderPreset> {
    PRESETS.iter().find(|p| p.name == name)
}

/// Check if given parameters match any preset
pub fn matches_preset(params: &DecoderParams) -> Option<&'static str> {
    PRESETS.iter().find_map(|preset| {
        if params_equal(&preset.params, params) {
            Some(preset.name)
        } else {
            None
        }
    })
}

/// Compare two DecoderParams for equality (with floating point tolerance)
fn params_equal(a: &DecoderParams, b: &DecoderParams) -> bool {
    const EPSILON: f32 = 0.001;
    const EPSILON_F64: f64 = 0.001;

    (a.line_duration_ms - b.line_duration_ms).abs() < EPSILON
        && (a.threshold - b.threshold).abs() < EPSILON
        && (a.decode_window_secs - b.decode_window_secs).abs() < EPSILON_F64
        && a.mode == b.mode
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_presets_have_unique_names() {
        let mut names = std::collections::HashSet::new();
        for preset in PRESETS {
            assert!(
                names.insert(preset.name),
                "Duplicate preset name: {}",
                preset.name
            );
        }
    }

    #[test]
    fn test_find_preset_by_name() {
        let preset = find_preset("Voyager Default");
        assert!(preset.is_some());
        assert_eq!(preset.unwrap().name, "Voyager Default");

        let missing = find_preset("Nonexistent");
        assert!(missing.is_none());
    }

    #[test]
    fn test_matches_preset() {
        let default_params = DecoderParams::default();
        let matched = matches_preset(&default_params);
        assert_eq!(matched, Some("Voyager Default"));
    }

    #[test]
    fn test_custom_params_dont_match() {
        let custom = DecoderParams {
            line_duration_ms: 99.9,
            threshold: 0.5,
            decode_window_secs: 5.0,
            mode: DecoderMode::BinaryGrayscale,
        };
        let matched = matches_preset(&custom);
        assert_eq!(matched, None);
    }

    #[test]
    fn test_params_equal_with_tolerance() {
        let a = DecoderParams {
            line_duration_ms: 8.3,
            threshold: 0.2,
            decode_window_secs: 2.0,
            mode: DecoderMode::BinaryGrayscale,
        };
        let b = DecoderParams {
            line_duration_ms: 8.3001, // Within tolerance
            threshold: 0.2,
            decode_window_secs: 2.0,
            mode: DecoderMode::BinaryGrayscale,
        };
        assert!(params_equal(&a, &b));

        let c = DecoderParams {
            line_duration_ms: 8.5, // Outside tolerance
            threshold: 0.2,
            decode_window_secs: 2.0,
            mode: DecoderMode::BinaryGrayscale,
        };
        assert!(!params_equal(&a, &c));
    }
}
