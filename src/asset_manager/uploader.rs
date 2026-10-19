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
use super::tasks::{LogCallback, ProgressCallback, UploadContext, collect_upload_results};

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
    pub(super) cached_user_id: RwLock<Option<u64>>,
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
            cached_user_id: RwLock::new(None),
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

    /// Returns the authenticated user's ID, fetching and caching it via roboat if necessary.
    pub async fn user_id(&self) -> Result<u64, RoboatError> {
        if let Some(id) = *self.cached_user_id.read().await {
            return Ok(id);
        }
        let id = self.roboat_client.user_id().await?;
        *self.cached_user_id.write().await = Some(id);
        Ok(id)
    }

    /// Uploads an animation using only the .ROBLOSECURITY cookie via Roblox Studio user-auth API
    /// (https://apis.roblox.com/assets/user-auth/v1/assets).
    pub async fn upload_asset_via_cookie(
        &self,
        asset_data: Bytes,
        group_id: Option<u64>,
        asset_name: Option<String>,
    ) -> Result<String, RoboatError> {
        let name = asset_name.unwrap_or_else(|| DEFAULT_ANIMATION_NAME.to_string());
        let cookie_header = format!(".ROBLOSECURITY={}", self.roblosecurity);

        let creator_value = if let Some(gid) = group_id {
            serde_json::json!({ "groupId": gid.to_string() })
        } else {
            let uid = self.user_id().await?;
            serde_json::json!({ "userId": uid.to_string() })
        };

        let request_payload = serde_json::json!({
            "assetType": "Animation",
            "displayName": name,
            "description": DEFAULT_ANIMATION_DESCRIPTION,
            "creationContext": {
                "creator": creator_value,
                "expectedPrice": 0
            }
        });
        let request_json = serde_json::to_string(&request_payload)
            .map_err(|_| RoboatError::MalformedResponse)?;

        let mut xcsrf = self.xcsrf_token.read().await.clone();
        let upload_url = "https://apis.roblox.com/assets/user-auth/v1/assets";

        for _ in 0..2 {
            let form = reqwest::multipart::Form::new()
                .part(
                    "fileContent",
                    reqwest::multipart::Part::bytes(asset_data.to_vec())
                        .file_name("animation.rbxm")
                        .mime_str("model/x-rbxm")
                        .map_err(|_| RoboatError::MalformedResponse)?,
                )
                .text("request", request_json.clone());

            let mut req = self
                .http_client
                .post(upload_url)
                .timeout(Duration::from_secs(UPLOAD_TIMEOUT_SECS))
                .header("Cookie", &cookie_header)
                .header("User-Agent", "RobloxStudio/WinInet")
                .header("Requester", "Client");

            if !xcsrf.is_empty() {
                req = req.header("X-CSRF-TOKEN", &xcsrf);
            }

            let resp = req
                .multipart(form)
                .send()
                .await
                .map_err(RoboatError::ReqwestError)?;

            let status = resp.status();

            if status.as_u16() == 403 {
                if let Some(new_token) = resp
                    .headers()
                    .get("x-csrf-token")
                    .and_then(|v| v.to_str().ok())
                {
                    xcsrf = new_token.to_string();
                    *self.xcsrf_token.write().await = xcsrf.clone();
                    continue;
                }
                let body = resp.text().await.unwrap_or_default();
                warn!("Cookie upload 403 Forbidden: {}", body);
                return Err(RoboatError::UnknownRobloxErrorCode {
                    code: 403,
                    message: format!("Permission/Group role denied: {}", body),
                });
            }

            if status.as_u16() == 401 {
                return Err(RoboatError::InvalidRoblosecurity);
            }

            if status.as_u16() == 429 {
                return Err(RoboatError::TooManyRequests);
            }

            if status.is_server_error() {
                return Err(RoboatError::InternalServerError);
            }

            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                warn!("Cookie upload failed (status {}): {}", status, body);
                return Err(RoboatError::UnknownRobloxErrorCode {
                    code: status.as_u16(),
                    message: format!("HTTP {}: {}", status, body),
                });
            }

fn extract_asset_id_from_op(val: &serde_json::Value) -> Option<String> {
    if let Some(resp) = val.get("response") {
        if let Some(asset_id) = resp.get("assetId") {
            if let Some(s) = asset_id.as_str() {
                if !s.is_empty() {
                    return Some(s.to_string());
                }
            }
            if let Some(n) = asset_id.as_u64() {
                return Some(n.to_string());
            }
            if let Some(n) = asset_id.as_i64() {
                return Some(n.to_string());
            }
        }
        if let Some(path) = resp.get("path").and_then(|v| v.as_str()) {
            if let Some(id) = path.strip_prefix("assets/") {
                if !id.is_empty() {
                    return Some(id.to_string());
                }
            }
        }
    }
    if let Some(asset_id) = val.get("assetId") {
        if let Some(s) = asset_id.as_str() {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
        if let Some(n) = asset_id.as_u64() {
            return Some(n.to_string());
        }
    }
    None
}

            // Success (200/201): Parse operation response
            let body_text = resp.text().await.map_err(RoboatError::ReqwestError)?;
            let op: serde_json::Value = serde_json::from_str(&body_text)
                .map_err(|_| RoboatError::MalformedResponse)?;

            // If operation is already done
            if op.get("done").and_then(|v| v.as_bool()).unwrap_or(false) {
                if let Some(asset_id) = extract_asset_id_from_op(&op) {
                    info!("Upload complete (Studio Cookie Mode)! Asset ID: {}", asset_id);
                    return Ok(asset_id);
                }
            }

            // If operation is pending, poll it
            if let Some(path) = op.get("path").and_then(|v| v.as_str()) {
                let poll_url = if path.starts_with("http") {
                    path.to_string()
                } else {
                    format!("https://apis.roblox.com/assets/user-auth/v1/{}", path)
                };

                for _ in 0..MAX_POLL_ATTEMPTS {
                    tokio::time::sleep(Duration::from_secs(1)).await;

                    let mut poll_req = self
                        .http_client
                        .get(&poll_url)
                        .timeout(Duration::from_secs(15))
                        .header("Cookie", &cookie_header)
                        .header("User-Agent", "RobloxStudio/WinInet");

                    if !xcsrf.is_empty() {
                        poll_req = poll_req.header("X-CSRF-TOKEN", &xcsrf);
                    }

                    let poll_resp = match poll_req.send().await {
                        Ok(r) => r,
                        Err(e) => {
                            warn!("Polling error: {}", e);
                            continue;
                        }
                    };

                    if poll_resp.status().as_u16() == 403 {
                        if let Some(new_token) = poll_resp
                            .headers()
                            .get("x-csrf-token")
                            .and_then(|v| v.to_str().ok())
                        {
                            xcsrf = new_token.to_string();
                            *self.xcsrf_token.write().await = xcsrf.clone();
                            continue;
                        }
                    }

                    if !poll_resp.status().is_success() {
                        continue;
                    }

                    let poll_body = poll_resp.text().await.unwrap_or_default();
                    if let Ok(poll_val) = serde_json::from_str::<serde_json::Value>(&poll_body) {
                        if poll_val.get("done").and_then(|v| v.as_bool()).unwrap_or(false) {
                            if let Some(asset_id) = extract_asset_id_from_op(&poll_val) {
                                info!(
                                    "Upload complete (Studio Cookie Mode polled)! Asset ID: {}",
                                    asset_id
                                );
                                return Ok(asset_id);
                            }
                            if let Some(err_msg) = poll_val
                                .get("error")
                                .and_then(|e| e.get("message"))
                                .and_then(|v| v.as_str())
                            {
                                warn!("Operation reported failure: {}", err_msg);
                                return Err(RoboatError::UnknownRobloxErrorCode {
                                    code: 400,
                                    message: err_msg.to_string(),
                                });
                            }
                        }
                    }
                }
                return Err(RoboatError::InternalServerError);
            }

            // Fallback: check if the body was directly the asset id (legacy behavior)
            let trimmed = body_text.trim();
            if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
                info!("Upload complete (Direct ID)! {:?}", trimmed);
                return Ok(trimmed.to_string());
            }

            return Err(RoboatError::MalformedResponse);
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

        let resp = match self
            .roboat_client
            .upload_animation(
                self.api_key.clone(),
                group_id,
                asset_name.clone().unwrap_or_else(|| DEFAULT_ANIMATION_NAME.to_string()),
                DEFAULT_ANIMATION_DESCRIPTION.to_string(),
                &asset_data,
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!("Open Cloud API upload failed: {:?}", e);
                // If Open Cloud fails (e.g. invalid key or missing group permissions) but we have a cookie,
                // attempt Studio Cookie upload as a fallback.
                if !self.roblosecurity.trim().is_empty() {
                    warn!("Attempting fallback to Studio Cookie upload...");
                    return self
                        .upload_asset_via_cookie(asset_data, group_id, asset_name)
                        .await;
                }
                return Err(e);
            }
        };

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
                if let Some(err) = operation.error {
                    warn!("Open Cloud operation error: {}", err.message);
                    return Err(RoboatError::UnknownRobloxErrorCode {
                        code: err.code as u16,
                        message: err.message,
                    });
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
        self.reupload_all_assets_with_callbacks(assets, group_id, task_count, keep_names, None, None)
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
        self.reupload_all_assets_with_callbacks(assets, group_id, task_count, keep_names, on_progress, None)
            .await
    }

    /// Reuploads multiple animations concurrently with optional progress and log callbacks.
    pub async fn reupload_all_assets_with_callbacks(
        self: Arc<Self>,
        assets: Vec<roboat::assetdelivery::AssetBatchResponse>,
        group_id: Option<u64>,
        task_count: Option<u64>,
        keep_names: bool,
        on_progress: Option<ProgressCallback>,
        on_log: Option<LogCallback>,
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
            on_log,
        );

        let tasks = ctx.spawn_upload_tasks(assets);
        collect_upload_results(tasks).await
    }
}
