//! The GM-only pointer sets cover an instanced token's embedded actor copies by one rule.

use super::*;

/// A token whose first embedded actor copy carries a facing arc and a carried light.
fn instanced_token_json() -> serde_json::Value {
    serde_json::json!({
        "engine": {},
        "embedded": { "actor": [ { "engine": {
            "facing_vision": { "arc": 90.0, "near": 0.0 },
            "light": { "bright": 2.0, "dim": 4.0 }
        } } ] }
    })
}

#[test]
fn a_facing_arc_written_on_an_embedded_actor_copy_is_a_facing_vision_touch() {
    let whole = instanced_token_json();
    let wide = serde_json::json!({ "arc": 360.0, "near": 0.0 });
    assert!(facing_vision_touched(
        "token",
        "/embedded/actor/0/engine/facing_vision",
        false,
        &whole,
        &wide
    ));
    let same = serde_json::json!({ "arc": 90.0, "near": 0.0 });
    assert!(
        !facing_vision_touched(
            "token",
            "/embedded/actor/0/engine/facing_vision",
            false,
            &whole,
            &same
        ),
        "restating the stored arc is not a touch"
    );
}

#[test]
fn an_ancestor_write_that_introduces_an_embedded_facing_arc_is_a_touch() {
    let whole = serde_json::json!({ "engine": {} });
    let new = serde_json::json!({ "actor": [ { "engine": {
        "facing_vision": { "arc": 360.0, "near": 0.0 }
    } } ] });
    assert!(facing_vision_touched(
        "token",
        "/embedded",
        false,
        &whole,
        &new
    ));
}

#[test]
fn a_created_token_body_carrying_an_embedded_facing_arc_is_flagged() {
    assert!(facing_vision_in_body("token", &instanced_token_json()));
}

#[test]
fn facing_vision_and_carried_light_name_the_same_embedded_copies() {
    let whole = instanced_token_json();
    let new = serde_json::Value::Null;
    let light = carried_light_pointers("token", &whole, "/engine", &new);
    let facing = facing_vision_pointers("token", &whole, "/engine", &new);
    let copies = |set: &[String]| {
        set.iter()
            .filter(|p| p.starts_with("/embedded/actor/"))
            .map(|p| p.split('/').nth(3).unwrap().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(copies(&light), vec!["0"]);
    assert_eq!(copies(&light), copies(&facing));
}
