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

    // Every point of this tile draws the same icon, so its runs are built once.
    let icon_stamp = IconStamp::new(icon_shape, point_radius);

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

        match &icon_stamp {
            Some(stamp) => stamp.draw(image, offset, color),
            None => {
                let result = draw_plus_outlined(
                    image,
                    offset,
                    color,
                    Some(point_radius),
                    Some((OUTLINE_COLOR, OUTLINE_WIDTH)),
                );

                if let Err(e) = result {
                    log::error!("Could not draw image: {:?}", e);
                }
            }
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

/// Colour and width of the icon outline.
const OUTLINE_COLOR: Rgba<u8> = Rgba([0, 0, 0, 254]);
const OUTLINE_WIDTH: i32 = 1;

/// One icon, as a pixel run per row.
///
/// Every point of a tile draws the same shape at the same radius, so the runs are built
/// once. A run then replaces the distance test, the bounds check and the pixel call that
/// a per pixel loop repeats for every pixel of every icon.
struct IconStamp {
    /// Row offset of the first entry. The rest follow in steps of one.
    top: i32,
    /// Half width of the fill run and of the outline run, per row. `-1` draws no run.
    rows: Vec<(i32, i32)>,
    outlined: bool,
}

impl IconStamp {
    /// Runs of `shape` at `radius`. An unknown shape gives a circle.
    ///
    /// `None` means the shape has no run form. The caller draws it pixel by pixel.
    fn new(shape: &str, radius: i32) -> Option<IconStamp> {
        let stamp = match shape {
            "circle_outlined" => IconStamp::disc(radius, OUTLINE_WIDTH),
            "square" => IconStamp::square(radius, 0),
            "square_outlined" => IconStamp::square(radius, OUTLINE_WIDTH),
            "plus" => IconStamp::plus(radius),
            "triangle" => IconStamp::triangle(radius, 0),
            "triangle_outlined" => IconStamp::triangle(radius, OUTLINE_WIDTH),
            "plus_outlined" => return None,
            _ => IconStamp::disc(radius, 0),
        };

        Some(stamp)
    }

    /// Widest pixel offset of a disc of `radius` on row `dy`.
    fn disc_half_width(radius: i32, dy: i32) -> i32 {
        (radius * radius - dy * dy).isqrt()
    }

    /// The outline of a disc sits outside the radius.
    fn disc(radius: i32, outline: i32) -> IconStamp {
        let outer = radius + outline;

        let rows = (-outer..=outer)
            .map(|dy| {
                let fill = if dy.abs() <= radius {
                    IconStamp::disc_half_width(radius, dy)
                } else {
                    -1
                };

                (fill, IconStamp::disc_half_width(outer, dy))
            })
            .collect();

        IconStamp { top: -outer, rows, outlined: outline > 0 }
    }

    /// The outline of a square sits inside the edge.
    fn square(half: i32, outline: i32) -> IconStamp {
        let inner = half - outline;

        let rows = (-half..=half)
            .map(|dy| (if dy.abs() <= inner { inner } else { -1 }, half))
            .collect();

        IconStamp { top: -half, rows, outlined: outline > 0 }
    }

    /// The outline of a triangle sits outside the edge.
    fn triangle(half: i32, outline: i32) -> IconStamp {
        let outer = half + outline;

        let rows = (-outer..=outer)
            .map(|dy| {
                let fill = if dy.abs() <= half { (dy + half) / 2 } else { -1 };

                (fill, (dy + outer) / 2)
            })
            .collect();

        IconStamp { top: -outer, rows, outlined: outline > 0 }
    }

    /// A plus of arm length `half`. The bar carries the thickness of the arms.
    fn plus(half: i32) -> IconStamp {
        let bar = (half / 2).max(1) / 2;

        let rows = (-half..=half)
            .map(|dy| {
                let fill = if dy.abs() <= bar { half } else { bar };

                (fill, fill)
            })
            .collect();

        IconStamp { top: -half, rows, outlined: false }
    }

    /// Draw the icon around `centre`. The outline goes first, so the fill covers it.
    fn draw(&self, image: &mut RgbaImage, centre: (i32, i32), color: Rgba<u8>) {
        let (cx, cy) = centre;

        if self.outlined {
            for (row, &(fill, outline)) in self.rows.iter().enumerate() {
                let y = cy + self.top + row as i32;

                if fill < 0 {
                    draw_run(image, y, cx - outline, cx + outline, OUTLINE_COLOR);
                    continue;
                }

                draw_run(image, y, cx - outline, cx - fill - 1, OUTLINE_COLOR);
                draw_run(image, y, cx + fill + 1, cx + outline, OUTLINE_COLOR);
            }
        }

        for (row, &(fill, _)) in self.rows.iter().enumerate() {
            if fill < 0 {
                continue;
            }

            draw_run(image, cy + self.top + row as i32, cx - fill, cx + fill, color);
        }
    }
}

/// Byte range of one clipped run, or `None` when the run misses the image.
fn clip_run(image: &RgbaImage, y: i32, x0: i32, x1: i32) -> Option<(usize, usize)> {
    let (width, height) = image.dimensions();

    if y < 0 || y >= height as i32 {
        return None;
    }

    let x0 = x0.max(0);
    let x1 = x1.min(width as i32 - 1);

    if x0 > x1 {
        return None;
    }

    let start = (y as usize * width as usize + x0 as usize) * 4;

    Some((start, start + (x1 - x0 + 1) as usize * 4))
}

/// Write one horizontal run under the rules of `draw_pixel`.
///
/// An opaque colour overwrites. A transparent one lands on empty pixels only.
fn draw_run(image: &mut RgbaImage, y: i32, x0: i32, x1: i32, color: Rgba<u8>) {
    let Some((start, end)) = clip_run(image, y, x0, x1) else {
        return;
    };

    let buffer: &mut [u8] = image;

    if color[3] == 255 {
        for pixel in buffer[start..end].chunks_exact_mut(4) {
            pixel.copy_from_slice(&color.0);
        }

        return;
    }

    for pixel in buffer[start..end].chunks_exact_mut(4) {
        if pixel[3] == 0 {
            pixel.copy_from_slice(&color.0);
        }
    }
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
