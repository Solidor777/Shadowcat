use super::*;
use crate::data::document::tests::world_scoped_doc;
use crate::data::document::{WorldCapDefaults, WorldRole};
use crate::scene::SceneEcs;
use crate::ws::protocol::{LightSample, PosSample, VisionSample};
use serde_json::json;

/// Polygon whose first vertex x-coordinate encodes `idx`, so a chosen sample is identifiable.
fn tagged(idx: usize) -> Vec<Vec<[f64; 2]>> {
    let i = idx as f64;
    vec![vec![[i, 0.0], [i, 1.0], [i + 1.0, 1.0]]]
}

fn fixture_samples() -> (Vec<VisionSample>, Vec<(f64, usize)>) {
    let raw: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../client/render/src/__fixtures__/chosen-vision-sample.json"
    ))
    .unwrap();
    let samples = raw["samples"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, t)| VisionSample {
            t_ms: t.as_f64().unwrap(),
            polygons: tagged(i),
        })
        .collect();
    let probes = raw["probes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["elapsed"].as_f64().unwrap(),
                p["expectIndex"].as_u64().unwrap() as usize,
            )
        })
        .collect();
    (samples, probes)
}

#[test]
fn chosen_vision_sample_matches_the_shared_parity_fixture() {
    let (samples, probes) = fixture_samples();
    for (elapsed, expect) in probes {
        let got = chosen_vision_sample(&samples, elapsed).unwrap();
        assert_eq!(got.polygons, tagged(expect), "elapsed={elapsed}");
    }
}

#[test]
fn chosen_vision_sample_is_none_on_empty() {
    assert!(chosen_vision_sample::<VisionSample>(&[], 0.0).is_none());
}

// --- The clip fixture: one 100-unit square scene, the target's token at (50,50), a sight
// wall at x=100 (y ±500). `lit: false` keeps the engine's lighting default (on, pitch dark).

const WORLD: Uuid = Uuid::from_u128(0xC1);
const SCENE: Uuid = Uuid::from_u128(0xC2);
const TARGET: Uuid = Uuid::from_u128(0xC3);
const TARGET_TOKEN: Uuid = Uuid::from_u128(0xC4);
const STRANGER: Uuid = Uuid::from_u128(0xC5);

/// The scene ECS for a lit or dark walled scene. `vision` (an actor's `vision` assignment
/// list, or `None` for a raw normal-vision token) is what makes the darkvision case buildable.
fn fixture(lit: bool, vision: Option<serde_json::Value>) -> SceneEcs {
    let mut scene = world_scoped_doc(WORLD, SCENE, "scene");
    let mut engine = json!({ "grid": { "kind": "square", "size": 100 }, "background": null });
    if lit {
        engine["lighting"] = json!({ "enabled": false });
    }
    scene.engine = Some(engine);
    let mut tok = world_scoped_doc(WORLD, TARGET_TOKEN, "token");
    tok.parent_id = Some(SCENE);
    tok.owner = Some(TARGET);
    let mut tok_engine = crate::ws::test_support::token_engine(50.0, 50.0);
    let actor_id = Uuid::from_u128(0xC6);
    if vision.is_some() {
        tok_engine["actor_id"] = json!(actor_id.to_string());
    }
    tok.engine = Some(tok_engine);
    let mut wall = world_scoped_doc(WORLD, Uuid::from_u128(0xC7), "wall");
    wall.parent_id = Some(SCENE);
    wall.engine = Some(json!({
        "seg": { "x1": 100, "y1": -500, "x2": 100, "y2": 500 }, "blocksSight": true
    }));
    let mut ecs = SceneEcs::from_documents(vec![scene, tok, wall], 0);
    if let Some(v) = vision {
        let mut actor = world_scoped_doc(WORLD, actor_id, "actor");
        actor.owner = Some(TARGET);
        actor.engine = Some(json!({
            "displayName": "Watcher", "visual": { "kind": "image", "asset": "a.png" },
            "size": { "w": 1.0, "h": 1.0 }, "shape": "square", "conditions": [],
            "prototype": true, "vision": v,
        }));
        ecs.set_actors(vec![actor]);
    }
    ecs
}

fn sight(ecs: &SceneEcs, exclude: &[Uuid]) -> crate::scene::RecipientSight {
    ecs.recipient_sight(
        TARGET,
        WorldRole::Player,
        &WorldCapDefaults::default(),
        SCENE,
        exclude,
    )
}

fn pos(t_ms: f64, x: f64, y: f64) -> PosSample {
    PosSample { t_ms, pos: [x, y] }
}

/// A unit-intensity light sample at `pos` with dim reach `dim` (scene units) and an
/// unoccluded square occluder around it; the polygon's first vertex is nudged by `idx`
/// micro-units so an admitted sample is identifiable (`tag_of`).
fn light(idx: usize, t_ms: f64, pos: [f64; 2], dim: f64) -> LightSample {
    let i = idx as f64;
    LightSample {
        t_ms,
        pos,
        bright: dim / 2.0,
        dim,
        intensity: 1.0,
        falloff: crate::data::engine::FalloffCurve::Linear,
        color: 0xFFCC66,
        polygons: vec![vec![
            [pos[0] - dim + i * 1e-6, pos[1] - dim],
            [pos[0] + dim, pos[1] - dim],
            [pos[0] + dim, pos[1] + dim],
            [pos[0] - dim, pos[1] + dim],
        ]],
    }
}

/// The index `light` tagged a sample with, recovered from its polygon's first vertex.
fn tag_of(s: &LightSample) -> usize {
    ((s.polygons[0][0][0] - (s.pos[0] - s.dim)) / 1e-6).round() as usize
}

// --- Position clip ---

#[test]
fn clip_samples_admits_only_the_samples_inside_the_targets_line_of_sight() {
    let ecs = fixture(true, None);
    let sight = sight(&ecs, &[]);
    let inputs = ClipInputs {
        sight: &sight,
        in_flight: &[],
        target: TARGET,
    };
    let samples = vec![pos(0.0, 50.0, 60.0), pos(100.0, 150.0, 60.0)];
    let out = clip_samples(&samples, 1000.0, &inputs);
    assert_eq!(
        out,
        vec![pos(0.0, 50.0, 60.0)],
        "x=150 is behind the x=100 wall"
    );
}

#[test]
fn clip_samples_reads_the_targets_own_in_flight_viewpoint_per_instant() {
    // The target's own token walks past the wall from 1100: at t_abs=1000 its committed
    // viewpoint (50,50) cannot see (150,60); at t_abs=1200 its chosen position sample
    // (250,50) is past the wall, so the same point is visible.
    let ecs = fixture(true, None);
    let sight = sight(&ecs, &[]);
    let own = vec![pos(0.0, 50.0, 50.0), pos(100.0, 250.0, 50.0)];
    let in_flight = [InFlight {
        start_server_ms: 1100.0,
        mover: TARGET,
        token: TARGET_TOKEN,
        positions: &own,
        light: None,
    }];
    let inputs = ClipInputs {
        sight: &sight,
        in_flight: &in_flight,
        target: TARGET,
    };
    let samples = vec![pos(0.0, 150.0, 60.0), pos(200.0, 150.0, 60.0)];
    let out = clip_samples(&samples, 1000.0, &inputs);
    assert_eq!(out, vec![pos(200.0, 150.0, 60.0)]);
    // A stranger's in-flight move never substitutes the target's viewpoint.
    let foreign = [InFlight {
        start_server_ms: 1100.0,
        mover: STRANGER,
        token: Uuid::from_u128(0xC8),
        positions: &own,
        light: None,
    }];
    let inputs = ClipInputs {
        sight: &sight,
        in_flight: &foreign,
        target: TARGET,
    };
    assert!(clip_samples(&samples, 1000.0, &inputs).is_empty());
}

#[test]
fn clip_samples_requires_illumination_for_a_normal_vision_target() {
    // Pitch dark: the sample at (50,60) is inside the target's line of sight but unlit, so a
    // normal-vision target does not see it — the lit mask's own decision, not LOS alone.
    let ecs = fixture(false, None);
    let dark = sight(&ecs, &[]);
    let inputs = ClipInputs {
        sight: &dark,
        in_flight: &[],
        target: TARGET,
    };
    let samples = vec![pos(0.0, 50.0, 60.0)];
    assert!(clip_samples(&samples, 1000.0, &inputs).is_empty());

    // The mover's own torch (the frame's light timeline) lights the cell it stands in.
    let torch = vec![light(0, 0.0, [50.0, 60.0], 150.0)];
    let bearer = [InFlight {
        start_server_ms: 1000.0,
        mover: STRANGER,
        token: Uuid::from_u128(0xC8),
        positions: &samples,
        light: Some(&torch),
    }];
    let inputs = ClipInputs {
        sight: &dark,
        in_flight: &bearer,
        target: TARGET,
    };
    assert_eq!(clip_samples(&samples, 1000.0, &inputs), samples);

    // A glow-worm below the dim floor (0.34) lights nothing a normal-vision target sees.
    let faint = vec![LightSample {
        intensity: 0.2,
        ..light(0, 0.0, [50.0, 60.0], 150.0)
    }];
    let bearer = [InFlight {
        start_server_ms: 1000.0,
        mover: STRANGER,
        token: Uuid::from_u128(0xC8),
        positions: &samples,
        light: Some(&faint),
    }];
    let inputs = ClipInputs {
        sight: &dark,
        in_flight: &bearer,
        target: TARGET,
    };
    assert!(clip_samples(&samples, 1000.0, &inputs).is_empty());
}

#[test]
fn clip_samples_admits_a_dark_sample_to_a_darkvision_target_within_range() {
    let ecs = fixture(false, Some(json!([{ "mode": "darkvision", "range": 3.0 }])));
    let dark = sight(&ecs, &[]);
    let inputs = ClipInputs {
        sight: &dark,
        in_flight: &[],
        target: TARGET,
    };
    // (50,60) is 0.1 cells from the viewpoint: within range, dark floor met.
    // (50,460) is 4.1 cells away: beyond the 3-cell range, so darkvision does not reach it.
    let samples = vec![pos(0.0, 50.0, 60.0), pos(100.0, 50.0, 460.0)];
    assert_eq!(
        clip_samples(&samples, 1000.0, &inputs),
        vec![pos(0.0, 50.0, 60.0)]
    );
}

#[test]
fn another_movers_in_flight_torch_lights_a_bystander_per_instant() {
    // A torch walks from (50,50) at t=0 to (350,50) at t=500 (reach 150). A bystander's
    // sample at (60,50) is lit while the torch is near it and dark once the torch has moved
    // three cells east — the torch is composed in at its timeline position, never at its
    // committed (end-of-move) position.
    let ecs = fixture(false, None);
    let torch_token = Uuid::from_u128(0xC9);
    let dark = sight(&ecs, &[torch_token]);
    let torch_positions = vec![pos(0.0, 50.0, 50.0), pos(500.0, 350.0, 50.0)];
    let torch = vec![
        light(0, 0.0, [50.0, 50.0], 150.0),
        light(1, 500.0, [350.0, 50.0], 150.0),
    ];
    let in_flight = [InFlight {
        start_server_ms: 1000.0,
        mover: STRANGER,
        token: torch_token,
        positions: &torch_positions,
        light: Some(&torch),
    }];
    let inputs = ClipInputs {
        sight: &dark,
        in_flight: &in_flight,
        target: TARGET,
    };
    let bystander = vec![pos(0.0, 60.0, 50.0), pos(500.0, 60.0, 50.0)];
    assert_eq!(
        clip_samples(&bystander, 1000.0, &inputs),
        vec![pos(0.0, 60.0, 50.0)]
    );
}

// --- Carried-light admission (`admit_light_samples` / `disc_intersects_polys`) ---

#[test]
fn chosen_vision_sample_selects_light_samples_by_the_same_fixture_rule() {
    // ONE rule for both timelines: the light timeline is selected through the identical
    // generic `chosen_vision_sample` the vision clip uses, pinned on the shared fixture.
    let (vision, probes) = fixture_samples();
    let lights: Vec<LightSample> = vision
        .iter()
        .enumerate()
        .map(|(i, v)| light(i, v.t_ms, [0.0, 0.0], 1.0))
        .collect();
    for (elapsed, expect) in probes {
        let got = chosen_vision_sample(&lights, elapsed).unwrap();
        assert_eq!(tag_of(got), expect, "elapsed={elapsed}");
    }
}

#[test]
fn chosen_vision_sample_selects_position_samples_by_the_same_fixture_rule() {
    let (vision, probes) = fixture_samples();
    let positions: Vec<PosSample> = vision
        .iter()
        .enumerate()
        .map(|(i, v)| pos(v.t_ms, i as f64, 0.0))
        .collect();
    for (elapsed, expect) in probes {
        let got = chosen_vision_sample(&positions, elapsed).unwrap();
        assert_eq!(got.pos[0] as usize, expect, "elapsed={elapsed}");
    }
}

#[test]
fn disc_intersects_polys_center_inside_or_edge_within_reach() {
    let unit: Vec<P> = vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    let polys = || [unit.as_slice()];
    assert!(
        disc_intersects_polys((0.5, 0.5), 0.0, polys()),
        "center inside"
    );
    assert!(
        disc_intersects_polys((1.4, 0.5), 0.5, polys()),
        "edge within reach"
    );
    assert!(
        !disc_intersects_polys((1.6, 0.5), 0.5, polys()),
        "edge beyond reach"
    );
    assert!(
        !disc_intersects_polys((1.4, 0.5), 0.0, polys()),
        "zero reach is the point test"
    );
    assert!(
        !disc_intersects_polys((1.4, 0.5), -1.0, polys()),
        "negative reach is the point test"
    );
}

#[test]
fn disc_intersects_polys_fails_closed_on_non_finite_or_degenerate_input() {
    let unit: Vec<P> = vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    let polys = || [unit.as_slice()];
    assert!(!disc_intersects_polys((0.5, 0.5), f64::NAN, polys()));
    assert!(!disc_intersects_polys((0.5, 0.5), f64::INFINITY, polys()));
    assert!(!disc_intersects_polys((f64::NAN, 0.5), 1.0, polys()));
    let degenerate: Vec<P> = vec![(0.0, 0.0), (1.0, 0.0)];
    assert!(!disc_intersects_polys(
        (0.5, 0.0),
        10.0,
        [degenerate.as_slice()]
    ));
    assert!(!disc_intersects_polys((0.5, 0.5), 1.0, [] as [&[P]; 0]));
}

#[test]
fn admit_light_samples_is_none_in_none_out_and_none_when_nothing_reaches() {
    let ecs = fixture(true, None);
    let sight = sight(&ecs, &[]);
    let inputs = ClipInputs {
        sight: &sight,
        in_flight: &[],
        target: TARGET,
    };
    assert!(admit_light_samples(None, 0.0, &inputs).is_none());
    // 5,000 units past the wall with a 1-unit reach: nothing reaches the target's sight.
    let far = vec![light(0, 0.0, [5000.0, 50.0], 1.0)];
    assert!(
        admit_light_samples(Some(&far), 0.0, &inputs).is_none(),
        "a timeline nothing of which reaches the recipient is None, never an empty list"
    );
}

#[test]
fn admit_light_samples_keeps_only_the_samples_whose_glow_reaches_the_line_of_sight() {
    // The target's sight ends at the x=100 wall. With lighting off every line-of-sight cell
    // qualifies, so admission reduces to the glow's own reach into that sight.
    let ecs = fixture(true, None);
    let sight = sight(&ecs, &[]);
    let samples = vec![
        light(0, 0.0, [50.0, 60.0], 60.0), // inside (reach 0.6 cells; its own cell is within the bright radius)
        light(1, 100.0, [300.0, 50.0], 100.0), // 200 past the wall, reach 100 → out
        light(2, 200.0, [300.0, 50.0], 260.0), // reach 260 crosses the wall → in
    ];
    let positions: Vec<PosSample> = samples
        .iter()
        .map(|l| pos(l.t_ms, l.pos[0], l.pos[1]))
        .collect();
    let flight = [InFlight {
        start_server_ms: 0.0,
        mover: STRANGER,
        token: Uuid::from_u128(0xC8),
        positions: &positions,
        light: Some(&samples),
    }];
    let inputs = ClipInputs {
        sight: &sight,
        in_flight: &flight,
        target: TARGET,
    };
    let out = admit_light_samples(Some(&samples), 0.0, &inputs).unwrap();
    assert_eq!(out.iter().map(tag_of).collect::<Vec<_>>(), vec![0, 2]);
}

#[test]
fn admit_light_samples_drops_a_glow_whose_occluded_polygon_lights_no_cell_in_sight() {
    // The disc of a lamp at (150,50) with reach 120 crosses the x=100 wall into the target's
    // sight, but its own illumination polygon (raycast against a `blocksLight` wall at the
    // emitter's side) stops at x=120: no cell center the target sees is inside it, so the
    // client would paint nothing and the sample is dropped. The same lamp with an open
    // polygon is admitted. Lighting off: every line-of-sight cell qualifies, so the
    // occluder alone decides.
    let ecs = fixture(true, None);
    let sight = sight(&ecs, &[]);
    let positions = vec![pos(0.0, 150.0, 50.0)];
    let mut blocked = light(0, 0.0, [150.0, 50.0], 120.0);
    blocked.polygons = vec![vec![
        [120.0, -500.0],
        [500.0, -500.0],
        [500.0, 500.0],
        [120.0, 500.0],
    ]];
    let blocked = [blocked];
    let flight = [InFlight {
        start_server_ms: 0.0,
        mover: STRANGER,
        token: Uuid::from_u128(0xC8),
        positions: &positions,
        light: Some(&blocked),
    }];
    let inputs = ClipInputs {
        sight: &sight,
        in_flight: &flight,
        target: TARGET,
    };
    assert!(admit_light_samples(Some(&blocked), 0.0, &inputs).is_none());
    let open = [light(1, 0.0, [150.0, 50.0], 120.0)];
    let flight = [InFlight {
        start_server_ms: 0.0,
        mover: STRANGER,
        token: Uuid::from_u128(0xC8),
        positions: &positions,
        light: Some(&open),
    }];
    let inputs = ClipInputs {
        sight: &sight,
        in_flight: &flight,
        target: TARGET,
    };
    assert_eq!(
        admit_light_samples(Some(&open), 0.0, &inputs)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn glow_reaches_fails_closed_on_a_degenerate_reach_and_keeps_the_disc_verdict_past_the_cap() {
    // Lighting off: every line-of-sight cell qualifies, so reach geometry alone decides.
    let ecs = fixture(true, None);
    let sight = sight(&ecs, &[]);
    let instant = sight.at(&[]);
    assert!(!glow_reaches(
        &instant,
        &[],
        &light(0, 0.0, [50.0, 50.0], f64::NAN)
    ));
    assert!(!glow_reaches(
        &instant,
        &[],
        &light(0, 0.0, [50.0, 50.0], 0.0)
    ));
    // A reach of 10,000 units is a 200×200-cell box, past `MAX_GLOW_ADMISSION_CELLS`: the disc
    // test decides — it touches the target's sight, so the sample is admitted even though its
    // polygon has been emptied.
    let mut huge = light(0, 0.0, [50.0, 50.0], 10_000.0);
    huge.polygons.clear();
    assert!(glow_reaches(&instant, &[], &huge));
    // An emptied polygon under the cap composes as an unoccluded light and reaches its own
    // cell; a polygon that excludes every cell in reach paints nothing → dropped.
    let mut small = light(0, 0.0, [50.0, 50.0], 100.0);
    small.polygons.clear();
    assert!(glow_reaches(&instant, &[], &small));
    small.polygons = vec![vec![[5000.0, 5000.0], [5001.0, 5000.0], [5001.0, 5001.0]]];
    assert!(!glow_reaches(&instant, &[], &small));
}

#[test]
fn admit_light_samples_reads_the_same_instant_sight_as_clip_samples() {
    // The target's own sweep past the wall starts at 1100. A glow at (150,50) with a tiny
    // reach is dropped at t_abs=1000 (committed viewpoint behind the wall) and admitted at
    // t_abs=1200 (viewpoint (250,50) past it) — exactly the instants `clip_samples` admits a
    // position there, because both read `ClipInputs::at`.
    let ecs = fixture(true, None);
    let sight = sight(&ecs, &[]);
    let own = vec![pos(0.0, 250.0, 50.0)];
    let in_flight = [InFlight {
        start_server_ms: 1100.0,
        mover: TARGET,
        token: TARGET_TOKEN,
        positions: &own,
        light: None,
    }];
    let inputs = ClipInputs {
        sight: &sight,
        in_flight: &in_flight,
        target: TARGET,
    };
    let lights = vec![
        light(0, 0.0, [150.0, 50.0], 0.01),
        light(1, 200.0, [150.0, 50.0], 0.01),
    ];
    let out = admit_light_samples(Some(&lights), 1000.0, &inputs).unwrap();
    assert_eq!(out.iter().map(tag_of).collect::<Vec<_>>(), vec![1]);
    let positions = vec![pos(0.0, 150.0, 50.0), pos(200.0, 150.0, 50.0)];
    assert_eq!(
        clip_samples(&positions, 1000.0, &inputs),
        vec![pos(200.0, 150.0, 50.0)]
    );
}

/// A stranger's in-flight move carrying `light` along `positions`, starting at 1000 ms.
fn bearer<'a>(positions: &'a [PosSample], light: &'a [LightSample]) -> [InFlight<'a>; 1] {
    [InFlight {
        start_server_ms: 1000.0,
        mover: STRANGER,
        token: Uuid::from_u128(0xC8),
        positions,
        light: Some(light),
    }]
}

#[test]
fn glow_admission_requires_the_glow_to_light_a_cell_the_target_sees() {
    // An ember (intensity 0.2, below the dim floor 0.34) at (50,60), in plain line of sight of a
    // normal-vision target in a dark scene, lights nothing that target sees — the lit mask
    // lights nothing for it either — so no glow-only frame discloses its bearer's position.
    // A torch above the floor at the same spot is admitted; a darkvision target within range
    // (dark floor) is shown the ember.
    let positions = vec![pos(0.0, 50.0, 60.0)];
    let ember = vec![LightSample {
        intensity: 0.2,
        ..light(0, 0.0, [50.0, 60.0], 150.0)
    }];
    let torch = vec![light(0, 0.0, [50.0, 60.0], 150.0)];
    let ecs = fixture(false, None);
    let dark = sight(&ecs, &[]);
    let ember_flight = bearer(&positions, &ember);
    let inputs = ClipInputs {
        sight: &dark,
        in_flight: &ember_flight,
        target: TARGET,
    };
    assert!(
        admit_light_samples(Some(&ember), 1000.0, &inputs).is_none(),
        "an ember below the dim floor lights no cell a normal-vision target sees"
    );
    let torch_flight = bearer(&positions, &torch);
    let inputs = ClipInputs {
        sight: &dark,
        in_flight: &torch_flight,
        target: TARGET,
    };
    assert_eq!(
        admit_light_samples(Some(&torch), 1000.0, &inputs)
            .unwrap()
            .len(),
        1
    );
    let ecs = fixture(false, Some(json!([{ "mode": "darkvision", "range": 3.0 }])));
    let darkvision = sight(&ecs, &[]);
    let inputs = ClipInputs {
        sight: &darkvision,
        in_flight: &ember_flight,
        target: TARGET,
    };
    assert_eq!(
        admit_light_samples(Some(&ember), 1000.0, &inputs)
            .unwrap()
            .len(),
        1,
        "a darkvision target within range sees the ember-lit cell"
    );
}
