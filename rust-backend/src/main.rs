use axum::{
    extract::Query,
    http::{HeaderMap, HeaderValue, Request, Response, StatusCode},
    middleware::{self, Next},
    response::IntoResponse,
    routing::get,
    Router,
};
use std::{collections::HashMap, fs::File};
use serde_json::Value;
use tokio::{runtime::Builder, sync::OnceCell};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::{
    boundingbox::BoundingBox, color_maps::ColorMapsConfig, config::LayerConfig, map_querying::get_feature_info_collection::{Feature, GetFeatureInfoCollection}, query_parameters::{GetFeatureInfoRequestParameters, GetMapRequestParameters}, request_profiling::RequestProfiling
};

pub mod beacon_api;
pub mod boundingbox;
pub mod cache_engine;
pub mod color_maps;
pub mod config;
pub mod data_utils;
pub mod errors;
pub mod image_utils;
pub mod map_drawing;
pub mod map_querying;
pub mod misc;
pub mod viewparams;
pub mod queries;
pub mod query_parameters;
pub mod request_profiling;

use lazy_static::lazy_static;

type LockMap = Arc<Mutex<HashMap<String, Arc<OnceCell<File>>>>>;

lazy_static! {
    pub static ref LOCK_MAP: LockMap =  {
        LockMap::default()
    };
}



#[derive(Clone)]
pub struct AppState{
    pub lock_map: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

fn main() {
    misc::configure_logger();

    let address = misc::get_env_var("HTTP_ADDRESS", Some("0.0.0.0"));
    let port: u16 = misc::get_env_var("HTTP_PORT", Some("8000"))
        .parse()
        .expect("Invalid port number (must be u16)");
    let workers: usize = misc::get_env_var("WORKERS", Some("12"))
        .parse()
        .expect("Invalid number of workers (must be usize)");

    Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async move {
            log::info!("Starting server on http://{}:{}", address, port);

            // build our application with a route
            let app = Router::new()
                .route("/", get(index))
                .route("/get-map", get(get_map))
                .route("/get-feature-info", get(get_feature_info))
                .route("/update-layers", get(update_layers))
                .route("/available-styles", get(available_styles))
                .layer(middleware::from_fn(log_middleware));

            let address = format!("{}:{}", address, port);

            let listener = tokio::net::TcpListener::bind(address).await.unwrap();

            axum::serve(listener, app).await.unwrap();
        });

}

async fn log_middleware(req: Request<axum::body::Body>, next: Next) -> Response<axum::body::Body> {
    log::info!("{} {}", req.method(), req.uri().path());
    next.run(req).await
}

async fn index() -> impl IntoResponse {
    (StatusCode::OK, "Beacon WMS Backend is running")
}

// test query
// http://localhost:3000/workspaces/default/wms?viewparams=year:2024;depth:[-10,-20];bbox[-90,-45,90,45]

async fn get_map(get_map_params: Query<GetMapRequestParameters>) -> impl IntoResponse {

    log::info!("Get map request: {:?}", get_map_params);

    let mut profiling = RequestProfiling::new();

    let config = misc::read_config_file();

    // add vars to queries, e.g. jaartal, maandtal, etc.

    // parse get_map_params.viewparams jaartal
    // check if dataset exists
    // if not execute query for dataset
    // problem is that multiple requests can come in for different the same dataset
    //need to lock an object (per layer + year) while query is being executed
    //other requests wait until it's done, then read the file

    // parse viewparams
    let requested_viewparams: HashMap<String, Value> = viewparams::parse_viewparams(&get_map_params.viewparams);

    // parse ocg dimensions and check for validity
    let requested_dimensions: HashMap<String, Value> =
        match viewparams::parse_time_elevation(&get_map_params.time, &get_map_params.elevation) {
            Ok(map) => map,
            Err(e) => {
                return (StatusCode::BAD_REQUEST, e).into_response();
            }
        };

    // should bbox be in the view params?
    let bounding_box = match BoundingBox::from_string(
        get_map_params.bbox.as_str(),
        get_map_params.crs.as_str(),
        get_map_params.version.as_str(),
    ) {
        Ok(bbox) => bbox,
        Err(e) => {
            log::error!("Error parsing bounding box: {}", e);
            return (
                StatusCode::BAD_REQUEST,
                format!("Error parsing bounding box: {}", e),
            )
                .into_response();
        }
    };

    let workspace = match config.workspaces {
        Some(workspaces) => workspaces
            .into_iter()
            .find(|ws| ws.id == get_map_params.workspace)
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    format!("Workspace not found: {}", get_map_params.workspace),
                )
            }),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                "No workspaces configured".to_string(),
            )
                .into_response();
        }
    };

    let workspace = match workspace {
        Ok(ws) => ws,
        Err(e) => return e.into_response(),
    };

    let wms_layers = get_map_params
        .layers
        .split(',')
        .map(|s| s.trim().to_string())
        .collect::<Vec<String>>();

    for layer_id in &wms_layers {
        if !workspace.layers.iter().any(|layer| layer.id == *layer_id) {
            return (
                StatusCode::NOT_FOUND,
                format!(
                    "Layer not found in workspace {}: {}",
                    workspace.id, layer_id
                ),
            )
                .into_response();
        }
    }

    let mut layers_configs : Vec<LayerConfig> = workspace
        .layers
        .iter()
        .filter(|layer| wms_layers.contains(&layer.id))
        .cloned()
        .collect::<Vec<config::LayerConfig>>();

    // check dimensions and apply dimensions and viewparams to layer config
    for layer in layers_configs.iter_mut() {
        // 1. Apply dimensions for this specific layer
        let applied_viewparams = match viewparams::apply_dimensions_to_viewparams(
            &requested_viewparams,
            &requested_dimensions,
            &layer.config.dimensions,
            &layer.id,
        ) {
            Ok(vp) => vp,
            Err(e) => {
                log::error!("Dimension error for layer {}: {}", layer.id, e);
                return (StatusCode::BAD_REQUEST, e).into_response();
            }
        };

        // 2. Assign into the same layer (in-place mutation)
        if let Err((status, msg)) =
            viewparams::assign_viewparams_in_config(layer, &applied_viewparams).await
        {
            log::error!(
                "Viewparams assignment failed for layer {}: {}",
                layer.id,
                msg
            );
            return (status, msg).into_response();
        }
    }

    let mut styles = match &get_map_params.styles {
        Some(s) => String::from(s),
        None => String::new(),
    };

    // Why are we making a string here?
    if styles.trim().is_empty() {
        // If no styles provided, default to "thermal" for each layer
        styles = (0..wms_layers.len())
            .map(|i: usize| {
                match &layers_configs[i].config.default_style {
                    Some(style) => style.clone(),
                    None => String::from("thermal"),
                }
            })
            .collect::<Vec<_>>()
            .join(",");
    }

    // and then turning that string back into a vector here?
    let styles_vec = styles
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<String>>();

    if styles_vec.len() > 0 {
        if styles_vec.len() != wms_layers.len() {
            return (
                StatusCode::BAD_REQUEST,
                "Number of styles must match number of layers".to_string(),
            )
                .into_response();
        }
    }

    profiling.mark("query parsed");

    let mut image: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> = image_utils::create_rgba_image(get_map_params.width, get_map_params.height);

    let layers_styles_wms_iter = layers_configs
        .iter()
        .zip(styles_vec.iter())
        .zip(wms_layers.iter());

    for ((layer_config, style), wms_layer) in layers_styles_wms_iter {

        // use the assigned viewparams to create a hash for the filename, so we can store different versions of the same layer with different viewparams
        let viewparams_hash = layer_config
            .config
            .assigned_viewparams
            .as_ref()
            .map(|vp| misc::hash_viewparams(vp));

        let viewparams_hash = viewparams_hash.as_deref();

        let layer_filepath = match misc::get_layer_filepath(&workspace.id, &layer_config.id, viewparams_hash) {
            Ok(path) => path,
            Err(e) => {
                log::error!("Error getting layer filepath: {:?}", e);
                String::new() // Return empty string on error, will be filtered out later
            }
        };

        // if empty (invalid string) return an error
        if layer_filepath.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                "Invalid layer file path: cannot convert to string".to_string(),
            ).into_response();
        }

        let file = match queries::get_dataset_file(&LOCK_MAP, layer_filepath.clone(), layer_config.clone()).await{
            Ok(f) => f,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        };

        // what do these min/max values do? why not in viewparams? why in layer config?
        let min_value = layer_config.config.min_value.unwrap_or(-10.0);
        let max_value = layer_config.config.max_value.unwrap_or(100.0);

        let color_map = match crate::color_maps::ColorMap::get_named(style, min_value, max_value, Some(false))
        {
            Some(map) => map,
            None => {
                log::error!("Color map not found: {}", style);
                return (
                    StatusCode::BAD_REQUEST,
                    format!("Style not found: {}", style),
                )
                    .into_response();
            }
        };

        let icon_shape = match &layer_config.config.shape {
            Some(shape) => shape.as_str(),
            None => "circle",
        }; 

        let drawing_result = map_drawing::get_map(
            &mut image,
            bounding_box.clone(),
            wms_layer,
            color_map, 
            &get_map_params.crs,
            layer_filepath,
            file,
            icon_shape,
            &mut profiling,
        );

        if drawing_result.is_err() {
            let e = drawing_result.err().unwrap();
            log::error!("Error drawing map: {:?}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Error drawing map: {:?}", e),
            )
                .into_response();
        } else {
            // log::info!("Successfully drew layer from file: {}", layer_filepath);
            profiling.mark(&format!("drawn {}", wms_layer));
        }
    }

    // profiling.mark(&format!("applying shadow"));

    // image_utils::apply_shadow(
    //     &mut image,
    //     1, 
    //     image::Rgba([0, 0, 0, 100]), 
    // );

    // profiling.mark(&format!("shadow applied"));

    let mut png_data: Vec<u8> = Vec::new();
    let output_buffer = image_utils::rgba_image_to_png(&image, &mut png_data);

    profiling.mark(&format!("image encoded"));

    // profiling.log_report(); // --> get profiling report in logs

    match output_buffer {
        Ok(_) => {
            let response = Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "image/png")
                .header("Content-Length", png_data.len().to_string())
                .body(axum::body::Body::from(png_data.clone()))
                .unwrap();

            response
        }
        Err(e) => {
            log::error!("Error encoding PNG: {:?}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Error encoding PNG: {:?}", e),
            )
                .into_response()
        }
    }
}

async fn get_feature_info(
    get_feature_info_params: Query<GetFeatureInfoRequestParameters>,
) -> impl IntoResponse {
    log::info!("Get feature info request: {:?}", get_feature_info_params);

    let config = misc::read_config_file();

    let requested_viewparams: HashMap<String, Value> = viewparams::parse_viewparams(&get_feature_info_params.viewparams);

    let requested_dimensions: HashMap<String, Value> =
    match viewparams::parse_time_elevation(&get_feature_info_params.time, &get_feature_info_params.elevation) {
        Ok(map) => map,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, e).into_response();
        }
    };

    let bounding_box = match BoundingBox::from_string(
        get_feature_info_params.bbox.as_str(),
        get_feature_info_params.crs.as_str(),
        get_feature_info_params.version.as_str(),
    ) {
        Ok(bbox) => bbox,
        Err(e) => {
            log::error!("Error parsing bounding box: {}", e);
            return (
                StatusCode::BAD_REQUEST,
                format!("Error parsing bounding box: {}", e),
            )
                .into_response();
        }
    };

    let workspace = match config.workspaces {
        Some(workspaces) => workspaces
            .into_iter()
            .find(|ws| ws.id == get_feature_info_params.workspace)
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    format!("Workspace not found: {}", get_feature_info_params.workspace),
                )
            }),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                "No workspaces configured".to_string(),
            )
                .into_response();
        }
    };

    let workspace = match workspace {
        Ok(ws) => ws,
        Err(e) => return e.into_response(),
    };

    let wms_layers = get_feature_info_params
        .layers
        .split(',')
        .map(|s| s.trim().to_string())
        .collect::<Vec<String>>();
    
    let image_dimensions = (
        get_feature_info_params.width,
        get_feature_info_params.height,
    );
    let click_coordinates = (get_feature_info_params.x, get_feature_info_params.y);
    
    let feature_count = match get_feature_info_params.feature_count {
        Some(count) => count,
        None => 10,
    };

    let query_layers = get_feature_info_params
        .query_layers
        .split(',')
        .map(|s| s.trim().to_string())
        .collect::<Vec<String>>();

        
    for query_layer_id in &query_layers {
        if !wms_layers.contains(query_layer_id) {
            return (
                StatusCode::BAD_REQUEST,
                format!("Query layer not in requested layers: {}", query_layer_id),
            )
                .into_response();
        }
    }


    for layer_id in &wms_layers {
        if !workspace.layers.iter().any(|layer| layer.id == *layer_id) {
            return (
                StatusCode::NOT_FOUND,
                format!(
                    "Layer not found in workspace {}: {}",
                    workspace.id, layer_id
                ),
            )
                .into_response();
        }
    }

    let mut layers_configs : Vec<LayerConfig> = workspace
        .layers
        .iter()
        .filter(|layer| wms_layers.contains(&layer.id))
        .cloned()
        .collect::<Vec<config::LayerConfig>>();

    // check dimensions and apply dimensions and viewparams to layer config
    for layer in layers_configs.iter_mut() {
        // 1. Apply dimensions for this specific layer
        let applied_viewparams = match viewparams::apply_dimensions_to_viewparams(
            &requested_viewparams,
            &requested_dimensions,
            &layer.config.dimensions,
            &layer.id,
        ) {
            Ok(vp) => vp,
            Err(e) => {
                log::error!("Dimension error for layer {}: {}", layer.id, e);
                return (StatusCode::BAD_REQUEST, e).into_response();
            }
        };

        // 2. Assign into the same layer (in-place mutation)
        if let Err((status, msg)) =
            viewparams::assign_viewparams_in_config(layer, &applied_viewparams).await
        {
            log::error!(
                "Viewparams assignment failed for layer {}: {}",
                layer.id,
                msg
            );
            return (status, msg).into_response();
        }
    }

    let mut feature_info_results: Vec<Feature> = Vec::new();

    for layer_config in layers_configs.iter() {

        // use the assigned viewparams to create a hash for the filename, so we can store different versions of the same layer with different viewparams
        let viewparams_hash = layer_config
            .config
            .assigned_viewparams
            .as_ref()
            .map(|vp| misc::hash_viewparams(vp));

        let viewparams_hash = viewparams_hash.as_deref();

        let layer_filepath = match misc::get_layer_filepath(&workspace.id, &layer_config.id, viewparams_hash) {
            Ok(path) => path,
            Err(e) => {
                log::error!("Error getting layer filepath: {:?}", e);
                String::new() // Return empty string on error, will be filtered out later
            }
        };

        // if empty (invalid string) return an error
        if layer_filepath.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                "Invalid layer file path: cannot convert to string".to_string(),
            ).into_response();
        }
        
        let file = match queries::get_dataset_file(&LOCK_MAP, layer_filepath.clone(), layer_config.clone()).await{
            Ok(f) => f,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        };

        let result = map_querying::get_feature_info(
            image_dimensions,
            click_coordinates,
            bounding_box.clone(),
            get_feature_info_params.crs.as_str(),
            feature_count,
            &layer_filepath,
            file
        );

        if result.is_err() {
            let e = result.err().unwrap();
            log::error!("Error getting feature info: {:?}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Error getting feature info: {:?}", e),
            )
                .into_response();
        } else {
            let mut features = result.ok().unwrap();
            feature_info_results.append(&mut features);
        }
    }

    // let mut feature_collection_properties: serde_json::map::Map<String, serde_json::Value> =
    //     serde_json::map::Map::new();

    // feature_collection_properties.insert(
    //     String::from("crs"),
    //     serde_json::Value::from(get_feature_info_params.crs.as_str()),
    // );

    let result = GetFeatureInfoCollection::new(
        feature_info_results,
        None // Some(serde_json::Value::from(feature_collection_properties)),
    );

    match &get_feature_info_params.info_format {
        format if format == "application/json" || format == "json" => {
            let json = result.to_json_string();
            let mut headers = HeaderMap::new();
            headers.insert("Content-Type", HeaderValue::from_static("application/json"));

            return (StatusCode::OK, headers, json).into_response();
        }

        format if format == "text/html" || format == "html" => {
            let html = result.to_html();
            let mut headers = HeaderMap::new();
            headers.insert("Content-Type", HeaderValue::from_static("text/html"));
            return (StatusCode::OK, headers, html).into_response();
        }

        format if format == "application/vnd.ogc.gml" || format == "gml" => {
            // GML not implemented yet
            let xml = result.to_xml();
            let mut headers = HeaderMap::new();
            headers.insert(
                "Content-Type",
                HeaderValue::from_static("application/vnd.ogc.gml"),
            );
            return (StatusCode::OK, headers, xml).into_response();
        }

        _ => {
            return (
                StatusCode::BAD_REQUEST,
                format!(
                    "Unsupported info_format: {}",
                    get_feature_info_params.info_format
                ),
            )
                .into_response()
        }
    }
}

// Downloads the layer data for the layers defined in the config.json.
// saves the data to a parquet file
// saves to path based on workspace id and layer id
async fn update_layers() -> impl IntoResponse {
    let config = misc::read_config_file();

    let mut updated_layer_files: Vec<String> = Vec::new();

    // for each layer in each workspace in config.workspaces,
    if let Some(workspaces) = config.workspaces {
        for workspace in workspaces {
            for layer in workspace.layers {
                // Here you would add the logic to update the layer in your application
                let query = serde_json::to_string(&layer.config.query).unwrap();

                // LOOK HERE!!!!!
                //==============
                //==============
                //==============
                //==============
                let layer_filepath = match misc::get_layer_filepath(&workspace.id, &layer.id, None) {
                    Ok(path) => path,
                    Err(e) => {
                        log::error!("Error getting layer filepath: {:?}", e);
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("Error getting layer filepath: {:?}", e),
                        );
                    }
                };

                // log::info!("Layer query: {:?}", query);
                log::info!("Layer file path: {:?}", layer_filepath);
                let instance_url = layer.config.instance_url;
                let auth_token = layer.config.token;

                let result =
                    beacon_api::query(&query, &instance_url, &auth_token, layer_filepath.as_str())
                        .await;

                if result.is_err() {
                    let e = result.err().unwrap();
                    log::error!("Error adding layer: {:?}", e);
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Error adding layer: {:?}", e),
                    );
                } else {
                    updated_layer_files.push(layer_filepath.to_string());
                }
            }
        }
    } else {
        log::warn!("No workspaces found in configuration");
    }

    (
        StatusCode::OK,
        format!("Layers updated: {:?}", updated_layer_files),
    )
}


async fn available_styles() -> impl IntoResponse {
    let color_maps_config = match ColorMapsConfig::load() {
        Some(config) => config,
        None => {
            log::error!("Could not load color maps configuration");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not load color maps configuration",
            )
                .into_response();
        }
    };

    let available_styles = color_maps_config.all();

    let mut data: Vec<Value> = Vec::new();

    for cm in available_styles {
        data.push(serde_json::json!({
            "name": cm.name,
            "description": cm.description
        }));
    }

    let json = serde_json::to_string(&data).unwrap();

    let mut headers = HeaderMap::new();

    headers.insert("Content-Type", HeaderValue::from_static("application/json"));

    (StatusCode::OK, headers, json).into_response()
}


// =========================================================================
// TESTS


// =========================================================================
// parse dimensions

#[cfg(test)]
mod test_parse_dimensions {
    use super::*;
    use serde_json::{Value};
    use std::collections::HashMap;

    #[test]
    fn test_both_valid() {
        let time = Some("2024-01-01T12:00:00Z".to_string());
        let elevation = Some("0/10".to_string());

        let result = viewparams::parse_time_elevation(&time, &elevation).unwrap();

        let mut expected = HashMap::<String, Value>::new();
        expected.insert("time".to_string(), Value::String("2024-01-01T12:00:00Z".to_string()));
        expected.insert("elevation".to_string(), Value::String("0/10".to_string()));

        println!("parsed dimensions: {:?}", result);

        assert_eq!(result, expected);
    }

    #[test]
    fn test_only_time_valid() {
        let time = Some("2024-01-01T12:00:00Z".to_string());
        let elevation = None;

        let result = viewparams::parse_time_elevation(&time, &elevation).unwrap();

        let mut expected = HashMap::<String, Value>::new();
        expected.insert("time".to_string(), Value::String("2024-01-01T12:00:00Z".to_string()));
        println!("parsed dimensions: {:?}", result);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_only_elevation_valid() {
        let time = None;
        let elevation = Some("0/10".to_string());

        let result = viewparams::parse_time_elevation(&time, &elevation).unwrap();

        let mut expected = HashMap::<String, Value>::new();
        expected.insert("elevation".to_string(), Value::String("0/10".to_string()));
        println!("parsed dimensions: {:?}", result);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_empty_inputs() {
        let time: Option<String> = None;
        let elevation: Option<String> = None;

        let result = viewparams::parse_time_elevation(&time, &elevation).unwrap();

        let expected = HashMap::<String, Value>::new();
        println!("parsed dimensions: {:?}", result);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_invalid_time_returns_error() {
        let time = Some("2024-01-01".to_string()); // invalid
        let elevation = None;

        let result = viewparams::parse_time_elevation(&time, &elevation);
        println!("parsed dimensions: {:?}", result);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Invalid time format. Expected YYYY-MM-DDThh:mm:ssZ"
        );
    }

    #[test]
    fn test_invalid_elevation_returns_error() {
        let time = None;
        let elevation = Some("10/0".to_string()); // invalid ordering

        let result = viewparams::parse_time_elevation(&time, &elevation);
        println!("parsed dimensions: {:?}", result);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Invalid order of elevation, expected min/max"
        );
    }

    #[test]
    fn test_invalid_time_short_circuits() {
        let time = Some("bad".to_string());
        let elevation = Some("0/10".to_string());

        let result = viewparams::parse_time_elevation(&time, &elevation);
        println!("parsed dimensions: {:?}", result);
        // should fail fast on time, elevation ignored
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_elevation_short_circuits() {
        let time = Some("2024-01-01T12:00:00Z".to_string());
        let elevation = Some("a/b".to_string());

        let result = viewparams::parse_time_elevation(&time, &elevation);
        println!("parsed dimensions: {:?}", result);
        assert!(result.is_err());
    }

    // negative elevation values
    #[test]
    fn test_valid_negative_elevation() {
        let time = None;
        let elevation = Some("-20/-10".to_string()); // invalid ordering

        let result = viewparams::parse_time_elevation(&time, &elevation).unwrap();

        let mut expected = HashMap::<String, Value>::new();
        expected.insert("elevation".to_string(), Value::String("10/20".to_string()));
        println!("parsed dimensions: {:?}", result);
        assert_eq!(result, expected);
    }
}

// =========================================================================
// check dimensions

#[cfg(test)]
mod test_check_dimensions {
    use super::*;
    use serde_json::{json};
    use std::collections::HashMap;

    fn sample_layer_dimensions() -> HashMap<String, serde_json::Value> {
        serde_json::from_value(json!({
            "time": {
                "default": "2021-01-01T00:00:00Z",
                "units": "ISO8601",
                "accepted": "R500/1950-01-01T00:00:00Z/P1Y"
            },
            "elevation": {
                "default": "0-5",
                "units": "m",
                "viewparam": "depth",
                "accepted": [
                    "0-5",
                    "5-10",
                    "10-20",
                    "20-30",
                    "30-50",
                    "50-75",
                    "75-100",
                    "100-125",
                    "125-150",
                    "150-200",
                    "200-250",
                    "250-300",
                    "300-400",
                    "400-500",
                    "500-600",
                    "600-700",
                    "700-800",
                    "800-900",
                    "900-1000",
                    "1000-1100",
                    "1100-1200",
                    "1200-1300",
                    "1300-1400",
                    "1400-1500",
                    "1500-1750",
                    "1750-2000",
                    "2000-2500",
                    "2500-3000",
                    "3000-3500",
                    "3500-4000",
                    "4000-4500",
                    "4500-5000",
                    "5000-12000"
                ]
            }
        }))
        .unwrap()
    }

    // elevation
    #[test]
    fn test_valid_elevation_returns_range() {
        let dims = sample_layer_dimensions();

        let result = viewparams::check_accepted_elevations(
            "5-10",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);

        assert!(result.is_ok());

        let range = result.unwrap();
        assert_eq!(range.len(), 2);
    }

    #[test]
    fn test_invalid_elevation_not_allowed() {
        let dims = sample_layer_dimensions();

        let result = viewparams::check_accepted_elevations(
            "9999-10000",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "requested elevation not allowed for this layer testlayer"
        );
    }

    #[test]
    fn test_missing_elevation_dimension() {
        let mut dims = sample_layer_dimensions();
        dims.remove("elevation");

        let result = viewparams::check_accepted_elevations(
            "0-5",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "elevation dimension not allowed"
        );
    }

    #[test]
    fn test_accepted_not_array() {
        let mut dims = sample_layer_dimensions();

        dims.get_mut("elevation").map(|e| {
            if let Some(obj) = e.as_object_mut() {
                obj.insert(
                    "accepted".to_string(),
                    serde_json::Value::String("0-5".to_string()),
                );
            }
        });

        let result = viewparams::check_accepted_elevations(
            "0-5",
            &dims,
            "testlayer",
        );
        println!("{:?}", result);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "elevation dimension in config must be an array"
        );
    }

    // time
    #[test]
    fn test_valid_time_start_of_range() {
        let dims = sample_layer_dimensions();

        let result = viewparams::check_accepted_times(
            "1950-01-01T00:00:00Z",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);

        assert!(result.is_ok());

        let map = result.unwrap();
        assert_eq!(map.get("year").unwrap(), &serde_json::json!(1950));
    }

    #[test]
    fn test_valid_time_within_range() {
        let dims = sample_layer_dimensions();

        let result = viewparams::check_accepted_times(
            "1955-01-01T00:00:00Z",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);

        assert!(result.is_ok());

        let map = result.unwrap();
        assert_eq!(map.get("year").unwrap(), &serde_json::json!(1955));
    }

    #[test]
    fn test_time_outside_range() {
        let dims = sample_layer_dimensions();

        let result = viewparams::check_accepted_times(
            "2050-01-01T00:00:00Z",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "date not accepted for layer testlayer"
        );
    }

    #[test]
    fn test_missing_time_dimension() {
        let mut dims = sample_layer_dimensions();
        dims.remove("time");

        let result = viewparams::check_accepted_times(
            "1950-01-01T00:00:00Z",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "time dimension not allowed"
        );
    }

    #[test]
    fn test_invalid_time_format() {
        let dims = sample_layer_dimensions();

        let result = viewparams::check_accepted_times(
            "1950/01/01",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);

        assert!(result.is_err());
    }    

    #[test]
    fn test_time_boundary_last_valid_year() {
        let dims = sample_layer_dimensions();

        let result = viewparams::check_accepted_times(
            "1999-01-01T00:00:00Z",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);

        assert!(result.is_ok());

        let map = result.unwrap();
        assert_eq!(map.get("year").unwrap(), &serde_json::json!(1999));
    }

}