use super::*;
use crate::chat::{DocLinkTarget, OEmbedSegment};
use crate::dice::eval::{evaluate, roll};
use crate::dice::notation::{parse, ParseContext};
use crate::dice::rng::NoiseRng;
use uuid::Uuid;

fn sample_outcome() -> crate::dice::outcome::RollOutcome {
    let spec = parse("1d6", ParseContext::default()).unwrap();
    let mut rng = NoiseRng::from_seed(1);
    let raws = roll(&spec, &mut rng);
    evaluate(&spec, &raws)
}

#[test]
fn empty_list_yields_empty_string() {
    assert_eq!(segments_search_text(&[]), "");
}

#[test]
fn text_segment_contributes_its_text() {
    let segments = vec![Segment::Text {
        text: "hello world".to_string(),
    }];
    assert_eq!(segments_search_text(&segments), "hello world");
}

#[test]
fn html_segment_strips_tags_and_decodes_entities() {
    let segments = vec![Segment::Html {
        sanitized_html: "<strong>bold &amp; brave</strong>".to_string(),
    }];
    let text = segments_search_text(&segments);
    assert!(text.contains("bold"));
    assert!(text.contains("&"));
    assert!(text.contains("brave"));
    assert!(!text.contains("strong"));
}

#[test]
fn entity_decoding_covers_named_and_numeric_forms() {
    assert_eq!(strip_tags_and_decode("a &amp; b"), "a & b");
    assert_eq!(strip_tags_and_decode("&#39;"), "'");
    assert_eq!(strip_tags_and_decode("&#x27;"), "'");
    assert_eq!(strip_tags_and_decode("&lt;&gt;&quot;"), "<>\"");
}

#[test]
fn roll_embed_contributes_only_the_formula() {
    let roll_id = Uuid::new_v4();
    let segments = vec![Segment::RollEmbed {
        formula: "2d20".to_string(),
        outcome: sample_outcome(),
        roll_id,
        spec: None,
        raw: None,
        recalc_history: None,
    }];
    let text = segments_search_text(&segments);
    assert!(text.contains("2d20"));
    assert!(!text.contains(&roll_id.to_string()));
}

#[test]
fn roll_button_contributes_label_and_formula() {
    let segments = vec![Segment::RollButton {
        formula: "1d8".to_string(),
        label: Some("Attack".to_string()),
    }];
    let text = segments_search_text(&segments);
    assert!(text.contains("1d8"));
    assert!(text.contains("Attack"));
}

#[test]
fn link_preview_contributes_title_description_and_url_never_image_id() {
    let image_asset_id = Uuid::new_v4();
    let segments = vec![Segment::LinkPreview {
        url: "https://example.com".to_string(),
        title: "Example Title".to_string(),
        description: "Example Description".to_string(),
        image_asset_id: Some(image_asset_id),
    }];
    let text = segments_search_text(&segments);
    assert!(text.contains("Example Title"));
    assert!(text.contains("Example Description"));
    assert!(text.contains("example.com"));
    assert!(!text.contains(&image_asset_id.to_string()));
}

#[test]
fn oembed_contributes_title_author_and_provider_name_never_ids_or_url() {
    let thumbnail_id = Uuid::new_v4();
    let segments = vec![Segment::OEmbed(OEmbedSegment {
        url: "https://example.com/video".to_string(),
        provider_name: "ExampleProvider".to_string(),
        title: Some("A Video".to_string()),
        author_name: Some("Some Author".to_string()),
        thumbnail_asset_id: Some(thumbnail_id),
    })];
    let text = segments_search_text(&segments);
    assert!(text.contains("A Video"));
    assert!(text.contains("Some Author"));
    assert!(text.contains("ExampleProvider"));
    assert!(!text.contains("example.com"));
    assert!(!text.contains(&thumbnail_id.to_string()));
}

#[test]
fn doc_link_contributes_label_never_target_ids() {
    let token_id = Uuid::new_v4();
    let segments = vec![Segment::DocLink {
        target: DocLinkTarget::Token { token_id },
        label: "The Villain".to_string(),
    }];
    let text = segments_search_text(&segments);
    assert!(text.contains("The Villain"));
    assert!(!text.contains(&token_id.to_string()));
}

#[test]
fn image_contributes_alt_never_asset_id() {
    let asset_id = Uuid::new_v4();
    let segments = vec![Segment::Image {
        asset_id,
        alt: "a portrait".to_string(),
    }];
    let text = segments_search_text(&segments);
    assert_eq!(text, "a portrait");
    assert!(!text.contains(&asset_id.to_string()));
}

#[test]
fn table_draw_contributes_name_row_label_and_content_never_formula_or_ids() {
    let table_id = Uuid::new_v4();
    let roll_id = Uuid::new_v4();
    let segments = vec![Segment::TableDraw(TableDrawSegment {
        table_id,
        table_name: "Loot Table".to_string(),
        roll_id,
        formula: "1d6".to_string(),
        outcome: sample_outcome(),
        spec: None,
        raw: None,
        row: Some(DrawnRow {
            index: 0,
            label: "Gold coin".to_string(),
            content: vec![Segment::Text {
                text: "A shiny gold coin".to_string(),
            }],
            nested: Vec::new(),
        }),
    })];
    let text = segments_search_text(&segments);
    assert!(text.contains("Loot Table"));
    assert!(text.contains("Gold coin"));
    assert!(text.contains("shiny gold coin"));
    assert!(!text.contains("1d6"));
    assert!(!text.contains(&table_id.to_string()));
    assert!(!text.contains(&roll_id.to_string()));
}

#[test]
fn table_draw_recurses_through_nested_draws() {
    let grandchild = TableDrawSegment {
        table_id: Uuid::nil(),
        table_name: "Gem Table".to_string(),
        roll_id: Uuid::new_v4(),
        formula: "1d4".to_string(),
        outcome: sample_outcome(),
        spec: None,
        raw: None,
        row: Some(DrawnRow {
            index: 0,
            label: "Ruby".to_string(),
            content: Vec::new(),
            nested: Vec::new(),
        }),
    };
    let segments = vec![Segment::TableDraw(TableDrawSegment {
        table_id: Uuid::nil(),
        table_name: "Loot Table".to_string(),
        roll_id: Uuid::new_v4(),
        formula: "1d6".to_string(),
        outcome: sample_outcome(),
        spec: None,
        raw: None,
        row: Some(DrawnRow {
            index: 0,
            label: "Chest".to_string(),
            content: Vec::new(),
            nested: vec![grandchild],
        }),
    })];
    let text = segments_search_text(&segments);
    assert!(text.contains("Ruby"));
    assert!(!text.contains("1d4"));
}
