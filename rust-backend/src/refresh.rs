use futures::stream::StreamExt;
use lazy_static::lazy_static;
use std::collections::HashSet;
use tokio::sync::Mutex;
use tokio::time::MissedTickBehavior;

use indexmap::IndexMap;

use crate::{config::LayerConfig, misc, queries};
use crate::queries::DatasetMap;

/// One dataset that needs a new query.
#[derive(Clone)]
pub struct RefreshJob {
    pub layer_filepath: String,
    pub layer_config: LayerConfig,
}

lazy_static! {
    /// Datasets that wait for a refresh. The key is the layer file path, so a
    /// dataset can only be in the queue one time. The order is first in, first out.
    static ref QUEUE: Mutex<IndexMap<String, RefreshJob>> = Mutex::new(IndexMap::new());

    /// Datasets that the worker handles at this moment.
    static ref IN_PROGRESS: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
}

/// Add a dataset to the refresh queue.
///
/// The function returns immediately. It does no I/O and never blocks the request.
pub async fn enqueue(layer_filepath: &str, layer_config: &LayerConfig) -> bool {
    if IN_PROGRESS.lock().await.contains(layer_filepath) {
        return false;
    }

    let mut queue = QUEUE.lock().await;

    if queue.contains_key(layer_filepath) {
        return false;
    }

    queue.insert(
        layer_filepath.to_string(),
        RefreshJob {
            layer_filepath: layer_filepath.to_string(),
            layer_config: layer_config.clone(),
        },
    );

    log::info!(
        "Queued refresh for {} ({} jobs in queue)",
        layer_filepath,
        queue.len()
    );

    return true;
}

/// Run the refresh worker until the process stops.
///
/// The worker wakes up every REFRESH_INTERVAL_SECONDS and handles all queued jobs.
pub async fn run_scheduler(dataset_map: DatasetMap) {
    let refresh_interval = misc::get_refresh_interval();
    let concurrency = misc::get_refresh_concurrency();

    log::info!(
        "Refresh worker started, interval: {}s, concurrency: {}, dataset TTL: {}s",
        refresh_interval.as_secs(),
        concurrency,
        misc::get_dataset_ttl().as_secs()
    );

    let mut interval = tokio::time::interval(refresh_interval);

    // Do not run extra ticks after a long round of jobs.
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        interval.tick().await;

        let jobs = take_queued_jobs().await;

        if jobs.is_empty() {
            continue;
        }

        log::info!("Refresh worker starts {} job(s)", jobs.len());

        // for_each_concurrent keeps at most `concurrency` queries active
        futures::stream::iter(jobs)
            .for_each_concurrent(concurrency, |job| {
                let dataset_map = dataset_map.clone();

                async move {
                    run_job(&dataset_map, job).await;
                }
            })
            .await;

        log::info!("Refresh worker is done");
    }
}

/// Move all queued jobs to the in progress set and return them.
async fn take_queued_jobs() -> Vec<RefreshJob> {
    let mut queue = QUEUE.lock().await;

    if queue.is_empty() {
        return Vec::new();
    }

    let jobs: Vec<RefreshJob> = queue.drain(..).map(|(_, job)| job).collect();

    let mut in_progress = IN_PROGRESS.lock().await;

    for job in &jobs {
        in_progress.insert(job.layer_filepath.clone());
    }

    jobs
}

/// Query Beacon for one dataset and put the result in place.
///
/// A failed query keeps the old file. The next request queues the dataset again.
async fn run_job(dataset_map: &DatasetMap, job: RefreshJob) {
    // Another job can have replaced the file already. Then this job has no work.
    if queries::stale_age(&job.layer_filepath).is_none() {
        log::info!(
            "Skipping refresh of {}, the file is fresh",
            &job.layer_filepath
        );

        IN_PROGRESS.lock().await.remove(&job.layer_filepath);

        return;
    }

    log::info!("Refreshing dataset {}", &job.layer_filepath);

    if let Err(e) = queries::fetch_dataset(dataset_map, &job.layer_filepath, &job.layer_config).await
    {
        log::error!("Refresh failed for {}: {}", &job.layer_filepath, e);
    }

    IN_PROGRESS.lock().await.remove(&job.layer_filepath);
}
