//! Token-anchored regions: a region whose shape is offsets from a carrier token reads at the
//! carrier's current position through every geometry consumer, is dormant without a resolvable
//! carrier, and does not affect its own carrier unless it says so.

use super::*;
use std::collections::BTreeSet;

const SCENE: u128 = 10;
const CARRIER: u128 = 11;
const OTHER: u128 = 12;
const REGION: u128 = 30;

/// A scene on a 100-unit square grid.
fn scene_doc() -> Document {
    entity_doc_top_eng(
        SCENE,
        "scene",
        json!({ "grid": { "kind": "square", "size": 100 }, "background": null }),
    )
}

/// A token fixture centred at `(x, y)` with `rotation` degrees, parented to `parent`.
fn token_at(id: u128, parent: u128, x: f64, y: f64, rotation: f64) -> Document {
    entity_doc_eng(
        id,
        parent,
        "token",
        json!({ "x": x, "y": y, "w": 100.0, "h": 100.0, "rotation": rotation }),
    )
}

/// An enabled rect region over the offset box `rect`, anchored to `carrier` (`None` = static),
/// carrying `behavior` and `triggers`.
fn anchored_region(
    carrier: Option<u128>,
    include_carrier: bool,
    follow_rotation: bool,
    behavior: &str,
    rect: (f64, f64, f64, f64),
    triggers: serde_json::Value,
) -> Document {
    let (x0, y0, x1, y1) = rect;
    let mut body = json!({
        "shape": { "kind": "rect", "points": [x0, y0, x1, y1] },
        "behavior": behavior,
        "cost": 1.0,
        "enabled": true,
        "triggers": triggers,
    });
    if let Some(carrier) = carrier {
        body["anchor"] = json!({
            "token": crate::test_ids::test_uuid(carrier).to_string(),
            "include_carrier": include_carrier,
            "follow_rotation": follow_rotation,
        });
    }
    entity_doc_eng(REGION, SCENE, "region", body)
}

/// A one-trigger list: a GM-only chat notice on Enter, so the region is trigger-bearing.
fn enter_notice() -> serde_json::Value {
    json!([{ "on": "enter", "effect": { "type": "chat_notice", "text": "hi", "audience": "gm_only" } }])
}

/// The cells of the authoritative field a mover on the ground sees.
fn field_cells(ecs: &SceneEcs, mover: Option<u128>) -> BTreeSet<(i32, i32)> {
    ecs.region_field(
        crate::test_ids::test_uuid(SCENE),
        None,
        elevation::GROUND,
        mover.map(crate::test_ids::test_uuid),
    )
    .expect("scene exists")
    .iter_cells()
    .map(|(c, _)| c)
    .collect()
}

/// The cells of the one trigger row the scene produces for `mover`, if any.
fn trigger_cells(ecs: &SceneEcs, mover: Option<u128>) -> Option<BTreeSet<(i32, i32)>> {
    let rows = ecs
        .trigger_regions(
            crate::test_ids::test_uuid(SCENE),
            elevation::GROUND,
            mover.map(crate::test_ids::test_uuid),
        )
        .expect("scene exists");
    rows.into_iter()
        .next()
        .map(|r| r.cells.into_iter().collect())
}

/// The ecs for a carrier at `(x, y)` with one anchored region (offset box covering cell (0, 0)
/// around the carrier's centre: `[-50, -50, 50, 50]`).
fn carrier_ecs(x: f64, y: f64, region: Document) -> SceneEcs {
    SceneEcs::from_documents(
        vec![
            scene_doc(),
            token_at(CARRIER, SCENE, x, y, 0.0),
            token_at(OTHER, SCENE, 900.0, 900.0, 0.0),
            region,
        ],
        0,
    )
}

#[test]
fn an_anchored_region_reads_at_the_carriers_position_in_the_field_and_the_trigger_rows() {
    let region = || {
        anchored_region(
            Some(CARRIER),
            true,
            false,
            "impassable",
            (-50.0, -50.0, 50.0, 50.0),
            enter_notice(),
        )
    };
    let at_origin = carrier_ecs(50.0, 50.0, region());
    let moved = carrier_ecs(450.0, 250.0, region());
    assert_eq!(field_cells(&at_origin, None), BTreeSet::from([(0, 0)]));
    assert_eq!(
        field_cells(&moved, None),
        BTreeSet::from([(4, 2)]),
        "the carrier moving shifts the field"
    );
    // The identity rows and the composed field derive from one geometry step: they cover the
    // same cells wherever the carrier stands.
    assert_eq!(
        trigger_cells(&at_origin, None),
        Some(field_cells(&at_origin, None))
    );
    assert_eq!(trigger_cells(&moved, None), Some(field_cells(&moved, None)));
}

#[test]
fn an_anchored_region_covers_the_same_cells_as_a_static_region_at_the_translated_shape() {
    let anchored = carrier_ecs(
        250.0,
        150.0,
        anchored_region(
            Some(CARRIER),
            true,
            false,
            "terrain",
            (-100.0, -50.0, 100.0, 50.0),
            json!([]),
        ),
    );
    let fixed = carrier_ecs(
        250.0,
        150.0,
        anchored_region(
            None,
            true,
            false,
            "terrain",
            (150.0, 100.0, 350.0, 200.0),
            json!([]),
        ),
    );
    assert_eq!(field_cells(&anchored, None), field_cells(&fixed, None));
    assert!(!field_cells(&anchored, None).is_empty());
}

#[test]
fn follow_rotation_turns_the_shape_with_the_carrier() {
    let region = |rotation: f64| {
        let ecs = SceneEcs::from_documents(
            vec![
                scene_doc(),
                token_at(CARRIER, SCENE, 450.0, 450.0, rotation),
                anchored_region(
                    Some(CARRIER),
                    true,
                    true,
                    "impassable",
                    // A bar reaching right of the carrier: offsets x in [0, 200], y in [-40, 40].
                    (0.0, -40.0, 200.0, 40.0),
                    json!([]),
                ),
            ],
            0,
        );
        field_cells(&ecs, None)
    };
    let unturned = region(0.0);
    let quarter = region(90.0);
    assert!(
        unturned.contains(&(5, 4)) && !unturned.contains(&(4, 5)),
        "rotation 0 reaches along +x: {unturned:?}"
    );
    assert!(
        quarter.contains(&(4, 5)) && !quarter.contains(&(5, 4)),
        "rotation 90 reaches along +y (clockwise on a y-down canvas): {quarter:?}"
    );
}

#[test]
fn without_follow_rotation_the_carriers_rotation_is_ignored() {
    let ecs = SceneEcs::from_documents(
        vec![
            scene_doc(),
            token_at(CARRIER, SCENE, 450.0, 450.0, 90.0),
            anchored_region(
                Some(CARRIER),
                true,
                false,
                "impassable",
                (0.0, -40.0, 200.0, 40.0),
                json!([]),
            ),
        ],
        0,
    );
    let cells = field_cells(&ecs, None);
    assert!(cells.contains(&(5, 4)) && !cells.contains(&(4, 5)));
}

#[test]
fn an_anchored_region_is_dormant_without_a_resolvable_carrier() {
    let region = || {
        anchored_region(
            Some(CARRIER),
            true,
            false,
            "impassable",
            (-50.0, -50.0, 50.0, 50.0),
            enter_notice(),
        )
    };
    // The control: the same region with its carrier present contributes cells.
    assert!(!field_cells(&carrier_ecs(50.0, 50.0, region()), None).is_empty());

    let scene_without_carrier = SceneEcs::from_documents(vec![scene_doc(), region()], 0);
    assert!(
        field_cells(&scene_without_carrier, None).is_empty(),
        "a deleted carrier leaves the region dormant"
    );
    assert!(trigger_cells(&scene_without_carrier, None).is_none());

    let other_scene = SceneEcs::from_documents(
        vec![
            scene_doc(),
            entity_doc_top_eng(
                SCENE + 90,
                "scene",
                json!({ "grid": { "kind": "square", "size": 100 }, "background": null }),
            ),
            token_at(CARRIER, SCENE + 90, 50.0, 50.0, 0.0),
            region(),
        ],
        0,
    );
    assert!(
        field_cells(&other_scene, None).is_empty(),
        "a carrier parented to another scene leaves the region dormant"
    );

    let not_a_token = SceneEcs::from_documents(
        vec![
            scene_doc(),
            entity_doc_eng(CARRIER, SCENE, "light", json!({ "x": 50.0, "y": 50.0 })),
            region(),
        ],
        0,
    );
    assert!(
        field_cells(&not_a_token, None).is_empty(),
        "an anchor naming a non-token document leaves the region dormant"
    );
}

#[test]
fn the_carrier_is_exempt_from_its_own_region_unless_include_carrier_is_set() {
    let build = |include_carrier: bool| {
        carrier_ecs(
            50.0,
            50.0,
            anchored_region(
                Some(CARRIER),
                include_carrier,
                false,
                "impassable",
                (-150.0, -150.0, 150.0, 150.0),
                enter_notice(),
            ),
        )
    };
    let aura = build(false);
    assert!(
        field_cells(&aura, Some(CARRIER)).is_empty(),
        "the carrier's own field omits its aura"
    );
    assert!(
        !field_cells(&aura, Some(OTHER)).is_empty(),
        "another mover's field carries the aura"
    );
    assert!(
        !field_cells(&aura, None).is_empty(),
        "a token-less read (viewer-only, lighting) is not exempted"
    );
    assert!(trigger_cells(&aura, Some(CARRIER)).is_none());
    assert!(trigger_cells(&aura, Some(OTHER)).is_some());

    let self_affecting = build(true);
    assert!(
        !field_cells(&self_affecting, Some(CARRIER)).is_empty(),
        "include_carrier makes the carrier feel its own region"
    );
    assert!(trigger_cells(&self_affecting, Some(CARRIER)).is_some());
}

#[test]
fn a_static_region_is_never_exempted_for_any_mover() {
    let ecs = carrier_ecs(
        50.0,
        50.0,
        anchored_region(
            None,
            false,
            false,
            "impassable",
            (0.0, 0.0, 100.0, 100.0),
            json!([]),
        ),
    );
    assert!(!field_cells(&ecs, Some(CARRIER)).is_empty());
}

#[test]
fn attached_regions_of_lists_the_enabled_regions_following_the_carrier() {
    let ecs = carrier_ecs(
        50.0,
        50.0,
        anchored_region(
            Some(CARRIER),
            false,
            false,
            "terrain",
            (-50.0, -50.0, 50.0, 50.0),
            json!([]),
        ),
    );
    assert_eq!(
        ecs.attached_regions_of(crate::test_ids::test_uuid(CARRIER)),
        vec![crate::test_ids::test_uuid(REGION)]
    );
    assert!(
        ecs.attached_regions_of(crate::test_ids::test_uuid(OTHER))
            .is_empty(),
        "a token nothing follows has no attached regions"
    );
    assert!(
        ecs.attached_regions_of(crate::test_ids::test_uuid(REGION))
            .is_empty(),
        "a non-token id has no attached regions"
    );

    let mut disabled = anchored_region(
        Some(CARRIER),
        false,
        false,
        "terrain",
        (-50.0, -50.0, 50.0, 50.0),
        json!([]),
    );
    disabled.engine.as_mut().unwrap()["enabled"] = json!(false);
    let ecs = carrier_ecs(50.0, 50.0, disabled);
    assert!(ecs
        .attached_regions_of(crate::test_ids::test_uuid(CARRIER))
        .is_empty());
}

#[test]
fn an_anchored_occluder_outline_follows_the_carrier() {
    let occluding = || {
        let mut d = anchored_region(
            Some(CARRIER),
            true,
            false,
            "terrain",
            (-50.0, -50.0, 50.0, 50.0),
            json!([]),
        );
        d.engine.as_mut().unwrap()["occludes"] =
            json!({ "sight": true, "light": false, "sound": false });
        d
    };
    let scene = crate::test_ids::test_uuid(SCENE);
    let xs = |ecs: &SceneEcs| -> Vec<f64> {
        let entries = ecs.region_occluder_entries(scene, eng::WallSense::Sight);
        assert!(!entries.is_empty(), "the anchored region occludes");
        let mut xs: Vec<f64> = entries
            .iter()
            .flat_map(|w| [w.seg.a.0, w.seg.b.0])
            .collect::<Vec<_>>();
        xs.sort_by(f64::total_cmp);
        xs.dedup();
        xs
    };
    let mut ecs = carrier_ecs(50.0, 50.0, occluding());
    assert_eq!(xs(&ecs), vec![0.0, 100.0]);
    // Move the carrier through the document write path while the memoized outline is warm: the
    // memo is keyed on the resolved parts, so the next read must not serve the old outline.
    ecs.apply_op(&crate::data::command::Operation::Update {
        doc_id: crate::test_ids::test_uuid(CARRIER),
        changes: vec![fc("/engine/x", json!(450.0))],
    });
    assert_eq!(
        xs(&ecs),
        vec![400.0, 500.0],
        "the outline is read at the carrier's current position"
    );
}
