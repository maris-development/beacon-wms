use futures::stream::StreamExt;
use lazy_static::lazy_static;
use serde::Serialize;
use std::collections::HashSet;
use std::time::Instant;
use tokio::sync::Mutex;
use tokio::time::MissedTickBehavior;

use indexmap::IndexMap;

use crate::queries::DatasetMap;
use crate::{config::LayerConfig, misc, queries};

/// One dataset that needs a new query.
#[derive(Clone)]
pub struct RefreshJob {
    pub layer_filepath: String,
    pub layer_config: LayerConfig,
    /// Moment the job entered the queue. The queue endpoint reports the wait time.
    pub queued_at: Instant,
}

lazy_static! {
    /// Datasets that wait for a refresh. The key is the layer file path, so a
    /// dataset can only be in the queue one time. The order is first in, first out.
    static ref QUEUE: Mutex<IndexMap<String, RefreshJob>> = Mutex::new(IndexMap::new());

    /// Datasets that the worker handles at this moment.
    static ref IN_PROGRESS: Mutex<HashSet<String>> = Mutex::new(HashSet::new());

    /// Held while a manual update runs. It keeps manual updates to one at a time.
    static ref MANUAL_UPDATE_LOCK: Mutex<()> = Mutex::new(());
}

/// One queued dataset in the queue report.
#[derive(Serialize)]
pub struct QueueEntry {
    pub layer_filepath: String,
    pub layer_id: String,
    pub layer_name: String,
    /// Seconds the job waits in the queue.
    pub waiting_seconds: u64,
    /// Age of the file on disk. Null when the file is gone or fresh again.
    pub stale_age_seconds: Option<u64>,
}

impl QueueEntry {
    fn from_job(job: &RefreshJob) -> Self {
        QueueEntry {
            layer_filepath: job.layer_filepath.clone(),
            layer_id: job.layer_config.id.clone(),
            layer_name: job.layer_config.name.clone(),
            waiting_seconds: job.queued_at.elapsed().as_secs(),
            stale_age_seconds: queries::stale_age(&job.layer_filepath).map(|age| age.as_secs()),
        }
    }
}

/// State of the refresh queue at one moment.
#[derive(Serialize)]
pub struct QueueStatus {
    pub queued_count: usize,
    pub in_progress_count: usize,
    pub queued: Vec<QueueEntry>,
    /// File paths of the datasets that a worker handles at this moment.
    pub in_progress: Vec<String>,
    pub dataset_ttl_seconds: u64,
    pub refresh_interval_seconds: u64,
}

/// Result of one manual update call.
#[derive(Serialize)]
pub struct UpdateResult {
    /// One of: updated, skipped, failed, empty, busy.
    pub status: &'static str,
    /// The status in words.
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer_filepath: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer_id: Option<String>,
    /// New generation of the dataset. Set only after a successful query.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub duration_seconds: f64,
    /// Jobs that still wait. Call the endpoint again while this is above zero.
    pub queued_count: usize,
}

/// Outcome of one refresh job.
enum JobOutcome {
    Updated(u64),
    Skipped,
    Failed(String),
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
            queued_at: Instant::now(),
        },
    );

    log::info!(
        "Queued refresh for {} ({} jobs in queue)",
        layer_filepath,
        queue.len()
    );

    return true;
}

/// Read the queue and the set of jobs that run at this moment.
pub async fn queue_status() -> QueueStatus {
    let queued: Vec<QueueEntry> = {
        let queue = QUEUE.lock().await;

        queue.values().map(QueueEntry::from_job).collect()
    };

    let mut in_progress: Vec<String> = IN_PROGRESS.lock().await.iter().cloned().collect();

    in_progress.sort();

    QueueStatus {
        queued_count: queued.len(),
        in_progress_count: in_progress.len(),
        queued,
        in_progress,
        dataset_ttl_seconds: misc::get_dataset_ttl().as_secs(),
        refresh_interval_seconds: misc::get_refresh_interval().as_secs(),
    }
}

/// Take the first queued job, run it, and wait for the result.
///
/// The call handles one dataset. Status `empty` means the queue is empty. Status
/// `busy` means another manual update still runs.
pub async fn run_next_job(dataset_map: &DatasetMap) -> UpdateResult {
    let _guard = match MANUAL_UPDATE_LOCK.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            return UpdateResult {
                status: "busy",
                message: "Another manual update still runs. Try again later.".to_string(),
                layer_filepath: None,
                layer_id: None,
                generation: None,
                error: None,
                duration_seconds: 0.0,
                queued_count: queued_count().await,
            }
        }
    };

    let job = match take_next_job().await {
        Some(job) => job,
        None => {
            return UpdateResult {
                status: "empty",
                message: "The refresh queue is empty. There is no work.".to_string(),
                layer_filepath: None,
                layer_id: None,
                generation: None,
                error: None,
                duration_seconds: 0.0,
                queued_count: 0,
            }
        }
    };

    let layer_filepath = job.layer_filepath.clone();
    let layer_id = job.layer_config.id.clone();
    let started = Instant::now();

    log::info!("Manual update starts for {}", &layer_filepath);

    let outcome = run_job(dataset_map, job).await;

    let duration_seconds = started.elapsed().as_secs_f64();
    let queued_count = queued_count().await;

    match outcome {
        JobOutcome::Updated(generation) => UpdateResult {
            status: "updated",
            message: format!("Dataset {} is up to date.", &layer_filepath),
            layer_filepath: Some(layer_filepath),
            layer_id: Some(layer_id),
            generation: Some(generation),
            error: None,
            duration_seconds,
            queued_count,
        },
        JobOutcome::Skipped => UpdateResult {
            status: "skipped",
            message: format!("Dataset {} is fresh already.", &layer_filepath),
            layer_filepath: Some(layer_filepath),
            layer_id: Some(layer_id),
            generation: None,
            error: None,
            duration_seconds,
            queued_count,
        },
        JobOutcome::Failed(error) => UpdateResult {
            status: "failed",
            message: format!("The query for {} failed. The old file stays.", &layer_filepath),
            layer_filepath: Some(layer_filepath),
            layer_id: Some(layer_id),
            generation: None,
            error: Some(error),
            duration_seconds,
            queued_count,
        },
    }
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

/// Number of jobs that wait in the queue.
async fn queued_count() -> usize {
    QUEUE.lock().await.len()
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

/// Move the first queued job to the in progress set and return it.
///
/// The queue lock stays held, so the scheduler cannot take the same job.
async fn take_next_job() -> Option<RefreshJob> {
    let mut queue = QUEUE.lock().await;

    let (_, job) = queue.shift_remove_index(0)?;

    IN_PROGRESS.lock().await.insert(job.layer_filepath.clone());

    Some(job)
}

/// Query Beacon for one dataset and put the result in place.
///
/// A failed query keeps the old file. The next request queues the dataset again.
async fn run_job(dataset_map: &DatasetMap, job: RefreshJob) -> JobOutcome {
    // Another job can have replaced the file already. Then this job has no work.
    if queries::stale_age(&job.layer_filepath).is_none() {
        log::info!(
            "Skipping refresh of {}, the file is fresh",
            &job.layer_filepath
        );

        IN_PROGRESS.lock().await.remove(&job.layer_filepath);

        return JobOutcome::Skipped;
    }

    log::info!("Refreshing dataset {}", &job.layer_filepath);

    let result = queries::fetch_dataset(dataset_map, &job.layer_filepath, &job.layer_config).await;

    IN_PROGRESS.lock().await.remove(&job.layer_filepath);

    match result {
        Ok(generation) => JobOutcome::Updated(generation),
        Err(e) => {
            log::error!("Refresh failed for {}: {}", &job.layer_filepath, e);

            JobOutcome::Failed(e)
        }
    }
}
