use super::*;

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
    }
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
        asset: "a1".to_string(),
        radius: 5.0,
        volume: 0.8,
        loop_: true,
        enabled: true,
    }
}

fn vfx() -> VfxEmission {
    VfxEmission {
        asset: "a2".to_string(),
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
        "visual": { "kind": "image", "asset": "img" },
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
        "visual": { "kind": "image", "asset": "img" },
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
        light: None,
        movement: None,
        aura: Some(aura()),
        sound: Some(sound()),
        vfx: Some(vfx()),
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
