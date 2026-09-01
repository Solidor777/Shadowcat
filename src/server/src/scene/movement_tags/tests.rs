//! Tests for `scene::movement_tags` — the tag-set resolution precedence and the
//! reserved-semantics predicate.

use super::*;
use serde_json::json;

/// Builds a world-scoped fixture document of type `ty`, parented to `parent` when given.
fn doc(id: u128, parent: Option<u128>, ty: &str) -> crate::data::document::Document {
    let mut d = crate::data::document::tests::world_scoped_doc(
        Uuid::from_u128(9),
        Uuid::from_u128(id),
        ty,
    );
    d.parent_id = parent.map(Uuid::from_u128);
    d
}

/// Builds a scene-entity fixture with `engine` set to `body`.
fn entity_doc_eng(
    id: u128,
    parent: u128,
    ty: &str,
    body: serde_json::Value,
) -> crate::data::document::Document {
    let mut d = doc(id, Some(parent), ty);
    d.engine = Some(body);
    d
}

/// A minimal, structurally-complete `eng::ActorEngine` body; the caller layers on
/// `movement`/`faction` via `serde_json`'s object mutation.
fn actor_body() -> serde_json::Value {
    json!({
        "displayName": "Fixture Actor",
        "visual": { "kind": "image", "asset": "a" },
        "size": { "w": 1.0, "h": 1.0 },
        "shape": "square",
        "faction": null,
        "conditions": [],
        "prototype": true
    })
}

/// A token engine body at the origin, `actor_id` linked when given, `overrides` merged in raw.
fn token_body(actor_id: Option<u128>, overrides: Option<serde_json::Value>) -> serde_json::Value {
    let mut b = json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 });
    if let Some(id) = actor_id {
        b["actor_id"] = json!(Uuid::from_u128(id).to_string());
    }
    if let Some(o) = overrides {
        b["overrides"] = o;
    }
    b
}

/// A faction-registry singleton whose `skyborn` faction carries `["flying"]`.
fn faction_registry_doc() -> crate::data::document::Document {
    let mut d = doc(50, None, "faction-registry");
    d.engine = Some(json!({
        "factions": {
            "skyborn": { "name": "Skyborn", "color": "#88c", "stance": "neutral",
                         "movement": ["flying"] }
        }
    }));
    d
}

/// An ECS holding the scene shell, the given extra docs (actors join the actor side-table via
/// `set_actors`, the SAME slot the room hydration uses — `from_documents` hydrates scene
/// entities only), and the faction registry hydrated through `set_world_config`.
fn ecs_with(docs: Vec<crate::data::document::Document>) -> SceneEcs {
    let (actors, entities): (Vec<_>, Vec<_>) = docs.into_iter().partition(|d| d.doc_type == "actor");
    let mut all = vec![doc(10, None, "scene")];
    all.extend(entities);
    let mut ecs = SceneEcs::from_documents(all, 0);
    ecs.set_actors(actors);
    ecs.set_world_config(None, None, None, None, None, Some(faction_registry_doc()));
    ecs
}

#[test]
fn linked_token_unions_actor_and_faction_tags_deduplicated() {
    let mut ab = actor_body();
    ab["faction"] = json!("skyborn");
    ab["movement"] = json!(["flying", "ethereal-step"]);
    let actor = doc(200, None, "actor");
    let actor = crate::data::document::Document {
        engine: Some(ab),
        ..actor
    };
    let linked = entity_doc_eng(300, 10, "token", token_body(Some(200), None));
    let ecs = ecs_with(vec![actor, linked]);
    let tags = ecs.token_movement_tags(Uuid::from_u128(300));
    assert_eq!(
        tags.iter().cloned().collect::<Vec<_>>(),
        vec!["ethereal-step", "flying"] // BTreeSet order; the duplicate "flying" deduped
    );
    assert!(ignores_terrain_cost(&tags));
}

#[test]
fn token_override_replaces_the_whole_set_even_when_empty() {
    let mut ab = actor_body();
    ab["faction"] = json!("skyborn");
    ab["movement"] = json!(["ethereal-step"]);
    let mut actor = doc(200, None, "actor");
    actor.engine = Some(ab);

    // An explicit EMPTY override strips every inherited tag (wholesale replacement).
    let grounded = entity_doc_eng(
        300,
        10,
        "token",
        token_body(Some(200), Some(json!({ "movement": [] }))),
    );
    let ecs = ecs_with(vec![actor.clone(), grounded]);
    assert_eq!(
        ecs.token_movement_tags(Uuid::from_u128(300)),
        BTreeSet::new()
    );

    // A non-empty override replaces actor ∪ faction entirely.
    let burrower = entity_doc_eng(
        301,
        10,
        "token",
        token_body(Some(200), Some(json!({ "movement": ["burrowing"] }))),
    );
    let ecs = ecs_with(vec![actor, burrower]);
    let tags = ecs.token_movement_tags(Uuid::from_u128(301));
    assert_eq!(tags.into_iter().collect::<Vec<_>>(), vec!["burrowing"]);
    assert!(!ignores_terrain_cost(&ecs.token_movement_tags(Uuid::from_u128(301))));
}

#[test]
fn dangling_actor_link_yields_empty_overrides_ignored() {
    // Mirrors `SceneEcs::token_vision_assignments`'s dangling arm: a link to an absent actor
    // resolves nothing, and the token's own override does not rescue it.
    let dangling = entity_doc_eng(
        300,
        10,
        "token",
        token_body(Some(999), Some(json!({ "movement": ["flying"] }))),
    );
    let ecs = ecs_with(vec![dangling]);
    assert_eq!(
        ecs.token_movement_tags(Uuid::from_u128(300)),
        BTreeSet::new()
    );
}

#[test]
fn dangling_faction_link_contributes_nothing() {
    let mut ab = actor_body();
    ab["faction"] = json!("gone");
    ab["movement"] = json!(["burrowing"]);
    let mut actor = doc(200, None, "actor");
    actor.engine = Some(ab);
    let linked = entity_doc_eng(300, 10, "token", token_body(Some(200), None));
    let ecs = ecs_with(vec![actor, linked]);
    let tags = ecs.token_movement_tags(Uuid::from_u128(300));
    assert_eq!(tags.into_iter().collect::<Vec<_>>(), vec!["burrowing"]);
}

#[test]
fn instanced_token_reads_embedded_copy_and_joins_faction_registry() {
    let mut ab = actor_body();
    ab["faction"] = json!("skyborn");
    let mut tok = entity_doc_eng(300, 10, "token", token_body(None, None));
    let mut embedded = doc(400, None, "actor");
    embedded.engine = Some(ab);
    tok.embedded.insert("actor".to_string(), vec![embedded]);
    let ecs = ecs_with(vec![tok]);
    let tags = ecs.token_movement_tags(Uuid::from_u128(300));
    assert_eq!(tags.into_iter().collect::<Vec<_>>(), vec!["flying"]);

    // Overrides do not apply to instanced tokens.
    let mut tok2 = entity_doc_eng(
        301,
        10,
        "token",
        token_body(None, Some(json!({ "movement": ["burrowing"] }))),
    );
    let mut embedded2 = doc(401, None, "actor");
    embedded2.engine = Some(actor_body());
    tok2.embedded.insert("actor".to_string(), vec![embedded2]);
    let ecs2 = ecs_with(vec![tok2]);
    assert_eq!(
        ecs2.token_movement_tags(Uuid::from_u128(301)),
        BTreeSet::new()
    );
}

#[test]
fn embedded_actor_edits_are_never_stale() {
    // The embedded branch is deliberately UNCACHED: an `/embedded/actor/0/...` write must be
    // visible to the very next resolution (the rule `token_vision_assignments`'s embedded
    // branch documents).
    let mut tok = entity_doc_eng(300, 10, "token", token_body(None, None));
    let mut embedded = doc(400, None, "actor");
    embedded.engine = Some(actor_body());
    tok.embedded.insert("actor".to_string(), vec![embedded]);
    let mut ecs = ecs_with(vec![tok]);
    assert_eq!(
        ecs.token_movement_tags(Uuid::from_u128(300)),
        BTreeSet::new()
    );
    ecs.apply_op(&crate::data::command::Operation::Update {
        doc_id: Uuid::from_u128(300),
        changes: vec![crate::data::command::FieldChange {
            path: "/embedded/actor/0/engine/movement".to_string(),
            old: serde_json::Value::Null,
            new: json!(["incorporeal"]),
            remove: false,
        }],
    });
    let tags = ecs.token_movement_tags(Uuid::from_u128(300));
    assert_eq!(tags.iter().cloned().collect::<Vec<_>>(), vec!["incorporeal"]);
    assert!(ignores_terrain_cost(&tags));
}

#[test]
fn raw_or_unknown_token_resolves_empty() {
    let raw = entity_doc_eng(300, 10, "token", token_body(None, None));
    let ecs = ecs_with(vec![raw]);
    assert_eq!(
        ecs.token_movement_tags(Uuid::from_u128(300)),
        BTreeSet::new()
    );
    // An id the ECS does not hold at all fails closed the same way.
    assert_eq!(
        ecs.token_movement_tags(Uuid::from_u128(999)),
        BTreeSet::new()
    );
}

#[test]
fn reserved_tag_predicate_matches_exactly_the_reserved_set() {
    let tags = |tags: &[&str]| tags.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
    assert!(ignores_terrain_cost(&tags(&["flying"])));
    assert!(ignores_terrain_cost(&tags(&["incorporeal"])));
    assert!(ignores_terrain_cost(&tags(&["burrowing", "flying"])));
    assert!(!ignores_terrain_cost(&tags(&["burrowing"])));
    assert!(!ignores_terrain_cost(&tags(&["flyingness"]))); // prefix is not membership
    assert!(!ignores_terrain_cost(&tags(&[])));
}
