use image::{RgbaImage, Rgba};

use crate::color_maps::ColorMap;

/// Draw a vertical color-bar legend graphic.
///
/// The top of the image corresponds to `max_value`, the bottom to `min_value`.
/// Every row is filled with a solid color sampled from the color map.
pub fn draw_legend_graphic(color_map: &ColorMap, width: u32, height: u32) -> RgbaImage {
    let mut image = RgbaImage::new(width, height);

    let min = color_map.get_min_value();
    let max = color_map.get_max_value();

    for y in 0..height {
        // y=0 → max_value, y=(height-1) → min_value
        let t = if height <= 1 {
            1.0
        } else {
            y as f64 / (height - 1) as f64
        };
        let value = max - t * (max - min);
        let color: Rgba<u8> = color_map.query(value);

        for x in 0..width {
            image.put_pixel(x, y, color);
        }
    }

    image
}
