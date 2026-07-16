//! Shared `#[cfg(test)]` fixture helpers for `ws::room`'s and `ws::conn`'s test modules.
//!
//! Dual-write helpers: world-settings/scene/token/wall/region/light `.system` fixture
//! values in `room.rs`/`conn.rs` tests are already field-name/casing-parity with the
//! corresponding `engine` band shapes (the scene/vision/region/lighting readers consume
//! `engine`; `token_move` still reads `system`). `token_engine` fills the `w`/`h`/`rotation`
//! fields `TokenEngine` requires that fixture `system` values never carry. `ws_engine` fills
//! `scene.movementModel`, a field `WorldSceneDefaults` requires that fixture `system` values
//! never carry. Every other doc type's `engine` is a verbatim clone of its `system` fixture
//! value.

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
