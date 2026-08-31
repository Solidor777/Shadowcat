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
