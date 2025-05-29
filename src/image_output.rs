use egui::ColorImage;

pub fn image_from_pixels(pixels: &[u8]) -> ColorImage {
    let width = 512;
    let height = pixels.len() / width;

    let mut img = ColorImage::new([width, height], egui::Color32::BLACK);
    for (i, p) in pixels.iter().enumerate() {
        img.pixels[i] = egui::Color32::from_gray(*p);
    }

    img
}
