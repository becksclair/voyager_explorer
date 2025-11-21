use crate::sstv::DecoderMode;
use egui::ColorImage;

/// Convert pixel array to an egui ColorImage
///
/// # Arguments
/// * `pixels` - Pixel data (Grayscale: 1 byte/pixel, Color: 3 bytes/pixel RGB)
/// * `mode` - Decoder mode to interpret the pixel data
///
/// # Returns
/// A ColorImage with fixed width of 512 pixels
pub fn image_from_pixels(pixels: &[u8], mode: DecoderMode) -> ColorImage {
    let width = 512;

    if pixels.is_empty() {
        return ColorImage::new([width, 1], vec![egui::Color32::BLACK; width]);
    }

    match mode {
        DecoderMode::BinaryGrayscale => {
            let height = pixels.len() / width;
            let height = if height == 0 { 1 } else { height };

            let mut img =
                ColorImage::new([width, height], vec![egui::Color32::BLACK; width * height]);

            let pixel_count = width * height;
            for (i, p) in pixels.iter().take(pixel_count).enumerate() {
                img.pixels[i] = egui::Color32::from_gray(*p);
            }
            img
        }
        DecoderMode::PseudoColor => {
            let bytes_per_pixel = 3;
            let height = pixels.len() / (width * bytes_per_pixel);
            let height = if height == 0 { 1 } else { height };

            let mut img =
                ColorImage::new([width, height], vec![egui::Color32::BLACK; width * height]);

            // Process RGB triplets
            for i in 0..img.pixels.len() {
                let src_idx = i * 3;
                if src_idx + 2 < pixels.len() {
                    let r = pixels[src_idx];
                    let g = pixels[src_idx + 1];
                    let b = pixels[src_idx + 2];
                    img.pixels[i] = egui::Color32::from_rgb(r, g, b);
                }
            }
            img
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_from_empty_pixels() {
        let empty_pixels = Vec::new();
        let img = image_from_pixels(&empty_pixels, DecoderMode::BinaryGrayscale);

        assert_eq!(img.size, [512, 1]);
        assert_eq!(img.pixels.len(), 512);

        // All pixels should be black for empty input
        for pixel in &img.pixels {
            assert_eq!(*pixel, egui::Color32::BLACK);
        }
    }

    #[test]
    fn test_image_from_single_line() {
        let mut pixels = Vec::new();
        for i in 0..512 {
            pixels.push((i % 256) as u8); // Pattern: 0, 1, 2, ..., 255, 0, 1, ...
        }

        let img = image_from_pixels(&pixels, DecoderMode::BinaryGrayscale);

        assert_eq!(img.size, [512, 1]);
        assert_eq!(img.pixels.len(), 512);

        // Check that grayscale conversion worked
        for (i, pixel) in img.pixels.iter().enumerate() {
            let expected_gray = (i % 256) as u8;
            let expected_color = egui::Color32::from_gray(expected_gray);
            assert_eq!(*pixel, expected_color);
        }
    }

    #[test]
    fn test_image_from_multiple_lines() {
        // Create a 2-line image (1024 pixels)
        let mut pixels = Vec::new();
        for line in 0..2 {
            for _col in 0..512 {
                let value = if line == 0 { 100 } else { 200 };
                pixels.push(value);
            }
        }

        let img = image_from_pixels(&pixels, DecoderMode::BinaryGrayscale);

        assert_eq!(img.size, [512, 2]);
        assert_eq!(img.pixels.len(), 1024);

        // Check first line
        for i in 0..512 {
            assert_eq!(img.pixels[i], egui::Color32::from_gray(100));
        }

        // Check second line
        for i in 512..1024 {
            assert_eq!(img.pixels[i], egui::Color32::from_gray(200));
        }
    }

    #[test]
    fn test_image_from_partial_line() {
        // Create pixels that don't fill complete lines
        let pixels: Vec<u8> = (0..100).collect(); // Only 100 pixels

        let img = image_from_pixels(&pixels, DecoderMode::BinaryGrayscale);

        assert_eq!(img.size, [512, 1]); // Still creates 1 line
        assert_eq!(img.pixels.len(), 512);

        // First 100 pixels should match input
        for (i, pixel) in img.pixels.iter().enumerate().take(100) {
            assert_eq!(*pixel, egui::Color32::from_gray(i as u8));
        }

        // Remaining pixels should be black (default)
        for pixel in img.pixels.iter().skip(100) {
            assert_eq!(*pixel, egui::Color32::BLACK);
        }
    }

    #[test]
    fn test_image_grayscale_boundaries() {
        let pixels = vec![0, 127, 128, 255];
        let img = image_from_pixels(&pixels, DecoderMode::BinaryGrayscale);

        assert_eq!(img.pixels[0], egui::Color32::from_gray(0)); // Black
        assert_eq!(img.pixels[1], egui::Color32::from_gray(127)); // Dark gray
        assert_eq!(img.pixels[2], egui::Color32::from_gray(128)); // Light gray
        assert_eq!(img.pixels[3], egui::Color32::from_gray(255)); // White
    }
}
