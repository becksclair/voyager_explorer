//! Utility functions for the Voyager Explorer application

/// Format duration in seconds to MM:SS.SS format
pub fn format_duration(duration_secs: f32) -> String {
    let minutes = (duration_secs / 60.0) as u32;
    let seconds = duration_secs % 60.0;
    format!("{:02}:{:05.2}", minutes, seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration_zero() {
        let result = format_duration(0.0);
        assert_eq!(result, "00:00.00");
    }

    #[test]
    fn test_format_duration_seconds_only() {
        let result = format_duration(45.67);
        assert_eq!(result, "00:45.67");
    }

    #[test]
    fn test_format_duration_minutes_and_seconds() {
        let result = format_duration(125.45);
        assert_eq!(result, "02:05.45");
    }

    #[test]
    fn test_format_duration_exact_minute() {
        let result = format_duration(60.0);
        assert_eq!(result, "01:00.00");
    }

    #[test]
    fn test_format_duration_long() {
        let result = format_duration(3661.25); // 1 hour, 1 minute, 1.25 seconds
        assert_eq!(result, "61:01.25"); // Should show as 61 minutes
    }

    #[test]
    fn test_format_duration_fractional_seconds() {
        let result = format_duration(12.345);
        assert_eq!(result, "00:12.35"); // Should round to 2 decimal places
    }
}
