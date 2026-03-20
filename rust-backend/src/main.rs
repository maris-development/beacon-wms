use axum::{
    extract::Query,
    http::{HeaderMap, HeaderValue, Request, Response, StatusCode},
    middleware::{self, Next},
    response::IntoResponse,
    routing::get,
    Router,
};
use std::{collections::HashMap, fs::File, future::Future, hash::Hash};
use serde_json::Value;
use tokio::{runtime::Builder, sync::OnceCell};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use axum::extract::State;

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
pub mod query_parameters;
pub mod request_profiling;

use lazy_static::lazy_static;

type LockMap = Arc<Mutex<HashMap<String, Arc<OnceCell<File>>>>>;

lazy_static! {
    pub static ref LOCK_MAP: LockMap =  {
        LockMap::default()
    };
}


async fn get_or_insert<F : FnOnce() -> Fut, Fut: Future<Output = Result<File, String>>>(lock_map: &LockMap, view_params_key: String, fut: F) -> Result<File,String> {
    let mut locked_map = lock_map.lock().await;
    let once_cell= locked_map.entry(view_params_key).or_insert(Arc::new(OnceCell::new()));
    let resolved = once_cell.get_or_try_init(fut).await;
    resolved.map(|f| f.try_clone().unwrap())
} 

fn query_file(layer_filepath: String, layer_config: LayerConfig) -> impl Future<Output = Result<File,String>> {
    async move {
        // Run the query inside this async move that returns a Result<File,String>

        // build query
        let assigned_viewparams = layer_config
            .config
            .assigned_viewparams
            .as_ref();

        let query_str_raw: String = serde_json::to_string(&layer_config.config.query)
            .map_err(|e| format!("Error serializing query: {:?}", e))?;

        // apply the view params of the layer to the beacon query
        // edit this function so the viewparams are applied correctly
        let query_str = apply_viewparams_to_query(
            query_str_raw,
            assigned_viewparams,
        );

        // Log the layer and filepath
        log::info!("Updating layer at path: {:?}", &layer_filepath);

        // run query
        let instance_url = &layer_config.config.instance_url;
        let auth_token = &layer_config.config.token;

        beacon_api::query(
            &query_str,
            instance_url,
            auth_token,
            &layer_filepath
        )
        .await
        .map_err(|e| format!("Error updating '{}': {:?}", &layer_filepath, e))?;

        match File::open(layer_filepath) {
            Ok(file) => Ok(file),
            Err(err) => Err(err.to_string()),
        }
    }
}

/// t1 => lock
/// t1 => file bestaat niet => insert view params & once cell with F -> T
/// t1 => clone the arc -> unlocked/drop -> get_or_init 
/// t2 => lock -> ziet de viewparams staan met once cell -> get_or_init -> synced met t1
/// t2 => unlocked -> arc<once<f>>
/// t3 => lock -> zit nog niet in -> 
async fn get_view_params_queried_file(
    lock_map: &LockMap,
    layer_filepath: String,
    layer_config: LayerConfig
) -> Result<File, String> {
    get_or_insert(
        lock_map,
        layer_filepath.clone(),
        move|| query_file(layer_filepath, layer_config)
    ).await
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
    let requested_viewparams: HashMap<String, Value> = match &get_map_params.viewparams {
        Some(vp_str) => {
            let mut vp_map = HashMap::<String, Value>::new();

            for pair in vp_str.split(';') {
                let mut iter = pair.splitn(2, ':');
                if let (Some(key), Some(value_str)) = (iter.next(), iter.next()) {
                    // Attempt to parse the value as JSON
                    // This will handle numbers, arrays, strings
                    let parsed_value: Value = serde_json::from_str(value_str)
                        // fallback to string if parsing fails
                        .unwrap_or(Value::String(value_str.to_string()));
                    vp_map.insert(key.to_string(), parsed_value);
                }
            }
            vp_map
        }
        None => HashMap::<String, Value>::new(),
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

    let mut workspace = match workspace {
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


    // assign the requested_viewparams by checking them against the available_viewparams and overwriting the default values in assigned_viewparams
    for layer_config in &mut layers_configs {
        // Ensure assigned_viewparams exists (with defaults if present)
        let assigned_viewparams = layer_config
            .config
            .assigned_viewparams
            .get_or_insert_with(HashMap::new);

        // If there are allowed viewparams, validate & assign requested ones
        if let Some(allowed) = &layer_config.config.available_viewparams {
            if let Err((status, msg)) = assign_viewparams(
                allowed,
                assigned_viewparams,       // mutable reference to defaults
                &requested_viewparams      // user input to overwrite defaults
            ).await {
                return (status, msg).into_response();  // stop immediately on invalid input
            }
        }
        // assigned_viewparams now contains defaults + any valid input
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

        let file = match get_view_params_queried_file(&LOCK_MAP, layer_filepath.clone(), layer_config.clone()).await{
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

    // what we do here?
    // we need to look at this again because we now have custom queries
    let layers_filepaths = workspace
        .layers
        .iter()
        .filter(|layer| query_layers.contains(&layer.id))
        .map(|layer| {

            match misc::get_layer_filepath(&workspace.id, &layer.id, None) {
                Ok(path) => path,
                Err(e) => {
                    log::error!("Error getting layer filepath: {:?}", e);
                    String::new() // Return empty string on error, will be filtered out later
                }
            }
        })
        .filter(|path| !path.is_empty()) // Filter out any empty paths
        .collect::<Vec<String>>();

    //dont care about styles, point/feature size is always decided by zoom level

    let layers_styles_wms_iter = layers_filepaths.iter().zip(query_layers.iter());

    let image_dimensions = (
        get_feature_info_params.width,
        get_feature_info_params.height,
    );
    let click_coordinates = (get_feature_info_params.x, get_feature_info_params.y);
    let feature_count = match get_feature_info_params.feature_count {
        Some(count) => count,
        None => 10,
    };

    let mut feature_info_results: Vec<Feature> = Vec::new();

    for (layer_filepath, query_layer) in layers_styles_wms_iter {



        let result = map_querying::get_feature_info(
            image_dimensions,
            click_coordinates,
            bounding_box.clone(),
            query_layer,
            get_feature_info_params.crs.as_str(),
            feature_count,
            &layer_filepath,
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

    let mut feature_collection_properties: serde_json::map::Map<String, serde_json::Value> =
        serde_json::map::Map::new();

    feature_collection_properties.insert(
        String::from("crs"),
        serde_json::Value::from(get_feature_info_params.crs.as_str()),
    );

    let result = GetFeatureInfoCollection::new(
        feature_info_results,
        Some(serde_json::Value::from(feature_collection_properties)),
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

/**
 * Resolves and validates viewparams from the request against the allowed parameters defined in the config.
 * Only parameters that are defined in the config and have valid the type will they be included in the returned HashMap.
 * Else throw error
 */
async fn assign_viewparams(
    allowed: &HashMap<String, Value>,
    assigned: &mut HashMap<String, Value>,
    input: &HashMap<String, Value>,
) -> Result<(), (StatusCode, String)> {
    for (param, value) in input {
        // Parameter must exist in allowed config
        let expected = match allowed.get(param) {
            Some(v) => v,
            None => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("Param '{}' not allowed", param),
                ));
            }
        };

        // Validate type
        let valid = match expected {
            Value::String(t) if t == "numeric" => value.is_number(),
            Value::String(t) if t == "string" => value.is_string(),
            Value::String(t) if t == "bool" => value.is_boolean(),
            Value::Array(expected_arr) => {
                if let Some(arr) = value.as_array() {
                    arr.len() == expected_arr.len()
                        && arr.iter().all(|v| v.is_number())
                } else {
                    false
                }
            }
            _ => false,
        };

        if !valid {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Type mismatch for param '{}'", param),
            ));
        }

        // Assign the value, overwriting default if present
        assigned.insert(param.clone(), value.clone());
    }

    Ok(())
}


fn apply_viewparams_to_query(
    mut query_str: String,
    assigned_viewparams: Option<&HashMap<String, Value>>,
) -> String {

    // Convert query JSON to string
    let params = match assigned_viewparams {
        Some(p) if !p.is_empty() => p,
        _ => return query_str,
    };

    println!("original query string {}", query_str);

    for (key, value) in params {
        match value {
            // ---- STRING ----
            Value::String(s) => {
                // Only replace unquoted placeholder
                let placeholder = format!("%{}%", key);
                query_str = query_str.replace(&placeholder, s);
            }

            // ---- NUMBER / BOOL ----
            Value::Number(_) | Value::Bool(_) => {
                let replacement = value.to_string();

                // Replace quoted first
                let quoted_placeholder = format!("\"%{}%\"", key);
                query_str = query_str.replace(&quoted_placeholder, &replacement);

                // Then unquoted
                let placeholder = format!("%{}%", key);
                query_str = query_str.replace(&placeholder, &replacement);
            }

            // ---- ARRAY ----
            Value::Array(arr) => {
                for (i, elem) in arr.iter().enumerate() {
                    match elem {
                        Value::String(s) => {
                            let placeholder = format!("%{}[{}]%", key, i);
                            query_str = query_str.replace(&placeholder, s);
                        }

                        Value::Number(_) | Value::Bool(_) => {
                            let replacement = elem.to_string();

                            let quoted_placeholder =
                                format!("\"%{}[{}]%\"", key, i);
                            query_str =
                                query_str.replace(&quoted_placeholder, &replacement);

                            let placeholder = format!("%{}[{}]%", key, i);
                            query_str = query_str.replace(&placeholder, &replacement);
                        }

                        _ => {
                            log::warn!(
                                "Unhandled array element type for key {}[{}]",
                                key,
                                i
                            );
                        }
                    }
                }
            }

            _ => {
                log::warn!("Unhandled viewparam type for key {}", key);
            }
        }
    }

    println!("applied query string {}", query_str);

    query_str
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sha2::{Sha256, Digest};

    fn parse_viewparams(vp_str: &str) -> HashMap<String, Value> {
    let mut vp_map = HashMap::<String, Value>::new();

    for pair in vp_str.split(';') {
        let mut iter = pair.splitn(2, ':');
        if let (Some(key), Some(value_str)) = (iter.next(), iter.next()) {
            // Try to parse as JSON, fallback to string
            let parsed_value: Value = serde_json::from_str(value_str)
                .unwrap_or(Value::String(value_str.to_string()));
            vp_map.insert(key.to_string(), parsed_value);
            }
        }

        vp_map
    }

    fn hash_viewparams(viewparams: &HashMap<String, Value>) -> String {
        // Serialize to JSON string in a deterministic way
        let mut sorted: Vec<_> = viewparams.iter().collect();
        sorted.sort_by_key(|(k, _)| *k); // sort keys for deterministic hash
        let json_string = serde_json::to_string(&sorted).unwrap();

        // Compute SHA256 hash
        let mut hasher = Sha256::new();
        hasher.update(json_string.as_bytes());
        let result = hasher.finalize();

        // Return as hex string
        hex::encode(result)
    }

    #[tokio::test]
    async fn test_assign_viewparams() {

        let mut allowed = HashMap::<String, Value>::new();
        allowed.insert("year".to_string(), json!("numeric"));
        allowed.insert("month".to_string(), json!("numeric"));
        allowed.insert("bbox".to_string(), json!(["numeric","numeric","numeric","numeric"]));
        allowed.insert("depth".to_string(), json!(["numeric","numeric"]));
        allowed.insert("station".to_string(), json!("string"));

        let test_str = "year:2024;depth:[-10,-20]";

        let viewparams = parse_viewparams(test_str);

        let assigned = assign_viewparams(&allowed, &viewparams).await.unwrap();

        assert_eq!(assigned.get("year"), Some(&json!(2024)));
        assert_eq!(assigned.get("depth"), Some(&json!([-10, -20])));

        // month should fail type check (string instead of numeric)
        assert!(assigned.get("month").is_none());

        // not provided
        assert!(assigned.get("station").is_none());

        let hash = hash_viewparams(&assigned);

        println!("hash: {}", hash);
    }

    #[tokio::test]
    async fn test_json_str(){
        let query = json!({
            "from": "default",
            "select": [
                {"column": "TIME", "alias": "time"},
                {"column": "LONGITUDE", "alias": "longitude"},
                {"column": "LATITUDE", "alias": "latitude"},
                {"column": "DEPTH", "alias": "depth"},
                {"column": "TEMPPR01", "alias": "value"}
            ],
            "filters": [
                {"column": "time", "gt_eq": "%year%-01-01T00:00:00", "lt_eq": "%year%-12-31T23:59:59"},
                {"column": "longitude", "gt_eq": "%bbox[2]%", "lt_eq": "%bbox[0]%"},
                {"column": "latitude", "gt_eq": "%bbox[3]%", "lt_eq": "%bbox[1]%"},
                {"is_not_null": {"column": "value"}},
                {"column": "depth", "gt_eq": "%depth[0]%", "lt_eq": "%depth[1]%"}
            ],
            "output": {"format": "parquet"}
        });

        let mut query_str = serde_json::to_string(&query).unwrap();

        let year = 2024;
        query_str = query_str.replace("%year%", &year.to_string());

        let depth = [5, 0];
        query_str = query_str.replace("\"%depth[0]%\"", &depth[0].to_string());
        query_str = query_str.replace("\"%depth[1]%\"", &depth[1].to_string());

        let test_query_str = "{\"from\":\"default\",\"select\":[{\"column\":\"TIME\",\"alias\":\"time\"},{\"column\":\"LONGITUDE\",\"alias\":\"longitude\"},{\"column\":\"LATITUDE\",\"alias\":\"latitude\"},{\"column\":\"DEPTH\",\"alias\":\"depth\"},{\"column\":\"TEMPPR01\",\"alias\":\"value\"}],\"filters\":[{\"column\":\"time\",\"gt_eq\":\"%year%-01-01T00:00:00\",\"lt_eq\":\"%year%-12-31T23:59:59\"},{\"column\":\"longitude\",\"gt_eq\":\"%bbox[2]%\",\"lt_eq\":\"%bbox[0]%\"},{\"column\":\"latitude\",\"gt_eq\":\"%bbox[3]%\",\"lt_eq\":\"%bbox[1]%\"},{\"is_not_null\":{\"column\":\"value\"}},{\"column\":\"depth\",\"gt_eq\":\"%depth[0]%\",\"lt_eq\":\"%depth[1]%\"}],\"output\":{\"format\":\"parquet\"}}";

        println!("{}", test_query_str);
    }

    #[tokio::test]
    async fn test_query_file() {

        let layer_config_json = json!(
            {
                    "id": "temperature",
                    "name": "default temperature",
                    "config": {
                        "available_viewparams": {
                            "year": {"type": "numeric", "format": "yyyy"},
                            "depth": {"type": "numeric", "format": ["min", "max"]},
                            "bbox": {"type": "numeric", "format": ["minlon", "minlat", "maxlon", "maxlat"]}
                        },
                        "assigned_viewparams": {
                            "year": 2021,
                            "depth": [0, 10],
                            "bbox": [-180, -90, 180, 90]
                        },
                        "default_style": "thermal",
                        "instance_url": "https://beacon-cdi.maris.nl/",
                        "token": "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJpc3MiOiJodHRwczpcL1wvZGF0YS5ibHVlLWNsb3VkLm9yZyIsImF1ZCI6Imh0dHBzOlwvXC9kYXRhLmJsdWUtY2xvdWQub3JnIiwiaWF0IjoxNzY5NjAxMjYyLCJleHAiOjE4MDExMzcyNjIsInVzciI6MzIsImlkIjoicGF1bEBtYXJpcy5ubCIsImVwX29yZ2FuaXNhdGlvbiI6Ik1BUklTIn0.t3P2PAewHYy4JHdyu0MWnyUzS3MtIZrI5vdAz2tuGmI",
                        "query": {"from":"default","select":[{"column":"TIME","alias":"time"},{"column":"LONGITUDE","alias":"longitude"},{"column":"LATITUDE","alias":"latitude"},{"column":"DEPTH","alias":"depth"},{"column":"TEMPPR01","alias":"value"}],"filters":[{"column":"time","gt_eq":"%year%-01-01T00:00:00","lt_eq":"%year%-12-31T23:59:59"},{"column":"longitude","gt_eq":"%bbox[2]%","lt_eq":"%bbox[0]%"},{"column":"latitude","gt_eq":"%bbox[3]%","lt_eq":"%bbox[1]%"},{"is_not_null":{"column":"value"}},{"column":"depth","gt_eq":"%depth[0]%","lt_eq":"%depth[1]%"}],"output":{"format":"parquet"}},
                        "min_value": -5.0,
                        "max_value": 40.0,
                        "shape": "circle"
                    }
                }
        );

        let layer_config_str = serde_json::to_string(&layer_config_json).unwrap();

        let layer_config: crate::config::LayerConfig = serde_json::from_str(&layer_config_str).unwrap();

        // println!("{:?}", &layer_config);

        let workspace_id = "ihm";
        let layer_id = "temperature";

        let viewparams_hash = layer_config
        .config
        .assigned_viewparams
        .as_ref()
        .map(|vp| misc::hash_viewparams(vp));

        let viewparams_hash = viewparams_hash.as_deref();

        let layer_filepath = match misc::get_layer_filepath(&workspace_id, &layer_id, viewparams_hash) {
            Ok(path) => path,
            Err(e) => {
                log::error!("Error getting layer filepath: {:?}", e);
                String::new() // Return empty string on error, will be filtered out later
            }
        };
        // test the query thingy
        // let file = query_file(layer_filepath, layer_config).await;

        let file = match get_view_params_queried_file(&LOCK_MAP, layer_filepath.clone(), layer_config.clone()).await{
            Ok(f) => f,
            Err(e) => panic!("Error querying file: {}", e),
        };

        println!("file succesfully downloaded {:?}", file);
    }
}