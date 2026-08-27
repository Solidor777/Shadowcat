use super::*;
use crate::data::document::{Document, PermissionSet, Scope, Visibility};
use uuid::Uuid;

fn doc(doc_type: &str, system: serde_json::Value) -> Document {
    Document {
        id: Uuid::from_u128(1),
        scope: Scope::World {
            world_id: Uuid::from_u128(9),
        },
        doc_type: doc_type.into(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: None,
        permissions: PermissionSet::default(),
        embedded: Default::default(),
        parent_id: None,
        engine: None,
        system,
        created_at: 0,
        updated_at: 0,
    }
}

#[test]
fn extracts_string_and_number_leaves_and_doc_type() {
    let d = doc(
        "actor",
        serde_json::json!({
            "name": "Goblin Scout",
            "hp": 12,
            "traits": ["sneaky", "cowardly"],
            "nested": { "weapon": "shortbow" },
            "hidden": true,
            "nothing": null
        }),
    );
    let c = index_content(&d);
    for needle in [
        "actor",
        "Goblin Scout",
        "12",
        "sneaky",
        "cowardly",
        "shortbow",
    ] {
        assert!(c.contains(needle), "content missing {needle:?}: {c}");
    }
    // Keys and non-text leaves are not indexed.
    assert!(!c.contains("weapon"));
    assert!(!c.contains("true"));
}

#[test]
fn indexes_envelope_name_and_engine_alongside_system() {
    let mut d = doc(
        "actor",
        serde_json::json!({ "bio": "vampire lord of Barovia" }),
    );
    d.name = Some("Strahd".into());
    d.engine = Some(serde_json::json!({ "x": 3, "faction": "undead" }));

    let c = index_content(&d);
    for needle in ["Strahd", "3", "undead", "vampire", "Barovia"] {
        assert!(c.contains(needle), "content missing {needle:?}: {c}");
    }
}

#[test]
fn name_gm_only_hides_from_public_index_but_gm_index_retains_it() {
    let mut d = doc("actor", serde_json::json!({}));
    d.name = Some("Strahd".into());
    d.permissions
        .property_overrides
        .insert("/name".into(), Visibility::GmOnly);

    let public = index_content_public(&d);
    assert!(
        !public.contains("Strahd"),
        "gm-only name leaked into the public index: {public}"
    );

    let full = index_content(&d);
    assert!(full.contains("Strahd"), "GM index must retain the name");
}

#[test]
fn engine_leaf_gm_only_hides_from_public_index_but_gm_index_retains_it() {
    // A redacted NESTED engine leaf (not the whole `/engine` band) must
    // still be absent from the public index — proves `index_content_public`'s
    // redaction-first property covers engine leaves, not just the band root.
    let mut d = doc("actor", serde_json::json!({}));
    d.engine = Some(serde_json::json!({ "x": 3, "faction": "undead" }));
    d.permissions
        .property_overrides
        .insert("/engine/faction".into(), Visibility::GmOnly);

    let public = index_content_public(&d);
    assert!(
        !public.contains("undead"),
        "gm-only engine leaf leaked into the public index: {public}"
    );
    assert!(
        public.contains('3'),
        "unrestricted engine leaf must remain in the public index: {public}"
    );

    let full = index_content(&d);
    assert!(full.contains("undead"), "GM index must retain the leaf");
}

#[test]
fn build_match_quotes_terms_and_prefixes_last() {
    assert_eq!(build_match("gob scout").unwrap(), "\"gob\" \"scout\"*");
    assert_eq!(build_match("dragon").unwrap(), "\"dragon\"*");
}

#[test]
fn build_match_neutralizes_fts_operators() {
    let m = build_match("fire OR \"x\" -bomb").unwrap();
    // Bare operators do not reach MATCH as syntax (every token is quoted).
    assert!(!m.contains("OR "));
    assert!(m.starts_with('"'));
    // The stray quote is stripped, not emitted as a raw operator quote.
    assert!(!m.contains("\"x\"\"x\""));
}

#[test]
fn build_match_empty_is_none() {
    assert!(build_match("   ").is_none());
    assert!(build_match("").is_none());
}

#[test]
fn build_match_caps_length_and_term_count() {
    // Term count is capped at 16.
    let many = (0..50)
        .map(|i| format!("t{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let m = build_match(&many).unwrap();
    assert_eq!(
        m.matches('"').count() / 2,
        16,
        "term count must be capped at 16"
    );
    // Length is capped at 256 chars (one giant token is truncated, not rejected).
    let huge = "a".repeat(10_000);
    let m = build_match(&huge).unwrap();
    // The single quoted+prefixed term is bounded by the 256-char cap.
    assert!(m.len() <= 256 + 4, "query length must be capped");
}

#[test]
fn build_match_punctuation_only_is_none() {
    // Punctuation-only input must not reach FTS5 as a term-less phrase
    // (which the parser rejects). It reduces to no terms → None → empty page.
    for q in ["---", "*", "\"\"\"", "()", ":::", "^", "- ^ *"] {
        assert!(build_match(q).is_none(), "expected None for {q:?}");
    }
}
