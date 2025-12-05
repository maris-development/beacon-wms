use image::{codecs::{png::PngEncoder}, ImageEncoder, ImageError, Rgba, RgbaImage, ImageBuffer, GenericImageView, Pixel};
use image::imageops;

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

/// Applies a shadow or glow effect to colored features in an RGBA image.
///
/// This function modifies the image buffer in place by:
/// 1. Extracting the features (dots) into a separate layer.
/// 2. Blurring that layer to create the glow/shadow.
/// 3. Blending the blurred layer underneath the original features.
///
/// The effect is clipped to the image boundaries automatically by `imageops::blur`.
///
/// # Arguments
///
/// * `image`: A mutable reference to the `RgbaImage` buffer to be modified.
/// * `blur_radius`: The radius (in pixels) for the box blur filter. Higher values mean a wider glow.
/// * `effect_color`: The color to use for the shadow/glow (e.g., black for shadow, white for glow).
pub fn apply_shadow(
    image: &mut RgbaImage,
    iterations: u32,
    effect_color: Rgba<u8>,
) {
    let (width, height) = image.dimensions();

    // 1. Create the Shadow/Glow Layer (Extraction)
    // We create a buffer containing only the features using the desired effect color.
    let mut shadow_layer: RgbaImage = ImageBuffer::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let pixel = image.get_pixel(x, y);
            let alpha = pixel.channels()[3];

            if alpha > 0 {
                // Use the fixed effect color but keep the original feature's alpha
                let mut shadow_pixel = effect_color;
                shadow_pixel.channels_mut()[3] = alpha; 
                shadow_layer.put_pixel(x, y, shadow_pixel);
            } else {
                // Background is fully transparent
                shadow_layer.put_pixel(x, y, Rgba([0, 0, 0, 0]));
            }
        }
    }

    // 2. Apply a fast 3x3 convolution filter multiple times
    let kernel_spread: [f32; 9] = [
        1.0, 1.0, 1.0, 
        1.0, 1.0, 1.0, 
        1.0, 1.0, 1.0
    ];

    let mut blurred_shadow = shadow_layer;
    
    for _ in 0..iterations {
        // Repeatedly apply the 3x3 filter to widen the spread.
        // The result is assigned back to blurred_shadow for the next iteration.
        // This is a Box Filter approximation, which is fast.
        blurred_shadow = imageops::filter3x3(&blurred_shadow, &kernel_spread);
    }

    // 3. Blend Layers (Shadow/Glow layer is blended first, then original image)
    // We will blend the blurred shadow *into* the original image's buffer.
    
    // We use a temporary buffer for the result since we are modifying the original
    // 'image' buffer in place at the end.
    let mut blended_result: RgbaImage = ImageBuffer::new(width, height);

    for y in 0..height {
        for x in 0..width {
            let shadow_px = blurred_shadow.get_pixel(x, y);
            let original_px = image.get_pixel(x, y);

            // Simple Alpha Blending function: BOTTOM-OVER-TOP (P = T_alpha * T + (1 - T_alpha) * B)
            // Note: We need a complex blend function because we are combining two RGBA layers.
            // The imageops::overlay function is simpler but only handles opaque source over destination.
            
            // Define the blend logic for two RGBA pixels
            let blend_rgba = |p_bottom: Rgba<u8>, p_top: Rgba<u8>| -> Rgba<u8> {
                let a_b = p_bottom[3] as f32 / 255.0;
                let a_t = p_top[3] as f32 / 255.0;

                let a_out = a_t + a_b * (1.0 - a_t);

                if a_out == 0.0 {
                    return Rgba([0, 0, 0, 0]);
                }

                let blend_channel = |c_b: u8, c_t: u8| -> u8 {
                    let c_b_f = c_b as f32;
                    let c_t_f = c_t as f32;

                    // Blending formula for color channels
                    let c_out = (c_t_f * a_t + c_b_f * a_b * (1.0 - a_t)) / a_out;
                    c_out.round() as u8
                };

                Rgba([
                    blend_channel(p_bottom[0], p_top[0]),
                    blend_channel(p_bottom[1], p_top[1]),
                    blend_channel(p_bottom[2], p_top[2]),
                    (a_out * 255.0).round() as u8,
                ])
            };

            // 3a. Blend the blurred shadow onto a transparent background
            let transparent_bg = Rgba([0, 0, 0, 0]);
            let shadow_over_bg = blend_rgba(transparent_bg, *shadow_px); 
            
            // 3b. Blend the original feature *over* the shadow layer
            let final_px = blend_rgba(shadow_over_bg, *original_px);

            blended_result.put_pixel(x, y, final_px);
        }
    }
    
    // Replace the content of the original image buffer with the blended result
    *image = blended_result;
}