//! These tests cover `scene::audibility`.
//!
//! They exercise the falloff/pan/occlusion primitives, overlay resolution, carried-emission
//! precedence, listener selection, and the resolved per-scene slice.

use super::*;
use crate::data::command::Operation;
use crate::data::document::{DocRole, WorldRole};
use crate::data::membership::PermissionContext;
use crate::scene::tests::{
    actor_body, entity_doc_eng, entity_doc_top_eng, no_world_grants, ws_body,
};
use serde_json::json;

/// A sound emission body (`eng::SoundEmission`'s wire shape) for the fixtures: radius 6 cells
/// (= 600 world units on the fixtures' 100-unit square grid), volume 0.8.
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

/// A scene-parented token LINKED to actor `crate::test_ids::test_uuid(actor_id)` at `(x, y)`.
fn linked_token(id: u128, scene: u128, actor_id: u128, x: f64, y: f64) -> Document {
    entity_doc_eng(
        id,
        scene,
        "token",
        json!({ "x": x, "y": y, "w": 100.0, "h": 100.0, "rotation": 0.0,
                "actor_id": crate::test_ids::test_uuid(actor_id).to_string() }),
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
    d.permissions.default = DocRole::Observer;
    d
}

/// A `WorldRole::Player` context for `user`.
fn player_ctx(user: Uuid) -> PermissionContext {
    PermissionContext {
        user_id: user,
        world_role: WorldRole::Player,
    }
}

/// A `WorldRole::Gm` context for `user`.
fn gm_ctx(user: Uuid) -> PermissionContext {
    PermissionContext {
        user_id: user,
        world_role: WorldRole::Gm,
    }
}

#[test]
fn falloff_truth_table() {
    assert_eq!(falloff(0.0), 1.0);
    assert_eq!(falloff(1.0), 0.0);
    assert_eq!(falloff(2.0), 0.0);
    assert!((falloff(0.5) - 0.75).abs() < 1e-9);
    assert_eq!(falloff(f64::NAN), 0.0);
    assert_eq!(falloff(f64::INFINITY), 0.0);
}

#[test]
fn pan_for_truth_table() {
    assert_eq!(pan_for(0.0, 10.0), 0.0);
    assert_eq!(pan_for(10.0, 10.0), 1.0);
    assert_eq!(pan_for(-20.0, 10.0), -1.0);
    assert_eq!(pan_for(5.0, 0.0), 0.0);
    assert_eq!(pan_for(5.0, f64::NAN), 0.0);
}

#[test]
fn segment_occluded_agrees_with_segments_cross() {
    // Anti-drift parity: audibility's occlusion IS `segments_cross` over the caller-filtered
    // wall set, so a crossing and a non-crossing case must agree with the primitive the movement
    // gate also uses, verbatim.
    let crossing = vision::Seg {
        a: (5.0, -5.0),
        b: (5.0, 5.0),
        one_way: None,
        limited: false,
        open_within: None,
    };
    let clear = vision::Seg {
        a: (5.0, -5.0),
        b: (5.0, -1.0),
        one_way: None,
        limited: false,
        open_within: None,
    };
    for (listener, emitter, wall) in [
        ((0.0, 0.0), (10.0, 0.0), crossing),
        ((0.0, 0.0), (3.0, 0.0), crossing),
        ((0.0, 0.0), (10.0, 0.0), clear),
        // A touching endpoint counts as blocked (`segments_cross` reads a touching endpoint as crossing).
        ((0.0, 0.0), (5.0, 5.0), crossing),
    ] {
        assert_eq!(
            segment_occluded(listener, emitter, &[wall]),
            (segments_cross(listener, emitter, wall.a, wall.b), 1.0),
            "segment_occluded must agree with segments_cross for {listener:?} -> {emitter:?}"
        );
    }
}

#[test]
fn resolve_audio_overlay_defaults_without_a_world_settings_doc() {
    let ecs = SceneEcs::new();
    assert_eq!(
        ecs.resolve_audio_overlay(),
        ResolvedAudioOverlay {
            spatial: true,
            occlusion: eng::Occlusion::Walls,
            through_wall_gain: 0.25
        },
    );
}

#[test]
fn resolve_audio_overlay_fills_only_the_unauthored_leaves() {
    let mut ecs = SceneEcs::new();
    ecs.set_world_settings_for_test(ws_body(&[("/audio/occlusion", json!("none"))]));
    assert_eq!(
        ecs.resolve_audio_overlay(),
        ResolvedAudioOverlay {
            spatial: true,
            occlusion: eng::Occlusion::None,
            through_wall_gain: 0.25
        },
    );
}

#[test]
fn token_sound_emission_override_replaces_the_actors() {
    let mut tok = linked_token(11, 10, 200, 50.0, 50.0);
    tok.engine.as_mut().unwrap()["overrides"] = json!({ "sound": { "asset": "a-override", "radius": 2.0, "volume": 0.5, "loop": false, "enabled": true } });
    let mut ecs = SceneEcs::from_documents(vec![scene_doc(10), tok.clone()], 0);
    ecs.set_actors(vec![entity_doc_top_eng(
        200,
        "actor",
        actor_with_sound(sound()),
    )]);
    let resolved = ecs
        .token_sound_emission(&tok)
        .expect("the override resolves");
    assert_eq!(resolved.asset, "a-override");
    assert_eq!(
        resolved.radius, 2.0,
        "the override replaces wholesale, never merges"
    );
}

#[test]
fn token_sound_emission_falls_back_to_the_linked_actors() {
    let tok = linked_token(11, 10, 200, 50.0, 50.0);
    let mut ecs = SceneEcs::from_documents(vec![scene_doc(10), tok.clone()], 0);
    ecs.set_actors(vec![entity_doc_top_eng(
        200,
        "actor",
        actor_with_sound(sound()),
    )]);
    let resolved = ecs
        .token_sound_emission(&tok)
        .expect("the actor's emission carries");
    assert_eq!(resolved.asset, "a-wind");
}

#[test]
fn token_sound_emission_dangling_link_and_raw_token_resolve_none() {
    let mut dangling = linked_token(11, 10, 200, 50.0, 50.0);
    dangling.engine.as_mut().unwrap()["overrides"] = json!({ "sound": sound() });
    let raw = entity_doc_eng(
        12,
        10,
        "token",
        json!({ "x": 150.0, "y": 150.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    let ecs = SceneEcs::from_documents(vec![scene_doc(10), dangling.clone(), raw.clone()], 0);
    assert!(
        ecs.token_sound_emission(&dangling).is_none(),
        "a dangling link resolves nothing (overrides ignored — the token_light_emission rule)"
    );
    assert!(
        ecs.token_sound_emission(&raw).is_none(),
        "a raw token carries no emission"
    );
}

#[test]
fn token_sound_emission_reads_an_embedded_actor() {
    let tok = entity_doc_eng(
        11,
        10,
        "token",
        json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
    );
    let mut tok = tok;
    tok.embedded.insert(
        "actor".to_string(),
        vec![entity_doc_top_eng(300, "actor", actor_with_sound(sound()))],
    );
    let ecs = SceneEcs::from_documents(vec![scene_doc(10), tok.clone()], 0);
    let resolved = ecs
        .token_sound_emission(&tok)
        .expect("the embedded actor's emission carries");
    assert_eq!(resolved.asset, "a-wind");
}

#[test]
fn select_listener_accepts_a_readable_in_scene_override() {
    let user = crate::test_ids::test_uuid(7);
    let ecs = SceneEcs::from_documents(
        vec![
            scene_doc(10),
            readable(owned_token(
                11,
                10,
                50.0,
                50.0,
                crate::test_ids::test_uuid(99),
            )),
        ],
        0,
    );
    assert_eq!(
        ecs.select_listener(
            &player_ctx(user),
            &no_world_grants(),
            crate::test_ids::test_uuid(10),
            Some(crate::test_ids::test_uuid(11))
        ),
        Some(crate::test_ids::test_uuid(11)),
        "an override the recipient can READ in the named scene wins even with no owned token"
    );
}

#[test]
fn select_listener_refuses_an_override_in_another_scene() {
    let user = crate::test_ids::test_uuid(7);
    let ecs = SceneEcs::from_documents(
        vec![
            scene_doc(10),
            scene_doc(20),
            readable(owned_token(11, 10, 50.0, 50.0, user)),
            readable(owned_token(
                21,
                20,
                50.0,
                50.0,
                crate::test_ids::test_uuid(99),
            )),
        ],
        0,
    );
    assert_eq!(
        ecs.select_listener(
            &player_ctx(user),
            &no_world_grants(),
            crate::test_ids::test_uuid(10),
            Some(crate::test_ids::test_uuid(21))
        ),
        Some(crate::test_ids::test_uuid(11)),
        "an override naming a token of ANOTHER scene is refused; the owned-token rule decides"
    );
}

#[test]
fn select_listener_refuses_an_unreadable_override() {
    let user = crate::test_ids::test_uuid(7);
    let ecs = SceneEcs::from_documents(
        vec![
            scene_doc(10),
            owned_token(11, 10, 50.0, 50.0, user),
            owned_token(12, 10, 150.0, 50.0, crate::test_ids::test_uuid(99)), // DocRole::None: unreadable
        ],
        0,
    );
    assert_eq!(
        ecs.select_listener(
            &player_ctx(user),
            &no_world_grants(),
            crate::test_ids::test_uuid(10),
            Some(crate::test_ids::test_uuid(12))
        ),
        Some(crate::test_ids::test_uuid(11)),
        "an override the recipient cannot READ is refused; the owned-token rule decides"
    );
}

#[test]
fn select_listener_picks_the_lowest_id_owned_token() {
    let user = crate::test_ids::test_uuid(7);
    let ecs = SceneEcs::from_documents(
        vec![
            scene_doc(10),
            owned_token(30, 10, 150.0, 50.0, user),
            owned_token(15, 10, 50.0, 50.0, user),
        ],
        0,
    );
    assert_eq!(
        ecs.select_listener(
            &player_ctx(user),
            &no_world_grants(),
            crate::test_ids::test_uuid(10),
            None
        ),
        Some(crate::test_ids::test_uuid(15)),
        "deterministic: the LOWEST-id owned token listens, regardless of insertion order"
    );
}

#[test]
fn select_listener_returns_none_with_no_override_and_no_owned_token() {
    let user = crate::test_ids::test_uuid(7);
    let ecs = SceneEcs::from_documents(
        vec![
            scene_doc(10),
            owned_token(11, 10, 50.0, 50.0, crate::test_ids::test_uuid(99)),
        ],
        0,
    );
    assert_eq!(
        ecs.select_listener(
            &player_ctx(user),
            &no_world_grants(),
            crate::test_ids::test_uuid(10),
            None
        ),
        None,
    );
}

/// The shared audibility fixture: scene 10 (100-unit square grid), `user`'s listener token at
/// (50, 50), and one emitter (id `emitter_id`, linked to actor 200 carrying `sound()`) at
/// (250, 50) — 200 world units away, one third of the 600-unit radius.
fn listener_and_emitter(user: Uuid, emitter_id: u128) -> SceneEcs {
    let mut ecs = SceneEcs::from_documents(
        vec![
            scene_doc(10),
            owned_token(11, 10, 50.0, 50.0, user),
            readable(linked_token(emitter_id, 10, 200, 250.0, 50.0)),
        ],
        0,
    );
    ecs.set_actors(vec![entity_doc_top_eng(
        200,
        "actor",
        actor_with_sound(sound()),
    )]);
    ecs
}

#[test]
fn compute_audibility_echoes_the_scene_and_resolves_falloff_and_pan() {
    let user = crate::test_ids::test_uuid(7);
    let ecs = listener_and_emitter(user, 12);
    let slice = ecs.compute_audibility(
        &player_ctx(user),
        &no_world_grants(),
        crate::test_ids::test_uuid(10),
        None,
    );
    assert_eq!(
        slice.scene,
        crate::test_ids::test_uuid(10),
        "the slice echoes the scene argument verbatim"
    );
    assert_eq!(slice.listener, Some(crate::test_ids::test_uuid(11)));
    assert!(slice.spatial);
    assert_eq!(slice.emitters.len(), 1);
    let em = &slice.emitters[0];
    assert_eq!(em.token, crate::test_ids::test_uuid(12));
    assert_eq!(em.asset, "a-wind");
    assert!(em.loop_);
    // t = 200/600 = 1/3 → falloff = 1 - 1/9 = 8/9; gain = 0.8 * 8/9.
    assert!(
        (em.gain - 0.8 * 8.0 / 9.0).abs() < 1e-9,
        "gain folds volume * falloff: {}",
        em.gain
    );
    assert!(
        (em.pan - 1.0 / 3.0).abs() < 1e-9,
        "pan scales dx by the radius: {}",
        em.pan
    );
}

#[test]
fn compute_audibility_spatial_false_mixes_flat_regardless_of_listener() {
    let user = crate::test_ids::test_uuid(7);
    let mut ecs = listener_and_emitter(user, 12);
    ecs.set_world_settings_for_test(ws_body(&[("/audio/spatial", json!(false))]));
    // A player owning NO token (no listener at all) still hears the flat mix under
    // `spatial: false` — the overlay overrides the no-listener silence.
    let stranger = player_ctx(crate::test_ids::test_uuid(42));
    let slice = ecs.compute_audibility(
        &stranger,
        &no_world_grants(),
        crate::test_ids::test_uuid(10),
        None,
    );
    assert!(!slice.spatial);
    assert_eq!(slice.listener, None);
    assert_eq!(slice.emitters.len(), 1);
    assert_eq!(
        slice.emitters[0].gain, 0.8,
        "spatial false: gain is the authored volume verbatim"
    );
    assert_eq!(slice.emitters[0].pan, 0.0);
}

#[test]
fn compute_audibility_wall_occlusion_reduces_but_never_silences() {
    let user = crate::test_ids::test_uuid(7);
    let mut ecs = listener_and_emitter(user, 12);
    ecs.apply_op(&Operation::Create {
        doc: entity_doc_eng(
            50,
            10,
            "wall",
            json!({ "seg": {"x1": 150, "y1": -50, "x2": 150, "y2": 150}, "blocksSight": true }),
        ),
    });
    let slice = ecs.compute_audibility(
        &player_ctx(user),
        &no_world_grants(),
        crate::test_ids::test_uuid(10),
        None,
    );
    let em = &slice.emitters[0];
    assert!(
        (em.gain - 0.8 * 8.0 / 9.0 * 0.25).abs() < 1e-9,
        "an occluded emitter drops to volume * falloff * throughWallGain: {}",
        em.gain
    );
    assert!(
        em.gain > 0.0,
        "occlusion attenuates, never silences outright"
    );
    assert!(
        em.muffled,
        "the same occluded branch that reduces gain flags the emitter muffled"
    );
}

#[test]
fn a_sound_occluding_region_muffles_an_emitter_behind_it_like_a_wall() {
    let user = crate::test_ids::test_uuid(7);
    let region = |occludes: serde_json::Value| {
        entity_doc_eng(
            50,
            10,
            "region",
            json!({
                "shape": { "kind": "rect", "points": [100.0, -100.0, 200.0, 200.0] },
                "behavior": "terrain", "cost": 1.0, "enabled": true,
                "occludes": occludes,
            }),
        )
    };
    let gain_of = |occludes: serde_json::Value| {
        let mut ecs = listener_and_emitter(user, 12);
        ecs.apply_op(&Operation::Create {
            doc: region(occludes),
        });
        only_emitter(&ecs, user)
    };
    let silenced = gain_of(json!({ "sight": false, "light": false, "sound": true }));
    assert!(
        silenced.muffled,
        "the region boundary occludes the sound path"
    );
    assert!(
        (silenced.gain - 0.8 * 8.0 / 9.0 * 0.25).abs() < 1e-9,
        "the same through-wall attenuation a wall applies: {}",
        silenced.gain
    );
    let sight_only = gain_of(json!({ "sight": true, "light": true, "sound": false }));
    assert!(
        !sight_only.muffled,
        "a region that does not flag sound leaves the emitter clear"
    );
}

/// The shared fixture plus a vertical wall between the listener (50, 50) and the emitter
/// (250, 50), 100 units (one cell) from the listener, carrying the given `proximity` body.
fn fixture_with_proximity_wall(user: Uuid, proximity: serde_json::Value) -> SceneEcs {
    let mut ecs = listener_and_emitter(user, 12);
    ecs.apply_op(&Operation::Create {
        doc: entity_doc_eng(
            50,
            10,
            "wall",
            json!({
                "seg": {"x1": 150, "y1": -50, "x2": 150, "y2": 150},
                "blocksSight": true,
                "proximity": proximity,
            }),
        ),
    });
    ecs
}

/// The single emitter of the fixture's slice for `user`.
fn only_emitter(ecs: &SceneEcs, user: Uuid) -> AudibleEmitter {
    let slice = ecs.compute_audibility(
        &player_ctx(user),
        &no_world_grants(),
        crate::test_ids::test_uuid(10),
        None,
    );
    slice.emitters[0].clone()
}

#[test]
fn a_listener_within_a_sound_tier_wall_hears_at_the_sound_gain_unmuffled() {
    let user = crate::test_ids::test_uuid(7);
    let ecs = fixture_with_proximity_wall(
        user,
        json!({ "within": 1.5, "senses": ["sound"], "soundGain": 0.5 }),
    );
    let em = only_emitter(&ecs, user);
    assert!(
        (em.gain - 0.8 * 8.0 / 9.0 * 0.5).abs() < 1e-9,
        "an open wall scales by soundGain only: {}",
        em.gain
    );
    assert!(!em.muffled, "an open wall is not an occluded path");
}

#[test]
fn an_absent_sound_gain_passes_the_sound_at_full_volume() {
    let user = crate::test_ids::test_uuid(7);
    let ecs = fixture_with_proximity_wall(user, json!({ "within": 1.5, "senses": ["sound"] }));
    let em = only_emitter(&ecs, user);
    assert!((em.gain - 0.8 * 8.0 / 9.0).abs() < 1e-9, "{}", em.gain);
    assert!(!em.muffled);
}

#[test]
fn a_listener_beyond_the_tier_distance_hears_the_wall_as_ordinary() {
    // The listener stands 1 cell from the wall; a 0.5-cell tier leaves it closed.
    let user = crate::test_ids::test_uuid(7);
    let ecs = fixture_with_proximity_wall(
        user,
        json!({ "within": 0.5, "senses": ["sound"], "soundGain": 0.5 }),
    );
    let em = only_emitter(&ecs, user);
    assert!(
        (em.gain - 0.8 * 8.0 / 9.0 * 0.25).abs() < 1e-9,
        "{}",
        em.gain
    );
    assert!(em.muffled);
}

#[test]
fn a_tier_that_omits_the_sound_sense_leaves_the_wall_closed_to_sound() {
    let user = crate::test_ids::test_uuid(7);
    let ecs =
        fixture_with_proximity_wall(user, json!({ "within": 1.5, "senses": ["sight", "light"] }));
    let em = only_emitter(&ecs, user);
    assert!(
        (em.gain - 0.8 * 8.0 / 9.0 * 0.25).abs() < 1e-9,
        "{}",
        em.gain
    );
    assert!(em.muffled);
}

#[test]
fn segment_occluded_multiplies_open_wall_gains_and_still_blocks_on_a_closed_one() {
    let open = |x: f64, gain: f64| vision::Seg {
        a: (x, -5.0),
        b: (x, 5.0),
        one_way: None,
        limited: false,
        open_within: Some(vision::OpenWithin {
            distance: 100.0,
            gain,
        }),
    };
    let closed = vision::Seg {
        a: (9.0, -5.0),
        b: (9.0, 5.0),
        one_way: None,
        limited: false,
        open_within: None,
    };
    let (blocked, gain) =
        segment_occluded((0.0, 0.0), (10.0, 0.0), &[open(3.0, 0.5), open(6.0, 0.5)]);
    assert!(!blocked);
    assert!((gain - 0.25).abs() < 1e-12);
    let (blocked, gain) = segment_occluded((0.0, 0.0), (10.0, 0.0), &[open(3.0, 0.5), closed]);
    assert!(blocked, "one closed crossing blocks");
    assert!((gain - 0.5).abs() < 1e-12);
}

#[test]
fn compute_audibility_unoccluded_emitter_is_not_muffled() {
    let user = crate::test_ids::test_uuid(7);
    let ecs = listener_and_emitter(user, 12);
    let slice = ecs.compute_audibility(
        &player_ctx(user),
        &no_world_grants(),
        crate::test_ids::test_uuid(10),
        None,
    );
    assert!(!slice.emitters[0].muffled, "no wall between: not muffled");
}

#[test]
fn compute_audibility_spatial_false_never_muffles_despite_a_wall() {
    let user = crate::test_ids::test_uuid(7);
    let mut ecs = listener_and_emitter(user, 12);
    ecs.apply_op(&Operation::Create {
        doc: entity_doc_eng(
            50,
            10,
            "wall",
            json!({ "seg": {"x1": 150, "y1": -50, "x2": 150, "y2": 150}, "blocksSight": true }),
        ),
    });
    ecs.set_world_settings_for_test(ws_body(&[("/audio/spatial", json!(false))]));
    let slice = ecs.compute_audibility(
        &player_ctx(user),
        &no_world_grants(),
        crate::test_ids::test_uuid(10),
        None,
    );
    assert!(
        !slice.emitters[0].muffled,
        "flat mixing runs no occlusion test, so nothing is muffled"
    );
}

#[test]
fn compute_audibility_occlusion_none_ignores_walls() {
    let user = crate::test_ids::test_uuid(7);
    let mut ecs = listener_and_emitter(user, 12);
    ecs.apply_op(&Operation::Create {
        doc: entity_doc_eng(
            50,
            10,
            "wall",
            json!({ "seg": {"x1": 150, "y1": -50, "x2": 150, "y2": 150}, "blocksSight": true }),
        ),
    });
    ecs.set_world_settings_for_test(ws_body(&[("/audio/occlusion", json!("none"))]));
    let slice = ecs.compute_audibility(
        &player_ctx(user),
        &no_world_grants(),
        crate::test_ids::test_uuid(10),
        None,
    );
    assert!(
        (slice.emitters[0].gain - 0.8 * 8.0 / 9.0).abs() < 1e-9,
        "Occlusion::None: the wall between listener and emitter changes nothing"
    );
    assert!(
        !slice.emitters[0].muffled,
        "Occlusion::None turns occlusion off, so a geometrically intervening wall never muffles"
    );
}

#[test]
fn compute_audibility_blocks_sound_false_wall_never_occludes_despite_blocking_sight() {
    // The de-aliasing pin: `blocksSight: true, blocksSound: false` keeps the wall in the
    // SIGHT set (the document parses and occludes vision) while excluding it from the SOUND
    // set — the emitter's gain is the bare falloff, untouched by `throughWallGain`.
    let user = crate::test_ids::test_uuid(7);
    let mut ecs = listener_and_emitter(user, 12);
    ecs.apply_op(&Operation::Create {
        doc: entity_doc_eng(
            50,
            10,
            "wall",
            json!({ "seg": {"x1": 150, "y1": -50, "x2": 150, "y2": 150},
                    "blocksSight": true, "blocksSound": false }),
        ),
    });
    assert_eq!(
        ecs.sight_wall_entries(crate::test_ids::test_uuid(10)).len(),
        1,
        "the wall still parses and blocks sight — only the sound set drops it"
    );
    let slice = ecs.compute_audibility(
        &player_ctx(user),
        &no_world_grants(),
        crate::test_ids::test_uuid(10),
        None,
    );
    assert!(
        (slice.emitters[0].gain - 0.8 * 8.0 / 9.0).abs() < 1e-9,
        "a sound-transparent wall between listener and emitter changes nothing: {}",
        slice.emitters[0].gain
    );
    assert!(!slice.emitters[0].muffled);
}

#[test]
fn compute_audibility_blocks_sound_true_wall_occludes_despite_transparent_sight() {
    // The other half of the de-aliasing pin: `blocksSight: false, blocksSound: true`
    // occludes sound while leaving vision untouched — a window that blocks noise.
    let user = crate::test_ids::test_uuid(7);
    let mut ecs = listener_and_emitter(user, 12);
    ecs.apply_op(&Operation::Create {
        doc: entity_doc_eng(
            50,
            10,
            "wall",
            json!({ "seg": {"x1": 150, "y1": -50, "x2": 150, "y2": 150},
                    "blocksSight": false, "blocksSound": true }),
        ),
    });
    assert!(
        ecs.sight_wall_entries(crate::test_ids::test_uuid(10))
            .is_empty(),
        "the wall stays out of the sight set"
    );
    let slice = ecs.compute_audibility(
        &player_ctx(user),
        &no_world_grants(),
        crate::test_ids::test_uuid(10),
        None,
    );
    assert!(
        (slice.emitters[0].gain - 0.8 * 8.0 / 9.0 * 0.25).abs() < 1e-9,
        "a sound-blocking wall attenuates to volume * falloff * throughWallGain: {}",
        slice.emitters[0].gain
    );
}

#[test]
fn compute_audibility_honors_the_wall_elevation_band() {
    let user = crate::test_ids::test_uuid(7);
    let mut ecs = listener_and_emitter(user, 12);
    // The SAME wall position as the occluding case, banded to elevations ≥ 5 — the ground-level
    // listener is outside the band, so it must NOT occlude.
    ecs.apply_op(&Operation::Create {
        doc: entity_doc_eng(
            50,
            10,
            "wall",
            json!({ "seg": {"x1": 150, "y1": -50, "x2": 150, "y2": 150}, "blocksSight": true,
                    "elevation": {"bottom": 5.0} }),
        ),
    });
    let slice = ecs.compute_audibility(
        &player_ctx(user),
        &no_world_grants(),
        crate::test_ids::test_uuid(10),
        None,
    );
    assert!(
        (slice.emitters[0].gain - 0.8 * 8.0 / 9.0).abs() < 1e-9,
        "an out-of-band wall does not occlude a ground-level listener"
    );
}

#[test]
fn compute_audibility_wall_occlusion_inherits_across_levels_from_the_listeners_own_band() {
    // `compute_audibility` filters occluding walls to the LISTENER's own elevation
    // (`elevation::walls_at_elevation`) — it never reads the EMITTER's elevation at all. A
    // `SceneLevel`-bearing scene therefore needs no dedicated cross-level logic of its own: a
    // floor-1-banded wall between the listener and a floor-2 emitter occludes it exactly as it
    // would a floor-1 emitter at the identical horizontal distance — levels compose with the
    // existing per-listener band filter for free.
    let user = crate::test_ids::test_uuid(7);
    let scene_id = crate::test_ids::test_uuid(10);
    let scene = entity_doc_top_eng(
        10,
        "scene",
        json!({
            "grid": { "kind": "square", "size": 100.0 }, "background": null,
            "levels": [
                { "id": "1", "name": "Floor 1", "bottom": 0.0, "top": 10.0 },
                { "id": "2", "name": "Floor 2", "bottom": 10.0, "top": 20.0 },
            ],
        }),
    );
    // Listener: no authored elevation ⇒ `elevation_or_ground` = 0.0, floor "1".
    let listener = owned_token(11, 10, 50.0, 50.0, user);
    // Cross-floor emitter: floor "2" (elevation 15, inside [10, 20)), across the wall.
    let mut cross_floor_emitter = readable(linked_token(12, 10, 300, 250.0, 50.0));
    cross_floor_emitter.engine.as_mut().unwrap()["elevation"] = json!(15.0);
    // Same-floor emitter: floor "1" (elevation 0, matching the listener), the IDENTICAL 200-unit
    // horizontal distance on the OPPOSITE side of the listener — never crosses the wall.
    let same_floor_emitter = readable(linked_token(13, 10, 301, -150.0, 50.0));
    let mut ecs = SceneEcs::from_documents(
        vec![scene, listener, cross_floor_emitter, same_floor_emitter],
        0,
    );
    ecs.set_actors(vec![
        entity_doc_top_eng(300, "actor", actor_with_sound(sound())),
        entity_doc_top_eng(301, "actor", actor_with_sound(sound())),
    ]);
    // The SAME wall position `compute_audibility_wall_occlusion_reduces_but_never_silences` uses,
    // banded to floor "1" only — the listener's own floor.
    ecs.apply_op(&Operation::Create {
        doc: entity_doc_eng(
            50,
            10,
            "wall",
            json!({ "seg": {"x1": 150, "y1": -50, "x2": 150, "y2": 150}, "blocksSight": true,
                    "elevation": {"bottom": 0.0, "top": 10.0} }),
        ),
    });
    let slice = ecs.compute_audibility(&player_ctx(user), &no_world_grants(), scene_id, None);
    let cross = slice
        .emitters
        .iter()
        .find(|e| e.token == crate::test_ids::test_uuid(12))
        .expect("cross-floor emitter present")
        .gain;
    let same = slice
        .emitters
        .iter()
        .find(|e| e.token == crate::test_ids::test_uuid(13))
        .expect("same-floor emitter present")
        .gain;
    // Identical horizontal distance (200 world units, 600-unit radius) ⇒ an IDENTICAL falloff
    // term for both emitters; `throughWallGain` (0.25) is the ONLY thing distinguishing them.
    assert!(
        (same - 0.8 * 8.0 / 9.0).abs() < 1e-9,
        "same-level pair at the identical distance: unoccluded falloff-only gain: {same}"
    );
    assert!(
        (cross - same * 0.25).abs() < 1e-9,
        "cross-level pair: occluded via the listener's OWN elevation-band wall filter alone, \
         with no code change for levels: {cross}"
    );
}

#[test]
fn compute_audibility_skips_a_disabled_emission() {
    let user = crate::test_ids::test_uuid(7);
    let mut ecs = SceneEcs::from_documents(
        vec![
            scene_doc(10),
            owned_token(11, 10, 50.0, 50.0, user),
            readable(linked_token(12, 10, 200, 250.0, 50.0)),
        ],
        0,
    );
    let mut disabled = sound();
    disabled["enabled"] = json!(false);
    ecs.set_actors(vec![entity_doc_top_eng(
        200,
        "actor",
        actor_with_sound(disabled),
    )]);
    let slice = ecs.compute_audibility(
        &player_ctx(user),
        &no_world_grants(),
        crate::test_ids::test_uuid(10),
        None,
    );
    assert!(
        slice.emitters.is_empty(),
        "a disabled emission contributes nothing"
    );
}

#[test]
fn compute_audibility_beyond_radius_is_silent() {
    let user = crate::test_ids::test_uuid(7);
    let mut ecs = SceneEcs::from_documents(
        vec![
            scene_doc(10),
            owned_token(11, 10, 50.0, 50.0, user),
            readable(linked_token(12, 10, 200, 700.0, 50.0)),
        ],
        0,
    );
    ecs.set_actors(vec![entity_doc_top_eng(
        200,
        "actor",
        actor_with_sound(sound()),
    )]);
    let slice = ecs.compute_audibility(
        &player_ctx(user),
        &no_world_grants(),
        crate::test_ids::test_uuid(10),
        None,
    );
    assert_eq!(
        slice.emitters[0].gain, 0.0,
        "past the authored radius the falloff is exactly 0"
    );
}

#[test]
fn compute_audibility_without_a_listener_every_spatial_emitter_is_silent() {
    let user = crate::test_ids::test_uuid(7);
    let ecs = listener_and_emitter(user, 12);
    // A player owning nothing in the scene has no listener: spatial emitters resolve to
    // silence (nothing to spatialize FROM).
    let stranger = player_ctx(crate::test_ids::test_uuid(42));
    let slice = ecs.compute_audibility(
        &stranger,
        &no_world_grants(),
        crate::test_ids::test_uuid(10),
        None,
    );
    assert_eq!(slice.listener, None);
    assert_eq!(slice.emitters.len(), 1);
    assert_eq!(slice.emitters[0].gain, 0.0);
    assert_eq!(slice.emitters[0].pan, 0.0);
}

#[test]
fn compute_audibility_omits_an_emitter_the_recipient_cannot_read() {
    let user = crate::test_ids::test_uuid(7);
    let mut ecs = SceneEcs::from_documents(
        vec![
            scene_doc(10),
            owned_token(11, 10, 50.0, 50.0, user),
            linked_token(12, 10, 200, 250.0, 50.0), // DocRole::None: permission-hidden
            readable(linked_token(13, 10, 201, 250.0, 50.0)),
        ],
        0,
    );
    ecs.set_actors(vec![
        entity_doc_top_eng(200, "actor", actor_with_sound(sound())),
        entity_doc_top_eng(201, "actor", actor_with_sound(sound())),
    ]);
    let slice = ecs.compute_audibility(
        &player_ctx(user),
        &no_world_grants(),
        crate::test_ids::test_uuid(10),
        None,
    );
    assert_eq!(
        slice.emitters.len(),
        1,
        "a permission-hidden token's carried sound never names it to the recipient"
    );
    assert_eq!(slice.emitters[0].token, crate::test_ids::test_uuid(13));
    // The GM, holding READ on everything, hears both.
    let gm_slice = ecs.compute_audibility(
        &gm_ctx(crate::test_ids::test_uuid(1)),
        &no_world_grants(),
        crate::test_ids::test_uuid(10),
        None,
    );
    assert_eq!(gm_slice.emitters.len(), 2);
}

#[test]
fn compute_audibility_emitter_order_is_deterministic() {
    let user = crate::test_ids::test_uuid(7);
    let mut ecs = SceneEcs::from_documents(
        vec![
            scene_doc(10),
            owned_token(11, 10, 50.0, 50.0, user),
            readable(linked_token(40, 10, 200, 250.0, 50.0)),
            readable(linked_token(25, 10, 201, 200.0, 50.0)),
        ],
        0,
    );
    ecs.set_actors(vec![
        entity_doc_top_eng(200, "actor", actor_with_sound(sound())),
        entity_doc_top_eng(201, "actor", actor_with_sound(sound())),
    ]);
    let first = ecs.compute_audibility(
        &player_ctx(user),
        &no_world_grants(),
        crate::test_ids::test_uuid(10),
        None,
    );
    let second = ecs.compute_audibility(
        &player_ctx(user),
        &no_world_grants(),
        crate::test_ids::test_uuid(10),
        None,
    );
    assert_eq!(
        first, second,
        "identical input yields an identical slice across calls"
    );
    let ids: Vec<Uuid> = first.emitters.iter().map(|e| e.token).collect();
    assert_eq!(
        ids,
        vec![
            crate::test_ids::test_uuid(25),
            crate::test_ids::test_uuid(40)
        ],
        "sorted by token id"
    );
}
