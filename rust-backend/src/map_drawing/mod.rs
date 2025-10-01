use arrow::array::{AsArray, PrimitiveArray};
use arrow::datatypes::{Float64Type, UInt32Type};
use boundingbox::BoundingBox;
use image::{Pixel, Rgba, RgbaImage};
use lazy_static::lazy_static;
use log;
use std::collections::HashSet;

use crate::cache_engine::ReprojectedDatasetCacheEngine;
use crate::color_maps::ColorMap;
use crate::data_utils::{self};
use crate::errors::MapError;
use crate::request_profiling::RequestProfiling;
use crate::{boundingbox, image_utils, misc};

lazy_static! {
    pub static ref REPROJECTED_DATASET_CACHE: ReprojectedDatasetCacheEngine =
        ReprojectedDatasetCacheEngine::new();
}


pub const LONGITUDE_COLUMN: &'static str = "longitude";
pub const LATITUDE_COLUMN: &'static str = "latitude";
pub const VALUE_COLUMN: &'static str = "value";
pub const TIME_COLUMN: &'static str = "time";

pub const COLOR_ONLY_ZOOMLEVEL: u32 = 6;
pub const SMALL_ICON_ZOOMLEVEL: u32 = 8;

/// Draw map on image
///
pub fn get_map(
    image: &mut RgbaImage,
    bounding_box: BoundingBox,
    layer: &str,
    color_map: ColorMap,
    crs: &str,
    layer_filepath: &str,
    profiling: &mut RequestProfiling
) -> Result<usize, MapError> {
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

    //split layers by , and check if the .ipc file exists:
    let degree_per_pixel = bounding_box.get_width_degrees() / image.width() as f64;
    let zoom = misc::degrees_per_pixel_to_zoom(degree_per_pixel, None);
    let point_radius = misc::calculate_point_radius(zoom, 5.0, 40.0);
    // let scale_factor: f64 = misc::calculate_scale_factor(degree_per_pixel);
    let bbox_margin = Some(reprojected_bbox.get_width() * 0.1);
    let mut drawn_points_set: HashSet<(i64, i64)> = HashSet::new();

    let reader = data_utils::open_parquet_reader(layer, layer_filepath)?;

    profiling.mark("parquet reader created");

    for (i, batch) in reader.enumerate() {


        let record_batch_name = format!("{}_{}_{}", layer, target_projection_code, i);

        profiling.mark(&format!("start reading batch {}", record_batch_name));

        let batch = match batch {
            Ok(batch) => {

                if let Some(projected_batch) =
                    REPROJECTED_DATASET_CACHE.get_projection_applied_batch(target_projection_code, &record_batch_name)
                {
                    projected_batch
                } else {
                    profiling.mark(&format!("reprojecting {}", record_batch_name));


                    // Reproject batch if needed:
                    let res = REPROJECTED_DATASET_CACHE.apply_projection_to_batch(
                        source_projection_code,
                        target_projection_code,
                        &record_batch_name,
                        batch
                    );

                    if res.is_err() {
                        log::error!(
                            "Could not apply projection to batch: {}",
                            res.err().unwrap()
                        );
                    }

                    // log::info!(
                    //     "Reprojected batch: {} with projection: {}",
                    //     record_batch_name,
                    //     target_projection_code
                    // );

                    profiling.mark(&format!("reprojecting done {}", record_batch_name));
                    
                    REPROJECTED_DATASET_CACHE
                        .get_projection_applied_batch(target_projection_code, &record_batch_name)
                        .unwrap()
                }
            }

            Err(e) => {
                log::error!("Error reading batch: {}", e);
                return Err(MapError::Error(format!("Error reading batch: {}", e)));
            }
        };
        
        let latitude_column = batch
            .column_by_name(LATITUDE_COLUMN)
            .unwrap()
            .as_primitive::<Float64Type>()
            .clone();
        let latitude_column = latitude_column.into_iter();

        let longitude_column = batch
            .column_by_name(LONGITUDE_COLUMN)
            .unwrap()
            .as_primitive::<Float64Type>()
            .clone();
        let longitude_column = longitude_column.into_iter();

        let value_column = batch
            .column_by_name(VALUE_COLUMN)
            .unwrap()
            .as_primitive::<Float64Type>()
            .clone();


        profiling.mark(&format!("done reading batch {}", record_batch_name));

        let color_values: PrimitiveArray<UInt32Type> = value_column.unary(|x| {
            let rgba = color_map.query(x);
            let [r, g, b, a] = rgba.0;
            ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | (a as u32)
        });

        
        profiling.mark(&format!("done colormapping batch {}", record_batch_name));


        let color_values = color_values.into_iter();
        let zipped_iterator = latitude_column.zip(longitude_column).zip(color_values);


        profiling.mark(&format!("start drawing batch {}", record_batch_name));
                

        for ((lat, lng), color) in zipped_iterator {
            if lat.is_none() || lng.is_none() || color.is_none() {
                continue;
            }

            let coordinates = (lng.unwrap(), lat.unwrap()); // X Y
            let color = image_utils::unpack_rgba(color.unwrap());

            // log::info!("coordinates: {:?}", coordinates);
            // log::info!("bbox: {:?}", reprojected_bbox);
            // log::info!("in_bbox: {:?}", reprojected_bbox.in_bbox(coordinates, bbox_margin));

            let point_key  = (coordinates.0 as i64, coordinates.1 as i64);

            if drawn_points_set.contains(&point_key) {
                continue;
            } else {
                drawn_points_set.insert(point_key);
            }
        
            if reprojected_bbox.in_bbox(coordinates, bbox_margin) {
        
                let offset = misc::coordinates_to_pixel_offset(
                    &reprojected_bbox,
                    image.dimensions(),
                    coordinates,
                );

                // let color = Rgba([255u8, 0u8, 0u8, 255u8]);//ColorMap::get_color(val);

                let draw_result: Result<(), MapError> = draw_point(image, offset, color, Some(point_radius));

                if draw_result.is_err() {
                    log::error!("Could not draw image: {:?}", draw_result.err().unwrap());
                }
            }
        }

        profiling.mark(&format!("done drawing batch {}", record_batch_name));
    }


    // misc::print_bbox_on_image(&reprojected_bbox, image); //debugging

    return Ok(drawn_points_set.len());
}



fn draw_point(image: &mut RgbaImage, point: (i32, i32), color: Rgba<u8>, radius: Option<i32>) -> Result<(), MapError> {
    let radius = radius.unwrap_or(2);

    for x in -radius..=radius {
        for y in -radius..=radius {
            if x * x + y * y <= radius * radius {
                let x = point.0 + x;
                let y = point.1 + y;

                if misc::inside_image(image, (x, y)) {
                    image.put_pixel(x as u32, y as u32, color);
                }
            }
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
