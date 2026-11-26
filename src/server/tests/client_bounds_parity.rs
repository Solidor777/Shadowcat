//! Parity between the numeric bounds the scene-tools client copies (`server-bounds.ts`) and the
//! server's engine ingress validation. Both sides read one fixture, so a bound that moves on
//! either side fails the other.

use serde_json::{json, Value};
use shadowcat::data::engine::{validate_engine, MAX_PROXIMITY_CELLS};

/// The shared fixture the client test reads too.
fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../modules/scene-tools/src/__fixtures__/server-bounds.json"
    ))
    .expect("the shared bounds fixture is valid JSON")
}

/// The sample values listed under `key` (`accepted` or `rejected`) of the fixture entry `entry`.
fn samples(entry: &Value, key: &str) -> Vec<f64> {
    entry[key]
        .as_array()
        .expect("the fixture lists samples")
        .iter()
        .map(|v| v.as_f64().expect("every sample is a number"))
        .collect()
}

/// Asserts the server accepts every `accepted` sample and refuses every `rejected` one, building
/// each engine body with `body`.
fn assert_agrees(label: &str, doc_type: &str, entry: &Value, body: impl Fn(f64) -> Value) {
    for v in samples(entry, "accepted") {
        assert!(
            validate_engine(doc_type, Some(&body(v))).is_ok(),
            "{label} {v} must be accepted"
        );
    }
    for v in samples(entry, "rejected") {
        assert!(
            validate_engine(doc_type, Some(&body(v))).is_err(),
            "{label} {v} must be refused"
        );
    }
}

/// A wall body carrying a proximity tier of distance `within`.
fn wall(within: f64) -> Value {
    json!({
        "seg": { "x1": 0.0, "y1": 0.0, "x2": 100.0, "y2": 0.0 },
        "proximity": { "within": within, "senses": ["sight"], "soundGain": null }
    })
}

/// A template body with a cone of aperture `angle` degrees.
fn cone(angle: f64) -> Value {
    json!({
        "shape": { "kind": "cone", "x": 0.0, "y": 0.0, "size": 50.0, "direction": 0.0, "angle": angle, "width": null },
        "color": "#3388ff"
    })
}

/// A template body with a line of perpendicular thickness `width`.
fn line(width: f64) -> Value {
    json!({
        "shape": { "kind": "line", "x": 0.0, "y": 0.0, "size": 50.0, "direction": 0.0, "angle": null, "width": width },
        "color": "#3388ff"
    })
}

/// An `ambient_light` region body whose override is `mode` at `level`.
fn ambient(mode: &str, level: f64) -> Value {
    json!({
        "shape": { "kind": "rect", "points": [0.0, 0.0, 100.0, 100.0] },
        "behavior": "ambient_light", "cost": 1.0, "cost_by_tag": {}, "enabled": true,
        "triggers": [], "elevation": null, "occludes": null, "anchor": null,
        "ambient_override": { "mode": mode, "level": level }
    })
}

#[test]
fn the_client_proximity_ceiling_is_the_servers() {
    assert_eq!(
        fixture()["maxProximityCells"].as_f64(),
        Some(MAX_PROXIMITY_CELLS)
    );
}

#[test]
fn proximity_distance_agrees_with_the_client() {
    assert_agrees(
        "proximity within",
        "wall",
        &fixture()["proximityWithin"],
        wall,
    );
}

#[test]
fn template_angle_agrees_with_the_client() {
    assert_agrees(
        "template angle",
        "template",
        &fixture()["templateAngle"],
        cone,
    );
}

#[test]
fn template_width_agrees_with_the_client() {
    assert_agrees(
        "template width",
        "template",
        &fixture()["templateWidth"],
        line,
    );
}

#[test]
fn ambient_level_agrees_with_the_client_for_every_mode() {
    let fixture = fixture();
    for mode in ["override", "darken", "brighten"] {
        assert_agrees(
            &format!("ambient {mode} level"),
            "region",
            &fixture["ambientLevel"][mode],
            |level| ambient(mode, level),
        );
    }
}
