use log;
use std::{
    collections::HashMap,
    fs::File,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::Mutex;

use crate::{beacon_api, config::LayerConfig, misc, refresh, viewparams};

/// State of one dataset file.
///
/// The key is the layer file path. The file handle is not kept, because a refresh
/// replaces the file on disk. A stored handle keeps the old data visible.
pub struct DatasetEntry {
    /// Held while a fetch runs. Concurrent misses wait instead of querying twice.
    fetch_lock: Arc<Mutex<()>>,
    /// Increases after every successful fetch. Part of the reprojection cache key.
    generation: u64,
}

impl DatasetEntry {
    fn new() -> Self {
        DatasetEntry {
            fetch_lock: Arc::new(Mutex::new(())),
            generation: 0,
        }
    }
}

pub type DatasetMap = Arc<Mutex<HashMap<String, DatasetEntry>>>;

/// Makes each temporary download file name unique.
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Get the dataset file for a layer.
///
/// The function returns the file that is on disk. If that file is older than the
/// TTL, the function adds a refresh job to the queue and still returns the old file.
/// Only a missing file makes the caller wait for a Beacon query.
///
/// The returned generation belongs to the returned file. Callers use it in cache keys.
pub async fn get_dataset_file(
    dataset_map: &DatasetMap,
    layer_filepath: String,
    layer_config: LayerConfig,
) -> Result<(File, u64), String> {
    let fetch_lock = get_fetch_lock(dataset_map, &layer_filepath).await;

    if let Some(result) = open_current(dataset_map, &layer_filepath).await {
        if stale_age(&layer_filepath).is_some() {
            refresh::enqueue(&layer_filepath, &layer_config).await;
        }

        return result;
    }

    // No file on disk. This caller must wait for the query.
    let _guard = fetch_lock.lock().await;

    // Another task can have fetched the file while this task waited for the lock.
    if let Some(result) = open_current(dataset_map, &layer_filepath).await {
        return result;
    }

    fetch_dataset(dataset_map, &layer_filepath, &layer_config).await?;

    match open_current(dataset_map, &layer_filepath).await {
        Some(result) => result,
        None => Err(format!(
            "Dataset file '{}' is missing after the query",
            &layer_filepath
        )),
    }
}

/// Query Beacon and put the result at the layer file path.
///
/// The result goes to a temporary file first. A rename then puts it in place. The
/// rename is atomic, so a reader never sees a part of the new file. The generation
/// of the dataset increases with the same lock that readers use, so a reader always
/// gets a file and a generation that belong together.
///
/// The function returns the new generation.
pub async fn fetch_dataset(
    dataset_map: &DatasetMap,
    layer_filepath: &str,
    layer_config: &LayerConfig,
) -> Result<u64, String> {
    let assigned_viewparams = layer_config.config.assigned_viewparams.as_ref();

    let query_str_raw: String = serde_json::to_string(&layer_config.config.query)
        .map_err(|e| format!("Error serializing query: {:?}", e))?;

    // apply the view params of the layer to the beacon query
    let query_str = viewparams::apply_viewparams_to_query(query_str_raw, assigned_viewparams);

    log::info!("Updating layer at path: {:?}", layer_filepath);

    let temp_filepath = temp_filepath_for(layer_filepath);

    let instance_url = &layer_config.config.instance_url;
    let auth_token = misc::get_env_var("BEACON_TOKEN", None);

    let result = beacon_api::query(
        &query_str,
        instance_url,
        auth_token.as_str(),
        &temp_filepath,
    )
    .await;

    if let Err(e) = result {
        let _ = tokio::fs::remove_file(&temp_filepath).await;
        return Err(format!("Error updating '{}': {:?}", layer_filepath, e));
    }

    let generation = commit_dataset(dataset_map, &temp_filepath, layer_filepath).await?;

    log::info!(
        "Layer updated: {:?}, generation is now {}",
        layer_filepath,
        generation
    );

    Ok(generation)
}

/// Move the new file in place and increase the generation, both under the map lock.
async fn commit_dataset(
    dataset_map: &DatasetMap,
    temp_filepath: &str,
    layer_filepath: &str,
) -> Result<u64, String> {
    let mut map = dataset_map.lock().await;

    if let Err(e) = tokio::fs::rename(temp_filepath, layer_filepath).await {
        let _ = tokio::fs::remove_file(temp_filepath).await;

        return Err(format!(
            "Error moving '{}' to '{}': {:?}",
            temp_filepath, layer_filepath, e
        ));
    }

    let entry = map
        .entry(layer_filepath.to_string())
        .or_insert_with(DatasetEntry::new);

    entry.generation += 1;

    Ok(entry.generation)
}

/// Open the dataset file and read its generation under the map lock.
///
/// The lock makes the pair consistent: a refresh cannot replace the file between
/// the open call and the read of the generation.
///
/// Returns None when there is no file on disk yet.
async fn open_current(
    dataset_map: &DatasetMap,
    layer_filepath: &str,
) -> Option<Result<(File, u64), String>> {
    let map = dataset_map.lock().await;

    let file = match File::open(layer_filepath) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            return Some(Err(format!(
                "Error opening file '{}': {:?}",
                layer_filepath, e
            )))
        }
    };

    let generation = map.get(layer_filepath).map(|e| e.generation).unwrap_or(0);

    Some(Ok((file, generation)))
}

/// Age of the dataset file, but only when it passed the TTL. Otherwise None.
pub fn stale_age(layer_filepath: &str) -> Option<Duration> {
    let metadata = std::fs::metadata(layer_filepath).ok()?;
    let modified = metadata.modified().ok()?;
    let age = modified.elapsed().unwrap_or(Duration::ZERO);

    if age < misc::get_dataset_ttl() {
        return None;
    }

    Some(age)
}

/// Get the fetch lock of a dataset. The lock keeps concurrent cold misses to one query.
async fn get_fetch_lock(dataset_map: &DatasetMap, layer_filepath: &str) -> Arc<Mutex<()>> {
    let mut map = dataset_map.lock().await;

    let entry = map
        .entry(layer_filepath.to_string())
        .or_insert_with(DatasetEntry::new);

    entry.fetch_lock.clone()
}

fn temp_filepath_for(layer_filepath: &str) -> String {
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);

    format!("{}.{}.tmp", layer_filepath, counter)
}
