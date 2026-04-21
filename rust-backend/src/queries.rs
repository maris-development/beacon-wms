use log;
use std::{collections::HashMap, fs::File, future::Future, sync::Arc};
use tokio::sync::{Mutex, OnceCell};
use crate::{beacon_api, config::LayerConfig, misc, viewparams};

type LockMap = Arc<Mutex<HashMap<String, Arc<OnceCell<File>>>>>;

/// t1 => lock
/// t1 => file bestaat niet => insert view params & once cell with F -> T
/// t1 => clone the arc -> unlocked/drop -> get_or_init 
/// t2 => lock -> ziet de viewparams staan met once cell -> get_or_init -> synced met t1
/// t2 => unlocked -> arc<once<f>>
/// t3 => lock -> zit nog niet in -> 
pub async fn get_dataset_file(
    lock_map: &LockMap,
    layer_filepath: String,
    layer_config: LayerConfig
) -> Result<File, String> {
    get_or_execute_dataset(
        lock_map,
        layer_filepath.clone(),
        move|| query_file(layer_filepath, layer_config)
    ).await
}


async fn get_or_execute_dataset<F : FnOnce() -> Fut, Fut: Future<Output = Result<File, String>>>(lock_map: &LockMap, view_params_key: String, fut: F) -> Result<File,String> {
    // let key_for_log = view_params_key.clone();
    let mut locked_map = lock_map.lock().await;
    let once_cell= locked_map.entry(view_params_key).or_insert(Arc::new(OnceCell::new()));
    let resolved = once_cell.get_or_try_init(fut).await;
    // log::info!("Fetching file for viewparams key: {}", key_for_log);
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
        let query_str = viewparams::apply_viewparams_to_query(
            query_str_raw,
            assigned_viewparams,
        );

        // Log the layer and filepath
        log::info!("Updating layer at path: {:?}", &layer_filepath);

        //check if file exists and is less than one day old
        if let Ok(metadata) = std::fs::metadata(&layer_filepath) {
            if let Ok(modified) = metadata.modified() {
                if modified.elapsed().unwrap_or(std::time::Duration::from_secs(0)) < std::time::Duration::from_secs(24 * 60 * 60) {
                    log::info!("File {:?} is less than one day old, skipping update", &layer_filepath);
                    return File::open(layer_filepath).map_err(|e| format!("Error opening file: {:?}", e));
                }
            }
        }

        // run query
        let instance_url = &layer_config.config.instance_url;
        let auth_token = misc::get_env_var("BEACON_TOKEN", None);

        beacon_api::query(
            &query_str,
            instance_url,
            auth_token.as_str(),
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