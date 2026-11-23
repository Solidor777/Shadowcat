//! These tests cover the `"bars"` derived channel (`SceneEcs::resolved_bars`): which recipient
//! receives which nameplate and bar, and that every value is the server's own evaluation of the
//! recipient's redacted view of the host document.

use super::*;
use crate::data::document::{DocRole, Visibility};
use crate::data::engine::BarDisplayMode;
use crate::scene::bars::{BarsPayload, TokenBars};

/// The GM's context.
fn gm() -> PermissionContext {
    PermissionContext {
        user_id: crate::test_ids::test_uuid(1),
        world_role: WorldRole::Gm,
    }
}

/// The context of the player who owns the fixture token.
fn owner() -> PermissionContext {
    PermissionContext {
        user_id: crate::test_ids::test_uuid(77),
        world_role: WorldRole::Player,
    }
}

/// A player who owns nothing.
fn stranger() -> PermissionContext {
    PermissionContext {
        user_id: crate::test_ids::test_uuid(78),
        world_role: WorldRole::Player,
    }
}

/// `d`, readable by every player.
fn readable(mut d: Document) -> Document {
    d.permissions.default = DocRole::Observer;
    d
}

/// One authored bar as the JSON an actor's `engine.bars` carries.
fn bar_json(label: &str, current: &str, max: &str, mode: &str) -> serde_json::Value {
    json!({ "label": label, "current": current, "max": max, "color": "#cc2222", "mode": mode })
}

/// An actor engine body carrying `extra` keys (`bars`, `nameplate_mode`) over the fixture base.
fn actor_engine(extra: serde_json::Value) -> serde_json::Value {
    let mut body = json!({
        "displayName": "Fixture Actor",
        "visual": { "kind": "image", "asset": "11111111-1111-1111-1111-111111111111" },
        "size": { "w": 1.0, "h": 1.0 },
        "shape": "square",
        "conditions": [],
        "prototype": true,
    });
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    body
}

/// A readable actor (id 200) named `name`, whose `system` band holds `hp` and a `secret`.
fn actor(name: Option<&str>, extra: serde_json::Value) -> Document {
    let mut a = readable(entity_doc_top_eng(200, "actor", actor_engine(extra)));
    a.name = name.map(str::to_string);
    a.owner = Some(owner().user_id);
    a.system = json!({ "hp": { "value": 7.0, "max": 10.0 }, "secret": 3.0 });
    a
}

/// A readable token (id 11, scene 10) linked to actor 200, owned by `owner()`.
fn linked_token(engine_extra: serde_json::Value) -> Document {
    let mut body = json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0,
        "actor_id": crate::test_ids::test_uuid(200).to_string() });
    for (k, v) in engine_extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    let mut t = readable(entity_doc_eng(11, 10, "token", body));
    t.owner = Some(owner().user_id);
    t
}

/// A hydrated ECS: a readable scene, `token`, and `actor` as the shared actor.
fn ecs_with(token: Document, actor: Document) -> SceneEcs {
    let mut ecs = SceneEcs::from_documents(
        vec![
            readable(entity_doc_top_eng(
                10,
                "scene",
                json!({ "grid": { "kind": "square", "size": 100.0 }, "background": null }),
            )),
            token,
        ],
        0,
    );
    ecs.set_actors(vec![actor]);
    ecs
}

/// The fixture token's entry in the payload built for `ctx`.
fn entry_for(ecs: &SceneEcs, ctx: &PermissionContext) -> Option<TokenBars> {
    let payload = ecs.resolved_bars(ctx, &no_world_grants());
    payload
        .scenes
        .get(&crate::test_ids::test_uuid(10))
        .and_then(|s| s.tokens.get(&crate::test_ids::test_uuid(11)))
        .cloned()
}

/// The labels of an entry's bars.
fn labels(entry: &Option<TokenBars>) -> Vec<String> {
    entry
        .as_ref()
        .map(|e| e.bars.iter().map(|b| b.label.clone()).collect())
        .unwrap_or_default()
}

/// An actor with one HP bar of `mode` and no nameplate for anyone.
fn hp_actor(mode: &str) -> Document {
    actor(
        None,
        json!({ "bars": [bar_json("HP", "hp.value", "hp.max", mode)], "nameplate_mode": "none" }),
    )
}

#[test]
fn always_bar_reaches_a_non_owner_with_server_evaluated_numbers() {
    let ecs = ecs_with(linked_token(json!({})), hp_actor("always"));
    let got = entry_for(&ecs, &stranger()).expect("an always bar reaches every reader");
    assert_eq!(got.bars.len(), 1);
    assert_eq!((got.bars[0].current, got.bars[0].max), (7.0, 10.0));
    assert_eq!(got.bars[0].color, "#cc2222");
}

#[test]
fn owner_mode_bar_is_withheld_from_a_non_owner_and_delivered_to_owner_and_gm() {
    // Mutation: inverting the `trusted` test in `BarDisplayMode::delivered_to` fails this.
    let ecs = ecs_with(linked_token(json!({})), hp_actor("owner"));
    assert_eq!(entry_for(&ecs, &stranger()), None);
    assert_eq!(labels(&entry_for(&ecs, &owner())), vec!["HP"]);
    assert_eq!(labels(&entry_for(&ecs, &gm())), vec!["HP"]);
}

#[test]
fn hover_mode_bar_is_withheld_from_a_non_owner_and_delivered_to_owner_and_gm() {
    let ecs = ecs_with(linked_token(json!({})), hp_actor("hover"));
    assert_eq!(entry_for(&ecs, &stranger()), None);
    assert_eq!(labels(&entry_for(&ecs, &owner())), vec!["HP"]);
    assert_eq!(labels(&entry_for(&ecs, &gm())), vec!["HP"]);
}

#[test]
fn none_mode_bar_reaches_no_one_including_owner_and_gm() {
    let ecs = ecs_with(linked_token(json!({})), hp_actor("none"));
    for ctx in [stranger(), owner(), gm()] {
        assert_eq!(entry_for(&ecs, &ctx), None);
    }
}

#[test]
fn a_token_the_recipient_cannot_read_has_no_entry_whatever_the_mode() {
    // Mutation: dropping the `token_footprint_visible` gate in `resolved_bars` fails this.
    let mut hidden = linked_token(json!({}));
    hidden.permissions.default = DocRole::None;
    let ecs = ecs_with(hidden, hp_actor("always"));
    assert_eq!(entry_for(&ecs, &stranger()), None);
    assert_eq!(labels(&entry_for(&ecs, &gm())), vec!["HP"]);
}

#[test]
fn a_bar_over_a_leaf_the_recipient_cannot_read_is_omitted_for_that_recipient() {
    // The formula reads a GM-only `system` leaf. An `always` bar must not turn that leaf into a
    // number a player receives. Mutation: evaluating over the unredacted host fails this.
    let mut a = actor(
        None,
        json!({ "bars": [
            bar_json("Secret", "secret", "10", "always"),
            bar_json("HP", "hp.value", "hp.max", "always"),
        ] }),
    );
    a.permissions
        .property_overrides
        .insert("/system/secret".into(), Visibility::GmOnly);
    let ecs = ecs_with(linked_token(json!({})), a);
    assert_eq!(labels(&entry_for(&ecs, &stranger())), vec!["HP"]);
    assert_eq!(labels(&entry_for(&ecs, &owner())), vec!["HP"]);
    assert_eq!(labels(&entry_for(&ecs, &gm())), vec!["Secret", "HP"]);
}

#[test]
fn an_owner_or_gm_leaf_feeds_a_bar_only_to_the_owner_and_gm() {
    let mut a = hp_actor("always");
    a.permissions
        .property_overrides
        .insert("/system/hp".into(), Visibility::OwnerOrGm);
    let ecs = ecs_with(linked_token(json!({})), a);
    assert_eq!(entry_for(&ecs, &stranger()), None);
    assert_eq!(labels(&entry_for(&ecs, &owner())), vec!["HP"]);
}

#[test]
fn a_failing_formula_skips_only_its_own_bar() {
    let a = actor(
        None,
        json!({ "bars": [
            bar_json("Broken", "no.such.leaf", "10", "always"),
            bar_json("HP", "hp.value", "hp.max", "always"),
        ] }),
    );
    let ecs = ecs_with(linked_token(json!({})), a);
    assert_eq!(labels(&entry_for(&ecs, &stranger())), vec!["HP"]);
}

#[test]
fn a_non_finite_result_skips_its_bar() {
    let a = actor(
        None,
        json!({ "bars": [bar_json("Div", "1 / 0", "10", "always")] }),
    );
    let ecs = ecs_with(linked_token(json!({})), a);
    assert_eq!(entry_for(&ecs, &stranger()), None);
}

#[test]
fn a_token_override_replaces_the_actors_bars_wholesale() {
    let token = linked_token(json!({ "overrides": {
        "bars": [bar_json("Morale", "5", "8", "always")]
    } }));
    let ecs = ecs_with(token, hp_actor("always"));
    assert_eq!(labels(&entry_for(&ecs, &stranger())), vec!["Morale"]);
}

#[test]
fn an_instanced_token_reads_its_embedded_copy_and_ignores_overrides() {
    let mut token = readable(entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    ));
    token.owner = Some(owner().user_id);
    token
        .embedded
        .insert("actor".into(), vec![hp_actor("always")]);
    let mut ecs = SceneEcs::from_documents(
        vec![
            readable(entity_doc_top_eng(
                10,
                "scene",
                json!({ "grid": { "kind": "square", "size": 100.0 }, "background": null }),
            )),
            token,
        ],
        0,
    );
    ecs.set_actors(vec![]);
    let got = entry_for(&ecs, &stranger()).expect("the embedded copy's bar is delivered");
    assert_eq!((got.bars[0].current, got.bars[0].max), (7.0, 10.0));
}

#[test]
fn nameplate_is_the_override_name_then_actor_name_then_display_name() {
    let by_name = ecs_with(
        linked_token(json!({ "overrides": { "name": "Override" } })),
        actor(Some("Actor Name"), json!({ "nameplate_mode": "always" })),
    );
    assert_eq!(
        entry_for(&by_name, &stranger())
            .unwrap()
            .nameplate
            .as_deref(),
        Some("Override")
    );
    let by_actor = ecs_with(
        linked_token(json!({})),
        actor(Some("Actor Name"), json!({ "nameplate_mode": "always" })),
    );
    assert_eq!(
        entry_for(&by_actor, &stranger())
            .unwrap()
            .nameplate
            .as_deref(),
        Some("Actor Name")
    );
    let by_display = ecs_with(
        linked_token(json!({})),
        actor(None, json!({ "nameplate_mode": "always" })),
    );
    assert_eq!(
        entry_for(&by_display, &stranger())
            .unwrap()
            .nameplate
            .as_deref(),
        Some("Fixture Actor")
    );
}

#[test]
fn nameplate_default_mode_is_hover_so_a_non_owner_gets_none() {
    let ecs = ecs_with(
        linked_token(json!({})),
        actor(Some("Actor Name"), json!({})),
    );
    assert_eq!(entry_for(&ecs, &stranger()), None);
    assert_eq!(
        entry_for(&ecs, &owner()).unwrap().nameplate.as_deref(),
        Some("Actor Name")
    );
}

#[test]
fn a_token_override_replaces_the_nameplate_mode() {
    let ecs = ecs_with(
        linked_token(json!({ "overrides": { "nameplate_mode": "always" } })),
        actor(Some("Actor Name"), json!({ "nameplate_mode": "none" })),
    );
    assert_eq!(
        entry_for(&ecs, &stranger()).unwrap().nameplate.as_deref(),
        Some("Actor Name")
    );
}

#[test]
fn a_secret_identity_token_nameplate_is_blank_for_a_non_owner_and_named_for_the_owner() {
    // Mutation: resolving the name from the unredacted token (ignoring `filter_properties`)
    // makes the stranger's nameplate "Real Name" and fails this.
    let ecs = ecs_with(
        linked_token(json!({ "identity_hidden": true })),
        actor(Some("Real Name"), json!({ "nameplate_mode": "always" })),
    );
    // The nameplate is delivered (mode `always`) but names nothing: the entry carries the mode
    // and no text, and the client draws its own fallback label.
    let blank = entry_for(&ecs, &stranger()).expect("a delivered nameplate keeps its entry");
    assert_eq!(blank.nameplate, None);
    assert_eq!(blank.nameplate_mode, Some(BarDisplayMode::Always));
    assert_eq!(
        entry_for(&ecs, &owner()).unwrap().nameplate.as_deref(),
        Some("Real Name")
    );
    assert_eq!(
        entry_for(&ecs, &gm()).unwrap().nameplate.as_deref(),
        Some("Real Name")
    );
}

#[test]
fn the_entry_states_the_delivered_nameplate_mode_and_each_bars_mode() {
    // Mutation: dropping `nameplate_mode` from the entry, or `mode` from a bar, fails this.
    let ecs = ecs_with(
        linked_token(json!({})),
        actor(
            Some("Actor Name"),
            json!({
                "bars": [bar_json("HP", "hp.value", "hp.max", "always")],
                "nameplate_mode": "owner",
            }),
        ),
    );
    let seen_by_stranger = entry_for(&ecs, &stranger()).unwrap();
    assert_eq!(seen_by_stranger.nameplate, None);
    assert_eq!(seen_by_stranger.nameplate_mode, None);
    assert_eq!(seen_by_stranger.bars[0].mode, BarDisplayMode::Always);
    let seen_by_owner = entry_for(&ecs, &owner()).unwrap();
    assert_eq!(seen_by_owner.nameplate_mode, Some(BarDisplayMode::Owner));
}

#[test]
fn a_hidden_actor_name_is_not_a_nameplate_for_a_non_owner() {
    let mut a = actor(Some("Hidden Name"), json!({ "nameplate_mode": "always" }));
    a.permissions
        .property_overrides
        .insert("/name".into(), Visibility::OwnerOrGm);
    let ecs = ecs_with(linked_token(json!({})), a);
    // The name is withheld, so the recipient's own `displayName` fallback stands.
    assert_eq!(
        entry_for(&ecs, &stranger()).unwrap().nameplate.as_deref(),
        Some("Fixture Actor")
    );
    assert_eq!(
        entry_for(&ecs, &owner()).unwrap().nameplate.as_deref(),
        Some("Hidden Name")
    );
}

#[test]
fn compute_derived_serves_the_bars_channel() {
    let ecs = ecs_with(linked_token(json!({})), hp_actor("always"));
    let value = compute_derived("bars", &ecs, &stranger(), &no_world_grants(), None)
        .expect("bars is a registered channel");
    let payload: BarsPayload = serde_json::from_value(value).unwrap();
    assert_eq!(payload.scenes.len(), 1);
}
