//! Unit tests for the band types, snapshot builder, and placement
//! exclusions.

use serde_json::json;

use crate::merge::bands::{is_placement_excluded, placement_exclusions, snapshot_base};

use super::{doc, source_from};

#[test]
fn placement_exclusions_only_cover_token_placement() {
    assert_eq!(
        placement_exclusions("token"),
        ["/engine/x", "/engine/y", "/engine/rotation"]
    );
    assert!(placement_exclusions("actor").is_empty());
}

#[test]
fn is_placement_excluded_matches_exact_and_descendant_paths_only() {
    let excl = placement_exclusions("token");
    assert!(is_placement_excluded("/engine/x", &excl));
    assert!(is_placement_excluded("/engine/x/deep", &excl));
    assert!(!is_placement_excluded("/engine/xylophone", &excl));
    assert!(!is_placement_excluded("/engine/hp", &excl));
}

#[test]
fn snapshot_base_keys_children_by_source_id_and_coalesces_absent_bands() {
    let mut kid = doc("ic1");
    kid.name = Some("Kid".to_string());
    kid.source = Some(source_from("tc1"));
    kid.system = json!({ "hp": 3 });
    let mut d = doc("c1");
    d.name = Some("Inst".to_string());
    d.engine = Some(json!({ "hp": 9 }));
    d.system = json!({ "a": 1 });
    d.embedded.insert("items".to_string(), vec![kid]);

    let snap = snapshot_base(&d);
    assert_eq!(snap.name.as_deref(), Some("Inst"));
    assert_eq!(snap.engine, json!({ "hp": 9 }));
    assert_eq!(snap.system, json!({ "a": 1 }));
    let kids = &snap.embedded["items"];
    assert_eq!(kids.len(), 1);
    assert_eq!(kids[0].source_id, super::test_id("tc1").to_string());
    assert_eq!(kids[0].name.as_deref(), Some("Kid"));
    assert_eq!(kids[0].engine, json!(null));
    assert_eq!(kids[0].system, json!({ "hp": 3 }));
}

#[test]
fn snapshot_base_falls_back_to_own_id_for_non_provenance_children() {
    let kid = doc("local1");
    let mut d = doc("c1");
    d.embedded.insert("items".to_string(), vec![kid]);
    let snap = snapshot_base(&d);
    assert_eq!(
        snap.embedded["items"][0].source_id,
        super::test_id("local1").to_string()
    );
}

#[test]
fn merge_base_deserialization_defaults_missing_bands() {
    // A partial historical record reads with `null`/empty bands, matching the
    // client engine's `?? null` coalescing.
    let parsed: crate::merge::bands::MergeBase =
        serde_json::from_value(json!({ "system": { "hp": 1 } })).expect("partial base parses");
    assert_eq!(parsed.name, None);
    assert_eq!(parsed.engine, json!(null));
    assert_eq!(parsed.system, json!({ "hp": 1 }));
    assert!(parsed.embedded.is_empty());
}

#[test]
fn snapshot_base_records_the_mergeable_band_policy_at_every_depth() {
    use crate::data::document::Visibility;
    let mut kid = doc("ic1");
    kid.source = Some(source_from("tc1"));
    kid.permissions
        .property_overrides
        .insert("/engine/hp".to_string(), Visibility::OwnerOrGm);
    let mut d = doc("c1");
    d.permissions
        .property_overrides
        .insert("/system/secret".to_string(), Visibility::GmOnly);
    d.permissions
        .property_overrides
        .insert("/name".to_string(), Visibility::OwnerOrGm);
    // A policy over this document's OWN snapshot of some other template says
    // nothing about its bands: never recorded.
    d.permissions
        .property_overrides
        .insert("/base/system/x".to_string(), Visibility::GmOnly);
    d.embedded.insert("items".to_string(), vec![kid]);

    let snap = snapshot_base(&d);
    assert_eq!(
        snap.property_overrides,
        [
            ("/name".to_string(), Visibility::OwnerOrGm),
            ("/system/secret".to_string(), Visibility::GmOnly),
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(
        snap.embedded["items"][0].property_overrides,
        [("/engine/hp".to_string(), Visibility::OwnerOrGm)]
            .into_iter()
            .collect()
    );
    let wire = serde_json::to_value(&snap).unwrap();
    assert!(wire["property_overrides"].is_object());
    assert!(wire["embedded"]["items"][0]["propertyOverrides"].is_object());
}

#[test]
fn propagate_overrides_is_additive_keeps_the_stricter_tier_and_correlates_children_by_identity() {
    use crate::data::document::Visibility;
    use crate::merge::bands::propagate_overrides;

    let mut template = doc("t1");
    template.permissions.property_overrides.extend([
        ("/system/secret".to_string(), Visibility::GmOnly),
        ("/system/note".to_string(), Visibility::OwnerOrGm),
        ("/system/loosened".to_string(), Visibility::All),
        ("/base/system/x".to_string(), Visibility::GmOnly),
    ]);
    let mut t_kid = doc("tc1");
    t_kid
        .permissions
        .property_overrides
        .insert("/engine/hp".to_string(), Visibility::GmOnly);
    let mut t_other = doc("tc2");
    t_other
        .permissions
        .property_overrides
        .insert("/system/z".to_string(), Visibility::GmOnly);
    template
        .embedded
        .insert("items".to_string(), vec![t_other, t_kid]);

    let mut instance = doc("c1");
    instance.permissions.property_overrides.extend([
        ("/system/note".to_string(), Visibility::GmOnly),
        ("/system/loosened".to_string(), Visibility::OwnerOrGm),
        ("/system/mine".to_string(), Visibility::OwnerOrGm),
    ]);
    // The instance's ONLY child is stamped from `tc1`, sitting at index 0
    // where the template has `tc2`: correlation is by `source.id`.
    let mut i_kid = doc("ic1");
    i_kid.source = Some(source_from("tc1"));
    instance.embedded.insert("items".to_string(), vec![i_kid]);

    assert!(propagate_overrides(&mut instance, &template, true));
    let p = &instance.permissions.property_overrides;
    assert_eq!(p["/system/secret"], Visibility::GmOnly, "added");
    assert_eq!(
        p["/system/note"],
        Visibility::GmOnly,
        "the instance's stricter tier stands"
    );
    assert_eq!(
        p["/system/loosened"],
        Visibility::OwnerOrGm,
        "never widened to the template's `All`"
    );
    assert_eq!(
        p["/system/mine"],
        Visibility::OwnerOrGm,
        "instance-authored entries survive"
    );
    assert!(
        !p.contains_key("/base/system/x"),
        "a /base policy is not a band policy"
    );
    let kp = &instance.embedded["items"][0].permissions.property_overrides;
    assert_eq!(
        kp["/engine/hp"],
        Visibility::GmOnly,
        "the correlated template child's policy"
    );
    assert!(
        !kp.contains_key("/system/z"),
        "not the positional neighbour's"
    );
    assert!(
        !propagate_overrides(&mut instance, &template, true),
        "idempotent"
    );
}

#[test]
fn derive_create_base_propagates_the_template_policy_and_records_the_snapshot_policy() {
    use crate::data::document::Visibility;
    use crate::merge::bands::derive_create_base;

    let mut template = doc("t1");
    template
        .permissions
        .property_overrides
        .insert("/system/secret".to_string(), Visibility::GmOnly);
    let mut instance = doc("c1");
    instance.source = Some(source_from("t1"));
    instance.system = json!({ "hp": 1 });

    derive_create_base(&mut instance, Some(&template), true);
    assert_eq!(
        instance.permissions.property_overrides["/system/secret"],
        Visibility::GmOnly
    );
    let base = instance.base.expect("an instance derives a base");
    assert_eq!(
        base["property_overrides"]["/system/secret"],
        json!("gm_only")
    );
    assert_eq!(base["system"], json!({ "hp": 1 }));

    // No loadable template: nothing to propagate; the snapshot records the
    // document's own policy (empty here).
    let mut orphan = doc("c2");
    orphan.source = Some(source_from("t-missing"));
    derive_create_base(&mut orphan, None, true);
    assert!(orphan.permissions.property_overrides.is_empty());
    assert_eq!(orphan.base.unwrap()["property_overrides"], json!({}));
}

#[test]
fn relate_tier_re_expresses_owner_or_gm_across_an_ownership_boundary_only() {
    use crate::data::document::Visibility;
    use crate::merge::bands::{relate_tier, snapshot_for_instance};
    for tier in [Visibility::All, Visibility::GmOnly, Visibility::OwnerOrGm] {
        assert_eq!(
            relate_tier(tier, true),
            tier,
            "same owner: every tier stands"
        );
    }
    assert_eq!(relate_tier(Visibility::All, false), Visibility::All);
    assert_eq!(relate_tier(Visibility::GmOnly, false), Visibility::GmOnly);
    assert_eq!(
        relate_tier(Visibility::OwnerOrGm, false),
        Visibility::GmOnly,
        "another owner's private value is nobody's private value here"
    );

    let mut kid = doc("tc1");
    kid.permissions
        .property_overrides
        .insert("/engine/hp".to_string(), Visibility::OwnerOrGm);
    let mut template = doc("t1");
    template
        .permissions
        .property_overrides
        .insert("/system/note".to_string(), Visibility::OwnerOrGm);
    template.embedded.insert("items".to_string(), vec![kid]);
    let same = snapshot_for_instance(&template, true);
    assert_eq!(
        same.property_overrides["/system/note"],
        Visibility::OwnerOrGm
    );
    assert_eq!(
        same.embedded["items"][0].property_overrides["/engine/hp"],
        Visibility::OwnerOrGm
    );
    let other = snapshot_for_instance(&template, false);
    assert_eq!(other.property_overrides["/system/note"], Visibility::GmOnly);
    assert_eq!(
        other.embedded["items"][0].property_overrides["/engine/hp"],
        Visibility::GmOnly,
        "re-expressed at every depth"
    );
    // A propagated tier follows the same relation.
    let mut instance = doc("c1");
    assert!(crate::merge::bands::propagate_overrides(
        &mut instance,
        &template,
        false
    ));
    assert_eq!(
        instance.permissions.property_overrides["/system/note"],
        Visibility::GmOnly
    );
}
