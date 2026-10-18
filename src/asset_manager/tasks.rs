use super::ratelimiter::RateLimiter;
use super::uploader::AssetUploader;
use bytes::Bytes;
use roboat::RoboatError;
use roboat::assetdelivery::AssetBatchResponse;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::Duration;

use tracing::{error, info, warn};
const MAX_UPLOAD_RETRIES: usize = 5;

pub type UploadResult = Result<(Option<String>, String), (Option<String>, RoboatError)>;
pub type ProgressCallback = Arc<dyn Fn(usize, usize, &str, &str) + Send + Sync>;

// ============================================================================
// Upload Task Management
// ============================================================================
#[derive(Clone)]
pub struct UploadContext {
    pub uploader: Arc<AssetUploader>,
    pub group_id: Option<u64>,
    pub semaphore: Arc<Semaphore>,
    pub rate_limiter: Arc<RateLimiter>,
    pub total_assets: usize,
    pub completed_count: Arc<AtomicUsize>,
    /// When true, reuploaded animations keep the original asset's display name.
    pub keep_names: bool,
    pub on_progress: Option<ProgressCallback>,
}

impl UploadContext {
    pub fn new(
        uploader: Arc<AssetUploader>,
        group_id: Option<u64>,
        max_concurrent_tasks: u64,
        total_assets: usize,
        keep_names: bool,
        on_progress: Option<ProgressCallback>,
    ) -> Self {
        let concurrency = max_concurrent_tasks.max(1) as usize;
        Self {
            rate_limiter: Arc::clone(&uploader.rate_limiter),
            uploader,
            group_id,
            semaphore: Arc::new(Semaphore::new(concurrency)),
            total_assets,
            completed_count: Arc::new(AtomicUsize::new(0)),
            keep_names,
            on_progress,
        }
    }

    /// Spawns all upload tasks for concurrent asset uploads into a JoinSet
    pub fn spawn_upload_tasks(
        &self,
        asset_batch: Vec<AssetBatchResponse>,
    ) -> JoinSet<UploadResult> {
        let mut set = JoinSet::new();

        for asset in asset_batch {
            let Some(location) = asset
                .locations
                .as_ref()
                .and_then(|locs| locs.first())
                .and_then(|loc| loc.location.as_ref())
                .cloned()
            else {
                continue;
            };

            let Some(asset_type) = asset.asset_type else {
                continue;
            };

            let ctx = self.clone();
            let request_id = asset.request_id;

            set.spawn(async move {
                // Wait for rate limit before acquiring permit so permits aren't hogged during sleep
                ctx.rate_limiter.wait_if_limited().await;
                let _permit = ctx
                    .semaphore
                    .acquire()
                    .await
                    .map_err(|_| (request_id.clone(), RoboatError::InternalServerError))?;

                // Optionally fetch the original asset's name so the reupload keeps it.
                let asset_name = if ctx.keep_names {
                    match request_id.as_deref().and_then(|id| id.parse::<u64>().ok()) {
                        Some(asset_id) => ctx.uploader.fetch_asset_display_name(asset_id).await,
                        None => None,
                    }
                } else {
                    None
                };

                let asset_bytes: Bytes = ctx
                    .uploader
                    .file_bytes_from_url(&location)
                    .await
                    .map_err(|e| (request_id.clone(), e))?;

                ctx.rate_limiter.wait_if_limited().await;

                let old_id_str = request_id.clone().unwrap_or_else(|| "unknown".to_string());

                let new_asset_id = ctx
                    .upload_asset_with_retry(asset_bytes, asset_type, &old_id_str, asset_name)
                    .await
                    .map_err(|e| (request_id.clone(), e))?;

                Ok((request_id, new_asset_id))
            });
        }

        set
    }

    /// Uploads asset with automatic retry logic for rate limits and server errors
    async fn upload_asset_with_retry(
        &self,
        asset_bytes: Bytes,
        asset_type: roboat::catalog::AssetType,
        request_id: &str,
        asset_name: Option<String>,
    ) -> Result<String, RoboatError> {
        let mut last_error = None;

        for attempt in 1..=MAX_UPLOAD_RETRIES {
            match self
                .uploader
                .upload_asset(asset_bytes.clone(), self.group_id, asset_name.clone())
                .await
            {
                Ok(new_asset_id) => {
                    let done = self.completed_count.fetch_add(1, Ordering::Relaxed) + 1;
                    let remaining = self.total_assets.saturating_sub(done);
                    info!(
                        "[{:?}] Success uploading {} -> {} - {}/{} ({} remaining)",
                        asset_type, request_id, new_asset_id, done, self.total_assets, remaining
                    );
                    if let Some(ref cb) = self.on_progress {
                        cb(done, self.total_assets, request_id, &new_asset_id);
                    }
                    return Ok(new_asset_id);
                }
                Err(e) => {
                    warn!(
                        "Upload attempt {}/{} failed for {:?} {}: {}",
                        attempt, MAX_UPLOAD_RETRIES, asset_type, request_id, e
                    );

                    if matches!(
                        e,
                        RoboatError::TooManyRequests | RoboatError::InternalServerError
                    ) {
                        let sleep_time = (attempt as u64) * 30;
                        self.rate_limiter.set_rate_limit(sleep_time).await;
                        self.rate_limiter.wait_if_limited().await;
                    }

                    last_error = Some(e);

                    if attempt < MAX_UPLOAD_RETRIES {
                        tokio::time::sleep(Duration::from_millis(1000)).await;
                    } else {
                        error!("Failed to upload {:?} {}", asset_type, request_id);
                    }
                }
            }
        }

        Err(last_error.unwrap_or(RoboatError::InternalServerError))
    }
}

/// Collects results from all upload tasks as they finish (streaming completion)
pub async fn collect_upload_results(
    mut tasks: JoinSet<UploadResult>,
) -> Result<HashMap<String, String>, RoboatError> {
    let mut finished_asset_hashmap = HashMap::new();
    let mut errors = Vec::new();
    let total_tasks = tasks.len();

    while let Some(res) = tasks.join_next().await {
        match res {
            Ok(Ok((Some(request_id), new_asset_id))) => {
                finished_asset_hashmap.insert(request_id, new_asset_id);
            }
            Ok(Ok((None, _))) => {
                warn!("Upload succeeded but no request_id available");
            }
            Ok(Err((request_id, e))) => {
                if matches!(e, RoboatError::BadRequest) {
                    error!(
                        "Failed to upload asset: {:?} (cookie may lack required permissions)",
                        request_id
                    );
                }
                errors.push(e);
            }
            Err(join_error) => {
                error!("Task execution failed: {}", join_error);
            }
        }
    }

    info!(
        "Upload summary: {} succeeded, {} failed out of {} total tasks",
        finished_asset_hashmap.len(),
        errors.len(),
        total_tasks
    );

    Ok(finished_asset_hashmap)
}
