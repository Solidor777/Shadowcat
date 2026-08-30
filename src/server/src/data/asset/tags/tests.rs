use super::*;

fn meta(width: Option<u32>, height: Option<u32>, has_alpha: bool, animated: bool) -> AssetMeta {
    AssetMeta {
        width,
        height,
        has_alpha,
        animated,
        original_content_type: "image/png".into(),
        original_byte_size: 1,
        original_retained: false,
        conversion_note: None,
    }
}

#[test]
fn derives_kind_dimension_folder_and_provenance_tags() {
    let m = meta(Some(2048), Some(2048), true, false);
    let tags = derive(DeriveInput {
        content_type: "image/webp",
        meta: &m,
        folder_names: &["Maps".into(), "Crypt".into()],
        provenance: Provenance::Uploaded,
    });
    assert_eq!(
        tags,
        vec![
            "Crypt",
            "Maps",
            "image",
            "large",
            "square",
            "transparent",
            "uploaded",
            "webp"
        ]
    );
}

#[test]
fn animated_gif_passthrough_tags() {
    let m = meta(Some(10), Some(20), false, true);
    let tags = derive(DeriveInput {
        content_type: "image/gif",
        meta: &m,
        folder_names: &[],
        provenance: Provenance::LinkPreview,
    });
    assert_eq!(
        tags,
        vec!["animated", "gif", "gif-animated", "image", "link-preview"]
    );
}

#[test]
fn non_image_is_other() {
    let m = meta(None, None, false, false);
    let tags = derive(DeriveInput {
        content_type: "application/pdf",
        meta: &m,
        folder_names: &[],
        provenance: Provenance::Uploaded,
    });
    assert_eq!(tags, vec!["other", "uploaded"]);
}

#[test]
fn svg_subtype_drops_the_xml_suffix_and_empty_folder_names_are_skipped() {
    let m = meta(None, None, false, false);
    let tags = derive(DeriveInput {
        content_type: "image/svg+xml",
        meta: &m,
        folder_names: &["".into(), "Icons".into()],
        provenance: Provenance::Uploaded,
    });
    assert_eq!(tags, vec!["Icons", "image", "svg", "uploaded"]);
}

#[test]
fn provenance_round_trips_through_the_derived_set() {
    assert_eq!(
        provenance_of(&["image".into(), "link-preview".into()]),
        Provenance::LinkPreview
    );
    assert_eq!(
        provenance_of(&["image".into(), "uploaded".into()]),
        Provenance::Uploaded
    );
    assert_eq!(provenance_of(&[]), Provenance::Uploaded);
}
