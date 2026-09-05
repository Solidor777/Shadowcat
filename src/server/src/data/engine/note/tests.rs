use super::*;
use crate::auth::role::ServerRole;
use crate::chat::{DocLinkTarget, Segment};
use crate::data::document::Document;
use crate::data::sqlite::SqliteRepository;
use crate::data::DataError;
use uuid::Uuid;

fn note_body(source: &str, extra_body: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "source": source, "body": extra_body, "sort": 0 })
}

#[test]
fn a_note_with_plain_source_is_accepted() {
    let note = NoteEngine {
        source: "hello".into(),
        body: vec![],
        sort: 0,
    };
    assert!(note.validate().is_ok());
}

#[test]
fn over_cap_source_is_rejected_by_validate() {
    let note = NoteEngine {
        source: "a".repeat(MAX_NOTE_SOURCE_CHARS + 1),
        body: vec![],
        sort: 0,
    };
    assert!(note.validate().is_err());
}

#[test]
fn normalize_engine_discards_a_client_supplied_body_and_derives_from_source() {
    let body = note_body(
        "hello",
        serde_json::json!([{ "kind": "text", "text": "GARBAGE" }]),
    );
    let normalized = crate::data::engine::normalize_engine_opt(NOTE_DOC_TYPE, Some(&body))
        .unwrap()
        .unwrap();
    let typed: NoteEngine = serde_json::from_value(normalized).unwrap();
    assert!(
        matches!(typed.body.as_slice(), [Segment::Html { .. }]),
        "the derived body comes from source under NOTE_CONTENT_POLICY, not the client's body: {:?}",
        typed.body
    );
}

#[test]
fn normalize_engine_renders_markdown_to_html() {
    let body = note_body("**bold**", serde_json::json!([]));
    let normalized = crate::data::engine::normalize_engine_opt(NOTE_DOC_TYPE, Some(&body))
        .unwrap()
        .unwrap();
    let typed: NoteEngine = serde_json::from_value(normalized).unwrap();
    assert!(matches!(typed.body.as_slice(), [Segment::Html { .. }]));
}

#[test]
fn a_labeled_roll_span_becomes_a_roll_button_with_a_label() {
    let body = note_body("[[roll:1d6|Luck]]", serde_json::json!([]));
    let normalized = crate::data::engine::normalize_engine_opt(NOTE_DOC_TYPE, Some(&body))
        .unwrap()
        .unwrap();
    let typed: NoteEngine = serde_json::from_value(normalized).unwrap();
    assert_eq!(
        typed.body,
        vec![Segment::RollButton {
            formula: "1d6".to_string(),
            label: Some("Luck".to_string()),
        }]
    );
}

#[test]
fn a_bare_inline_span_becomes_a_roll_button_with_no_label() {
    let body = note_body("[[1d6]]", serde_json::json!([]));
    let normalized = crate::data::engine::normalize_engine_opt(NOTE_DOC_TYPE, Some(&body))
        .unwrap()
        .unwrap();
    let typed: NoteEngine = serde_json::from_value(normalized).unwrap();
    assert_eq!(
        typed.body,
        vec![Segment::RollButton {
            formula: "1d6".to_string(),
            label: None,
        }]
    );
}

#[test]
fn a_doc_span_becomes_a_doc_link() {
    let id = "00000000-0000-0000-0000-000000000001";
    let body = note_body(&format!("[[doc:{id}|x]]"), serde_json::json!([]));
    let normalized = crate::data::engine::normalize_engine_opt(NOTE_DOC_TYPE, Some(&body))
        .unwrap()
        .unwrap();
    let typed: NoteEngine = serde_json::from_value(normalized).unwrap();
    assert_eq!(
        typed.body,
        vec![Segment::DocLink {
            target: DocLinkTarget::Doc {
                doc_id: id.parse().unwrap(),
                embedded_path: None,
            },
            label: "x".to_string(),
        }]
    );
}

#[test]
fn an_asset_span_becomes_an_image() {
    let asset_id = Uuid::new_v4();
    let body = note_body(&format!("[[asset:{asset_id}|x]]"), serde_json::json!([]));
    let normalized = crate::data::engine::normalize_engine_opt(NOTE_DOC_TYPE, Some(&body))
        .unwrap()
        .unwrap();
    let typed: NoteEngine = serde_json::from_value(normalized).unwrap();
    assert_eq!(
        typed.body,
        vec![Segment::Image {
            asset_id,
            alt: "x".to_string(),
        }]
    );
}

#[test]
fn a_malformed_doc_span_is_a_bad_engine_naming_the_reason() {
    let body = note_body("[[doc:bad|x]]", serde_json::json!([]));
    let err = crate::data::engine::normalize_engine_opt(NOTE_DOC_TYPE, Some(&body)).unwrap_err();
    match err {
        DataError::BadEngine(msg) => {
            assert!(
                msg.contains("document/token link"),
                "expected the MalformedDocLink reason in {msg}"
            );
        }
        other => panic!("expected BadEngine, got {other:?}"),
    }
}

#[test]
fn an_over_cap_source_is_a_bad_engine() {
    let body = note_body(
        &"a".repeat(MAX_NOTE_SOURCE_CHARS + 1),
        serde_json::json!([]),
    );
    assert!(crate::data::engine::normalize_engine_opt(NOTE_DOC_TYPE, Some(&body)).is_err());
}

#[test]
fn deny_unknown_fields_rejects_an_extra_key() {
    let mut body = note_body("hello", serde_json::json!([]));
    body["extra"] = serde_json::json!(true);
    assert!(crate::data::engine::normalize_engine_opt(NOTE_DOC_TYPE, Some(&body)).is_err());
}

#[test]
fn validate_engine_rejects_a_note_body_on_a_non_engine_doc_type() {
    let body = note_body("hello", serde_json::json!([]));
    assert!(crate::data::engine::validate_engine("item", Some(&body)).is_err());
}

#[test]
fn a_note_document_may_be_embedded_nowhere() {
    let mut child = note_doc(Uuid::new_v4(), None, "hello");
    let mut parent: Document = serde_json::from_value(serde_json::json!({
        "id": Uuid::new_v4(),
        "scope": { "kind": "world", "world_id": Uuid::new_v4() },
        "doc_type": "item",
        "schema_version": 1,
        "system": {},
        "created_at": 0,
        "updated_at": 0
    }))
    .unwrap();
    child.embedded.clear();
    parent
        .embedded
        .entry("note".to_string())
        .or_default()
        .push(child);
    assert!(crate::data::validation::validate_containment(&parent).is_err());
}

fn note_doc(world_id: Uuid, parent_id: Option<Uuid>, source: &str) -> Document {
    serde_json::from_value(serde_json::json!({
        "id": Uuid::new_v4(),
        "scope": { "kind": "world", "world_id": world_id },
        "doc_type": NOTE_DOC_TYPE,
        "schema_version": 1,
        "parent_id": parent_id,
        "engine": { "source": source, "body": [], "sort": 0 },
        "system": {},
        "created_at": 0,
        "updated_at": 0
    }))
    .unwrap()
}

async fn seed_world() -> (SqliteRepository, Uuid, Uuid) {
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("W", gm, 0).await.unwrap();
    (repo, world.id, gm)
}

#[tokio::test]
async fn an_update_to_engine_source_through_apply_intent_re_derives_the_body() {
    use crate::data::command::{FieldChange, Operation, WriteOrigin};
    use crate::data::document::WorldRole;
    use crate::data::membership::PermissionContext;
    use crate::data::repository::Repository;

    let (repo, world_id, gm) = seed_world().await;
    let doc = note_doc(world_id, None, "hello");
    let doc_id = doc.id;
    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    repo.apply_intent(
        &ctx,
        world_id,
        vec![Operation::Create { doc }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    repo.apply_intent(
        &ctx,
        world_id,
        vec![Operation::Update {
            doc_id,
            changes: vec![FieldChange {
                path: "/engine/source".to_string(),
                old: serde_json::json!("hello"),
                new: serde_json::json!("**bold**"),
                remove: false,
            }],
        }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let stored = repo.get_document(doc_id).await.unwrap().unwrap();
    let engine: NoteEngine = serde_json::from_value(stored.engine.unwrap()).unwrap();
    assert!(matches!(engine.body.as_slice(), [Segment::Html { .. }]));
}

/// A `source` at `MAX_NOTE_SOURCE_CHARS`, made entirely of `&`, which
/// `chat::sanitize`'s HTML-escaping expands ~5x (`&` -> `&amp;`) -- well past
/// `crate::data::validation::MAX_SYSTEM_BYTES` in the DERIVED body, while
/// the raw `source` itself stays comfortably under the cap. This is the
/// exact shape `validate_system_size`'s pre-derivation check cannot see.
fn oversized_escaping_source() -> String {
    "&".repeat(crate::data::engine::note::MAX_NOTE_SOURCE_CHARS)
}

#[tokio::test]
async fn a_create_whose_derived_body_exceeds_the_size_cap_is_rejected() {
    use crate::data::command::{Operation, WriteOrigin};
    use crate::data::document::WorldRole;
    use crate::data::membership::PermissionContext;
    use crate::data::repository::Repository;

    let (repo, world_id, gm) = seed_world().await;
    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let doc = note_doc(world_id, None, &oversized_escaping_source());
    let err = repo
        .apply_intent(
            &ctx,
            world_id,
            vec![Operation::Create { doc }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DataError::TooLarge(_)),
        "expected TooLarge on the DERIVED body, got {err:?}"
    );
}

#[tokio::test]
async fn an_update_whose_derived_body_exceeds_the_size_cap_is_rejected() {
    use crate::data::command::{FieldChange, Operation, WriteOrigin};
    use crate::data::document::WorldRole;
    use crate::data::membership::PermissionContext;
    use crate::data::repository::Repository;

    let (repo, world_id, gm) = seed_world().await;
    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let doc = note_doc(world_id, None, "hello");
    let doc_id = doc.id;
    repo.apply_intent(
        &ctx,
        world_id,
        vec![Operation::Create { doc }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let big = oversized_escaping_source();
    let err = repo
        .apply_intent(
            &ctx,
            world_id,
            vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    path: "/engine/source".to_string(),
                    old: serde_json::json!("hello"),
                    new: serde_json::json!(big),
                    remove: false,
                }],
            }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DataError::TooLarge(_)),
        "expected TooLarge on the DERIVED body, got {err:?}"
    );

    // The rejected Update must not have persisted a half-written derived body.
    let stored = repo.get_document(doc_id).await.unwrap().unwrap();
    let engine: NoteEngine = serde_json::from_value(stored.engine.unwrap()).unwrap();
    assert_eq!(engine.source, "hello");
}
