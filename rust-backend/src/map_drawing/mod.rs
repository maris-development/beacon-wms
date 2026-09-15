use arrow::array::RecordBatch;
use boundingbox::BoundingBox;
use image::{GenericImage, Pixel, Rgba, RgbaImage};
use lazy_static::lazy_static;
use log;
use std::sync::Arc;

use crate::cache_engine::{self, DecodeGates};
use crate::color_maps::ColorMap;
use crate::data_utils::{self};
use crate::errors::MapError;
use crate::point_index::{LayerPoints, PointIndexCache};
use crate::request_profiling::RequestProfiling;
use crate::{boundingbox, image_utils, misc};
use std::fs::File;

lazy_static! {
    /// Built pyramids, one per layer file, generation and CRS.
    pub static ref POINT_INDEX_CACHE: PointIndexCache = PointIndexCache::from_env();

    /// Keeps concurrent tiles of the same layer and CRS to one parquet decode.
    pub static ref DECODE_GATES: DecodeGates = DecodeGates::new();
}


pub const LONGITUDE_COLUMN: &'static str = "longitude";
pub const LATITUDE_COLUMN: &'static str = "latitude";
pub const VALUE_COLUMN: &'static str = "value";

pub const COLOR_ONLY_ZOOMLEVEL: u32 = 6;
pub const SMALL_ICON_ZOOMLEVEL: u32 = 8;

/// What one layer cost on one tile.
#[derive(Debug, Default, Clone, Copy)]
pub struct DrawStats {
    /// Points the index handed to the draw loop.
    pub scanned: usize,
    /// Points that claimed a pixel and drew an icon.
    pub drawn: usize,
}

/// Draw map on image
///
pub fn get_map(
    image: &mut RgbaImage,
    bounding_box: BoundingBox,
    color_map: ColorMap,
    crs: &str,
    layer_filepath: String,
    file: File,
    // Version of the dataset file. A refresh increases it, so the cache entries of
    // the old file become unreachable and drop out of the LRU cache.
    generation: u64,
    icon_shape: &str,
    profiling: &mut RequestProfiling
) -> Result<DrawStats, MapError> {

    match file.metadata() {
        Ok(metadata) => {
            if metadata.len() == 0 {
                return Ok(DrawStats::default());
            }
        }
        Err(e) => {
            log::error!("Failed to get file metadata ({}): {}", layer_filepath,e);
            return Err(MapError::Error(e.to_string()));
        }
    }

    let source_projection_code = "EPSG:4326";
    let target_projection_code = crs;

    // Check Bounding Box
    if !bounding_box.is_correct() {
        log::error!("Bounding box is not correct!");
        return Err(MapError::BoundingBoxError(bounding_box));
    }

    let reprojected_bbox = bounding_box.reproject(target_projection_code).map_err(|e| {
        log::error!(
            "Could not reproject bounding box: {}, target projection: {} \n {:?}",
            e,
            target_projection_code,
            bounding_box
        );
        MapError::Error(e)
    })?;

    let degree_per_pixel = bounding_box.get_width_degrees() / image.width() as f64;
    let zoom = misc::degrees_per_pixel_to_zoom(degree_per_pixel, None);
    let point_radius = misc::calculate_point_radius(zoom, 5.0, 40.0);
    let (img_w, img_h) = image.dimensions();

    // An icon reaches point_radius + 1 pixels past its centre. A point further out than
    // that cannot colour a pixel of this tile, so the cull box needs no more margin.
    let reach = (point_radius + 1) as f64;
    let margin_x = reach * reprojected_bbox.get_width() / img_w as f64;
    let margin_y = reach * reprojected_bbox.get_height() / img_h as f64;

    let bbox_min_x = reprojected_bbox.get_min_x() - margin_x;
    let bbox_max_x = reprojected_bbox.get_max_x() + margin_x;
    let bbox_min_y = reprojected_bbox.get_min_y() - margin_y;
    let bbox_max_y = reprojected_bbox.get_max_y() + margin_y;

    let mut drawn_pixel_grid: Vec<bool> = vec![false; (img_w as usize) * (img_h as usize)];
    let mut drawn_count: usize = 0;

    // Shared color lookup table for O(1) color mapping
    let color_lut = color_map.lut();
    let lut_size = color_lut.len();
    let cm_min = color_map.get_min_value();
    let cm_range = color_map.get_max_value() - cm_min;

    // Cache keys carry the generation, so a refreshed file never reuses old points
    let cache_key_base = format!("{}#{}_{}", layer_filepath, generation, target_projection_code);

    let points = resolve_points(
        &cache_key_base,
        &layer_filepath,
        file,
        source_projection_code,
        target_projection_code,
        &reprojected_bbox,
        profiling,
    )?;

    let scanned = points.for_each_in_bbox(zoom, bbox_min_x, bbox_min_y, bbox_max_x, bbox_max_y, |x, y, value| {
        let offset =
            misc::coordinates_to_pixel_offset(&reprojected_bbox, (img_w, img_h), (x, y));

        // Pixel-grid deduplication: skip if this pixel was already drawn
        if offset.0 >= 0 && offset.0 < img_w as i32 && offset.1 >= 0 && offset.1 < img_h as i32 {
            let grid_idx = offset.1 as usize * img_w as usize + offset.0 as usize;
            if drawn_pixel_grid[grid_idx] {
                return;
            }
            drawn_pixel_grid[grid_idx] = true;
            drawn_count += 1;
        }

        // O(1) LUT lookup, on a point that survived the cull
        let normalized = ((value - cm_min) / cm_range).clamp(0.0, 1.0);
        let color = image_utils::unpack_rgba(color_lut[(normalized * (lut_size - 1) as f64) as usize]);

        let draw_result: Result<(), MapError> = match icon_shape {
            "circle" => draw_circle(image, offset, color, Some(point_radius)),
            "circle_outlined" => draw_circle_outlined(image, offset, color, point_radius, Some((Rgba([0, 0, 0, 254]), 1))),
            "square" => draw_square(image, offset, color, Some(point_radius)),
            "square_outlined" => draw_square_outlined(image, offset, color, Some(point_radius), Some((Rgba([0, 0, 0, 254]), 1))),
            "plus" => draw_plus(image, offset, color, Some(point_radius)),
            "plus_outlined" => draw_plus_outlined(image, offset, color, Some(point_radius), Some((Rgba([0, 0, 0, 254]), 1))),
            "triangle" => draw_triangle(image, offset, color, Some(point_radius)),
            "triangle_outlined" => draw_triangle_outlined(image, offset, color, Some(point_radius), Some((Rgba([0, 0, 0, 254]), 1))),
            _ => draw_circle(image, offset, color, Some(point_radius)),
        };

        if draw_result.is_err() {
            log::error!("Could not draw image: {:?}", draw_result.err().unwrap());
        }
    });

    profiling.mark("done drawing");

    // misc::print_bbox_on_image(&reprojected_bbox, image); //debugging

    return Ok(DrawStats {
        scanned,
        drawn: drawn_count,
    });
}

/// Pyramid of a layer in one CRS, from the cache or freshly built.
///
/// Tiles of one screen share the pyramid. The gate sends one thread to the file and
/// lets the rest read the cache after it. A waiter must check the cache again, because
/// the entry can be evicted between the two reads.
fn resolve_points(
    cache_key_base: &str,
    layer_filepath: &str,
    file: File,
    source_projection_code: &str,
    target_projection_code: &str,
    reprojected_bbox: &BoundingBox,
    profiling: &mut RequestProfiling,
) -> Result<Arc<LayerPoints>, MapError> {
    if let Some(points) = POINT_INDEX_CACHE.get(cache_key_base) {
        profiling.mark("point index cached");

        return Ok(points);
    }

    let gate = DECODE_GATES.acquire(cache_key_base);
    let guard = cache_engine::lock_gate(&gate);

    profiling.mark("decode gate acquired");

    let result = match POINT_INDEX_CACHE.get(cache_key_base) {
        Some(points) => {
            profiling.mark("point index built by another request");

            Ok(points)
        }
        None => build_points(
            layer_filepath,
            file,
            source_projection_code,
            target_projection_code,
            reprojected_bbox,
            profiling,
        )
        .map(|points| {
            let points = Arc::new(points);
            POINT_INDEX_CACHE.insert(cache_key_base.to_string(), Arc::clone(&points));

            log::info!(
                "Point index cache now holds {:.0} MB",
                POINT_INDEX_CACHE.bytes() as f64 / 1_048_576.0
            );

            points
        }),
    };

    drop(guard);
    DECODE_GATES.release(cache_key_base, gate);

    result
}

/// Read the layer file, reproject every batch and build the pyramid.
///
/// Call this under the decode gate of the same cache key.
fn build_points(
    layer_filepath: &str,
    file: File,
    source_projection_code: &str,
    target_projection_code: &str,
    reprojected_bbox: &BoundingBox,
    profiling: &mut RequestProfiling,
) -> Result<LayerPoints, MapError> {
    // Drawing needs three columns. A projection skips the decode of every other column.
    let reader = data_utils::parquet_reader(
        layer_filepath,
        file,
        Some(&[LONGITUDE_COLUMN, LATITUDE_COLUMN, VALUE_COLUMN]),
    )?;

    profiling.mark("parquet reader created");

    let mut batches: Vec<RecordBatch> = Vec::new();

    for batch in reader {
        let batch = match batch {
            Ok(batch) => cache_engine::reproject_batch(
                source_projection_code,
                target_projection_code,
                batch,
            )?,
            Err(e) => {
                log::error!("Error reading batch: {}", e);
                return Err(MapError::Error(format!("Error reading batch: {}", e)));
            }
        };

        batches.push(batch);
    }

    profiling.mark("layer read and reprojected");

    // The level grids follow the CRS extent, so a level cell equals a screen pixel.
    let max_bounds = reprojected_bbox.get_max_bounds();
    let world = (
        max_bounds.get_min_x(),
        max_bounds.get_min_y(),
        max_bounds.get_max_x(),
        max_bounds.get_max_y(),
    );

    let points = LayerPoints::build(&batches, world)?;

    log::info!(
        "Built point index for {} in {}: {} points, {} levels, {:.0} MB",
        layer_filepath,
        target_projection_code,
        points.len(),
        points.level_count(),
        points.bytes() as f64 / 1_048_576.0,
    );

    profiling.mark("point index built");

    Ok(points)
}

fn draw_pixel(image: &mut RgbaImage, x: i32, y: i32, color: Rgba<u8>) {
    if !misc::inside_image(image, (x, y)) {
        return;
    }

    if color[3] == 255 {
        // FILL: always overwrite
        unsafe {
            image.unsafe_put_pixel(x as u32, y as u32, color);
        }
        return;
    }

    // OUTLINE (any alpha < 255)
    let dst = image.get_pixel(x as u32, y as u32);

    // only draw on empty/background pixels
    if dst[3] == 0 {
        unsafe {
            image.unsafe_put_pixel(x as u32, y as u32, color);
        }
    }
}

fn draw_circle(image: &mut RgbaImage, point: (i32, i32), color: Rgba<u8>, radius: Option<i32>) -> Result<(), MapError> {
    let radius = radius.unwrap_or(2);
    let r_sq = radius * radius;

    for x in -radius..=radius {
        for y in -radius..=radius {
            if x * x + y * y <= r_sq {
                let px = point.0 + x;
                let py = point.1 + y;

                draw_pixel(image, px, py, color);
            }
        }
    }

    Ok(())
}

fn draw_circle_outlined(
    image: &mut RgbaImage,
    point: (i32, i32),
    fill: Rgba<u8>,
    radius: i32,
    outline: Option<(Rgba<u8>, i32)>, // (colour, thickness)
) -> Result<(), MapError> {

    let r_sq = radius * radius;

    // --- 1. outline pass (draw first, so fill sits on top) ---
    if let Some((outline_color, thickness)) = outline {
        let outer_r = radius + thickness;
        let outer_r_sq = outer_r * outer_r;

        for x in -outer_r..=outer_r {
            for y in -outer_r..=outer_r {
                let d = x * x + y * y;

                if d <= outer_r_sq && d > r_sq {
                    let px = point.0 + x;
                    let py = point.1 + y;

                    draw_pixel(image, px, py, outline_color);
                }
            }
        }
    }

    // --- 2. fill pass ---
    for x in -radius..=radius {
        for y in -radius..=radius {
            if x * x + y * y <= r_sq {
                let px = point.0 + x;
                let py = point.1 + y;

                draw_pixel(image, px, py, fill);
            }
        }
    }

    Ok(())
}

fn draw_square(
    image: &mut RgbaImage,
    point: (i32, i32),
    color: Rgba<u8>,
    half_size: Option<i32>,
) -> Result<(), MapError> {

    let half = half_size.unwrap_or(2);

    for x in -half..=half {
        for y in -half..=half {
            let px = point.0 + x;
            let py = point.1 + y;

            draw_pixel(image, px, py, color);
        }
    }

    Ok(())
}

fn draw_square_outlined(
    image: &mut RgbaImage,
    point: (i32, i32),
    color: Rgba<u8>,
    half_size: Option<i32>,
    outline: Option<(Rgba<u8>, i32)>,
) -> Result<(), MapError> {

    let half = half_size.unwrap_or(2);

    let (outline_color, thickness) = outline.unzip();

    let t = thickness.unwrap_or(0);

    for x in -half..=half {
        for y in -half..=half {

            let is_border =
                t > 0 &&
                (x.abs() > half - t || y.abs() > half - t);

            let px = point.0 + x;
            let py = point.1 + y;

            let color_to_draw = if is_border {
                outline_color.unwrap()
            } else {
                color
            };

            draw_pixel(image, px, py, color_to_draw);
        }
    }

    Ok(())
}

//  fn draw_plus(image: &mut RgbaImage, point: (i32, i32), color: Rgba<u8>, half_size: Option<i32>) -> Result<(), MapError> {

//     let half_size = half_size.unwrap_or(2);
//     let mut thickness = (half_size / 2).max(1); // Ensure a minimum thickness of 1 pixel
//     while thickness % 2 == 0 {
//         // Make sure thickness is odd for symmetry
//         thickness += 1;
//     }
//     // We can draw both the vertical and horizontal lines in one loop
//     // to be more efficient than iterating the whole bounding box (O(N) vs O(N^2)).

//     for i in -half_size..=half_size {
//         // Horizontal line points: (center_x + i, center_y)
//         for t in -(thickness / 2)..=(thickness / 2) {
//             let h_x = point.0 + i;
//             let h_y = point.1 + t;
//             if misc::inside_image(image, (h_x, h_y)) {
//                 // SAFETY: bounds verified by inside_image above
//                 unsafe { image.unsafe_put_pixel(h_x as u32, h_y as u32, color); }
//             }
//         }

    
//         // Vertical line points: (center_x, center_y + i)
//         for t in -(thickness / 2)..=(thickness / 2) {
//             let v_x = point.0 + t;
//             let v_y = point.1 + i;
//             if misc::inside_image(image, (v_x, v_y)) {
//                 // SAFETY: bounds verified by inside_image above
//                 unsafe { image.unsafe_put_pixel(v_x as u32, v_y as u32, color); }
//             }
//         }
//     }

//     Ok(())
// }

fn draw_plus(
    image: &mut RgbaImage,
    point: (i32, i32),
    color: Rgba<u8>,
    half_size: Option<i32>,
) -> Result<(), MapError> {

    let half = half_size.unwrap_or(2);
    let thickness = (half / 2).max(1);

    for i in -half..=half {

        // horizontal bar
        for t in -(thickness / 2)..=(thickness / 2) {
            let px = point.0 + i;
            let py = point.1 + t;

            draw_pixel(image, px, py, color);
        }

        // vertical bar
        for t in -(thickness / 2)..=(thickness / 2) {
            let px = point.0 + t;
            let py = point.1 + i;

            draw_pixel(image, px, py, color);
        }
    }

    Ok(())
}

fn draw_plus_outlined(
    image: &mut RgbaImage,
    point: (i32, i32),
    color: Rgba<u8>,
    half_size: Option<i32>,
    outline: Option<(Rgba<u8>, i32)>,
) -> Result<(), MapError> {

    let half = half_size.unwrap_or(2);
    let t = outline.as_ref().map(|(_, t)| *t).unwrap_or(0);

    let outline_color = outline.map(|(c, _)| c);

    for i in -half..=half {

        for thickness in -(half / 2 + t)..=(half / 2 + t) {

            let is_outline = t > 0 &&
                (i.abs() > half - t || thickness.abs() > (half / 2));

            let px_h = point.0 + i;
            let py_h = point.1 + thickness;

            let px_v = point.0 + thickness;
            let py_v = point.1 + i;

            let col = if is_outline {
                outline_color.unwrap()
            } else {
                color
            };

            draw_pixel(image, px_h, py_h, col);
            draw_pixel(image, px_v, py_v, col);
        }
    }

    Ok(())
}


// fn draw_triangle(image: &mut RgbaImage, point: (i32, i32), color: Rgba<u8>, half_size: Option<i32>) -> Result<(), MapError> {
//     let half_size = half_size.unwrap_or(2);

//     for y_offset in -half_size..=half_size {
//         // We calculate the maximum allowed x-width for the current row.
//         // As y goes down (increases), the triangle gets wider.
//         // The division by 2 ensures a standard triangle slope (approx 60 degrees) 
//         // rather than a very wide 90-degree slope.
//         let max_x_width = (y_offset + half_size) / 2;

//         for x_offset in -max_x_width..=max_x_width {
//             let px = point.0 + x_offset;
//             let py = point.1 + y_offset;

//             if misc::inside_image(image, (px, py)) {
//                 // SAFETY: bounds verified by inside_image above
//                 unsafe { image.unsafe_put_pixel(px as u32, py as u32, color); }
//             }
//         }
//     }

//     Ok(())
// }

fn draw_triangle(
    image: &mut RgbaImage,
    point: (i32, i32),
    color: Rgba<u8>,
    half_size: Option<i32>,
) -> Result<(), MapError> {

    let half = half_size.unwrap_or(2);

    for y_offset in -half..=half {

        let max_x_width = (y_offset + half) / 2;

        for x_offset in -max_x_width..=max_x_width {

            let px = point.0 + x_offset;
            let py = point.1 + y_offset;

            draw_pixel(image, px, py, color);
        }
    }

    Ok(())
}

fn draw_triangle_outlined(
    image: &mut RgbaImage,
    point: (i32, i32),
    fill: Rgba<u8>,
    half_size: Option<i32>,
    outline: Option<(Rgba<u8>, i32)>,
) -> Result<(), MapError> {

    let half = half_size.unwrap_or(2);

    let (outline_color, t) = if let Some(o) = outline {
        o
    } else {
        (Rgba([0, 0, 0, 0]), 0) // dummy
    };

    let outer = half + t;

    // --- OUTLINE PASS ---
    if t > 0 {
        for y in -outer..=outer {

            let outer_max_x = (y + outer) / 2;
            let inner_max_x = (y.abs() <= half).then(|| (y + half) / 2);

            for x in -outer_max_x..=outer_max_x {

                // OUTSIDE inner triangle but inside outer triangle
                let in_outer =
                    x.abs() <= outer_max_x;

                let in_inner = if let Some(ix) = inner_max_x {
                    x.abs() <= ix
                } else {
                    false
                };

                if in_outer && !in_inner {
                    let px = point.0 + x;
                    let py = point.1 + y;

                    draw_pixel(image, px, py, outline_color);
                }
            }
        }
    }

    // --- FILL PASS ---
    for y in -half..=half {

        let max_x = (y + half) / 2;

        for x in -max_x..=max_x {
            let px = point.0 + x;
            let py = point.1 + y;

            draw_pixel(image, px, py, fill);
        }
    }

    Ok(())
}

























/// Draw image on image
///
/// # Arguments
/// * `image` - Image to draw on
/// * `point` - Point to draw image on (center)
/// * `icon` - Image to draw
/// * `color` - Alternative color to draw instead of image incase the image becomes to small because of the zoomlevel
/// * `zoom` - Zoom level, used to calculate the size of the image to draw
#[allow(dead_code)]
fn draw_image(
    image: &mut RgbaImage,
    point: (i32, i32),
    icon: &RgbaImage,
    color: &Rgba<u8>,
    zoom: u32,
) -> Result<(), MapError> {
    // log::info!("dimensions: {:?}, zoom: {}, point: {:?}", icon_dimensions, zoom, point);

    let icon_dimensions: (u32, u32) = icon.dimensions();
    let half_width = icon_dimensions.0 / 2;
    let half_height = icon_dimensions.1 / 2;

    for x in 0..icon_dimensions.0 {
        for y in 0..icon_dimensions.1 {
            let mut pixel_color = icon.get_pixel(x, y).clone();

            let pixel_alpha = pixel_color
                .0
                .get(3)
                .ok_or(MapError::Error("Unable to read pixel alpha".to_string()))?
                .clone();

            if pixel_alpha > 0 {
                let x = point.0 + x as i32 - half_width as i32;
                let y = point.1 + y as i32 - half_height as i32;
                // log::info!("({}, {})", x, y);

                if misc::inside_image(image, (x, y)) {
                    if zoom < COLOR_ONLY_ZOOMLEVEL {
                        pixel_color = color.clone(); //forget about the border, and just draw the circle in one solid color.
                    }

                    if pixel_alpha < 255 {
                        let current_color = image.get_pixel(x as u32, y as u32);
                        pixel_color.blend(current_color);
                    }

                    image.put_pixel(x as u32, y as u32, pixel_color);
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::misc::inside_image;

    use super::*;

    #[test]
    fn test_mercator_projection() {
        let source_projection_code = "EPSG:4326";
        let target_projection_code = "EPSG:3857"; //web mercator

        let mut bbox = BoundingBox::new(-180.0, -90.0, 180.0, 90.0, "EPSG:4326");
        let mut image = image::open("../assets/world.png").unwrap().into_rgba8();

        //reproject bbox if needed:
        if bbox.get_projection_code() != target_projection_code {
            bbox = bbox.reproject(&target_projection_code).unwrap();
        }

        let red: Rgba<u8> = Rgba([255u8, 0u8, 0u8, 255u8]);
        let bbox_margin = Some(bbox.get_width() * 0.1);

        for lng in (-180..180).step_by(1) {
            for lat in (-90..90).step_by(1) {
                let mut coordinates = (lng as f64, lat as f64);

                misc::transform_coordinates(
                    source_projection_code,
                    target_projection_code,
                    &mut coordinates,
                )
                .unwrap();

                if bbox.in_bbox(coordinates, bbox_margin) {
                    let offset = misc::coordinates_to_pixel_offset(
                        &bbox,
                        (image.width(), image.height()),
                        (coordinates.0, coordinates.1),
                    );

                    // draw_circle(&mut image, offset, 1, Some(red));
                    if inside_image(&image, offset) {
                        image.put_pixel(offset.0 as u32, offset.1 as u32, red);
                    }
                    // image.put_pixel(offset.0 as u32, offset.1 as u32, red);
                }
            }
        }

        image
            .save("../assets/test_mercator_projection.png")
            .unwrap();
    }
}
