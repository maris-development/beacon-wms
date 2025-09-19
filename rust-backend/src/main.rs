use axum::{
    extract::Query,
    http::{HeaderMap, HeaderValue, Request, Response, StatusCode},
    middleware::{self, Next},
    response::IntoResponse,
    routing::get,
    Router,
};
use serde_json::Value;
use tokio::runtime::Builder;

use crate::{
    boundingbox::BoundingBox,
    color_maps::ColorMapsConfig,
    map_querying::get_feature_info_collection::{Feature, GetFeatureInfoCollection},
    query_parameters::{GetFeatureInfoRequestParameters, GetMapRequestParameters},
    request_profiling::RequestProfiling,
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

fn main() {
    misc::configure_logger();

    let address = misc::get_env_var("ADDRESS", Some("0.0.0.0"));
    let port: u16 = misc::get_env_var("PORT", Some("8000"))
        .parse()
        .expect("Invalid port number (must be u16)");
    let workers: usize = misc::get_env_var("WORKERS", Some("24"))
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

async fn get_map(get_map_params: Query<GetMapRequestParameters>) -> impl IntoResponse {
    log::info!("Get map request: {:?}", get_map_params);

    let mut profiling = RequestProfiling::new();

    let config = misc::read_config_file();

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

    let layers_filepaths = workspace
        .layers
        .iter()
        .filter(|layer| wms_layers.contains(&layer.id))
        .map(|layer| {
            match misc::get_layer_filepath(&workspace.id, &layer.id) {
                Ok(path) => path,
                Err(e) => {
                    log::error!("Error getting layer filepath: {:?}", e);
                    String::new() // Return empty string on error, will be filtered out later
                }
            }
        })
        .filter(|path| !path.is_empty()) // Filter out any empty paths
        .collect::<Vec<String>>();

    let mut styles = match &get_map_params.styles {
        Some(s) => String::from(s),
        None => String::new(),
    };

    if styles.trim().is_empty() {
        // If no styles provided, default to "rainbow" for each layer
        styles = (0..wms_layers.len())
            .map(|_| "thermal")
            .collect::<Vec<_>>()
            .join(",");
    }

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

    let mut image = image_utils::create_rgba_image(get_map_params.width, get_map_params.height);

    let layers_styles_wms_iter = layers_filepaths
        .iter()
        .zip(styles_vec.iter())
        .zip(wms_layers.iter());

    for ((layer_filepath, style), wms_layer) in layers_styles_wms_iter {
        let color_map = match crate::color_maps::ColorMap::get_named(style, -5.0, 40.0, Some(false))
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

        let drawing_result = map_drawing::get_map(
            &mut image,
            bounding_box.clone(),
            wms_layer,
            color_map,
            &get_map_params.crs,
            layer_filepath,
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

    let layers_filepaths = workspace
        .layers
        .iter()
        .filter(|layer| query_layers.contains(&layer.id))
        .map(|layer| {
            match misc::get_layer_filepath(&workspace.id, &layer.id) {
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

async fn update_layers() -> impl IntoResponse {
    let config = misc::read_config_file();

    let mut updated_layer_files: Vec<String> = Vec::new();

    // for each layer in each workspace in config.workspaces,
    if let Some(workspaces) = config.workspaces {
        for workspace in workspaces {
            for layer in workspace.layers {
                // Here you would add the logic to update the layer in your application
                let query = serde_json::to_string(&layer.config.query).unwrap();

                let layer_filepath = match misc::get_layer_filepath(&workspace.id, &layer.id) {
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
