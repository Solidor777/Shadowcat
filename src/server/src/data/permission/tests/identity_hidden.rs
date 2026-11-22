//! Secret-identity tokens: the `identity_hidden` flag withholds a token's presented identity from
//! every recipient below `OwnerOrGm`, through the one function every egress and merge path reads
//! (`own_overrides`), and its write gate is GM-only.

use super::*;
use crate::merge::{MergeVisibility, RequesterView, Side};

/// Marker text carried by every identity-bearing field; a leak anywhere is a substring hit.
const REAL_NAME: &str = "REAL-NAME-MARKER";

/// A `token` document whose identity-bearing fields are all populated, with the flag as given.
fn token(identity_hidden: bool) -> Document {
    let mut d = doc(PermissionSet::default(), serde_json::json!({}));
    d.doc_type = "token".into();
    d.name = Some(REAL_NAME.into());
    d.engine = Some(serde_json::json!({
        "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0,
        "visual": { "kind": "image", "asset": "11111111-1111-1111-1111-111111111111" },
        "actor_id": "22222222-2222-2222-2222-222222222222",
        "overrides": {
            "name": REAL_NAME,
            "visual": { "kind": "image", "asset": "33333333-3333-3333-3333-333333333333" },
            "disposition": "hostile"
        },
        "identity_hidden": identity_hidden,
    }));
    d
}

fn owner() -> Access {
    Access {
        is_owner: true,
        ..non_gm()
    }
}

fn gm() -> Access {
    Access {
        all: true,
        see_gm_only: true,
        is_owner: true,
        ..non_gm()
    }
}

/// The serialized document a recipient with `access` receives.
fn egress(d: &Document, access: &Access) -> serde_json::Value {
    serde_json::to_value(filter_properties(d, access).unwrap()).unwrap()
}

/// Asserts `pointer` is present on the unredacted token and withheld from a stranger, so the
/// absence is the redaction and not a missing fixture field.
fn assert_withheld_from_stranger(pointer: &str) {
    let t = token(true);
    let raw = serde_json::to_value(&t).unwrap();
    assert!(
        raw.pointer(pointer).is_some_and(|v| !v.is_null()),
        "fixture must carry {pointer}"
    );
    let out = egress(&t, &non_gm());
    if pointer == "/name" {
        assert!(out["name"].is_null(), "/name must be nulled in place");
    } else {
        assert!(
            out.pointer(pointer).is_none(),
            "{pointer} must be a stripped KEY, got {:?}",
            out.pointer(pointer)
        );
    }
    assert!(
        !out.to_string().contains(REAL_NAME),
        "the real name must not appear anywhere in the egress document"
    );
}

#[test]
fn identity_hidden_name_is_withheld_from_a_stranger() {
    assert_withheld_from_stranger("/name");
}

#[test]
fn identity_hidden_visual_key_is_absent_for_a_stranger() {
    assert_withheld_from_stranger("/engine/visual");
}

#[test]
fn identity_hidden_override_visual_is_withheld_from_a_stranger() {
    assert_withheld_from_stranger("/engine/overrides/visual");
}

#[test]
fn identity_hidden_override_name_is_withheld_from_a_stranger() {
    assert_withheld_from_stranger("/engine/overrides/name");
}

#[test]
fn identity_hidden_override_disposition_is_withheld_from_a_stranger() {
    assert_withheld_from_stranger("/engine/overrides/disposition");
}

#[test]
fn identity_hidden_actor_link_is_withheld_so_the_faction_border_cannot_be_joined() {
    assert_withheld_from_stranger("/engine/actor_id");
}

#[test]
fn identity_hidden_owner_and_gm_still_see_every_field() {
    let t = token(true);
    let raw = serde_json::to_value(&t).unwrap();
    assert_eq!(egress(&t, &owner()), raw, "the owner's token is unredacted");
    assert_eq!(egress(&t, &gm()), raw, "the GM's token is unredacted");
}

#[test]
fn identity_hidden_off_hides_nothing() {
    let t = token(false);
    assert_eq!(egress(&t, &non_gm()), serde_json::to_value(&t).unwrap());
}

#[test]
fn identity_hidden_key_on_a_non_token_hides_nothing() {
    let mut a = token(true);
    a.doc_type = "actor".into();
    let out = egress(&a, &non_gm());
    assert_eq!(out["name"], REAL_NAME);
    assert!(out.pointer("/engine/visual").is_some());
}

#[test]
fn identity_hidden_does_not_loosen_a_stricter_author_override() {
    // The author hid the visual from owners too; the synthesized `OwnerOrGm` entry for the same
    // pointer must not lower it.
    let mut t = token(true);
    t.permissions
        .property_overrides
        .insert("/engine/visual".into(), Visibility::GmOnly);
    let out = egress(&t, &owner());
    assert!(out.pointer("/engine/visual").is_none());
    assert_eq!(
        out["name"], REAL_NAME,
        "the rest stays visible to the owner"
    );
}

#[test]
fn identity_hidden_set_reaches_the_merge_oracle_and_the_commit_time_snapshot() {
    // `RequesterView::hidden` and `collect_overrides` (the `filter_command` and `OpSnapshot`
    // chain) are separate entry points from `filter_properties`; each must see the synthesis.
    let t = token(true);
    let stranger = non_gm();
    let view = RequesterView {
        template: &stranger,
        child: &stranger,
    };
    let hidden = view.hidden(Side::Template, &t).unwrap();
    let mut collected = Vec::new();
    collect_overrides(&t, "", &mut collected).unwrap();
    for pointer in [
        "/name",
        "/engine/visual",
        "/engine/overrides/visual",
        "/engine/overrides/name",
        "/engine/overrides/disposition",
        "/engine/actor_id",
    ] {
        assert!(
            hidden.iter().any(|h| h == pointer),
            "RequesterView must hide {pointer}"
        );
        assert!(
            collected
                .iter()
                .any(|(p, v)| p == pointer && *v == Visibility::OwnerOrGm),
            "collect_overrides must carry {pointer} at OwnerOrGm"
        );
    }
}

#[test]
fn identity_hidden_touched_is_value_aware() {
    let off = serde_json::json!({ "engine": { "x": 0.0, "identity_hidden": false } });
    let on = serde_json::json!({ "engine": { "x": 0.0, "identity_hidden": true } });
    // A direct write always counts.
    assert!(identity_hidden_touched(
        "token",
        "/engine/identity_hidden",
        false,
        &off,
        &serde_json::json!(true)
    ));
    // An ancestor write counts only when the effective value changes; absent reads as false.
    assert!(!identity_hidden_touched(
        "token",
        "/engine",
        false,
        &off,
        &serde_json::json!({ "x": 1.0 })
    ));
    assert!(identity_hidden_touched(
        "token",
        "/engine",
        false,
        &off,
        &serde_json::json!({ "x": 1.0, "identity_hidden": true })
    ));
    assert!(!identity_hidden_touched(
        "token",
        "/engine",
        false,
        &on,
        &serde_json::json!({ "x": 1.0, "identity_hidden": true })
    ));
    // Removing the ancestor of an enabled flag turns it off: a touch.
    assert!(identity_hidden_touched(
        "token",
        "/engine",
        true,
        &on,
        &serde_json::Value::Null
    ));
    // Unrelated sibling paths and other doc types never count.
    assert!(!identity_hidden_touched(
        "token",
        "/engine/x",
        false,
        &off,
        &serde_json::json!(5.0)
    ));
    assert!(!identity_hidden_touched(
        "scene",
        "/engine/identity_hidden",
        false,
        &off,
        &serde_json::json!(true)
    ));
}

#[test]
fn identity_hidden_in_body_reads_only_an_enabled_token_flag() {
    let t = |on| serde_json::to_value(token(on)).unwrap();
    assert!(identity_hidden_in_body("token", &t(true)));
    assert!(!identity_hidden_in_body("token", &t(false)));
    assert!(!identity_hidden_in_body("actor", &t(true)));
}
