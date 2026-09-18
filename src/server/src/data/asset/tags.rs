//! Derived asset tags: computed from pipeline metadata, folder placement and
//! provenance at every commit/rename/move/reconvert; never client-writable.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use super::{AssetMeta, Provenance};
use std::collections::BTreeSet;

/// Either axis at or above this many pixels earns the `large` tag (map-sized).
pub const LARGE_AXIS_PX: u32 = 2048;

/// Derived tag reserved for server-fetched link-preview images; its presence
/// in a stored derived set is how `Provenance` is recovered on refresh.
pub const LINK_PREVIEW_TAG: &str = "link-preview";
/// Derived tag for a GM upload.
pub const UPLOADED_TAG: &str = "uploaded";
/// Derived tag reserved for a server-fetched external chat image
/// (`Provenance::ChatImage`); same recovery role as `LINK_PREVIEW_TAG`.
pub const CHAT_IMAGE_TAG: &str = "chat-image";

/// Longest accepted explicit tag, in chars.
pub const MAX_TAG_CHARS: usize = 64;
/// Most explicit tags one asset carries.
pub const MAX_TAGS: usize = 64;

/// The one rule for GM-set tags, applied by every writer (routes and bundle
/// import alike): trimmed, non-empty, at most `MAX_TAG_CHARS` each and
/// `MAX_TAGS` total; duplicates collapse, order kept. `Err` names the
/// violation.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::tags::normalize_tags;
///
/// let tags = normalize_tags(vec![" Hero ".into(), "Hero".into()]).unwrap();
/// assert_eq!(tags, vec!["Hero".to_string()], "duplicates collapse after trimming");
/// ```
pub fn normalize_tags(tags: Vec<String>) -> Result<Vec<String>, String> {
    if tags.len() > MAX_TAGS {
        return Err(format!("at most {MAX_TAGS} tags"));
    }
    let mut out: Vec<String> = Vec::with_capacity(tags.len());
    for raw in tags {
        let tag = raw.trim();
        if tag.is_empty() {
            return Err("empty tag".into());
        }
        if tag.chars().count() > MAX_TAG_CHARS {
            return Err(format!("tag longer than {MAX_TAG_CHARS} chars"));
        }
        if !out.iter().any(|t| t == tag) {
            out.push(tag.to_string());
        }
    }
    Ok(out)
}

/// Everything `derive` reads.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::tags::{derive, DeriveInput};
/// use shadowcat::data::asset::{AssetMeta, Provenance};
///
/// let input = DeriveInput {
///     content_type: "image/png",
///     meta: &AssetMeta::unprocessed("image/png", 10),
///     folder_names: &[],
///     provenance: Provenance::Uploaded,
/// };
/// let tags = derive(input);
/// assert!(tags.contains(&"image".to_string()));
/// ```
pub struct DeriveInput<'a> {
    /// The served canonical's MIME type (`Asset.content_type`).
    pub content_type: &'a str,
    /// Pipeline metadata of the asset.
    pub meta: &'a AssetMeta,
    /// Root-first names of every ancestor folder (empty at world root).
    pub folder_names: &'a [String],
    /// Who authored the asset.
    pub provenance: Provenance,
}

/// The `Provenance` a stored derived-tag set encodes.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::tags::provenance_of;
/// use shadowcat::data::asset::Provenance;
///
/// let provenance = provenance_of(&["link-preview".to_string()]);
/// assert_eq!(provenance, Provenance::LinkPreview);
/// ```
pub fn provenance_of(derived: &[String]) -> Provenance {
    if derived.iter().any(|t| t == LINK_PREVIEW_TAG) {
        Provenance::LinkPreview
    } else if derived.iter().any(|t| t == CHAT_IMAGE_TAG) {
        Provenance::ChatImage
    } else {
        Provenance::Uploaded
    }
}

/// Computes the derived tag set — sorted, deduplicated:
/// kind (`image` + the subtype, or `other`), `animated` (+ `gif-animated`),
/// `square`, `large`, `transparent`, every folder name verbatim, and the
/// provenance tag.
///
/// # Examples
///
/// ```
/// use shadowcat::data::asset::tags::{derive, DeriveInput};
/// use shadowcat::data::asset::{AssetMeta, Provenance};
///
/// let mut meta = AssetMeta::unprocessed("image/png", 10);
/// meta.width = Some(64);
/// meta.height = Some(64);
/// let tags = derive(DeriveInput {
///     content_type: "image/png",
///     meta: &meta,
///     folder_names: &["Maps".into()],
///     provenance: Provenance::Uploaded,
/// });
/// assert!(tags.contains(&"square".to_string()));
/// assert!(tags.contains(&"Maps".to_string()));
/// ```
pub fn derive(input: DeriveInput<'_>) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    let (kind, subtype) = match input.content_type.split_once('/') {
        Some(("image", sub)) => ("image", Some(sub)),
        Some(("audio", sub)) => ("audio", Some(sub)),
        _ => ("other", None),
    };
    out.insert(kind.into());
    if let Some(sub) = subtype {
        // `image/svg+xml` → `svg`; the rest are already bare subtypes.
        let sub = sub.split('+').next().unwrap_or(sub);
        if !sub.is_empty() {
            out.insert(sub.to_string());
        }
        if kind == "image" && input.meta.animated {
            out.insert("animated".into());
            if sub == "gif" {
                out.insert("gif-animated".into());
            }
        }
    }
    // An audio upload whose transcode was skipped (over-cap or failed) is marked — the only
    // audio classification tag: pipeline outcome, never a content judgment (no
    // ambient/stinger-style tagging anywhere).
    if kind == "audio" && input.meta.conversion_note.is_some() {
        out.insert("audio:untranscoded".into());
    }
    if let (Some(w), Some(h)) = (input.meta.width, input.meta.height) {
        if w == h {
            out.insert("square".into());
        }
        if w >= LARGE_AXIS_PX || h >= LARGE_AXIS_PX {
            out.insert("large".into());
        }
    }
    if input.meta.has_alpha {
        out.insert("transparent".into());
    }
    for name in input.folder_names {
        if !name.is_empty() {
            out.insert(name.clone());
        }
    }
    out.insert(
        match input.provenance {
            Provenance::Uploaded => UPLOADED_TAG,
            Provenance::LinkPreview => LINK_PREVIEW_TAG,
            Provenance::ChatImage => CHAT_IMAGE_TAG,
        }
        .into(),
    );
    out.into_iter().collect()
}

#[cfg(test)]
mod tests;
