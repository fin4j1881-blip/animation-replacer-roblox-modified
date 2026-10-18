use crate::AssetUploader;
use crate::StudioParser;
use roboat::RoboatError;
use roboat::assetdelivery::AssetBatchResponse;
use std::sync::Arc;

pub use tasks::UploadResult;
pub type UploadTask = tokio::task::JoinHandle<UploadResult>;
pub mod credentials;
pub mod info;
pub mod ratelimiter;
pub mod tasks;
pub mod uploader;

impl StudioParser {
    /// Returns the shared `Arc<AssetUploader>` instance, or creates one if `roblosecurity` was set after building.
    pub fn animation_uploader(&self) -> Result<Arc<AssetUploader>, RoboatError> {
        if let Some(ref uploader) = self.uploader {
            return Ok(Arc::clone(uploader));
        }
        match &self.roblosecurity {
            Some(cookie) if !cookie.trim().is_empty() => {
                let api_key = self.api_key.clone().unwrap_or_default();
                Ok(Arc::new(AssetUploader::new(cookie.clone(), api_key)))
            }
            _ => Err(RoboatError::InvalidRoblosecurity),
        }
    }

    pub async fn fetch_animation_assets(
        &self,
        asset_ids: Vec<u64>,
    ) -> anyhow::Result<Vec<AssetBatchResponse>> {
        if asset_ids.is_empty() {
            return Ok(Vec::new());
        }
        let uploader = self.animation_uploader()?;
        uploader.fetch_animation_assets(asset_ids).await
    }
}
