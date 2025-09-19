use arrow::{array::AsArray, datatypes::Float64Type};
use serde_json::{Map};

use crate::{
    boundingbox::BoundingBox,
    data_utils,
    errors::MapError,
    map_drawing::{
        LATITUDE_COLUMN, LONGITUDE_COLUMN, REPROJECTED_DATASET_CACHE, TIME_COLUMN, VALUE_COLUMN,
    },
    map_querying::get_feature_info_collection::{Feature, GetFeatureInfoCollection},
    misc,
};

pub mod get_feature_info_collection;

pub const PIXEL_BUFFER: f64 = 10.0; // buffer around the click coordinates to search for features

pub fn get_feature_info(
    image_dimensions: (u32, u32),
    click_coordinates: (u32, u32),
    bounding_box: BoundingBox,
    query_layer: &str,
    crs: &str,
    feature_count: u32,
    layer_filepath: &str,
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

    let mut results: Vec<Feature> = Vec::new();
    let units_per_pixel = bounding_box.get_width() / image_dimensions.0 as f64;
    let mut coordinates =
        misc::pixel_offset_to_coordinates(&reprojected_bbox, image_dimensions, click_coordinates);

    {
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
    }

    let bbox_of_interest = BoundingBox::new(
        coordinates.0 - (units_per_pixel * PIXEL_BUFFER),
        coordinates.1 - (units_per_pixel * PIXEL_BUFFER),
        coordinates.0 + (units_per_pixel * PIXEL_BUFFER),
        coordinates.1 + (units_per_pixel * PIXEL_BUFFER),
        target_projection_code,
    );

    if !bbox_of_interest.in_bbox(coordinates, None) {
        log::error!(
            "Click coordinates are not in bbox of interest: {:?}",
            coordinates
        );

        return Ok(Vec::new());
    }

    let reader = data_utils::open_parquet_reader(query_layer, layer_filepath)?;

    for (i, batch) in reader.enumerate() {
        
        let record_batch_name = format!("{}_{}_{}", query_layer, target_projection_code, i);

        let batch = match batch {
            Ok(batch) => {

                if let Some(projected_batch) =
                    REPROJECTED_DATASET_CACHE.get_projection_applied_batch(target_projection_code, &record_batch_name)
                {
                    projected_batch
                } else {


                    // Reproject batch if needed:
                    let res = REPROJECTED_DATASET_CACHE.apply_projection_to_batch(
                        source_projection_code,
                        target_projection_code,
                        &record_batch_name,
                        batch,
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
        let value_column = value_column.into_iter();

        let time_column = batch
            .column_by_name(TIME_COLUMN)
            .unwrap()
            .as_primitive::<Float64Type>()
            .clone();
        let time_column = time_column.into_iter();

        let zipped_values_iterator = value_column.zip(time_column);
        let zipped_iterator = latitude_column
            .zip(longitude_column)
            .zip(zipped_values_iterator);

        for ((lat, lng), (val, time)) in zipped_iterator {
            if results.len() >= feature_count as usize {
                break;
            }

            if lat.is_none() || lng.is_none() || val.is_none() || time.is_none() {
                continue;
            }

            let lat = lat.unwrap();
            let lng = lng.unwrap();
            let val = val.unwrap();
            let time = time.unwrap();

            // log::info!("y: {}, x: {}", lat, lng);

            if bbox_of_interest.in_bbox((lng, lat), None) {
                //reproject coordinates to target projection for the response.
                let mut _coordinates = (lng, lat);

                let mut _properties: Map<String, serde_json::Value> = Map::new();

                _properties.insert(String::from("value"), serde_json::Value::from(val));
                _properties.insert(String::from("time"), serde_json::Value::from(time));

                let feature = GetFeatureInfoCollection::create_point_feature(
                    lng,
                    lat,
                    Some(_properties.clone()),
                );

                results.push(feature);
            }
        }
    }

    Ok(results)
}
