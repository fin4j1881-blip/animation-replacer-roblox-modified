use crate::StudioParser;
use crate::studio::workplace_parser::is_xml_file;
use rbx_binary::to_writer as to_binary;
use rbx_types::Variant;
use rbx_xml::to_writer_default as to_xml;
use regex::Regex;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use ustr::Ustr;

static DIGIT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d+").expect("static regex must compile"));
static ANIMATION_ID_KEY: LazyLock<Ustr> = LazyLock::new(|| Ustr::from("AnimationId"));

impl StudioParser {
    /// Gets all animation instances in the file, scans their animationId then replaces them if a
    /// new one is provided.
    ///
    /// Handles both `rbxassetid://{id}` and `roblox.com/asset/?id={id}` content IDs across
    /// both `Variant::ContentId` and `Variant::String` representations.
    pub fn update_game_animations(&mut self, animation_mapping: &HashMap<String, String>) {
        if animation_mapping.is_empty() {
            return;
        }

        let animation_instances_referent: Vec<_> = self
            .dom
            .descendants()
            .filter(|instance| instance.class.as_str() == "Animation")
            .map(|instance| instance.referent())
            .collect();

        for animation_ref in animation_instances_referent {
            if let Some(instance) = self.dom.get_by_ref_mut(animation_ref) {
                let matched_new_id = match instance.properties.get(&ANIMATION_ID_KEY) {
                    Some(Variant::ContentId(content_id)) => DIGIT_RE
                        .find(content_id.as_str())
                        .and_then(|mat| animation_mapping.get(mat.as_str()))
                        .cloned(),
                    Some(Variant::String(s)) => DIGIT_RE
                        .find(s.as_str())
                        .and_then(|mat| animation_mapping.get(mat.as_str()))
                        .cloned(),
                    _ => None,
                };

                if let Some(new_id) = matched_new_id {
                    let rbxasset = format!("rbxassetid://{}", new_id);
                    instance
                        .properties
                        .insert(*ANIMATION_ID_KEY, Variant::ContentId(rbxasset.into()));
                }
            }
        }
    }

    /// Saves the DOM to a `.rbxl` or `.rbxlx` file atomically using a temporary file
    /// to prevent data loss or truncation if an I/O error occurs mid-write.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use animation_replace_roblox::StudioParser;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let parser = StudioParser::builder()
    ///     .file_path("input.rbxl")
    ///     .build()?;
    /// parser.save_to_rbxl("output.rbxl")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn save_to_rbxl<P: AsRef<Path>>(&self, file_path: P) -> Result<(), anyhow::Error> {
        let path = file_path.as_ref();
        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("File path is not valid UTF-8: {:?}", path))?;
        let expanded_path = shellexpand::full(path_str)?;
        let target_path = Path::new(expanded_path.as_ref());

        // Get the children of the root instead of the root
        let root_children = self
            .dom
            .get_by_ref(self.dom.root_ref())
            .ok_or_else(|| anyhow::anyhow!("DOM root not found"))?
            .children();

        let is_xml = is_xml_file(target_path);

        // Write to a sibling temporary file first so an in-place save never truncates the original file on error
        let mut tmp_os = target_path.as_os_str().to_os_string();
        tmp_os.push(".tmp_rar");
        let tmp_path = PathBuf::from(tmp_os);

        let write_result = (|| -> Result<(), anyhow::Error> {
            let file = File::create(&tmp_path)?;
            let mut writer = BufWriter::with_capacity(256 * 1024, file);

            if is_xml {
                to_xml(&mut writer, &self.dom, root_children)?;
            } else {
                to_binary(&mut writer, &self.dom, root_children)?;
            }
            writer.flush()?;
            Ok(())
        })();

        if let Err(e) = write_result {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e);
        }

        if let Err(e) = std::fs::rename(&tmp_path, target_path) {
            // Fallback copy if rename across mount points fails
            if std::fs::copy(&tmp_path, target_path).is_ok() {
                let _ = std::fs::remove_file(&tmp_path);
                return Ok(());
            }
            let _ = std::fs::remove_file(&tmp_path);
            return Err(anyhow::anyhow!(
                "Failed to finalize saved file '{}': {}",
                target_path.display(),
                e
            ));
        }

        Ok(())
    }
}
