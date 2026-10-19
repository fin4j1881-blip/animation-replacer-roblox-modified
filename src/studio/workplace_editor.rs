use crate::StudioParser;
use crate::studio::workplace_parser::is_xml_file;
use rbx_binary::to_writer as to_binary;
use rbx_types::{Content, Variant};
use rbx_xml::to_writer_default as to_xml;
use regex::Regex;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tracing::info;
use ustr::Ustr;

static DIGIT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d+").expect("static regex must compile"));
static ANIMATION_ID_KEY: LazyLock<Ustr> = LazyLock::new(|| Ustr::from("AnimationId"));
static VALUE_KEY: LazyLock<Ustr> = LazyLock::new(|| Ustr::from("Value"));

impl StudioParser {
    /// Gets all animation instances and animation Value objects in the file, scans their IDs,
    /// and replaces them if a new one is provided in `animation_mapping`.
    ///
    /// Handles `rbxassetid://{id}`, `roblox.com/asset/?id={id}`, and raw ID representations
    /// across `Variant::Content`, `Variant::ContentId`, `Variant::String`, and `Variant::Int64`.
    pub fn update_game_animations(&mut self, animation_mapping: &HashMap<String, String>) {
        if animation_mapping.is_empty() {
            return;
        }

        let mut animation_refs = Vec::new();
        let mut string_value_refs = Vec::new();
        let mut int_value_refs = Vec::new();

        for instance in self.dom.descendants() {
            match instance.class.as_str() {
                "Animation" => animation_refs.push(instance.referent()),
                "StringValue" => string_value_refs.push(instance.referent()),
                "IntValue" => int_value_refs.push(instance.referent()),
                _ => {}
            }
        }

        let mut replaced_animations = 0;
        let mut replaced_values = 0;

        for animation_ref in animation_refs {
            if let Some(instance) = self.dom.get_by_ref_mut(animation_ref) {
                let current_prop = instance.properties.get(&ANIMATION_ID_KEY).cloned();
                let matched = match &current_prop {
                    Some(Variant::ContentId(content_id)) => DIGIT_RE
                        .find(content_id.as_str())
                        .and_then(|mat| animation_mapping.get(mat.as_str()))
                        .map(|new_id| (new_id.clone(), 0)), // 0 = ContentId
                    Some(Variant::Content(content)) => content
                        .as_uri()
                        .and_then(|uri| DIGIT_RE.find(uri))
                        .and_then(|mat| animation_mapping.get(mat.as_str()))
                        .map(|new_id| (new_id.clone(), 1)), // 1 = Content
                    Some(Variant::String(s)) => DIGIT_RE
                        .find(s.as_str())
                        .and_then(|mat| animation_mapping.get(mat.as_str()))
                        .map(|new_id| (new_id.clone(), 2)), // 2 = String
                    _ => None,
                };

                if let Some((new_id, prop_type)) = matched {
                    let rbxasset = format!("rbxassetid://{}", new_id);
                    let new_variant = match prop_type {
                        1 => Variant::Content(Content::from_uri(&rbxasset)),
                        2 => Variant::String(rbxasset.into()),
                        _ => Variant::ContentId(rbxasset.into()),
                    };
                    instance.properties.insert(*ANIMATION_ID_KEY, new_variant);
                    replaced_animations += 1;
                }
            }
        }

        for sv_ref in string_value_refs {
            if let Some(instance) = self.dom.get_by_ref_mut(sv_ref) {
                if let Some(Variant::String(s)) = instance.properties.get(&VALUE_KEY) {
                    let s_str = s.as_str();
                    if let Some(mat) = DIGIT_RE.find(s_str) {
                        if let Some(new_id) = animation_mapping.get(mat.as_str()) {
                            let mut new_str = s_str.to_string();
                            new_str.replace_range(mat.range(), new_id);
                            instance
                                .properties
                                .insert(*VALUE_KEY, Variant::String(new_str.into()));
                            replaced_values += 1;
                        }
                    }
                }
            }
        }

        for iv_ref in int_value_refs {
            if let Some(instance) = self.dom.get_by_ref_mut(iv_ref) {
                if let Some(Variant::Int64(val)) = instance.properties.get(&VALUE_KEY) {
                    let old_id_str = val.to_string();
                    if let Some(new_id) = animation_mapping.get(&old_id_str) {
                        if let Ok(new_val) = new_id.parse::<i64>() {
                            instance
                                .properties
                                .insert(*VALUE_KEY, Variant::Int64(new_val));
                            replaced_values += 1;
                        }
                    }
                }
            }
        }

        info!(
            "Workplace update complete: replaced {} Animation instances and {} Value objects",
            replaced_animations, replaced_values
        );
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
