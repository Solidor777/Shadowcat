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
fn merge_base_deserialization_rejects_a_record_missing_a_required_band() {
    // Ingest (`check_base_node_shape`) requires every band present; the read
    // path must fail the same way rather than coalescing an absent key into
    // `null`/empty, which would read a malformed stored base as an ordinary
    // one.
    let err =
        serde_json::from_value::<crate::merge::bands::MergeBase>(json!({ "system": { "hp": 1 } }))
            .expect_err("a record missing name/engine/embedded/property_overrides does not parse");
    assert!(err.to_string().contains("missing field"), "got {err}");
}

#[test]
fn merge_base_deserialization_rejects_a_record_missing_property_overrides() {
    // The full band set present, `property_overrides` alone absent: still a
    // shape violation, not a legitimate row to tolerate.
    let err = serde_json::from_value::<crate::merge::bands::MergeBase>(json!({
        "name": null,
        "engine": null,
        "system": null,
        "embedded": {}
    }))
    .expect_err("a record missing property_overrides does not parse");
    assert!(err.to_string().contains("property_overrides"), "got {err}");
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

    assert!(propagate_overrides(&mut instance, &template));
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
    assert!(!propagate_overrides(&mut instance, &template), "idempotent");
}

#[test]
fn derive_create_base_propagates_the_template_policy_and_records_the_snapshot_policy() {
    use crate::data::document::{OwnerStanding, Visibility};
    use crate::merge::bands::derive_create_base;

    let mut template = doc("t1");
    template
        .permissions
        .property_overrides
        .insert("/system/secret".to_string(), Visibility::GmOnly);
    let mut instance = doc("c1");
    instance.source = Some(source_from("t1"));
    instance.system = json!({ "hp": 1 });

    derive_create_base(&mut instance, Some(&template), OwnerStanding::Owner);
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
    assert_eq!(base["owner_standing"], json!("owner"));

    // No loadable template: nothing to propagate; the snapshot records the
    // document's own policy (empty here), with the caller-supplied standing
    // (`Stranger`, since `apply_intent`'s Create arm has no template to
    // resolve a standing against).
    let mut orphan = doc("c2");
    orphan.source = Some(source_from("t-missing"));
    derive_create_base(&mut orphan, None, OwnerStanding::Stranger);
    assert!(orphan.permissions.property_overrides.is_empty());
    assert_eq!(
        orphan.base.clone().unwrap()["property_overrides"],
        json!({})
    );
    assert_eq!(orphan.base.unwrap()["owner_standing"], json!("stranger"));
}

#[test]
fn propagate_overrides_carries_every_tier_verbatim_and_never_re_tightens_an_instance_entry() {
    use crate::data::document::Visibility;
    use crate::merge::bands::propagate_overrides;

    let mut kid = doc("tc1");
    kid.permissions
        .property_overrides
        .insert("/engine/hp".to_string(), Visibility::OwnerOrGm);
    let mut template = doc("t1");
    template.permissions.property_overrides.extend([
        ("/system/note".to_string(), Visibility::OwnerOrGm),
        ("/system/secret".to_string(), Visibility::GmOnly),
    ]);
    template.embedded.insert("items".to_string(), vec![kid]);

    // An instance with another owner: the template's `OwnerOrGm` lands
    // VERBATIM (naming the instance's own owner there), at every depth.
    let mut instance = doc("c1");
    instance.owner = Some(super::test_id("someone-else"));
    let mut i_kid = doc("ic1");
    i_kid.source = Some(source_from("tc1"));
    instance.embedded.insert("items".to_string(), vec![i_kid]);
    assert!(propagate_overrides(&mut instance, &template));
    assert_eq!(
        instance.permissions.property_overrides["/system/note"],
        Visibility::OwnerOrGm,
        "an owner-or-GM tier is carried as is across an ownership boundary"
    );
    assert_eq!(
        instance.embedded["items"][0].permissions.property_overrides["/engine/hp"],
        Visibility::OwnerOrGm,
        "verbatim at every depth"
    );

    // A tier a GM deliberately loosened on the instance stands: the
    // template's stricter tier lands only where the instance holds no entry.
    let mut loosened = doc("c2");
    loosened
        .permissions
        .property_overrides
        .insert("/system/secret".to_string(), Visibility::All);
    assert!(propagate_overrides(&mut loosened, &template));
    assert_eq!(
        loosened.permissions.property_overrides["/system/secret"],
        Visibility::All,
        "never re-tightened"
    );
    assert_eq!(
        loosened.permissions.property_overrides["/system/note"],
        Visibility::OwnerOrGm,
        "the template's tier lands where the instance has none"
    );
    assert!(
        !propagate_overrides(&mut loosened, &template),
        "idempotent: a second pass changes nothing"
    );
}

#[test]
fn propagate_overrides_skips_a_no_op_all_tier_at_root_and_on_embedded_records() {
    use crate::data::document::Visibility;
    use crate::merge::bands::propagate_overrides;

    // A no-op `All` audience recorded on the template's root AND on one of
    // its embedded children — propagating either would write
    // `/permissions/property_overrides` (root) or force a whole
    // `/embedded/<coll>` collection rewrite (record) for zero visibility
    // effect, needlessly demanding a capability the change accomplishes
    // nothing to justify.
    let mut kid = doc("tc1");
    kid.permissions
        .property_overrides
        .insert("/engine/hp".to_string(), Visibility::All);
    let mut template = doc("t1");
    template
        .permissions
        .property_overrides
        .insert("/system/note".to_string(), Visibility::All);
    template.embedded.insert("items".to_string(), vec![kid]);

    let mut instance = doc("c1");
    let mut i_kid = doc("ic1");
    i_kid.source = Some(source_from("tc1"));
    instance.embedded.insert("items".to_string(), vec![i_kid]);

    assert!(
        !propagate_overrides(&mut instance, &template),
        "an All-only template policy propagates nothing"
    );
    assert!(
        instance.permissions.property_overrides.is_empty(),
        "the no-op root entry is never written onto the instance"
    );
    assert!(
        instance.embedded["items"][0]
            .permissions
            .property_overrides
            .is_empty(),
        "the no-op record entry is never written onto the embedded child"
    );

    // A real hidden tier alongside the no-op one still propagates.
    template
        .permissions
        .property_overrides
        .insert("/system/secret".to_string(), Visibility::GmOnly);
    assert!(propagate_overrides(&mut instance, &template));
    assert_eq!(
        instance.permissions.property_overrides.len(),
        1,
        "only the real tier lands: {:?}",
        instance.permissions.property_overrides
    );
    assert_eq!(
        instance.permissions.property_overrides["/system/secret"],
        Visibility::GmOnly
    );
}
