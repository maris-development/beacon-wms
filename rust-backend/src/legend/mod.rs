use image::{Rgba, RgbaImage};
use imageproc::drawing::{draw_text_mut, text_size};
use rusttype::{Font, Scale};

use crate::color_maps::ColorMap;
use crate::misc;

/// Blips between the minimum and the maximum when the request asks for no number.
pub const DEFAULT_BLIPS: u32 = 2;

/// Upper limit for the requested blips. The bar length lowers it further.
pub const MAX_BLIPS: u32 = 20;

const MARGIN: i32 = 4;
const TICK_LENGTH: i32 = 5;
const LABEL_GAP: i32 = 3;
const FONT_SIZE: f32 = 12.0;
const INK: Rgba<u8> = Rgba([26, 26, 26, 255]);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Orientation {
    Vertical,
    Horizontal,
}

impl Orientation {
    /// Read the orientation from the request. Unknown text gives a horizontal legend.
    pub fn parse(value: Option<&str>) -> Orientation {
        match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("vertical") | Some("v") => Orientation::Vertical,
            _ => Orientation::Horizontal,
        }
    }

    /// Bar size for a request without a width or a height.
    fn default_bar_size(&self) -> (u32, u32) {
        match self {
            Orientation::Vertical => (20, 200),
            Orientation::Horizontal => (200, 20),
        }
    }
}

/// Resolved legend request. `bar_width` and `bar_height` size the color bar.
/// The image grows around the bar to hold the blip labels.
pub struct LegendOptions {
    pub bar_width: u32,
    pub bar_height: u32,
    pub orientation: Orientation,
    pub blips: u32,
}

impl LegendOptions {
    pub fn new(
        width: Option<u32>,
        height: Option<u32>,
        orientation: Option<&str>,
        blips: Option<u32>,
    ) -> LegendOptions {
        let orientation = Orientation::parse(orientation);
        let (default_width, default_height) = orientation.default_bar_size();

        LegendOptions {
            bar_width: width.unwrap_or(default_width).clamp(1, 2000),
            bar_height: height.unwrap_or(default_height).clamp(1, 2000),
            orientation,
            blips: blips.unwrap_or(DEFAULT_BLIPS).min(MAX_BLIPS),
        }
    }

    /// Length of the bar along the color axis.
    fn bar_length(&self) -> i32 {
        match self.orientation {
            Orientation::Vertical => self.bar_height as i32,
            Orientation::Horizontal => self.bar_width as i32,
        }
    }
}

/// One blip: a tick mark plus its value label.
struct Blip {
    /// Position on the color axis. 0.0 is the minimum, 1.0 is the maximum.
    fraction: f64,
    label: String,
    label_width: i32,
    label_height: i32,
}

/// Draw a color-bar legend graphic with value blips.
///
/// A vertical bar runs from `max_value` at the top to `min_value` at the bottom.
/// A horizontal bar runs from `min_value` at the left to `max_value` at the right.
/// The blips mark both ends plus `options.blips` steps between them. The image
/// holds no labels if the system holds no font.
pub fn draw_legend_graphic(color_map: &ColorMap, options: &LegendOptions) -> RgbaImage {
    let font = misc::label_font_data().and_then(Font::try_from_bytes);
    let scale = Scale::uniform(FONT_SIZE);

    let blips = match font.as_ref() {
        Some(font) => fit_blips(color_map, options, font, scale),
        None => Vec::new(),
    };

    let bar_width = options.bar_width as i32;
    let bar_height = options.bar_height as i32;

    // Without labels the image holds the bar alone.
    if blips.is_empty() {
        let mut image = RgbaImage::new(options.bar_width, options.bar_height);
        draw_color_bar(&mut image, color_map, options, 0, 0);

        return image;
    }

    let label_width = blips.iter().map(|blip| blip.label_width).max().unwrap_or(0);
    let label_height = blips.iter().map(|blip| blip.label_height).max().unwrap_or(0);

    // A label at either end of the bar reaches past the bar itself.
    let (bar_x, bar_y, image_width, image_height) = match options.orientation {
        Orientation::Vertical => {
            let overflow = label_height / 2;
            let gutter = TICK_LENGTH + LABEL_GAP + label_width;

            (
                MARGIN,
                MARGIN + overflow,
                MARGIN + bar_width + gutter + MARGIN,
                MARGIN + overflow + bar_height + overflow + MARGIN,
            )
        }
        Orientation::Horizontal => {
            let overflow = label_width / 2;
            let gutter = TICK_LENGTH + LABEL_GAP + label_height;

            (
                MARGIN + overflow,
                MARGIN,
                MARGIN + overflow + bar_width + overflow + MARGIN,
                MARGIN + bar_height + gutter + MARGIN,
            )
        }
    };

    let mut image = RgbaImage::new(image_width as u32, image_height as u32);

    draw_color_bar(&mut image, color_map, options, bar_x, bar_y);
    draw_bar_border(&mut image, bar_x, bar_y, bar_width, bar_height);

    if let Some(font) = font.as_ref() {
        for blip in &blips {
            draw_blip(&mut image, blip, options, bar_x, bar_y, scale, font);
        }
    }

    image
}

/// Fill the bar with the colors of the map. Every step on the color axis uses
/// the same normalized position as the blips, so a label matches its color.
fn draw_color_bar(
    image: &mut RgbaImage,
    color_map: &ColorMap,
    options: &LegendOptions,
    bar_x: i32,
    bar_y: i32,
) {
    let bar_width = options.bar_width as i32;
    let bar_height = options.bar_height as i32;
    let length = options.bar_length();

    for step in 0..length {
        let fraction = if length <= 1 {
            1.0
        } else {
            step as f64 / (length - 1) as f64
        };

        let color = color_map.query(value_at(color_map, fraction));

        match options.orientation {
            Orientation::Vertical => {
                // The maximum sits on the top row.
                let y = bar_y + (bar_height - 1 - step);

                for x in bar_x..(bar_x + bar_width) {
                    put_pixel(image, x, y, color);
                }
            }
            Orientation::Horizontal => {
                let x = bar_x + step;

                for y in bar_y..(bar_y + bar_height) {
                    put_pixel(image, x, y, color);
                }
            }
        }
    }
}

/// Outline the bar, so a pale color keeps a visible edge.
fn draw_bar_border(image: &mut RgbaImage, bar_x: i32, bar_y: i32, width: i32, height: i32) {
    for x in bar_x..(bar_x + width) {
        put_pixel(image, x, bar_y, INK);
        put_pixel(image, x, bar_y + height - 1, INK);
    }

    for y in bar_y..(bar_y + height) {
        put_pixel(image, bar_x, y, INK);
        put_pixel(image, bar_x + width - 1, y, INK);
    }
}

/// Draw the tick mark of one blip and its label next to the bar.
fn draw_blip(
    image: &mut RgbaImage,
    blip: &Blip,
    options: &LegendOptions,
    bar_x: i32,
    bar_y: i32,
    scale: Scale,
    font: &Font,
) {
    let bar_width = options.bar_width as i32;
    let bar_height = options.bar_height as i32;
    let offset = ((options.bar_length() - 1) as f64 * blip.fraction).round() as i32;

    match options.orientation {
        Orientation::Vertical => {
            let y = bar_y + (bar_height - 1 - offset);
            let tick_start = bar_x + bar_width;

            for x in tick_start..(tick_start + TICK_LENGTH) {
                put_pixel(image, x, y, INK);
            }

            let label_x = tick_start + TICK_LENGTH + LABEL_GAP;
            let label_y = y - blip.label_height / 2;

            draw_text_mut(image, INK, label_x, label_y, scale, font, &blip.label);
        }
        Orientation::Horizontal => {
            let x = bar_x + offset;
            let tick_start = bar_y + bar_height;

            for y in tick_start..(tick_start + TICK_LENGTH) {
                put_pixel(image, x, y, INK);
            }

            let label_x = x - blip.label_width / 2;
            let label_y = tick_start + TICK_LENGTH + LABEL_GAP;

            draw_text_mut(image, INK, label_x, label_y, scale, font, &blip.label);
        }
    }
}

/// Take the requested blips, and drop one at a time until the labels keep apart.
fn fit_blips(color_map: &ColorMap, options: &LegendOptions, font: &Font, scale: Scale) -> Vec<Blip> {
    let length = options.bar_length();
    let mut count = options.blips as i32 + 2;

    while count > 2 {
        let blips = build_blips(color_map, count as u32, font, scale);

        if labels_fit(&blips, length, options.orientation) {
            return blips;
        }

        count -= 1;
    }

    build_blips(color_map, 2, font, scale)
}

/// Check that the space between two blips holds a label.
fn labels_fit(blips: &[Blip], length: i32, orientation: Orientation) -> bool {
    let needed = blips
        .iter()
        .map(|blip| match orientation {
            Orientation::Vertical => blip.label_height + 4,
            Orientation::Horizontal => blip.label_width + 6,
        })
        .max()
        .unwrap_or(0);

    let steps = (blips.len() as i32 - 1).max(1);

    length / steps >= needed
}

/// Build `count` blips from the minimum to the maximum, both ends included.
fn build_blips(color_map: &ColorMap, count: u32, font: &Font, scale: Scale) -> Vec<Blip> {
    let count = count.max(2);
    let span = (color_map.get_max_value() - color_map.get_min_value()).abs();
    let mut blips = Vec::with_capacity(count as usize);

    for step in 0..count {
        let fraction = step as f64 / (count - 1) as f64;
        let value = value_at(color_map, fraction);
        let label = format_value(value, span, uses_log(color_map));
        let (label_width, label_height) = text_size(scale, font, &label);

        blips.push(Blip {
            fraction,
            label,
            label_width,
            label_height,
        });
    }

    blips
}

/// A logarithmic color map needs a positive range. `ColorMap::query` holds the
/// same rule, so both fall back to a linear scale together.
fn uses_log(color_map: &ColorMap) -> bool {
    color_map.is_log() && color_map.get_min_value() > 0.0 && color_map.get_max_value() > 0.0
}

/// Value at a normalized position on the color axis. A logarithmic map steps by
/// a constant factor, which keeps the blips even on the bar.
fn value_at(color_map: &ColorMap, fraction: f64) -> f64 {
    let min = color_map.get_min_value();
    let max = color_map.get_max_value();

    if uses_log(color_map) {
        let log_min = min.log10();
        let log_max = max.log10();

        return 10f64.powf(log_min + fraction * (log_max - log_min));
    }

    min + fraction * (max - min)
}

/// Format a blip label. The span of the color scale sets the decimal count.
fn format_value(value: f64, span: f64, log: bool) -> String {
    let magnitude = value.abs();

    if (magnitude > 0.0 && magnitude < 0.001) || magnitude >= 100000.0 {
        return format!("{:.1e}", value);
    }

    if log {
        return format_log_value(value);
    }

    let decimals = if span >= 100.0 {
        0
    } else if span >= 10.0 {
        1
    } else if span >= 1.0 {
        2
    } else {
        3
    };

    let text = format!("{:.*}", decimals, value);

    // Drop the sign of a value that rounds to zero.
    if text.starts_with('-') && text[1..].chars().all(|c| c == '0' || c == '.') {
        return text[1..].to_string();
    }

    text
}

/// Format a label on a logarithmic scale. Every blip holds a different power of
/// ten, so the decimal count follows the value itself.
fn format_log_value(value: f64) -> String {
    let magnitude = value.abs();

    let decimals = if magnitude >= 10.0 {
        0
    } else if magnitude >= 1.0 {
        1
    } else {
        (1.0 - magnitude.log10().floor()).clamp(0.0, 6.0) as usize
    };

    let text = format!("{:.*}", decimals, value);

    // A power of ten keeps no decimals to show.
    if text.contains('.') {
        return text.trim_end_matches('0').trim_end_matches('.').to_string();
    }

    text
}

fn put_pixel(image: &mut RgbaImage, x: i32, y: i32, color: Rgba<u8>) {
    if x < 0 || y < 0 || x >= image.width() as i32 || y >= image.height() as i32 {
        return;
    }

    image.put_pixel(x as u32, y as u32, color);
}
