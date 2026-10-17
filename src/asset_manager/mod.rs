use crate::AssetUploader;
use crate::StudioParser;
use roboat::RoboatError;
use roboat::assetdelivery::AssetBatchResponse;

pub use tasks::UploadResult;
pub type UploadTask = tokio::task::JoinHandle<UploadResult>;
pub mod info;
pub mod ratelimiter;
pub mod tasks;
pub mod uploader;
// Implement uploader code into the studio struct
impl StudioParser {
    pub fn animation_uploader(&self) -> Result<AssetUploader, RoboatError> {
        match &self.roblosecurity {
            Some(cookie) => {
                let api_key = self.api_key.clone().unwrap_or_default();
                Ok(AssetUploader::new(cookie.clone(), api_key))
            }
            None => Err(RoboatError::InvalidRoblosecurity),
        }
    }
    pub async fn fetch_animation_assets(
        &self,
        asset_ids: Vec<u64>,
    ) -> anyhow::Result<Vec<AssetBatchResponse>> {
        let uploader = self.animation_uploader()?;
        uploader.fetch_animation_assets(asset_ids).await
    }
}
