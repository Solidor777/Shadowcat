use super::*;
use crate::data::engine::bars::{MAX_BAR_ATTRIBUTES, MAX_BAR_LABEL_CHARS};

fn base() -> TokenEngine {
    TokenEngine {
        x: 0.0,
        y: 0.0,
        w: 100.0,
        h: 100.0,
        rotation: 0.0,
        visual: None,
        actor_id: None,
        overrides: None,
        face: None,
        elevation: None,
        lock_rotation: false,
        identity_hidden: false,
        facing_vision: None,
    }
}

#[test]
fn a_stored_token_without_lock_rotation_decodes_unlocked() {
    let t: TokenEngine = serde_json::from_value(
        serde_json::json!({ "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0 }),
    )
    .unwrap();
    assert!(!t.lock_rotation);
}

#[test]
fn finite_in_bound_token_validates() {
    assert!(base().validate().is_ok());
}

#[test]
fn non_finite_fields_are_rejected() {
    for f in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut t = base();
        t.x = f;
        assert!(t.validate().is_err(), "x = {f} must be rejected");
        let mut t = base();
        t.rotation = f;
        assert!(t.validate().is_err(), "rotation = {f} must be rejected");
        let mut t = base();
        t.elevation = Some(f);
        assert!(t.validate().is_err(), "elevation = {f} must be rejected");
    }
}

#[test]
fn facing_vision_validates_arc_and_near() {
    let ok = |arc: f64, near: f64| FacingVision { arc, near }.validate().is_ok();
    assert!(ok(120.0, 1.0));
    assert!(ok(360.0, 0.0), "a full circle is admissible");
    assert!(ok(0.5, 0.0));
    for bad in [0.0, -1.0, 360.5, f64::NAN, f64::INFINITY] {
        assert!(!ok(bad, 0.0), "arc {bad} must be rejected");
    }
    for bad in [-0.1, f64::NAN, f64::INFINITY] {
        assert!(!ok(90.0, bad), "near {bad} must be rejected");
    }
}

#[test]
fn facing_vision_is_validated_on_all_three_bands() {
    let bad = FacingVision {
        arc: 0.0,
        near: 0.0,
    };
    let mut t = base();
    t.facing_vision = Some(bad);
    assert!(t.validate().is_err(), "token band");

    let o = TokenOverrides {
        name: None,
        visual: None,
        size: None,
        shape: None,
        vision: None,
        facing_vision: Some(bad),
        light: None,
        movement: None,
        aura: None,
        sound: None,
        vfx: None,
        disposition: None,
        bars: None,
        nameplate_mode: None,
    };
    assert!(o.validate().is_err(), "overrides band");

    let mut actor: ActorEngine = serde_json::from_value(serde_json::json!({
        "displayName": "A",
        "visual": { "kind": "image", "asset": "11111111-1111-1111-1111-111111111111" },
        "size": { "w": 1.0, "h": 1.0 },
        "shape": "square",
        "faction": null,
        "conditions": [],
        "prototype": false,
    }))
    .unwrap();
    assert!(actor.validate().is_ok());
    actor.facing_vision = Some(bad);
    assert!(actor.validate().is_err(), "actor band");
}

/// A valid bar the bar-validation tests vary one field of.
fn bar() -> BarAttribute {
    BarAttribute {
        label: "HP".to_string(),
        current: crate::data::engine::combat::Formula::Text("hp.value".to_string()),
        max: crate::data::engine::combat::Formula::Number(10.0),
        color: "#cc2222".to_string(),
        mode: BarDisplayMode::Always,
    }
}

/// An `ActorEngine` carrying `bars`.
fn actor_with_bars(bars: Option<Vec<BarAttribute>>) -> ActorEngine {
    let mut actor: ActorEngine = serde_json::from_value(serde_json::json!({
        "displayName": "A",
        "visual": { "kind": "image", "asset": "11111111-1111-1111-1111-111111111111" },
        "size": { "w": 1.0, "h": 1.0 },
        "shape": "square",
        "faction": null,
        "conditions": [],
        "prototype": false,
    }))
    .unwrap();
    actor.bars = bars;
    actor
}

#[test]
fn bar_attribute_validate_rejects_a_bad_color_label_or_formula() {
    assert!(bar().validate("bars[0]").is_ok());
    let mut b = bar();
    b.color = "red".to_string();
    assert!(b.validate("bars[0]").is_err(), "colour must be #rrggbb");
    let mut b = bar();
    b.label = String::new();
    assert!(b.validate("bars[0]").is_err(), "label must not be empty");
    let mut b = bar();
    b.label = "x".repeat(MAX_BAR_LABEL_CHARS + 1);
    assert!(b.validate("bars[0]").is_err(), "label is bounded");
    let mut b = bar();
    b.current = crate::data::engine::combat::Formula::Text("1 +".to_string());
    assert!(b.validate("bars[0]").is_err(), "formula must parse");
    let mut b = bar();
    b.max = crate::data::engine::combat::Formula::Number(f64::NAN);
    assert!(b.validate("bars[0]").is_err(), "a number must be finite");
}

#[test]
fn bars_are_capped_on_the_actor_and_the_override_and_never_truncated() {
    assert!(actor_with_bars(Some(vec![bar(); MAX_BAR_ATTRIBUTES]))
        .validate()
        .is_ok());
    assert!(actor_with_bars(Some(vec![bar(); MAX_BAR_ATTRIBUTES + 1]))
        .validate()
        .is_err());
    let overrides = |bars: Vec<BarAttribute>| {
        let mut t = base();
        t.overrides = Some(serde_json::from_value(serde_json::json!({ "bars": bars })).unwrap());
        t
    };
    assert!(overrides(vec![bar(); MAX_BAR_ATTRIBUTES])
        .validate()
        .is_ok());
    assert!(overrides(vec![bar(); MAX_BAR_ATTRIBUTES + 1])
        .validate()
        .is_err());
}

#[test]
fn a_bar_without_a_mode_decodes_as_hover_and_unknown_keys_are_refused() {
    let b: BarAttribute = serde_json::from_value(serde_json::json!({
        "label": "HP", "current": 1.0, "max": 2.0, "color": "#ffffff"
    }))
    .unwrap();
    assert_eq!(b.mode, BarDisplayMode::Hover);
    assert!(serde_json::from_value::<BarAttribute>(serde_json::json!({
        "label": "HP", "current": 1.0, "max": 2.0, "color": "#ffffff", "extra": 1
    }))
    .is_err());
}

#[test]
fn absent_bars_and_nameplate_mode_are_not_serialized() {
    let v = serde_json::to_value(actor_with_bars(None)).unwrap();
    assert!(v.get("bars").is_none());
    assert!(v.get("nameplate_mode").is_none());
}

#[test]
fn an_absent_facing_vision_is_not_serialized() {
    let v = serde_json::to_value(base()).unwrap();
    assert!(v.get("facing_vision").is_none());
}

#[test]
fn ingress_bound_equals_gate_walks_exactly() {
    // Anti-drift: ingress and the movement gate read ONE symbol with the same
    // strictly-`>` sense.
    let bound = crate::scene::move_exec::MAX_GATE_WALK_COORD;
    let mut t = base();
    t.x = bound;
    assert!(t.validate().is_ok(), "AT the bound is admissible");
    t.x = bound + 1.0;
    assert!(t.validate().is_err(), "over the bound is refused");
    let mut t = base();
    t.y = -(bound + 1.0);
    assert!(t.validate().is_err());

    // The walk side of the same bound, asserted here so one test pins the EQUALITY of the two
    // senses rather than leaving it inferred across two files.
    let at = crate::scene::move_exec::MAX_GATE_WALK_COORD;
    assert!(
        crate::scene::move_exec::gate_walk(&[(at - 100.0, 0.0), (at, 0.0)], 100.0).is_some(),
        "gate_walk admits a coordinate exactly AT the bound"
    );
    assert!(
        crate::scene::move_exec::gate_walk(&[(at - 100.0, 0.0), (at + 1.0, 0.0)], 100.0).is_none(),
        "gate_walk refuses a coordinate over the bound"
    );
}

fn aura() -> AuraEmission {
    AuraEmission {
        color: "#ffcc66".to_string(),
        opacity: 0.4,
        radius: 2.0,
        enabled: true,
    }
}

fn sound() -> SoundEmission {
    SoundEmission {
        asset: "44444444-4444-4444-4444-444444444444".to_string(),
        radius: 5.0,
        volume: 0.8,
        loop_: true,
        enabled: true,
    }
}

fn vfx() -> VfxEmission {
    VfxEmission {
        asset: "55555555-5555-5555-5555-555555555555".to_string(),
        anchor: VfxAnchor::Token,
        loop_: true,
        enabled: true,
    }
}

#[test]
fn well_formed_emissions_validate() {
    assert!(aura().validate().is_ok());
    assert!(sound().validate().is_ok());
    assert!(vfx().validate().is_ok());
    // A disabled emission still validates its payload (fail-closed: storage,
    // not the enabled flag, is the ingress boundary).
    let mut a = aura();
    a.enabled = false;
    assert!(a.validate().is_ok());
}

#[test]
fn aura_color_must_be_css_hex() {
    for bad in ["ffcc66", "#fc6", "#ffcc6600", "#ggcc66", "", "#ffcc6g"] {
        let mut a = aura();
        a.color = bad.to_string();
        assert!(a.validate().is_err(), "color {bad:?} must be rejected");
    }
    let mut upper = aura();
    upper.color = "#FFCC66".to_string();
    assert!(
        upper.validate().is_ok(),
        "uppercase hex digits are css-valid"
    );
}

#[test]
fn emission_scalars_must_be_finite() {
    for f in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut a = aura();
        a.opacity = f;
        assert!(a.validate().is_err(), "opacity = {f} must be rejected");
        let mut s = sound();
        s.volume = f;
        assert!(s.validate().is_err(), "volume = {f} must be rejected");
        let mut r = aura();
        r.radius = f;
        assert!(r.validate().is_err(), "radius = {f} must be rejected");
    }
    // Out-of-range-but-finite opacity/volume pass ingress: the presentation
    // range is a read-side clamp where consumed, not a rejection.
    let mut a = aura();
    a.opacity = 1.5;
    assert!(a.validate().is_ok());
    let mut s = sound();
    s.volume = -0.25;
    assert!(s.validate().is_ok());
}

#[test]
fn emission_radius_shares_the_cell_radius_cap() {
    // Anti-drift: the emission radius bound reads the ONE cell-radius cap
    // symbol every cell-measured radius shares.
    let bound = crate::scene::pathfinding::MAX_FOOTPRINT_CELLS;
    let mut a = aura();
    a.radius = bound;
    assert!(a.validate().is_ok(), "AT the cap is admissible");
    a.radius = bound + 1.0;
    assert!(a.validate().is_err(), "over the cap is refused");
    a.radius = -1.0;
    assert!(a.validate().is_err(), "negative is refused");
    a.radius = 0.0;
    assert!(
        a.validate().is_ok(),
        "zero is admissible (the read side skips it)"
    );
    let mut s = sound();
    s.radius = bound + 1.0;
    assert!(s.validate().is_err(), "the sound radius reads the same cap");
}

#[test]
fn emission_asset_must_be_non_empty() {
    let mut s = sound();
    s.asset = String::new();
    assert!(s.validate().is_err());
    let mut v = vfx();
    v.asset = String::new();
    assert!(v.validate().is_err());
}

#[test]
fn actor_engine_validate_covers_only_emissions() {
    let engine: ActorEngine = serde_json::from_value(serde_json::json!({
        "displayName": "G",
        "visual": { "kind": "image", "asset": "66666666-6666-6666-6666-666666666666" },
        "size": { "w": 1.0, "h": 1.0 },
        "shape": "square",
        "faction": null,
        "conditions": [],
        "prototype": false,
        "aura": { "color": "red", "opacity": 0.4, "radius": 2.0, "enabled": true },
    }))
    .expect("well-formed engine body");
    assert!(
        engine.validate().is_err(),
        "a malformed emission color fails the actor"
    );
    let clean: ActorEngine = serde_json::from_value(serde_json::json!({
        "displayName": "G",
        "visual": { "kind": "image", "asset": "66666666-6666-6666-6666-666666666666" },
        "size": { "w": 1.0, "h": 1.0 },
        "shape": "square",
        "faction": null,
        "conditions": [],
        "prototype": false,
    }))
    .expect("absent emissions default to None");
    assert!(clean.validate().is_ok());
    assert_eq!(clean.aura, None);
    assert_eq!(clean.sound, None);
    assert_eq!(clean.vfx, None);
}

#[test]
fn token_validate_covers_override_emissions() {
    let mut t = base();
    t.overrides = Some(TokenOverrides {
        name: None,
        visual: None,
        size: None,
        shape: None,
        vision: None,
        facing_vision: None,
        light: None,
        movement: None,
        aura: Some(AuraEmission {
            color: "nope".to_string(),
            opacity: 0.4,
            radius: 2.0,
            enabled: true,
        }),
        sound: None,
        vfx: None,
        disposition: None,
        bars: None,
        nameplate_mode: None,
    });
    assert!(
        t.validate().is_err(),
        "a malformed override emission fails the token"
    );
    t.overrides = Some(TokenOverrides {
        name: None,
        visual: None,
        size: None,
        shape: None,
        vision: None,
        facing_vision: None,
        light: None,
        movement: None,
        aura: Some(aura()),
        sound: Some(sound()),
        vfx: Some(vfx()),
        disposition: Some(FactionStance::Hostile),
        bars: None,
        nameplate_mode: None,
    });
    assert!(t.validate().is_ok());
}

#[test]
fn emission_serde_names_match_the_client_contract() {
    // `loop_` serializes under the reserved-name-friendly `loop` key, and the
    // anchor under snake_case variants — the client's generated bindings read
    // these exact keys.
    let s = serde_json::to_value(sound()).unwrap();
    assert!(s.get("loop").is_some());
    assert!(s.get("loop_").is_none());
    let v = serde_json::to_value(VfxEmission {
        anchor: VfxAnchor::Above,
        ..vfx()
    })
    .unwrap();
    assert_eq!(v.get("anchor").unwrap(), "above");
}

fn pool_with(fallback: TokenVisual) -> TokenVisual {
    TokenVisual::RandomVariant {
        filter: TokenArtFilter::default(),
        fallback: Box::new(fallback),
    }
}

fn plain_image() -> TokenVisual {
    TokenVisual::Image {
        asset: Uuid::nil().to_string(),
        transform: None,
        drawn: false,
    }
}

#[test]
fn a_random_variant_with_a_concrete_fallback_validates() {
    assert!(pool_with(plain_image()).validate().is_ok());
}

#[test]
fn a_random_variant_whose_fallback_is_a_pool_is_rejected() {
    let nested = pool_with(pool_with(plain_image()));
    assert!(nested.validate().is_err());
}

#[test]
fn a_random_variant_validates_its_fallback() {
    let bad = pool_with(TokenVisual::Image {
        asset: "not-a-uuid".to_string(),
        transform: None,
        drawn: false,
    });
    assert!(bad.validate().is_err());
}

#[test]
fn a_random_variant_wire_shape_is_camel_case_tagged() {
    let json = serde_json::to_value(pool_with(plain_image())).unwrap();
    assert_eq!(json["kind"], "randomVariant");
    assert_eq!(json["filter"]["tags"], serde_json::json!([]));
    assert_eq!(json["fallback"]["kind"], "image");
    let back: TokenVisual = serde_json::from_value(json).unwrap();
    assert_eq!(back, pool_with(plain_image()));
}
