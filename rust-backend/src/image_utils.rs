use image::{codecs::{png::PngEncoder}, ImageEncoder, ImageError, Rgba, RgbaImage};

/// Encodes an RgbaImage into PNG format and writes the result to the provided output vector.
/// The image is encoded with 8 bits per channel.
pub fn rgba_image_to_png(image: &RgbaImage, output_vec: &mut Vec<u8>) -> Result<(), ImageError> {
    let mut buff = std::io::Cursor::new(output_vec);
    PngEncoder::new(&mut buff).write_image(
        image.as_raw(),
        image.width(),
        image.height(),
        image::ColorType::Rgba8,
    )
}

pub fn unpack_rgba(packed: u32) -> Rgba<u8> {
    let r = ((packed >> 24) & 0xFF) as u8;
    let g = ((packed >> 16) & 0xFF) as u8;
    let b = ((packed >> 8) & 0xFF) as u8;
    let a = (packed & 0xFF) as u8;

    Rgba([r, g, b, a])
}

pub fn create_rgba_image(width: u32, height: u32) -> RgbaImage {
    // Create a new RgbaImage with the specified dimensions.
    // Each pixel is initialized to transparent (0, 0, 0, 0).
    let mut img = RgbaImage::new(width, height);
    
    // Optionally, fill the image with a specific color.
    // For example, to fill with a transparent white background:
    for pixel in img.pixels_mut() {
        *pixel = Rgba([255, 255, 255, 0]); // Red, Green, Blue, Alpha
    }
    
    img
}

/// Simple linear interpolation in RGB
pub fn linear_color_interpolation(color_lower: &Rgba<u8>, color_upper: &Rgba<u8>, interpolation_fraction: f64) -> Rgba<u8> {
    let interpolate_channel = |channel_lower: u8, channel_upper: u8| -> u8 {
        ((channel_lower as f64)
            + interpolation_fraction * ((channel_upper as f64) - (channel_lower as f64)))
            .round()
            .clamp(0.0, 255.0) as u8
    };

    Rgba([
        interpolate_channel(color_lower[0], color_upper[0]),
        interpolate_channel(color_lower[1], color_upper[1]),
        interpolate_channel(color_lower[2], color_upper[2]),
        255,
    ])
}

/// Interpolate two colors in CIE Lab space
pub fn lab_color_interpolation(lab_color_lower: (f64, f64, f64), lab_color_upper: (f64, f64, f64), interpolation_fraction: f64) -> Rgba<u8> {
    // Convert both colors to Lab

    // Linear interpolation in Lab
    let interpolated_l = lab_color_lower.0 + interpolation_fraction * (lab_color_upper.0 - lab_color_lower.0);
    let interpolated_a = lab_color_lower.1 + interpolation_fraction * (lab_color_upper.1 - lab_color_lower.1);
    let interpolated_b = lab_color_lower.2 + interpolation_fraction * (lab_color_upper.2 - lab_color_lower.2);

    // Convert back to RGB
    lab_to_rgb((interpolated_l, interpolated_a, interpolated_b))
}

/// Convert sRGB (0–255) to Lab
pub fn rgb_to_lab(color: &Rgba<u8>) -> (f64, f64, f64) {
    let r = srgb_channel_to_linear(color[0]);
    let g = srgb_channel_to_linear(color[1]);
    let b = srgb_channel_to_linear(color[2]);

    // Convert linear RGB to XYZ
    let x = r * 0.4124564 + g * 0.3575761 + b * 0.1804375;
    let y = r * 0.2126729 + g * 0.7151522 + b * 0.0721750;
    let z = r * 0.0193339 + g * 0.1191920 + b * 0.9503041;

    // Normalize for D65 white point
    let x = x / 0.95047;
    let y = y / 1.00000;
    let z = z / 1.08883;

    // Convert XYZ to Lab
    let fx = lab_f(x);
    let fy = lab_f(y);
    let fz = lab_f(z);

    let l = (116.0 * fy) - 16.0;
    let a = 500.0 * (fx - fy);
    let b = 200.0 * (fy - fz);

    (l, a, b)
}

/// Convert Lab back to sRGB (0–255)
pub fn lab_to_rgb(lab: (f64, f64, f64)) -> Rgba<u8> {
    let (l, a, b) = lab;

    let fy = (l + 16.0) / 116.0;
    let fx = a / 500.0 + fy;
    let fz = fy - b / 200.0;

    let x = lab_f_inv(fx) * 0.95047;
    let y = lab_f_inv(fy) * 1.00000;
    let z = lab_f_inv(fz) * 1.08883;

    // Convert XYZ to linear RGB
    let r_linear = x * 3.2404542 + y * -1.5371385 + z * -0.4985314;
    let g_linear = x * -0.9692660 + y * 1.8760108 + z * 0.0415560;
    let b_linear = x * 0.0556434 + y * -0.2040259 + z * 1.0572252;

    // Linear RGB → sRGB (0–255)
    let r = linear_channel_to_srgb(r_linear);
    let g = linear_channel_to_srgb(g_linear);
    let b = linear_channel_to_srgb(b_linear);

    Rgba([r, g, b, 255])
}

/// Gamma correction: sRGB → linear
fn srgb_channel_to_linear(value: u8) -> f64 {
    let v = value as f64 / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Gamma correction: linear → sRGB
fn linear_channel_to_srgb(value: f64) -> u8 {
    let v = if value <= 0.0031308 {
        12.92 * value
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Lab helper f(t)
fn lab_f(t: f64) -> f64 {
    if t > 0.008856 {
        t.powf(1.0 / 3.0)
    } else {
        (7.787 * t) + (16.0 / 116.0)
    }
}

/// Lab helper f⁻¹(t)
fn lab_f_inv(t: f64) -> f64 {
    let t3 = t * t * t;
    if t3 > 0.008856 {
        t3
    } else {
        (t - 16.0 / 116.0) / 7.787
    }
}