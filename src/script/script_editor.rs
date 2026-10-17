use crate::StudioParser;
use rbx_types::Variant;
use regex::{Captures, Regex};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::LazyLock;
use ustr::Ustr;

static SOURCE_KEY: LazyLock<Ustr> = LazyLock::new(|| Ustr::from("Source"));
static UNFORMATTED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\d{6,}\b").expect("static regex must compile"));
static FORMATTED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(rbxassetid://|roblox\.com/asset/\?id=)(\d{6,})")
        .expect("static regex must compile")
});

/// Replaces asset IDs in a script source using the provided mapping.
///
/// Two modes exist:
/// - **Formatted (default):** only IDs inside `rbxassetid://{id}` or
///   `roblox.com/asset/?id={id}` are replaced. Everything else is untouched.
/// - **Unformatted:** every standalone 6+ digit number is replaced, matching
///   how scanning works with `--unformatted-ids`. Whole-token word boundaries
///   prevent corrupting longer numbers (e.g. `123456789` when mapping has
///   `123456`).
fn replace_asset_ids<'a>(
    source: &'a str,
    animation_mapping: &HashMap<String, String>,
    unformatted: bool,
) -> Cow<'a, str> {
    if unformatted {
        UNFORMATTED_RE.replace_all(source, |caps: &Captures| {
            let id = caps.get(0).map_or("", |m| m.as_str());
            match animation_mapping.get(id) {
                Some(new_id) => Cow::Borrowed(new_id.as_str()),
                None => Cow::Borrowed(&source[caps.get(0).unwrap().range()]),
            }
        })
    } else {
        FORMATTED_RE.replace_all(source, |caps: &Captures| {
            let prefix = caps.get(1).map_or("", |m| m.as_str());
            let id = caps.get(2).map_or("", |m| m.as_str());
            match animation_mapping.get(id) {
                Some(new_id) => {
                    let mut s = String::with_capacity(prefix.len() + new_id.len());
                    s.push_str(prefix);
                    s.push_str(new_id);
                    Cow::Owned(s)
                }
                None => Cow::Borrowed(&source[caps.get(0).unwrap().range()]),
            }
        })
    }
}

impl StudioParser {
    /// Updates animation IDs in script source code using the provided mapping.
    ///
    /// Set `unformatted` to true when scanning used `--unformatted-ids` so that
    /// bare numbers are replaced as whole tokens instead of only asset URLs.
    pub fn update_script_animations(
        &mut self,
        animation_mapping: &HashMap<String, String>,
        unformatted: bool,
    ) {
        if animation_mapping.is_empty() {
            return;
        }

        // Collect script refs first to avoid borrow checker issues
        let script_refs = self.get_script_refs();

        // Now modify each script only if changes were made
        for script_ref in script_refs {
            if let Some(instance) = self.dom.get_by_ref_mut(script_ref)
                && let Some(Variant::String(source)) = instance.properties.get(&SOURCE_KEY)
            {
                let new_source = replace_asset_ids(source, animation_mapping, unformatted);
                if let Cow::Owned(modified) = new_source {
                    instance
                        .properties
                        .insert(*SOURCE_KEY, Variant::String(modified));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::replace_asset_ids;

    fn mapping(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn formatted_replaces_asset_url_ids() {
        let map = mapping(&[("123456", "999999")]);
        let source = r#"local a = "rbxassetid://123456" local b = "roblox.com/asset/?id=123456""#;
        let out = replace_asset_ids(source, &map, false);
        assert!(out.contains("rbxassetid://999999"));
        assert!(out.contains("roblox.com/asset/?id=999999"));
    }

    #[test]
    fn formatted_ignores_bare_numbers() {
        let map = mapping(&[("123456", "999999")]);
        let source = "local a = 123456";
        let out = replace_asset_ids(source, &map, false);
        assert_eq!(out, source);
    }

    #[test]
    fn formatted_does_not_corrupt_longer_numbers() {
        let map = mapping(&[("123456", "999999")]);
        let source = r#"local a = "rbxassetid://123456789""#;
        let out = replace_asset_ids(source, &map, false);
        assert_eq!(out, source);
        assert!(!out.contains("999999789"));
    }

    #[test]
    fn formatted_leaves_unknown_ids_unchanged() {
        let map = mapping(&[("123456", "999999")]);
        let source = r#"local a = "rbxassetid://654321""#;
        let out = replace_asset_ids(source, &map, false);
        assert_eq!(out, source);
    }

    #[test]
    fn unformatted_replaces_bare_numbers() {
        let map = mapping(&[("123456", "999999")]);
        let source = "local a = 123456";
        let out = replace_asset_ids(source, &map, true);
        assert_eq!(out, "local a = 999999");
    }

    #[test]
    fn unformatted_replaces_inside_asset_urls_too() {
        let map = mapping(&[("123456", "999999")]);
        let source = r#"local a = "rbxassetid://123456""#;
        let out = replace_asset_ids(source, &map, true);
        assert_eq!(out, r#"local a = "rbxassetid://999999""#);
    }

    #[test]
    fn unformatted_does_not_corrupt_longer_numbers() {
        let map = mapping(&[("123456", "999999")]);
        let source = "local a = 123456789";
        let out = replace_asset_ids(source, &map, true);
        assert_eq!(out, source);
    }

    #[test]
    fn unformatted_does_not_replace_inside_identifiers() {
        let map = mapping(&[("123456", "999999")]);
        let source = "local _123456 = 5";
        let out = replace_asset_ids(source, &map, true);
        assert_eq!(out, source);
    }
}
