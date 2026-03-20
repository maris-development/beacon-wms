use arrow::{array::{AsArray}, datatypes::Float64Type};
use serde_json::{Map, Value};
use std::fs::File;

use crate::{
    boundingbox::BoundingBox,
    data_utils,
    errors::MapError,
    map_drawing::{LATITUDE_COLUMN, LONGITUDE_COLUMN},
    map_querying::get_feature_info_collection::{Feature, GetFeatureInfoCollection},
    misc,
};

pub mod get_feature_info_collection;

pub const PIXEL_BUFFER: f64 = 10.0; // buffer around the click coordinates to search for features

pub fn get_feature_info(
    image_dimensions: (u32, u32),
    click_coordinates: (u32, u32),
    bounding_box: BoundingBox,
    crs: &str,
    feature_count: u32,
    layer_filepath: &str,
    file: File
) -> Result<Vec<Feature>, MapError> {
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

    let mut results: Vec<Feature> = Vec::with_capacity(feature_count as usize);
    let units_per_pixel = bounding_box.get_width() / image_dimensions.0 as f64;
    let mut coordinates =
        misc::pixel_offset_to_coordinates(&reprojected_bbox, image_dimensions, click_coordinates);

    // take care of clicks over the edge of the world (> +/-180 degrees)
    let mut coordinates_wgs84 = coordinates.clone();
    misc::transform_coordinates(
        target_projection_code,
        source_projection_code,
        &mut coordinates_wgs84,
    )
    .map_err(|e| {
        log::error!(
            "Could not convert coordinates {:?}, target projection: {} \n{}",
            coordinates,
            source_projection_code,
            e
        );
        MapError::Error(e)
    })?;

    // log::info!("coordinates_wgs84: {:?} src {} trgt {}", coordinates_wgs84, target_projection_code, source_projection_code);

    //if coordinates X coordinate is over 180, than subtract 360 to get the correct coordinate:
    if coordinates_wgs84.0 > 180.0 {
        coordinates_wgs84.0 -= 360.0;
        coordinates.0 = coordinates_wgs84.0;
    }

    //if coordinates X coordinate is less than -180, than add 360 to get the correct coordinate:
    if coordinates_wgs84.0 < -180.0 {
        coordinates_wgs84.0 += 360.0;
        coordinates.0 = coordinates_wgs84.0;
    }

    let bbox_of_interest = BoundingBox::new(
        coordinates.0 - (units_per_pixel * PIXEL_BUFFER),
        coordinates.1 - (units_per_pixel * PIXEL_BUFFER),
        coordinates.0 + (units_per_pixel * PIXEL_BUFFER),
        coordinates.1 + (units_per_pixel * PIXEL_BUFFER),
        target_projection_code,
    );

    let bbox_of_interest_wgs84 = bbox_of_interest.reproject(source_projection_code).map_err(|e| {
        log::error!(
            "Could not reproject bbox_of_interest to WGS84: {}",
            e
        );
        MapError::Error(e)
    })?;

    if !bbox_of_interest.in_bbox(coordinates, None) {
        log::error!(
            "Click coordinates are not in bbox of interest: {:?}",
            coordinates
        );

        return Ok(Vec::new());
    }

    let reader = data_utils::parquet_reader(layer_filepath, file)?;

    for (_, batch) in reader.enumerate() {
        let batch = batch.map_err(|e| {
            log::error!("Error reading batch: {}", e);
            MapError::Error(format!("Error reading batch: {}", e))
        })?;

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

        let zipped_iterator = latitude_column
            .zip(longitude_column);

        let mut row_idx = 0;
        for (lat, lng) in zipped_iterator {
            row_idx += 1;
            
            if results.len() == results.capacity(){
                break;
            }

            if lat.is_none() || lng.is_none() {
                continue;
            }

            let lat = lat.unwrap();
            let lng = lng.unwrap();
        

            // log::info!("y: {}, x: {}", lat, lng);

            if bbox_of_interest_wgs84.in_bbox((lng, lat), None) {
                // Forward-project the matched WGS84 point to target CRS for the response geometry.
                let mut _coordinates = (lng, lat);
                misc::transform_coordinates(
                    source_projection_code,
                    target_projection_code,
                    &mut _coordinates,
                )
                .map_err(|e| {
                    log::error!("Could not reproject feature coordinates: {}", e);
                    MapError::Error(e)
                })?;

                let mut _properties: serde_json::Map<String, Value> = Map::new();

                // log::info!("Fields: {:?}", batch.schema().fields());

                // Add all other columns dynamically
                for (col_idx, field) in batch.schema().fields().iter().enumerate() {
                    let col_name = field.name();

                    

                    if col_name == LATITUDE_COLUMN
                        || col_name == LONGITUDE_COLUMN
                    {
                        continue;
                    }

                    // log::info!("Processing column: {}", col_name);

                    let col = batch.column(col_idx);

                    if col.is_null(row_idx) {
                        continue;
                    }

                    // Convert value to JSON (simple case: primitive types)
                    let string_value = misc::get_string_value(col, row_idx);

                    _properties.insert(col_name.clone(), Value::String(string_value));

                }

                let feature = GetFeatureInfoCollection::create_point_feature(
                    lng, // _coordinates.0,
                    lat, //_coordinates.1,
                    Some(_properties.clone()),
                );



                // log::info!(
                //     "Feature: {:?}",
                //     _properties.keys()
                // );

                results.push(feature);
            }
        }
    }

    Ok(results)
}
