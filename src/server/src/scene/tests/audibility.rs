//! Tests for `compute_derived`'s `"audibility"` arm: the multi-scene payload shape, the
//! per-scene `listen_as` independence, the scene-visibility gate shared with `"footprints"`,
//! and the catch-all's indifference to the new channel.

use super::*;
use crate::scene::audibility as audibility_channel;

/// A sound emission body (`eng::SoundEmission`'s wire shape) for the fixtures.
fn sound() -> serde_json::Value {
    json!({ "asset": "a-wind", "radius": 6.0, "volume": 0.8, "loop": true, "enabled": true })
}

/// An actor body carrying the given sound emission, built on the shared actor fixture.
fn actor_with_sound(snd: serde_json::Value) -> serde_json::Value {
    let mut body = actor_body(json!([]));
    body["sound"] = snd;
    body
}

/// A scene document on a 100-unit square grid (the scale every distance expectation derives from).
fn scene_doc(id: u128) -> Document {
    entity_doc_top_eng(
        id,
        "scene",
        json!({ "grid": { "kind": "square", "size": 100.0 }, "background": null }),
    )
}

/// A scene-parented token LINKED to actor `Uuid::from_u128(actor_id)` at `(x, y)`.
fn linked_token(id: u128, scene: u128, actor_id: u128, x: f64, y: f64) -> Document {
    entity_doc_eng(
        id,
        scene,
        "token",
        json!({ "x": x, "y": y, "w": 100.0, "h": 100.0, "rotation": 0.0,
                "actor_id": Uuid::from_u128(actor_id).to_string() }),
    )
}

/// A scene-parented raw (actor-less) token at `(x, y)` owned by `owner`.
fn owned_token(id: u128, scene: u128, x: f64, y: f64, owner: Uuid) -> Document {
    let mut d = entity_doc_eng(
        id,
        scene,
        "token",
        json!({ "x": x, "y": y, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    d.owner = Some(owner);
    d
}

/// A document a `WorldRole::Player` may READ (`PermissionSet::default` is `DocRole::None`, so a
/// fixture document is invisible to a player unless it says otherwise — a player-facing
/// assertion would otherwise pass vacuously).
fn readable(mut d: Document) -> Document {
    d.permissions.default = crate::data::document::DocRole::Observer;
    d
}

/// `compute_derived("audibility", ...)` deserialized into the payload type (the channel's own
/// parse, never a hand-read of the JSON).
fn audibility_payload(
    ecs: &SceneEcs,
    ctx: &PermissionContext,
    listen_as: Option<Uuid>,
) -> audibility_channel::AudibilityPayload {
    let value = compute_derived("audibility", ecs, ctx, &no_world_grants(), listen_as)
        .expect("the audibility arm always produces a payload");
    serde_json::from_value(value).expect("the payload deserializes into AudibilityPayload")
}

#[test]
fn audibility_with_no_tokens_anywhere_is_an_empty_scenes_list() {
    let ecs = SceneEcs::from_documents(vec![scene_doc(10)], 0);
    let payload = audibility_payload(&ecs, &player_ctx(), None);
    assert!(payload.scenes.is_empty(), "scenes: [] — never a sentinel");
}

/// The two-scene fixture: scene 10 and scene 20, `user`'s listener token in each, and one
/// emitter in each (both 200 world units from their listener — one third of the 600-unit
/// radius).
fn two_scene_fixture() -> (SceneEcs, Uuid) {
    let user = Uuid::from_u128(7);
    let mut ecs = SceneEcs::from_documents(
        vec![
            readable(scene_doc(10)),
            readable(scene_doc(20)),
            owned_token(11, 10, 50.0, 50.0, user),
            owned_token(21, 20, 50.0, 50.0, user),
            readable(linked_token(12, 10, 200, 250.0, 50.0)),
            readable(linked_token(22, 20, 201, 250.0, 50.0)),
        ],
        0,
    );
    ecs.set_actors(vec![
        entity_doc_top_eng(200, "actor", actor_with_sound(sound())),
        entity_doc_top_eng(201, "actor", actor_with_sound(sound())),
    ]);
    (ecs, user)
}

#[test]
fn audibility_payload_carries_one_slice_per_tokened_visible_scene() {
    let (ecs, user) = two_scene_fixture();
    let payload = audibility_payload(&ecs, &player_ctx_of(user), None);
    assert_eq!(
        payload.scenes.len(),
        2,
        "both scenes arrive in ONE payload — no second round-trip"
    );
    let a = payload
        .scenes
        .iter()
        .find(|s| s.scene == Uuid::from_u128(10))
        .expect("scene A's slice");
    let b = payload
        .scenes
        .iter()
        .find(|s| s.scene == Uuid::from_u128(20))
        .expect("scene B's slice");
    assert_eq!(a.listener, Some(Uuid::from_u128(11)));
    assert_eq!(b.listener, Some(Uuid::from_u128(21)));
    assert_eq!(a.emitters.len(), 1);
    assert_eq!(b.emitters.len(), 1);
    assert_eq!(a.emitters[0].token, Uuid::from_u128(12));
    assert_eq!(b.emitters[0].token, Uuid::from_u128(22));
    // Same geometry in both: gain folds volume * falloff(1/3) = 0.8 * 8/9, pan = 1/3.
    for slice in [a, b] {
        assert!((slice.emitters[0].gain - 0.8 * 8.0 / 9.0).abs() < 1e-9);
        assert!((slice.emitters[0].pan - 1.0 / 3.0).abs() < 1e-9);
    }
}

#[test]
fn listen_as_overrides_the_listener_per_scene_independently() {
    let (ecs, user) = two_scene_fixture();
    // An override naming a READABLE (not owned) token of scene A: scene A listens as the
    // override, scene B still resolves its own owned-token fallback.
    let mut override_token = owned_token(13, 10, 100.0, 50.0, Uuid::from_u128(99));
    override_token = readable(override_token);
    let mut ecs = ecs;
    ecs.apply_op(&Operation::Create {
        doc: override_token,
    });

    let first = audibility_payload(&ecs, &player_ctx_of(user), Some(Uuid::from_u128(13)));
    let a = first
        .scenes
        .iter()
        .find(|s| s.scene == Uuid::from_u128(10))
        .expect("scene A's slice");
    let b = first
        .scenes
        .iter()
        .find(|s| s.scene == Uuid::from_u128(20))
        .expect("scene B's slice");
    assert_eq!(
        a.listener,
        Some(Uuid::from_u128(13)),
        "the override decides scene A's listener"
    );
    assert_eq!(
        b.listener,
        Some(Uuid::from_u128(21)),
        "scene B keeps its owned-token fallback"
    );

    // Honored across a re-computed call: the same override yields the identical payload.
    let second = audibility_payload(&ecs, &player_ctx_of(user), Some(Uuid::from_u128(13)));
    assert_eq!(
        first, second,
        "a recompute with an unchanged override is stable"
    );
}

#[test]
fn an_unrelated_channel_still_returns_none() {
    let (ecs, user) = two_scene_fixture();
    assert!(compute_derived(
        "not-a-real-channel",
        &ecs,
        &player_ctx_of(user),
        &no_world_grants(),
        None
    )
    .is_none());
}

/// The hidden-scene fixture: scene 10 visible to the player, scene 20 (holding tokens, with an
/// emitter) denied to them via `DocRole::None`.
fn hidden_scene_fixture() -> (SceneEcs, Uuid) {
    let user = Uuid::from_u128(7);
    let hidden_scene = scene_doc(20); // DocRole::None: the player cannot see this scene at all
    let mut ecs = SceneEcs::from_documents(
        vec![
            readable(scene_doc(10)),
            hidden_scene,
            owned_token(11, 10, 50.0, 50.0, user),
            owned_token(21, 20, 50.0, 50.0, user),
            readable(linked_token(22, 20, 200, 250.0, 50.0)),
        ],
        0,
    );
    ecs.set_actors(vec![entity_doc_top_eng(
        200,
        "actor",
        actor_with_sound(sound()),
    )]);
    (ecs, user)
}

#[test]
fn audibility_never_discloses_a_scene_the_recipient_cannot_see() {
    let (ecs, user) = hidden_scene_fixture();
    let payload = audibility_payload(&ecs, &player_ctx_of(user), None);
    assert_eq!(
        payload.scenes.len(),
        1,
        "scene B holds tokens yet is withheld from a player who cannot see the scene document"
    );
    assert_eq!(payload.scenes[0].scene, Uuid::from_u128(10));
}

#[test]
fn audibility_and_footprints_agree_on_scene_visibility() {
    // Cross-channel anti-drift: for the SAME player the hidden scene is absent from BOTH
    // channels, and for the GM present in BOTH — a value change or inverted predicate in the
    // shared `scene_visible_to` fails this test on one channel or the other.
    let (ecs, user) = hidden_scene_fixture();
    let player = player_ctx_of(user);
    let footprints = ecs.resolved_footprints(&player, &no_world_grants());
    let audibility = audibility_payload(&ecs, &player, None);
    let fp_scenes: Vec<Uuid> = footprints.scenes.iter().map(|s| s.scene).collect();
    let au_scenes: Vec<Uuid> = audibility.scenes.iter().map(|s| s.scene).collect();
    assert_eq!(
        fp_scenes, au_scenes,
        "both channels withhold exactly the same scenes"
    );

    let gm = gm_ctx_of(Uuid::from_u128(1));
    let gm_footprints = ecs.resolved_footprints(&gm, &no_world_grants());
    let gm_audibility = audibility_payload(&ecs, &gm, None);
    let gm_fp: Vec<Uuid> = gm_footprints.scenes.iter().map(|s| s.scene).collect();
    let gm_au: Vec<Uuid> = gm_audibility.scenes.iter().map(|s| s.scene).collect();
    assert_eq!(
        gm_fp.len(),
        2,
        "the GM receives both scenes on the footprints channel"
    );
    assert_eq!(
        gm_au.len(),
        2,
        "the GM receives both scenes on the audibility channel"
    );
    assert_eq!(gm_fp, gm_au);
}

/// A `WorldRole::Player` context for the shared fixture user.
fn player_ctx_of(user: Uuid) -> PermissionContext {
    PermissionContext {
        user_id: user,
        world_role: WorldRole::Player,
    }
}

/// A `WorldRole::Player` context for a fresh user id.
fn player_ctx() -> PermissionContext {
    player_ctx_of(Uuid::from_u128(7))
}

/// A `WorldRole::Gm` context for `user`.
fn gm_ctx_of(user: Uuid) -> PermissionContext {
    PermissionContext {
        user_id: user,
        world_role: WorldRole::Gm,
    }
}
