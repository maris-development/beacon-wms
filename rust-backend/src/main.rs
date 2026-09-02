use axum::{
    extract::Query,
    http::{HeaderMap, HeaderValue, Request, Response, StatusCode},
    middleware::{self, Next},
    response::IntoResponse,
    routing::get,
    Router,
};
use std::{collections::HashMap, fs, fs::File};
use serde_json::Value;
use tokio::runtime::Builder;
use tokio::io::AsyncReadExt;
use tokio::sync::Semaphore;

use crate::{
    boundingbox::BoundingBox, color_maps::{ColorMap, ColorMapsConfig}, config::LayerConfig, map_querying::get_feature_info_collection::{Feature, GetFeatureInfoCollection}, queries::DatasetMap, query_parameters::{GetFeatureInfoRequestParameters, GetLegendGraphicRequestParameters, GetMapRequestParameters}, request_profiling::RequestProfiling, tile_cache::TileCache
};

pub mod tile_cache;
pub mod beacon_api;
pub mod boundingbox;
pub mod cache_engine;
pub mod color_maps;
pub mod config;
pub mod data_utils;
pub mod errors;
pub mod image_utils;
pub mod legend;
pub mod map_drawing;
pub mod map_querying;
pub mod misc;
pub mod viewparams;
pub mod queries;
pub mod query_parameters;
pub mod refresh;
pub mod request_profiling;

use lazy_static::lazy_static;

lazy_static! {
    /// State of every dataset file: its fetch lock and its generation.
    pub static ref DATASET_MAP: DatasetMap = DatasetMap::default();

    pub static ref TILE_CACHE: TileCache = {
        let tile_cache_dir = misc::get_env_var("TILE_CACHE_DIR", Some("../tile_cache"));

        TileCache::new(tile_cache_dir)
    };

    pub static ref TILE_CACHE_ENABLED: bool = {
        let enabled = misc::get_env_var("TILE_CACHE_ENABLED", Some("false"));
        matches!(enabled.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
    };

    /// Number of map renders that run at the same time. Defaults to the CPU count.
    pub static ref MAP_WORKERS: usize = {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);

        // Docker Compose passes an empty string for a variable that has no value.
        let value = misc::get_env_var("MAP_WORKERS", None);
        let value = value.trim();

        match value.parse::<usize>() {
            Ok(n) if n > 0 => n,
            _ => {
                if !value.is_empty() {
                    log::warn!("Invalid MAP_WORKERS '{}'. Using {}.", value, cores);
                }

                cores
            }
        }
    };

    /// Render slots for GetMap. Other routes take no slot, so GetMap cannot starve them.
    pub static ref MAP_RENDER_SLOTS: Semaphore = Semaphore::new(*MAP_WORKERS);
}



fn main() {
    misc::configure_logger();

    let address = misc::get_env_var("HTTP_ADDRESS", Some("0.0.0.0"));
    let port: u16 = misc::get_env_var("HTTP_PORT", Some("8000"))
        .parse()
        .expect("Invalid port number (must be u16)");
    let workers: usize = misc::get_env_var("WORKERS", Some("4"))
        .parse()
        .expect("Invalid number of workers (must be usize)");

    // The async workers only run protocol and I/O work. Drawing goes to the blocking
    // pool. The extra blocking threads serve feature info and tokio::fs.
    Builder::new_multi_thread()
        .worker_threads(workers)
        .max_blocking_threads(*MAP_WORKERS + 16)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async move {
            log::info!(
                "Starting server on http://{}:{} with {} async workers and {} map workers",
                address,
                port,
                workers,
                *MAP_WORKERS
            );

            // Refresh stale datasets in the background, so requests never wait for it
            tokio::spawn(refresh::run_scheduler(DATASET_MAP.clone()));

            // build our application with a route
            let app = Router::new()
                .route("/", get(index))
                .route("/get-map", get(get_map))
                .route("/get-feature-info", get(get_feature_info))
                .route("/clear-layers", get(clear_layers))
                .route("/queue", get(queue))
                .route("/update", get(update))
                .route("/available-styles", get(available_styles))
                .route("/get-legend-graphic", get(get_legend_graphic))
                .layer(middleware::from_fn(log_middleware));

            let address = format!("{}:{}", address, port);

            let listener = tokio::net::TcpListener::bind(address).await.unwrap();

            axum::serve(listener, app).await.unwrap();
        });

}

async fn log_middleware(req: Request<axum::body::Body>, next: Next) -> Response<axum::body::Body> {
    log::debug!("{} {}", req.method(), req.uri().path());
    next.run(req).await
}

async fn index() -> impl IntoResponse {
    (StatusCode::OK, "Beacon WMS Backend is running")
}

// test query
// http://localhost:3000/workspaces/default/wms?viewparams=year:2024;depth:[-10,-20];bbox[-90,-45,90,45]

async fn get_map(get_map_params: Query<GetMapRequestParameters>) -> impl IntoResponse {

    let cache_extension = misc::get_map_image_extension(&get_map_params.format);

    if *TILE_CACHE_ENABLED {
        if let Some(extension) = cache_extension {
            if let Some(mut cached_file) = TILE_CACHE.is_cached(&get_map_params, extension).await {
                let mut cached_data: Vec<u8> = Vec::new();
                if let Ok(_) = cached_file.read_to_end(&mut cached_data).await {
                    log::debug!("Tile cache hit");
                    return Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", "image/png")
                        .header("Content-Length", cached_data.len().to_string())
                        .header("X-Cache-Hit", "true")
                        .body(axum::body::Body::from(cached_data))
                        .unwrap();
                }
            }
        }
    }

    // log::info!("Get map request: {:?}", get_map_params);

    let mut profiling = RequestProfiling::new();

    let config = match misc::read_config_file() {
        Ok(config) => config,
        Err(e) => {
            log::error!("{}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
        }
    };

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

    let workspace = match config.workspaces.as_ref() {
        Some(workspaces) => workspaces
            .iter()
            .find(|ws| ws.id == get_map_params.workspace)
            .cloned()
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

    let requested_styles = get_map_params
        .styles
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<String>>();

    // Without requested styles, fall back to the default style of each layer.
    let styles_vec = if requested_styles.is_empty() {
        layers_configs
            .iter()
            .map(|layer| {
                layer
                    .config
                    .default_style
                    .clone()
                    .unwrap_or_else(|| String::from("thermal"))
            })
            .collect::<Vec<String>>()
    } else {
        requested_styles
    };

    if styles_vec.len() != layers_configs.len() {
        return (
            StatusCode::BAD_REQUEST,
            "Number of styles must match number of layers".to_string(),
        )
            .into_response();
    }

    profiling.mark("query parsed");

    let mut draw_jobs: Vec<LayerDrawJob> = Vec::with_capacity(layers_configs.len());

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

        let (file, generation) = match queries::get_dataset_file(&DATASET_MAP, layer_filepath.clone(), layer_config.clone()).await{
            Ok(f) => f,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        };

        // what do these min/max values do? why not in viewparams? why in layer config?
        let min_value = layer_config.config.min_value.unwrap_or(-10.0);
        let max_value = layer_config.config.max_value.unwrap_or(100.0);
        let log_style = layer_config.config.log_style;

        let color_map = match crate::color_maps::ColorMap::get_named(style, min_value, max_value, log_style)
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
            Some(shape) => shape.clone(),
            None => String::from("circle"),
        };

        draw_jobs.push(LayerDrawJob {
            filepath: layer_filepath,
            file,
            generation,
            color_map,
            shape: icon_shape,
            wms_layer: wms_layer.clone(),
        });
    }

    // The dataset files are ready. Take a render slot only for the draw work itself.
    let permit = match MAP_RENDER_SLOTS.acquire().await {
        Ok(permit) => permit,
        Err(e) => {
            log::error!("Render slots closed: {}", e);
            return (StatusCode::SERVICE_UNAVAILABLE, "Server shutting down").into_response();
        }
    };

    profiling.mark("render slot acquired");

    let bbox = bounding_box;
    let crs = get_map_params.crs.clone();
    let width = get_map_params.width;
    let height = get_map_params.height;

    let render_result = tokio::task::spawn_blocking(move || {
        render_png(draw_jobs, bbox, crs, width, height, profiling)
    })
    .await;

    drop(permit);

    let png_data = match render_result {
        Ok(Ok(png_data)) => png_data,
        Ok(Err(e)) => {
            log::error!("{}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
        }
        Err(e) => {
            log::error!("Render task failed: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Render task failed").into_response();
        }
    };

    if *TILE_CACHE_ENABLED {
        if let Some(extension) = cache_extension {
            if let Err(e) = TILE_CACHE.cache_tile(&get_map_params, &png_data, extension).await {
                log::warn!("Failed to write tile cache: {}", e);
            }
        }
    }

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "image/png")
        .header("Content-Length", png_data.len().to_string())
        .header("X-Cache-Hit", "false")
        .body(axum::body::Body::from(png_data))
        .unwrap()
}

/// One layer that is ready to draw. The async part resolves it, the blocking pool draws it.
struct LayerDrawJob {
    filepath: String,
    file: File,
    generation: u64,
    color_map: ColorMap,
    shape: String,
    wms_layer: String,
}

/// Draw every layer on one image and encode the PNG.
///
/// This function is synchronous and CPU bound. Run it on the blocking pool, never on
/// an async worker thread. All dataset files are open before the call, so the function
/// never waits for the network.
fn render_png(
    jobs: Vec<LayerDrawJob>,
    bounding_box: BoundingBox,
    crs: String,
    width: u32,
    height: u32,
    mut profiling: RequestProfiling,
) -> Result<Vec<u8>, String> {
    let mut image: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> =
        image_utils::create_rgba_image(width, height);

    for job in jobs {
        let LayerDrawJob {
            filepath,
            file,
            generation,
            color_map,
            shape,
            wms_layer,
        } = job;

        map_drawing::get_map(
            &mut image,
            bounding_box.clone(),
            color_map,
            &crs,
            filepath,
            file,
            generation,
            &shape,
            &mut profiling,
        )
        .map_err(|e| format!("Error drawing map: {:?}", e))?;

        profiling.mark(&format!("drawn {}", wms_layer));
    }

    let mut png_data: Vec<u8> = Vec::new();

    image_utils::rgba_image_to_png(&image, &mut png_data)
        .map_err(|e| format!("Error encoding PNG: {:?}", e))?;

    profiling.mark("image encoded");

    // profiling.log_report(); // --> get profiling report in logs

    Ok(png_data)
}

async fn get_feature_info(
    get_feature_info_params: Query<GetFeatureInfoRequestParameters>,
) -> impl IntoResponse {
    log::info!("Get feature info request: {:?}", get_feature_info_params);

    let config = match misc::read_config_file() {
        Ok(config) => config,
        Err(e) => {
            log::error!("{}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
        }
    };

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

    let workspace = match config.workspaces.as_ref() {
        Some(workspaces) => workspaces
            .iter()
            .find(|ws| ws.id == get_feature_info_params.workspace)
            .cloned()
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

    let mut query_jobs: Vec<FeatureInfoJob> = Vec::with_capacity(layers_configs.len());

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
        
        // log::info!("Getting feature info for layer {}, file path: {}", layer_config.id, layer_filepath);

        // get_feature_info reads the parquet directly, so it needs no generation
        let (file, _generation) = match queries::get_dataset_file(&DATASET_MAP, layer_filepath.clone(), layer_config.clone()).await{
            Ok(f) => f,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        };

        query_jobs.push(FeatureInfoJob {
            filepath: layer_filepath,
            file,
        });
    }

    let bbox = bounding_box;
    let crs = get_feature_info_params.crs.clone();

    // The parquet hit test is synchronous. It takes no render slot, so a burst of
    // GetMap requests cannot delay it.
    let query_result = tokio::task::spawn_blocking(move || {
        query_features(
            query_jobs,
            image_dimensions,
            click_coordinates,
            bbox,
            crs,
            feature_count,
        )
    })
    .await;

    let feature_info_results = match query_result {
        Ok(Ok(features)) => features,
        Ok(Err(e)) => {
            log::error!("{}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
        }
        Err(e) => {
            log::error!("Feature info task failed: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Feature info task failed")
                .into_response();
        }
    };

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

/// One layer that is ready for the hit test.
struct FeatureInfoJob {
    filepath: String,
    file: File,
}

/// Run the hit test on every layer and collect the features.
///
/// This function is synchronous. It reads parquet, so it runs on the blocking pool.
fn query_features(
    jobs: Vec<FeatureInfoJob>,
    image_dimensions: (u32, u32),
    click_coordinates: (u32, u32),
    bounding_box: BoundingBox,
    crs: String,
    feature_count: u32,
) -> Result<Vec<Feature>, String> {
    let mut results: Vec<Feature> = Vec::new();

    for job in jobs {
        let mut features = map_querying::get_feature_info(
            image_dimensions,
            click_coordinates,
            bounding_box.clone(),
            &crs,
            feature_count,
            &job.filepath,
            job.file,
        )
        .map_err(|e| format!("Error getting feature info: {:?}", e))?;

        results.append(&mut features);
    }

    Ok(results)
}

async fn clear_layers() -> impl IntoResponse {
    let layer_dir = misc::get_layer_directory();
    let all_parquet_files = misc::get_parquet_files(&layer_dir);

    for file in &all_parquet_files {
        if let Err(e) = fs::remove_file(file) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to delete {}: {}", file, e),
            );
        }
    }

    (
        StatusCode::OK,
        format!("Layer data cleared: {:?}", all_parquet_files),
    )
}

/// Report the datasets that wait for a refresh, and the ones that run now.
async fn queue() -> impl IntoResponse {
    json_response(StatusCode::OK, &refresh::queue_status().await)
}

/// Refresh one queued dataset and wait for the result.
///
/// The request stays open for the whole Beacon query, so it can take minutes. Call
/// it again while `queued_count` in the response is above zero.
async fn update() -> impl IntoResponse {
    let result = refresh::run_next_job(&DATASET_MAP).await;

    let status = match result.status {
        "failed" => StatusCode::BAD_GATEWAY,
        "busy" => StatusCode::CONFLICT,
        _ => StatusCode::OK,
    };

    json_response(status, &result)
}

fn json_response<T: serde::Serialize>(
    status: StatusCode,
    body: &T,
) -> Response<axum::body::Body> {
    let json = match serde_json::to_string(body) {
        Ok(json) => json,
        Err(e) => {
            log::error!("Error serializing response: {:?}", e);

            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Error serializing response",
            )
                .into_response();
        }
    };

    let mut headers = HeaderMap::new();

    headers.insert("Content-Type", HeaderValue::from_static("application/json"));

    (status, headers, json).into_response()
}


async fn get_legend_graphic(
    params: Query<GetLegendGraphicRequestParameters>,
) -> impl IntoResponse {
    let config = match misc::read_config_file() {
        Ok(config) => config,
        Err(e) => {
            log::error!("{}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
        }
    };

    let workspace = match config.workspaces.as_ref() {
        Some(workspaces) => workspaces
            .iter()
            .find(|ws| ws.id == params.workspace)
            .cloned()
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    format!("Workspace not found: {}", params.workspace),
                )
            }),
        None => {
            return (StatusCode::BAD_REQUEST, "No workspaces configured".to_string())
                .into_response();
        }
    };

    let workspace = match workspace {
        Ok(ws) => ws,
        Err(e) => return e.into_response(),
    };

    let layer_config = match workspace.layers.iter().find(|l| l.id == params.layer) {
        Some(l) => l,
        None => {
            return (
                StatusCode::NOT_FOUND,
                format!("Layer not found in workspace {}: {}", workspace.id, params.layer),
            )
                .into_response();
        }
    };

    let style = params
        .style
        .as_deref()
        .or(layer_config.config.default_style.as_deref())
        .unwrap_or("thermal");

    let min_value = layer_config.config.min_value.unwrap_or(-10.0);
    let max_value = layer_config.config.max_value.unwrap_or(100.0);
    let log_style = layer_config.config.log_style;

    let color_map = match crate::color_maps::ColorMap::get_named(style, min_value, max_value, log_style) {
        Some(map) => map,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                format!("Style not found: {}", style),
            )
                .into_response();
        }
    };

    let width = params.width.unwrap_or(20);
    let height = params.height.unwrap_or(200);

    let image = legend::draw_legend_graphic(&color_map, width, height);

    let mut png_data: Vec<u8> = Vec::new();
    match image_utils::rgba_image_to_png(&image, &mut png_data) {
        Ok(_) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "image/png")
            .header("Content-Length", png_data.len().to_string())
            .body(axum::body::Body::from(png_data))
            .unwrap(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Error encoding PNG: {:?}", e),
        )
            .into_response(),
    }
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


