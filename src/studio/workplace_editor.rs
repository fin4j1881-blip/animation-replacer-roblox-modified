use crate::StudioParser;
use crate::studio::workplace_parser::is_xml_file;
use rbx_binary::to_writer as to_binary;
use rbx_types::Variant;
use rbx_xml::to_writer_default as to_xml;
use regex::Regex;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::LazyLock;
use ustr::Ustr;

static DIGIT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d+").expect("static regex must compile"));
static ANIMATION_ID_KEY: LazyLock<Ustr> = LazyLock::new(|| Ustr::from("AnimationId"));

impl StudioParser {
    /// Gets all animation instances in the file, scans their animationId then replaces them if a
    /// new one is provided.
    ///
    /// Handles both `rbxassetid://{id}` and `roblox.com/asset/?id={id}` content IDs.
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
            if let Some(instance) = self.dom.get_by_ref_mut(animation_ref)
                && let Some(Variant::ContentId(content_id)) =
                    instance.properties.get(&ANIMATION_ID_KEY)
            {
                let raw = content_id.as_str();
                let old_id = match DIGIT_RE.find(raw) {
                    Some(mat) => mat.as_str(),
                    None => continue,
                };
                if let Some(new_id) = animation_mapping.get(old_id) {
                    // Replace the AnimationId with the new one, normalizing the format
                    let rbxasset = format!("rbxassetid://{}", new_id);
                    instance
                        .properties
                        .insert(*ANIMATION_ID_KEY, Variant::ContentId(rbxasset.into()));
                }
            }
        }
    }

    /// Saves the DOM to a .rbxl file.
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
        let file = File::create(expanded_path.as_ref())?;
        let mut writer = BufWriter::new(file);

        // Get the children of the root instead of the root
        let root_children = self
            .dom
            .get_by_ref(self.dom.root_ref())
            .ok_or_else(|| anyhow::anyhow!("DOM root not found"))?
            .children();

        if is_xml_file(expanded_path.as_ref()) {
            to_xml(&mut writer, &self.dom, root_children)?;
        } else {
            to_binary(&mut writer, &self.dom, root_children)?;
        }
        writer.flush()?;
        Ok(())
    }
}
