use crate::{AssetUploader, StudioParser};
use rbx_binary::from_reader as from_binary;
use rbx_types::Variant;
use rbx_xml::from_reader_default as from_xml;
use regex::Regex;
use roboat::assetdelivery::AssetBatchResponse;
use std::fmt;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::{Arc, LazyLock};
use ustr::Ustr;

static DIGIT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d+").expect("static regex must compile"));
static ANIMATION_ID_KEY: LazyLock<Ustr> = LazyLock::new(|| Ustr::from("AnimationId"));
static VALUE_KEY: LazyLock<Ustr> = LazyLock::new(|| Ustr::from("Value"));

impl StudioParser {
    /// Finds Animation instances and animation Value objects in the workspace and returns their metadata.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use animation_replace_roblox::StudioParser;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let parser = StudioParser::builder()
    ///     .file_path("MyPlace.rbxl")
    ///     .roblosecurity("cookie")
    ///     .build()?;
    /// let animations = parser.workspace_animations().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn workspace_animations(&self) -> anyhow::Result<Vec<AssetBatchResponse>> {
        let mut asset_ids: Vec<u64> = Vec::new();

        for instance in self.dom.descendants() {
            if instance.class == "Animation" {
                let id = match instance.properties.get(&ANIMATION_ID_KEY) {
                    Some(Variant::ContentId(content_id)) => DIGIT_RE
                        .find(content_id.as_str())
                        .and_then(|mat| mat.as_str().parse::<u64>().ok()),
                    Some(Variant::Content(content)) => content
                        .as_uri()
                        .and_then(|uri| DIGIT_RE.find(uri))
                        .and_then(|mat| mat.as_str().parse::<u64>().ok()),
                    Some(Variant::String(s)) => DIGIT_RE
                        .find(s.as_str())
                        .and_then(|mat| mat.as_str().parse::<u64>().ok()),
                    _ => None,
                };
                if let Some(id) = id {
                    asset_ids.push(id);
                }
            } else if instance.class == "StringValue" {
                if let Some(Variant::String(s)) = instance.properties.get(&VALUE_KEY) {
                    let s_str = s.as_str();
                    let is_anim_name = instance.name.to_ascii_lowercase().contains("anim");
                    let has_asset_prefix = s_str.contains("rbxassetid://") || s_str.contains("roblox.com/asset");
                    if has_asset_prefix || is_anim_name {
                        if let Some(mat) = DIGIT_RE.find(s_str) {
                            if let Ok(id) = mat.as_str().parse::<u64>() {
                                if id > 100_000 {
                                    asset_ids.push(id);
                                }
                            }
                        }
                    }
                }
            } else if instance.class == "IntValue" {
                let is_anim_name = instance.name.to_ascii_lowercase().contains("anim");
                if is_anim_name {
                    if let Some(Variant::Int64(val)) = instance.properties.get(&VALUE_KEY) {
                        if *val > 100_000 {
                            asset_ids.push(*val as u64);
                        }
                    }
                }
            }
        }

        asset_ids.sort_unstable();
        asset_ids.dedup();
        self.fetch_animation_assets(asset_ids).await
    }

    /// Creates a builder for fluent configuration with file path and authentication.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use animation_replace_roblox::StudioParser;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let parser = StudioParser::builder()
    ///     .file_path("~/Desktop/MyPlace.rbxl")
    ///     .roblosecurity("your_cookie")
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn builder() -> StudioParserBuilder {
        StudioParserBuilder::new()
    }
}

/// Builder for creating StudioParser instances with optional authentication.
#[derive(Default)]
pub struct StudioParserBuilder {
    file_path: Option<String>,
    roblosecurity: Option<String>,
    api_key: Option<String>,
}

impl fmt::Debug for StudioParserBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StudioParserBuilder")
            .field("file_path", &self.file_path)
            .field(
                "roblosecurity",
                &self.roblosecurity.as_ref().map(|_| "[REDACTED]"),
            )
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

impl StudioParserBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn file_path<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.file_path = Some(path.as_ref().to_string_lossy().to_string());
        self
    }

    /// Sets the Roblosecurity cookie for API authentication.
    /// Required for animation validation and re-uploading features.
    pub fn roblosecurity<S: Into<String>>(mut self, roblosecurity: S) -> Self {
        self.roblosecurity = Some(roblosecurity.into());
        self
    }

    pub fn api_key<S: Into<String>>(mut self, api_key: S) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    /// Builds the StudioParser. File path is required.
    pub fn build(self) -> Result<StudioParser, anyhow::Error> {
        let file_path = self
            .file_path
            .ok_or_else(|| anyhow::anyhow!("File path is required"))?;

        let expanded_path = shellexpand::full(&file_path)
            .map_err(|e| anyhow::anyhow!("Failed to expand path '{}': {}", file_path, e))?;

        let file = File::open(expanded_path.as_ref())
            .map_err(|e| anyhow::anyhow!("Failed to open file '{}': {}", expanded_path, e))?;
        let reader = BufReader::with_capacity(256 * 1024, file);

        let dom = if is_xml_file(expanded_path.as_ref()) {
            from_xml(reader).map_err(|e| anyhow::anyhow!("Failed to parse .rbxlx DOM: {}", e))?
        } else {
            from_binary(reader).map_err(|e| anyhow::anyhow!("Failed to parse .rbxl DOM: {}", e))?
        };

        let uploader = match &self.roblosecurity {
            Some(cookie) if !cookie.trim().is_empty() => {
                let api_key = self.api_key.clone().unwrap_or_default();
                Some(Arc::new(AssetUploader::new(cookie.clone(), api_key)))
            }
            _ => None,
        };

        Ok(StudioParser {
            roblosecurity: self.roblosecurity,
            api_key: self.api_key,
            dom,
            uploader,
        })
    }
}

/// Returns true if the given path refers to a Roblox XML format file
/// (`.rbxlx`/`.rbxmx`) rather than the binary format (`.rbxl`/`.rbxm`).
pub(crate) fn is_xml_file<P: AsRef<Path>>(path: P) -> bool {
    if let Some(ext) = path.as_ref().extension().and_then(|e| e.to_str()) {
        ext.eq_ignore_ascii_case("rbxlx") || ext.eq_ignore_ascii_case("rbxmx")
    } else {
        false
    }
}
