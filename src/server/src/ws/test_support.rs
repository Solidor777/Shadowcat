//! Shared `#[cfg(test)]` fixture helpers for `ws::room`'s and `ws::conn`'s test modules.
//!
//! `token_engine` fills the `w`/`h`/`rotation` fields `TokenEngine` requires that a bare
//! `(x, y)` fixture value never carries. `ws_engine` fills `scene.movementModel`, a field
//! `WorldSceneDefaults` requires that a hand-authored world-settings fixture body never
//! carries. Every fixture position/geometry doc (token/wall/region/scene/light) is
//! constructed with `engine` set directly — `system` stays whatever `world_scoped_doc`
//! defaults it to (opaque game-system data, never read by movement/vision/region code).

pub(crate) fn ws_engine(mut system: serde_json::Value) -> serde_json::Value {
    if let Some(scene) = system.get_mut("scene").and_then(|s| s.as_object_mut()) {
        scene
            .entry("movementModel")
            .or_insert(serde_json::json!("grid-stepped"));
    }
    system
}

pub(crate) fn token_engine(x: f64, y: f64) -> serde_json::Value {
    serde_json::json!({ "x": x, "y": y, "w": 1.0, "h": 1.0, "rotation": 0.0 })
}
