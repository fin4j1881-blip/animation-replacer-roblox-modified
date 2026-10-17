use crate::StudioParser;
use rbx_binary::from_reader as from_binary;
use rbx_types::Variant;
use rbx_xml::from_reader_default as from_xml;
use regex::Regex;
use roboat::assetdelivery::AssetBatchResponse;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::LazyLock;
use ustr::Ustr;

static DIGIT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d+").expect("static regex must compile"));
static ANIMATION_ID_KEY: LazyLock<Ustr> = LazyLock::new(|| Ustr::from("AnimationId"));

impl StudioParser {
    /// Finds Animation instances in the workspace and returns their metadata.
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
        let mut asset_ids: Vec<u64> = self
            .dom
            .descendants()
            .filter(|instance| instance.class == "Animation")
            .filter_map(
                |instance| match instance.properties.get(&ANIMATION_ID_KEY) {
                    Some(Variant::ContentId(content_id)) => DIGIT_RE
                        .find(content_id.as_str())
                        .and_then(|mat| mat.as_str().parse::<u64>().ok()),
                    _ => None,
                },
            )
            .collect();

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
#[derive(Debug, Default)]
pub struct StudioParserBuilder {
    file_path: Option<String>,
    roblosecurity: Option<String>,
    api_key: Option<String>,
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
        let reader = BufReader::new(file);

        let dom = if is_xml_file(expanded_path.as_ref()) {
            from_xml(reader).map_err(|e| anyhow::anyhow!("Failed to parse .rbxlx DOM: {}", e))?
        } else {
            from_binary(reader).map_err(|e| anyhow::anyhow!("Failed to parse .rbxl DOM: {}", e))?
        };

        Ok(StudioParser {
            roblosecurity: self.roblosecurity,
            api_key: self.api_key,
            dom,
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
