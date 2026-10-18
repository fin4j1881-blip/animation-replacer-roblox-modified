use bytes::Bytes;
use roboat::ClientBuilder;
use roboat::RoboatError;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{info, warn};

use super::ratelimiter::RateLimiter;
use super::tasks::{ProgressCallback, UploadContext, collect_upload_results};

const DEFAULT_CONCURRENT_TASKS: u64 = 50;
const DEFAULT_ANIMATION_NAME: &str = "reuploaded_animation";
const DEFAULT_ANIMATION_DESCRIPTION: &str = "This is an example";
const MAX_NAME_FETCH_RETRIES: usize = 3;
const MAX_POLL_ATTEMPTS: usize = 60;
const UPLOAD_TIMEOUT_SECS: u64 = 30;

pub struct AssetUploader {
    pub roblosecurity: String,
    pub api_key: String,
    pub(super) rate_limiter: Arc<RateLimiter>,
    pub(super) http_client: reqwest::Client,
    pub(super) roboat_client: roboat::Client,
    pub(super) place_id_cache: RwLock<HashMap<u64, u64>>,
    pub(super) failed_creators: RwLock<HashSet<u64>>,
    pub(super) xcsrf_token: RwLock<String>,
}

impl fmt::Debug for AssetUploader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssetUploader")
            .field("roblosecurity", &"[REDACTED]")
            .field(
                "api_key",
                if self.api_key.is_empty() {
                    &"[EMPTY]"
                } else {
                    &"[REDACTED]"
                },
            )
            .finish()
    }
}

impl AssetUploader {
    /// Creates a new AssetUploader with a roblosecurity cookie and optional Open Cloud API key.
    pub fn new(roblosecurity: String, api_key: String) -> Self {
        let http_client = reqwest::Client::builder()
            .pool_max_idle_per_host(50)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(60))
            .build()
            .unwrap_or_default();

        let roboat_client = ClientBuilder::new()
            .roblosecurity(roblosecurity.clone())
            .reqwest_client(http_client.clone())
            .build();

        Self {
            roblosecurity,
            api_key,
            rate_limiter: Arc::new(RateLimiter::new()),
            http_client,
            roboat_client,
            place_id_cache: RwLock::new(HashMap::new()),
            failed_creators: RwLock::new(HashSet::new()),
            xcsrf_token: RwLock::new(String::new()),
        }
    }

    /// Fetches the display name of an asset so reuploads can keep the original
    /// name. Returns `None` if the asset cannot be fetched.
    pub async fn fetch_asset_display_name(&self, asset_id: u64) -> Option<String> {
        for attempt in 1..=MAX_NAME_FETCH_RETRIES {
            self.rate_limiter.wait_if_limited().await;
            match self.roboat_client.get_asset_info(asset_id).await {
                Ok(info) => {
                    let name = info.display_name.trim().to_string();
                    if name.is_empty() {
                        return None;
                    }
                    return Some(name);
                }
                Err(RoboatError::TooManyRequests) => {
                    let sleep_time = (attempt as u64) * 5;
                    warn!(
                        "Rate limited fetching name for asset {} (attempt {}/{}), \
                         sleeping {} seconds...",
                        asset_id, attempt, MAX_NAME_FETCH_RETRIES, sleep_time
                    );
                    self.rate_limiter.set_rate_limit(sleep_time).await;
                    self.rate_limiter.wait_if_limited().await;
                }
                Err(e) => {
                    warn!("Could not fetch name for asset {}: {}", asset_id, e);
                    return None;
                }
            }
        }

        warn!(
            "Gave up fetching name for asset {} after {} attempts",
            asset_id, MAX_NAME_FETCH_RETRIES
        );
        None
    }

    /// Uploads an animation using only the .ROBLOSECURITY cookie via Roblox Studio IDE publish endpoint
    /// when no Open Cloud API key is provided.
    pub async fn upload_asset_via_cookie(
        &self,
        asset_data: Bytes,
        group_id: Option<u64>,
        asset_name: Option<String>,
    ) -> Result<String, RoboatError> {
        let name = asset_name.unwrap_or_else(|| DEFAULT_ANIMATION_NAME.to_string());
        let mut url = reqwest::Url::parse("https://www.roblox.com/ide/publish/UploadNewAnimation")
            .expect("valid static url");
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("assetTypeName", "Animation");
            query.append_pair("name", &name);
            query.append_pair("description", DEFAULT_ANIMATION_DESCRIPTION);
            query.append_pair("AllID", "1");
            query.append_pair("ispublic", "False");
            query.append_pair("allowComments", "True");
            query.append_pair("isGamesAsset", "False");
            if let Some(gid) = group_id {
                query.append_pair("groupId", &gid.to_string());
            }
        }

        let cookie_header = format!(".ROBLOSECURITY={}", self.roblosecurity);
        let mut xcsrf = self.xcsrf_token.read().await.clone();

        for _ in 0..2 {
            let mut req = self
                .http_client
                .post(url.clone())
                .timeout(Duration::from_secs(UPLOAD_TIMEOUT_SECS))
                .header("Cookie", &cookie_header)
                .header("User-Agent", "RobloxStudio/WinInet")
                .header("Content-Type", "application/octet-stream")
                .header("Requester", "Client");

            if !xcsrf.is_empty() {
                req = req.header("X-CSRF-TOKEN", &xcsrf);
            }

            let resp = req
                .body(asset_data.clone())
                .send()
                .await
                .map_err(RoboatError::ReqwestError)?;

            if resp.status().as_u16() == 403 {
                if let Some(new_token) = resp
                    .headers()
                    .get("x-csrf-token")
                    .and_then(|v| v.to_str().ok())
                {
                    xcsrf = new_token.to_string();
                    *self.xcsrf_token.write().await = xcsrf.clone();
                    continue;
                }
                return Err(RoboatError::InvalidRoblosecurity);
            }

            if resp.status().as_u16() == 429 {
                return Err(RoboatError::TooManyRequests);
            }

            if resp.status().is_server_error() {
                return Err(RoboatError::InternalServerError);
            }

            if !resp.status().is_success() {
                return Err(RoboatError::BadRequest);
            }

            let body_text = resp.text().await.map_err(RoboatError::ReqwestError)?;
            let trimmed = body_text.trim().to_string();
            if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
                info!("Uploaded complete (Studio Cookie Mode)! {:?}", trimmed);
                return Ok(trimmed);
            } else {
                return Err(RoboatError::MalformedResponse);
            }
        }

        Err(RoboatError::InternalServerError)
    }

    /// Uploads a single animation to Roblox (uses Open Cloud API if api_key is provided, otherwise falls back to Studio Cookie upload).
    pub async fn upload_asset(
        &self,
        asset_data: Bytes,
        group_id: Option<u64>,
        asset_name: Option<String>,
    ) -> Result<String, RoboatError> {
        if self.api_key.trim().is_empty() {
            return self
                .upload_asset_via_cookie(asset_data, group_id, asset_name)
                .await;
        }

        let resp = self
            .roboat_client
            .upload_animation(
                self.api_key.clone(),
                group_id,
                asset_name.unwrap_or_else(|| DEFAULT_ANIMATION_NAME.to_string()),
                DEFAULT_ANIMATION_DESCRIPTION.to_string(),
                &asset_data,
            )
            .await?;

        for _ in 0..MAX_POLL_ATTEMPTS {
            let operation = self
                .roboat_client
                .poll_upload_operation(self.api_key.clone(), resp.path.clone())
                .await?;

            if operation.done {
                if let Some(asset_id) = operation.response.and_then(|resp| resp.asset_id) {
                    info!("Uploaded complete! {:?}", asset_id);
                    return Ok(asset_id);
                }
                return Err(RoboatError::InternalServerError);
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        warn!(
            "Timed out waiting for Open Cloud upload operation '{}' after {}s",
            resp.path, MAX_POLL_ATTEMPTS
        );
        Err(RoboatError::InternalServerError)
    }

    /// Reuploads multiple animations concurrently.
    ///
    /// When `keep_names` is true, each reuploaded animation keeps the original
    /// asset's display name (requires one extra API call per asset).
    pub async fn reupload_all_assets(
        self: Arc<Self>,
        assets: Vec<roboat::assetdelivery::AssetBatchResponse>,
        group_id: Option<u64>,
        task_count: Option<u64>,
        keep_names: bool,
    ) -> Result<HashMap<String, String>, RoboatError> {
        self.reupload_all_assets_with_progress(assets, group_id, task_count, keep_names, None)
            .await
    }

    /// Reuploads multiple animations concurrently with an optional real-time progress callback.
    pub async fn reupload_all_assets_with_progress(
        self: Arc<Self>,
        assets: Vec<roboat::assetdelivery::AssetBatchResponse>,
        group_id: Option<u64>,
        task_count: Option<u64>,
        keep_names: bool,
        on_progress: Option<ProgressCallback>,
    ) -> Result<HashMap<String, String>, RoboatError> {
        let max_concurrent_tasks = task_count.unwrap_or(DEFAULT_CONCURRENT_TASKS);
        let total_assets = assets.len();

        let ctx = UploadContext::new(
            self,
            group_id,
            max_concurrent_tasks,
            total_assets,
            keep_names,
            on_progress,
        );

        let tasks = ctx.spawn_upload_tasks(assets);
        collect_upload_results(tasks).await
    }
}
