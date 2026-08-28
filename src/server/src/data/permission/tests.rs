use super::*;
use crate::data::document::{world_of, PermissionSet, Scope};
use crate::data::snapshot::{CommandSnapshot, OpSnapshot};

fn doc(perms: PermissionSet, system: serde_json::Value) -> Document {
    Document {
        id: Uuid::from_u128(1),
        scope: Scope::World {
            world_id: Uuid::from_u128(9),
        },
        doc_type: "actor".into(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: None,
        permissions: perms,
        embedded: Default::default(),
        parent_id: None,
        // "actor" is engine-defined; a minimal valid body so `Create`
        // clears the ingress gate. These tests exercise `/system`
        // redaction only, unrelated to `engine`'s content.
        engine: crate::data::document::tests::default_test_engine("actor"),
        system,
        created_at: 0,
        updated_at: 0,
    }
}

fn perms_with(overrides: &[(&str, Visibility)]) -> PermissionSet {
    let mut p = PermissionSet::default();
    for (ptr, v) in overrides {
        p.property_overrides.insert((*ptr).into(), *v);
    }
    p
}

/// Build a `PermissionSet` carrying one override at `pointer`, hidden from non-GMs.
fn perms_with_override(pointer: &str) -> PermissionSet {
    let mut p = PermissionSet {
        default: crate::data::document::DocRole::Observer,
        ..Default::default()
    };
    p.property_overrides.insert(
        pointer.to_string(),
        crate::data::document::Visibility::GmOnly,
    );
    p
}

fn non_gm() -> Access {
    Access {
        caps: Default::default(),
        all: false,
        see_gm_only: false,
        is_owner: false,
    }
}

/// A `CommandSnapshot` for `cmd` whose commit-time state exactly mirrors `current` (and, for
/// `Create`/`Delete`, the op's own carried `doc`) — for a pre-existing test that writes and
/// redacts at the SAME instant, so the commit-time and current-time halves agree by
/// construction and their union changes nothing observable. `gm_at_commit` lists every
/// recipient who holds GM standing (both at commit and now, in these single-instant tests).
fn immediate_snapshot<'a>(
    cmd: &Command,
    current: &HashMap<Uuid, CurrentDoc>,
    gm_at_commit: &[Uuid],
    actor_lookup: &impl Fn(&Uuid) -> Option<&'a Document>,
) -> CommandSnapshot {
    let mut per_op = Vec::with_capacity(cmd.ops.len());
    for op in &cmd.ops {
        let target_doc: Option<&Document> = match op {
            Operation::Update { doc_id, .. } => current.get(doc_id).map(|c| &c.doc),
            Operation::Create { doc } => Some(doc),
            Operation::Delete { doc } => Some(doc),
        };
        let Some(d) = target_doc else {
            per_op.push(None);
            continue;
        };
        // A test poisoning a document with an unclassifiable override (to exercise
        // `filter_properties`'s own fail-closed path) makes this best-effort: the partial
        // set collected before the error is used as-is, matching what a real snapshot
        // builder would have to do (this classifier never runs against genuinely
        // unclassifiable data outside a deliberately-poisoned test fixture).
        let mut overrides = Vec::new();
        let _ = collect_overrides(d, "", &mut overrides);
        let touches_perms = matches!(
            op,
            Operation::Update { changes, .. }
                if changes.iter().any(|c| touches_permissions(&c.path))
        );
        per_op.push(Some(OpSnapshot {
            owner_at_commit: effective_owner_via(d, actor_lookup),
            doc_type: d.doc_type.clone(),
            overrides_at_commit: overrides.clone(),
            retraction_hidden_at_commit: if touches_perms { Some(overrides) } else { None },
            created_seq_at_commit: None,
            permissions_at_commit: Some(PermissionSet {
                property_overrides: Default::default(),
                ..d.permissions.clone()
            }),
            // No before-image in this single-instant helper; `snapshot_with_before`
            // overrides this for tests exercising the READ-transition rule.
            permissions_before_commit: None,
        }));
    }
    CommandSnapshot {
        per_op,
        world_gm_at_commit: gm_at_commit.iter().map(|u| (*u, true)).collect(),
    }
}

#[test]
fn filter_properties_errors_instead_of_panicking_on_a_nested_permissions_override() {
    // A nested `/permissions/...` override strips a `PermissionSet` field carrying
    // no serde default, so the value cannot re-deserialize.
    let d = doc(
        perms_with_override("/permissions/default"),
        serde_json::json!({ "hp": 1 }),
    );
    let err = filter_properties(&d, &non_gm()).expect_err("must not deserialize");
    assert_eq!(err.pointer, "/permissions/default");
}

#[test]
fn filter_properties_errors_on_a_whole_permissions_override() {
    // A whole `/permissions` override is refused as unclassifiable rather than
    // substituting the fail-closed default permission set for the real one: that
    // substitution does not panic, it ships a wrong document.
    let d = doc(
        perms_with_override("/permissions"),
        serde_json::json!({ "hp": 1 }),
    );
    assert!(filter_properties(&d, &non_gm()).is_err());
}

#[test]
fn filter_properties_still_redacts_every_content_band() {
    for (pointer, check) in [
        ("/system/secret", "system"),
        ("/engine", "engine"),
        ("/name", "name"),
    ] {
        let mut d = doc(
            perms_with_override(pointer),
            serde_json::json!({ "secret": "MOCK_SECRET_A", "public": 1 }),
        );
        // A real name, so the "name" sub-case discriminates: `doc()` always
        // constructs `name: None`, and asserting `None` after redaction would
        // pass even if `/name` redaction never ran.
        d.name = Some("MOCK_NAME_A".into());
        let out = filter_properties(&d, &non_gm())
            .unwrap_or_else(|e| panic!("{pointer} must still redact cleanly: {e}"));
        match check {
            "system" => {
                assert!(out.system.get("secret").is_none());
                assert_eq!(out.system["public"], 1);
            }
            "engine" => assert!(out.engine.is_none()),
            "name" => assert!(out.name.is_none()),
            _ => unreachable!(),
        }
    }
}

#[test]
fn a_gm_recipient_is_unaffected_by_an_unclassifiable_override() {
    // The GM short-circuit returns before any classification runs, so a GM never
    // loses a document to a poisoned override.
    let d = doc(
        perms_with_override("/permissions/default"),
        serde_json::json!({ "hp": 1 }),
    );
    let gm = Access {
        caps: Default::default(),
        all: true,
        see_gm_only: true,
        is_owner: false,
    };
    assert!(filter_properties(&d, &gm).is_ok());
}

#[test]
fn owner_or_gm_visible_to_owner_and_gm_not_other_player() {
    let owner = Uuid::from_u128(1);
    let other = Uuid::from_u128(2);
    let mut d = doc(
        perms_with(&[("/system/name", Visibility::OwnerOrGm)]),
        serde_json::json!({ "name": "Goblin Skirmisher", "displayName": "Goblin" }),
    );
    d.owner = Some(owner);

    // Owner (non-GM) sees the real name.
    let a_owner = resolve_access(owner, WorldRole::Player, &d, d.owner);
    assert_eq!(
        filter_properties(&d, &a_owner).unwrap().system["name"],
        "Goblin Skirmisher"
    );

    // Another player does NOT (falls back to the non-secret displayName).
    let a_other = resolve_access(other, WorldRole::Player, &d, d.owner);
    let v_other = filter_properties(&d, &a_other).unwrap();
    assert!(v_other.system.get("name").is_none());
    assert_eq!(v_other.system["displayName"], "Goblin");

    // GM sees it.
    let a_gm = resolve_access(other, WorldRole::Gm, &d, d.owner);
    assert_eq!(
        filter_properties(&d, &a_gm).unwrap().system["name"],
        "Goblin Skirmisher"
    );
}

#[test]
fn owner_cannot_see_gm_only() {
    let owner = Uuid::from_u128(1);
    let mut d = doc(
        perms_with(&[
            ("/system/name", Visibility::OwnerOrGm),
            ("/system/secret", Visibility::GmOnly),
        ]),
        serde_json::json!({ "name": "PC", "secret": "GM note" }),
    );
    d.owner = Some(owner);

    let a_owner = resolve_access(owner, WorldRole::Player, &d, d.owner);
    let v = filter_properties(&d, &a_owner).unwrap();
    assert_eq!(v.system["name"], "PC"); // owner sees OwnerOrGm
    assert!(v.system.get("secret").is_none()); // owner still denied GmOnly
}

#[test]
fn embedded_owner_or_gm_redacted_for_non_owner() {
    let owner = Uuid::from_u128(1);
    let other = Uuid::from_u128(2);
    let child = doc(
        perms_with(&[("/system/name", Visibility::OwnerOrGm)]),
        serde_json::json!({ "name": "Hidden", "displayName": "Thing" }),
    );
    let mut parent = doc(PermissionSet::default(), serde_json::json!({}));
    parent.owner = Some(owner);
    parent.embedded.insert("actor".into(), vec![child]);

    let a_other = resolve_access(other, WorldRole::Player, &parent, parent.owner);
    let v = filter_properties(&parent, &a_other).unwrap();
    assert!(v.embedded["actor"][0].system.get("name").is_none());

    let a_owner = resolve_access(owner, WorldRole::Player, &parent, parent.owner);
    let vo = filter_properties(&parent, &a_owner).unwrap();
    assert_eq!(vo.embedded["actor"][0].system["name"], "Hidden");
}

#[test]
fn declared_caps_match_prefix_on_boundaries() {
    let reqs = vec![CapabilityRequirement {
        path_prefix: "/system/vision".into(),
        caps: ["dnd5e:gm_vision".to_string()].into_iter().collect(),
    }];
    // exact and descendant match
    assert_eq!(
        declared_caps_for_path("/system/vision", &reqs),
        vec!["dnd5e:gm_vision"]
    );
    assert_eq!(
        declared_caps_for_path("/system/vision/range", &reqs),
        vec!["dnd5e:gm_vision"]
    );
    // sibling that merely shares a string prefix does NOT match (boundary check)
    assert!(declared_caps_for_path("/system/visionmode", &reqs).is_empty());
    // unrelated path
    assert!(declared_caps_for_path("/system/hp", &reqs).is_empty());
    // ANCESTOR write that covers the protected subtree DOES match (a coarse
    // `/system` write replaces `/system/vision` wholesale).
    assert_eq!(
        declared_caps_for_path("/system", &reqs),
        vec!["dnd5e:gm_vision"]
    );
}

#[test]
fn declared_caps_for_document_matches_present_paths() {
    let reqs = vec![CapabilityRequirement {
        path_prefix: "/system/vision".into(),
        caps: ["dnd5e:gm_vision".to_string()].into_iter().collect(),
    }];
    // body with a populated /system/vision subtree → requirement applies
    let with = serde_json::json!({ "system": { "vision": { "range": 30 }, "hp": 10 } });
    assert_eq!(
        declared_caps_for_document(&with, &reqs),
        vec!["dnd5e:gm_vision"]
    );
    // body without the protected path → no requirement
    let without = serde_json::json!({ "system": { "hp": 10 } });
    assert!(declared_caps_for_document(&without, &reqs).is_empty());
}

#[test]
fn project_grants_drops_other_users() {
    use crate::data::document::CapabilityGrants;
    let me = Uuid::from_u128(1);
    let other = Uuid::from_u128(2);
    let mut grants = CapabilityGrants::default();
    grants
        .by_role
        .entry(DocRole::Owner)
        .or_default()
        .insert("core:manage_embedded".to_string());
    grants
        .by_user
        .entry(me)
        .or_default()
        .insert("dnd5e:cast".to_string());
    grants
        .by_user
        .entry(other)
        .or_default()
        .insert("dnd5e:secret".to_string());

    let projected = project_grants_for(&grants, me);
    // Role tiers are world policy — preserved.
    assert_eq!(projected.by_role, grants.by_role);
    // Only this user's own per-user grant survives; the other user's UUID
    // and grants are gone.
    assert!(projected.by_user.contains_key(&me));
    assert!(!projected.by_user.contains_key(&other));
    assert_eq!(projected.by_user.len(), 1);
}

#[test]
fn gm_holds_every_capability() {
    let a = resolve_access(
        Uuid::from_u128(5),
        WorldRole::Gm,
        &doc(Default::default(), serde_json::json!({})),
        None,
    );
    assert!(a.all && a.see_gm_only);
    assert!(a.has(cap::WRITE_FIELDS) && a.has(cap::MANAGE_EMBEDDED) && a.has("dnd5e:anything"));
}

#[test]
fn floor_grants_by_role() {
    let mut perms = PermissionSet::default();
    perms.users.insert(Uuid::from_u128(1), DocRole::Owner);
    perms.users.insert(Uuid::from_u128(2), DocRole::Observer);
    let d = doc(perms, serde_json::json!({}));
    // Owner: read + write fields, but NOT manage embedded by default.
    let owner = resolve_access(Uuid::from_u128(1), WorldRole::Player, &d, d.owner);
    assert!(owner.has(cap::READ) && owner.has(cap::WRITE_FIELDS));
    assert!(!owner.has(cap::MANAGE_EMBEDDED) && !owner.has(cap::DELETE));
    // Observer: read only.
    let obs = resolve_access(Uuid::from_u128(2), WorldRole::Player, &d, d.owner);
    assert!(obs.has(cap::READ) && !obs.has(cap::WRITE_FIELDS));
    // Stranger falls to default (None): nothing.
    let other = resolve_access(Uuid::from_u128(3), WorldRole::Player, &d, d.owner);
    assert!(!other.has(cap::READ));
}

#[test]
fn additive_grants_widen_the_floor() {
    use crate::data::document::CapabilityGrants;
    let mut perms = PermissionSet::default();
    perms.users.insert(Uuid::from_u128(1), DocRole::Owner);
    let mut grants = CapabilityGrants::default();
    // Grant Owners on this doc the ability to manage embedded documents.
    grants
        .by_role
        .entry(DocRole::Owner)
        .or_default()
        .insert(cap::MANAGE_EMBEDDED.to_string());
    // Grant a specific user a custom module capability.
    grants
        .by_user
        .entry(Uuid::from_u128(1))
        .or_default()
        .insert("dnd5e:cast".to_string());
    perms.capabilities = grants;
    let d = doc(perms, serde_json::json!({}));
    let a = resolve_access(Uuid::from_u128(1), WorldRole::Player, &d, d.owner);
    assert!(a.has(cap::WRITE_FIELDS)); // floor retained
    assert!(a.has(cap::MANAGE_EMBEDDED)); // role grant
    assert!(a.has("dnd5e:cast")); // user grant
    assert!(!a.has(cap::DELETE)); // not granted
}

#[test]
fn gm_only_property_is_stripped_for_non_gm() {
    let mut perms = PermissionSet {
        default: DocRole::Observer,
        ..Default::default()
    };
    perms
        .property_overrides
        .insert("/system/secret".into(), Visibility::GmOnly);
    let d = doc(perms, serde_json::json!({ "secret": 42, "public": 1 }));

    let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &d, d.owner);
    let view = filter_properties(&d, &player).unwrap();
    assert_eq!(view.system.get("secret"), None);
    assert_eq!(view.system["public"], serde_json::json!(1));

    let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &d, d.owner);
    assert_eq!(
        filter_properties(&d, &gm).unwrap().system["secret"],
        serde_json::json!(42)
    );
}

#[test]
fn whole_system_gm_only_nulls_rather_than_drops_the_required_field() {
    // A doc type (e.g. a secret region) may mark its ENTIRE `/system` body GmOnly,
    // not just a leaf field. `system` is a required `Document` field, so stripping
    // the key outright would make the redacted JSON fail to deserialize back into a
    // `Document` — it must be nulled instead, never dropped.
    let mut perms = PermissionSet {
        default: DocRole::Observer,
        ..Default::default()
    };
    perms
        .property_overrides
        .insert("/system".into(), Visibility::GmOnly);
    let d = doc(perms, serde_json::json!({ "secret": 42 }));

    let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &d, d.owner);
    let view = filter_properties(&d, &player).unwrap();
    assert_eq!(view.system, serde_json::Value::Null);

    let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &d, d.owner);
    assert_eq!(
        filter_properties(&d, &gm).unwrap().system["secret"],
        serde_json::json!(42)
    );
}

#[test]
fn whole_engine_gm_only_nulls_rather_than_drops_the_field() {
    // `/engine` is an `Option<Value>` envelope field — nulling it under a
    // whole-band GmOnly override must round-trip exactly like `None`, not
    // strip the key outright (which would be indistinguishable from a doc
    // that carries no `engine` band at all, but is still safe to
    // deserialize either way since the field is optional).
    let mut perms = PermissionSet {
        default: DocRole::Observer,
        ..Default::default()
    };
    perms
        .property_overrides
        .insert("/engine".into(), Visibility::GmOnly);
    let mut d = doc(perms, serde_json::json!({}));
    d.engine = Some(serde_json::json!({ "x": 1.0, "y": 2.0 }));

    let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &d, d.owner);
    let view = filter_properties(&d, &player).unwrap();
    assert_eq!(view.engine, None);

    let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &d, d.owner);
    assert_eq!(
        filter_properties(&d, &gm).unwrap().engine,
        Some(serde_json::json!({ "x": 1.0, "y": 2.0 }))
    );
}

#[test]
fn engine_leaf_gm_only_hides_the_leaf_but_not_a_boundary_neighbor() {
    // Boundary matching inside `/engine` must behave exactly like inside
    // `/system`: `/engine/vision` hides only that key, leaving a
    // string-prefixed sibling (`visionmode`) untouched.
    let mut perms = PermissionSet {
        default: DocRole::Observer,
        ..Default::default()
    };
    perms
        .property_overrides
        .insert("/engine/vision".into(), Visibility::GmOnly);
    let mut d = doc(perms, serde_json::json!({}));
    d.engine = Some(serde_json::json!({ "vision": 30, "visionmode": "dark" }));

    let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &d, d.owner);
    let view = filter_properties(&d, &player).unwrap();
    assert!(view.engine.as_ref().unwrap().get("vision").is_none());
    assert_eq!(view.engine.as_ref().unwrap()["visionmode"], "dark");

    let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &d, d.owner);
    assert_eq!(
        filter_properties(&d, &gm).unwrap().engine.unwrap()["vision"],
        30
    );
}

#[test]
fn gm_only_array_element_is_nulled_in_place_for_non_gm() {
    // An override may name an ARRAY element inside a band (`/system/inventory/0`);
    // the classifier accepts it, so egress must actually redact it. The element is
    // nulled, never removed: removal shifts every later index, and an array shrinks
    // only by whole-array replacement (`remove_pointer` refuses index removal for the
    // same reason). Length and sibling positions are therefore part of the contract.
    let mut perms = PermissionSet {
        default: DocRole::Observer,
        ..Default::default()
    };
    perms
        .property_overrides
        .insert("/system/inventory/0".into(), Visibility::GmOnly);
    let d = doc(
        perms,
        serde_json::json!({ "inventory": ["MOCK_SECRET_A", "visible"] }),
    );

    let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &d, d.owner);
    let view = filter_properties(&d, &player).unwrap();
    assert_eq!(
        view.system["inventory"],
        serde_json::json!([null, "visible"]),
        "the hidden element must be nulled without shifting its siblings"
    );

    let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &d, d.owner);
    assert_eq!(
        filter_properties(&d, &gm).unwrap().system["inventory"],
        serde_json::json!(["MOCK_SECRET_A", "visible"])
    );
}

#[test]
fn gm_only_key_beneath_an_array_element_is_stripped_for_non_gm() {
    // The same fail-open reaches the DESCENT step: an override may traverse an array
    // index on its way to an object key (`/system/inventory/0/secret`). The terminal
    // container is an object, so the key is genuinely removed; the sibling key and the
    // sibling element stay intact.
    let mut perms = PermissionSet {
        default: DocRole::Observer,
        ..Default::default()
    };
    perms
        .property_overrides
        .insert("/system/inventory/0/secret".into(), Visibility::GmOnly);
    let d = doc(
        perms,
        serde_json::json!({
            "inventory": [
                { "secret": "MOCK_SECRET_A", "public": 1 },
                { "secret": "MOCK_SECRET_B" }
            ]
        }),
    );

    let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &d, d.owner);
    let view = filter_properties(&d, &player).unwrap();
    assert_eq!(
        view.system["inventory"],
        serde_json::json!([{ "public": 1 }, { "secret": "MOCK_SECRET_B" }]),
        "only the pointed-at key is removed; the sibling element is untouched"
    );

    let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &d, d.owner);
    assert_eq!(
        filter_properties(&d, &gm).unwrap().system["inventory"][0]["secret"],
        serde_json::json!("MOCK_SECRET_A")
    );
}

#[test]
fn a_gm_receives_every_band_unredacted_whatever_the_overrides_name() {
    // Whole-document equality, not per-pointer assertions: every band, including the
    // unconditional `/base` policy and an array-index override, must survive intact.
    //
    // This pins the OUTPUT rule, which is the part a change can break. It cannot pin
    // `filter_properties`' `see_gm_only` early return, because that return is not
    // observable: `can_see` yields `true` for a GM at every tier, so the hidden-pointer
    // set is empty and the loop is a no-op regardless. The early return is a hot-path
    // guard against the serialize/deserialize round-trip, not a visibility decision.
    let mut perms = PermissionSet {
        default: DocRole::Observer,
        ..Default::default()
    };
    for ptr in ["/system/inventory/0", "/system", "/engine/vision", "/name"] {
        perms
            .property_overrides
            .insert(ptr.into(), Visibility::GmOnly);
    }
    let mut d = doc(
        perms,
        serde_json::json!({ "inventory": ["MOCK_SECRET_A", "visible"] }),
    );
    d.name = Some("MOCK_NAME_A".into());
    d.engine = Some(serde_json::json!({ "vision": 30 }));
    d.base = Some(serde_json::json!({ "system": { "hp": 1 } }));

    let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &d, d.owner);
    assert_eq!(filter_properties(&d, &gm).unwrap(), d);
}

#[test]
fn owner_or_gm_name_visible_to_owner_and_gm_not_other_player() {
    // `/name` mirrors the `/system/name` OwnerOrGm tier: an owner and the
    // GM see it; another player is redacted to `null` (not stripped, since
    // `name` is a top-level `Option` envelope field).
    let owner = Uuid::from_u128(1);
    let other = Uuid::from_u128(2);
    let mut d = doc(
        perms_with(&[("/name", Visibility::OwnerOrGm)]),
        serde_json::json!({}),
    );
    d.owner = Some(owner);
    d.name = Some("Goblin Skirmisher".into());

    let a_owner = resolve_access(owner, WorldRole::Player, &d, d.owner);
    assert_eq!(
        filter_properties(&d, &a_owner).unwrap().name.as_deref(),
        Some("Goblin Skirmisher")
    );

    let a_other = resolve_access(other, WorldRole::Player, &d, d.owner);
    assert_eq!(filter_properties(&d, &a_other).unwrap().name, None);

    let a_gm = resolve_access(other, WorldRole::Gm, &d, d.owner);
    assert_eq!(
        filter_properties(&d, &a_gm).unwrap().name.as_deref(),
        Some("Goblin Skirmisher")
    );
}

#[test]
fn whole_name_gm_only_nulls_to_null() {
    let mut perms = PermissionSet {
        default: DocRole::Observer,
        ..Default::default()
    };
    perms
        .property_overrides
        .insert("/name".into(), Visibility::GmOnly);
    let mut d = doc(perms, serde_json::json!({}));
    d.name = Some("Strahd".into());

    let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &d, d.owner);
    assert_eq!(filter_properties(&d, &player).unwrap().name, None);

    let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &d, d.owner);
    assert_eq!(
        filter_properties(&d, &gm).unwrap().name.as_deref(),
        Some("Strahd")
    );
}

#[test]
fn base_is_hardcoded_owner_or_gm_unconditional_of_overrides() {
    // `base` is a historical snapshot that may echo content hidden elsewhere in the
    // document (e.g. via `property_overrides`). Its own visibility is hardcoded
    // `OwnerOrGm` and must NOT depend on `property_overrides` at all — this doc has
    // NONE, proving the hiding isn't override-driven.
    let owner = Uuid::from_u128(1);
    let other = Uuid::from_u128(2);
    let mut d = doc(PermissionSet::default(), serde_json::json!({ "hp": 10 }));
    d.owner = Some(owner);
    d.base = Some(serde_json::json!({ "name": "Goblin", "system": { "hp": 10 } }));

    // Non-owner, non-GM: base is nulled.
    let a_other = resolve_access(other, WorldRole::Player, &d, d.owner);
    assert_eq!(filter_properties(&d, &a_other).unwrap().base, None);

    // Owner (non-GM): sees the real base.
    let a_owner = resolve_access(owner, WorldRole::Player, &d, d.owner);
    assert_eq!(filter_properties(&d, &a_owner).unwrap().base, d.base);

    // GM: sees the real base.
    let a_gm = resolve_access(other, WorldRole::Gm, &d, d.owner);
    assert_eq!(filter_properties(&d, &a_gm).unwrap().base, d.base);
}

#[tokio::test]
async fn filter_command_update_drops_base_field_change_for_non_owner_non_gm() {
    // A field-level `/base` FieldChange in a broadcast Update must be entirely dropped
    // for a non-owner non-GM recipient (via `collect_hidden`/`redact_change`), but
    // passed through unchanged for the owner and for a GM.
    use crate::auth::role::ServerRole;
    use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
    use crate::data::membership::PermissionContext;
    use crate::data::sqlite::SqliteRepository;

    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let owner = r
        .create_user("owner", None, ServerRole::User, 0)
        .await
        .unwrap();

    let mut d = doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({ "hp": 10 }),
    );
    d.scope = Scope::World { world_id: w.id };
    d.owner = Some(owner);
    d.base = Some(serde_json::json!({ "system": { "hp": 5 } }));
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create { doc: d.clone() }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let cmd = Command {
        seq: 2,
        world_id: w.id,
        author: gm,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/base".into(),
                old: serde_json::json!({ "system": { "hp": 5 } }),
                new: serde_json::json!({ "system": { "hp": 10 } }),
            }],
        }],
    };

    let current = load_current_docs(&r, &cmd).await;
    let snapshot = immediate_snapshot(&cmd, &current, &[gm], &|_| None);

    // Non-owner, non-GM: the change is dropped entirely.
    let other = PermissionContext {
        user_id: Uuid::from_u128(77),
        world_role: WorldRole::Player,
    };
    let out_other = filter_command(
        &cmd,
        &snapshot,
        &other,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out_other.ops[0] else {
        panic!("expected Update");
    };
    assert!(
        changes.is_empty(),
        "non-owner non-GM must not receive a /base FieldChange"
    );

    // Owner: passed through unchanged.
    let owner_ctx = PermissionContext {
        user_id: owner,
        world_role: WorldRole::Player,
    };
    let out_owner = filter_command(
        &cmd,
        &snapshot,
        &owner_ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out_owner.ops[0] else {
        panic!("expected Update");
    };
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].path, "/base");
    assert_eq!(
        changes[0].new,
        serde_json::json!({ "system": { "hp": 10 } })
    );

    // GM: passed through unchanged.
    let out_gm = filter_command(
        &cmd,
        &snapshot,
        &gm_ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out_gm.ops[0] else {
        panic!("expected Update");
    };
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].path, "/base");
}

#[test]
fn collect_hidden_embedded_engine_override_is_prefixed() {
    // An embedded child's `/engine/...` override must surface, parent-
    // absolute, as `/embedded/<key>/<i>/engine/...` — the same coverage
    // `filter_properties` gives whole-document egress, needed by
    // `filter_command`'s Update-delta redaction.
    let mut child = doc(PermissionSet::default(), serde_json::json!({}));
    child.engine = Some(serde_json::json!({ "x": 1.0 }));
    child
        .permissions
        .property_overrides
        .insert("/engine/x".into(), Visibility::GmOnly);
    let mut parent = doc(PermissionSet::default(), serde_json::json!({}));
    parent.embedded.insert("actor".into(), vec![child]);

    let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &parent, parent.owner);
    let mut hidden = Vec::new();
    collect_hidden(&parent, &player, "", &mut hidden).unwrap();
    assert!(hidden.contains(&"/embedded/actor/0/engine/x".to_string()));
}

#[test]
fn embedded_child_gm_only_is_stripped_for_non_gm() {
    let mut child = doc(
        PermissionSet::default(),
        serde_json::json!({ "secret": 9, "shown": 2 }),
    );
    child
        .permissions
        .property_overrides
        .insert("/system/secret".into(), Visibility::GmOnly);
    let mut parent = doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({ "public": 1 }),
    );
    parent.embedded.insert("items".into(), vec![child]);

    let player = resolve_access(Uuid::from_u128(7), WorldRole::Player, &parent, parent.owner);
    let view = filter_properties(&parent, &player).unwrap();
    let child_view = &view.embedded.get("items").unwrap()[0];
    assert_eq!(
        child_view.system.get("secret"),
        None,
        "child gm-only stripped"
    );
    assert_eq!(child_view.system["shown"], serde_json::json!(2));

    // The GM sees the embedded child's gm-only field.
    let gm = resolve_access(Uuid::from_u128(7), WorldRole::Gm, &parent, parent.owner);
    let gm_view = filter_properties(&parent, &gm).unwrap();
    assert_eq!(
        gm_view.embedded.get("items").unwrap()[0].system["secret"],
        serde_json::json!(9)
    );
}

#[test]
fn redact_change_preserves_remove_flag_on_ancestor_of_hidden_leaf() {
    // A GM removes `/system/sheet` — a subtree that contains a nested gm_only leaf
    // `/system/sheet/hidden`. The redacted broadcast to a non-privileged recipient must
    // stay a REMOVAL (remove: true, new: Null), never downgrade to a set-to-null: the
    // latter would leave the key present-as-null on the recipient's client, violating the
    // `null` != absent invariant.
    let ch = FieldChange {
        remove: true,
        path: "/system/sheet".into(),
        old: serde_json::json!({ "shown": 1, "hidden": 42 }),
        new: serde_json::Value::Null,
    };
    let redacted = redact_change(&ch, &["/system/sheet/hidden".to_string()]).unwrap();
    assert!(redacted.remove, "removal flag preserved through redaction");
    assert_eq!(
        redacted.new,
        serde_json::Value::Null,
        "a removal carries no new value"
    );
    assert_eq!(
        redacted.old,
        serde_json::json!({ "shown": 1 }),
        "hidden leaf stripped from the pre-image; shown sibling remains"
    );
}

#[tokio::test]
async fn filter_command_create_strips_embedded_gm_only() {
    use crate::auth::role::ServerRole;
    use crate::data::command::{Command, Operation};
    use crate::data::membership::PermissionContext;
    use crate::data::sqlite::SqliteRepository;

    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();

    let mut child = doc(
        PermissionSet::default(),
        serde_json::json!({ "secret": 9, "shown": 2 }),
    );
    child
        .permissions
        .property_overrides
        .insert("/system/secret".into(), Visibility::GmOnly);
    let mut parent = doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({ "public": 1 }),
    );
    parent.scope = Scope::World { world_id: w.id };
    parent.embedded.insert("items".into(), vec![child]);

    let cmd = Command {
        seq: 1,
        world_id: w.id,
        author: gm,
        ts: 0,
        ops: vec![Operation::Create {
            doc: parent.clone(),
        }],
    };
    let player = PermissionContext {
        user_id: Uuid::from_u128(77),
        world_role: WorldRole::Player,
    };
    let current = load_current_docs(&r, &cmd).await;
    let snapshot = immediate_snapshot(&cmd, &current, &[], &|_| None);
    let filtered = filter_command(
        &cmd,
        &snapshot,
        &player,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Create { doc } = &filtered.ops[0] else {
        panic!("expected Create");
    };
    assert!(
        doc.embedded.get("items").unwrap()[0]
            .system
            .get("secret")
            .is_none(),
        "embedded child gm-only stripped on the Create broadcast"
    );
}

#[tokio::test]
async fn filter_command_create_drops_op_entirely_for_default_none_region() {
    // A secret region declares `default: DocRole::None` (not just a
    // `/system` gm_only override), so `filter_command` must drop the Create op ENTIRELY
    // for a non-GM/non-owner recipient (no envelope at all — id/parent_id/existence must
    // never reach them), while a GM still receives the full op.
    use crate::auth::role::ServerRole;
    use crate::data::command::{Command, Operation};
    use crate::data::membership::PermissionContext;
    use crate::data::sqlite::SqliteRepository;

    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();

    let mut region = doc(
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
        serde_json::json!({ "shape": "rect", "behavior": "arrest" }),
    );
    region.scope = Scope::World { world_id: w.id };
    region
        .permissions
        .property_overrides
        .insert("/system".into(), Visibility::GmOnly);

    let cmd = Command {
        seq: 1,
        world_id: w.id,
        author: gm,
        ts: 0,
        ops: vec![Operation::Create {
            doc: region.clone(),
        }],
    };

    let current = load_current_docs(&r, &cmd).await;
    let snapshot = immediate_snapshot(&cmd, &current, &[gm], &|_| None);
    let player = PermissionContext {
        user_id: Uuid::from_u128(77),
        world_role: WorldRole::Player,
    };
    let filtered_for_player = filter_command(
        &cmd,
        &snapshot,
        &player,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert!(
        filtered_for_player.ops.is_empty(),
        "a default:none secret region's Create op must be dropped entirely for a non-GM \
         recipient, not merely nulled — the doc's existence/id must not reach them"
    );

    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let filtered_for_gm = filter_command(
        &cmd,
        &snapshot,
        &gm_ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert_eq!(
        filtered_for_gm.ops.len(),
        1,
        "the GM must still receive the region's Create op"
    );
    let Operation::Create { doc } = &filtered_for_gm.ops[0] else {
        panic!("expected Create");
    };
    assert_eq!(doc.system.get("behavior").unwrap(), "arrest");
}

#[tokio::test]
async fn filter_command_drops_a_create_whose_redaction_cannot_be_classified() {
    // A poisoned document is withheld through `filter_command`, the per-recipient
    // broadcast egress path — not merely through the `filter_properties` unit
    // called directly by the other tests above.
    use crate::auth::role::ServerRole;
    use crate::data::command::{Command, Operation};
    use crate::data::membership::PermissionContext;
    use crate::data::sqlite::SqliteRepository;

    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();

    let mut d = doc(
        perms_with_override("/permissions/default"),
        serde_json::json!({ "hp": 1 }),
    );
    d.scope = Scope::World { world_id: w.id };

    let cmd = Command {
        seq: 5,
        world_id: w.id,
        author: gm,
        ts: 0,
        ops: vec![Operation::Create { doc: d.clone() }],
    };
    let player = PermissionContext {
        user_id: Uuid::from_u128(77),
        world_role: WorldRole::Player,
    };
    let current = load_current_docs(&r, &cmd).await;
    let snapshot = immediate_snapshot(&cmd, &current, &[], &|_| None);
    let out = filter_command(
        &cmd,
        &snapshot,
        &player,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert!(
        out.ops.is_empty(),
        "the op must be withheld, not shipped half-redacted"
    );
    assert_eq!(
        out.seq, cmd.seq,
        "seq is preserved so the sequence guard sees no gap"
    );
}

#[tokio::test]
async fn filter_command_drops_a_delete_whose_redaction_cannot_be_classified() {
    // Mirrors `filter_command_drops_a_create_whose_redaction_cannot_be_classified`
    // for the Delete arm, which has no other test poisoning its document.
    use crate::auth::role::ServerRole;
    use crate::data::command::{Command, Operation};
    use crate::data::membership::PermissionContext;
    use crate::data::sqlite::SqliteRepository;

    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();

    let mut d = doc(
        perms_with_override("/permissions/default"),
        serde_json::json!({ "hp": 1 }),
    );
    d.scope = Scope::World { world_id: w.id };

    let cmd = Command {
        seq: 6,
        world_id: w.id,
        author: gm,
        ts: 0,
        ops: vec![Operation::Delete { doc: d.clone() }],
    };
    let player = PermissionContext {
        user_id: Uuid::from_u128(77),
        world_role: WorldRole::Player,
    };
    let current = load_current_docs(&r, &cmd).await;
    let snapshot = immediate_snapshot(&cmd, &current, &[], &|_| None);
    let out = filter_command(
        &cmd,
        &snapshot,
        &player,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert!(
        out.ops.is_empty(),
        "the op must be withheld, not shipped half-redacted"
    );
    assert_eq!(
        out.seq, cmd.seq,
        "seq is preserved so the sequence guard sees no gap"
    );
}

#[tokio::test]
async fn filter_command_strips_and_preserves_seq() {
    use crate::auth::role::ServerRole;
    use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
    use crate::data::membership::PermissionContext;
    use crate::data::sqlite::SqliteRepository;

    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };

    let mut d = doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({ "secret": 1, "public": 2 }),
    );
    d.scope = Scope::World { world_id: w.id };
    d.permissions
        .property_overrides
        .insert("/system/secret".into(), Visibility::GmOnly);
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create { doc: d.clone() }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // An update touching both a GmOnly and a public field.
    let cmd = Command {
        seq: 2,
        world_id: w.id,
        author: gm,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![
                FieldChange {
                    remove: false,
                    path: "/system/secret".into(),
                    old: serde_json::json!(1),
                    new: serde_json::json!(9),
                },
                FieldChange {
                    remove: false,
                    path: "/system/public".into(),
                    old: serde_json::json!(2),
                    new: serde_json::json!(8),
                },
            ],
        }],
    };

    let current = load_current_docs(&r, &cmd).await;
    let snapshot = immediate_snapshot(&cmd, &current, &[gm], &|_| None);

    // Player sees the public change only; seq is preserved.
    let player = PermissionContext {
        user_id: Uuid::from_u128(77),
        world_role: WorldRole::Player,
    };
    let filtered = filter_command(
        &cmd,
        &snapshot,
        &player,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert_eq!(filtered.seq, 2);
    if let Operation::Update { changes, .. } = &filtered.ops[0] {
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "/system/public");
    } else {
        panic!("expected Update");
    }

    // GM sees both changes.
    let gm_view = filter_command(
        &cmd,
        &snapshot,
        &gm_ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    if let Operation::Update { changes, .. } = &gm_view.ops[0] {
        assert_eq!(changes.len(), 2);
    } else {
        panic!("expected Update");
    }
}

#[tokio::test]
async fn permission_tightening_retracts_now_hidden_field_for_non_owner() {
    use crate::auth::role::ServerRole;
    use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
    use crate::data::membership::PermissionContext;
    use crate::data::sqlite::SqliteRepository;

    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    // A real user — `owner_id` is a foreign key.
    let owner = r
        .create_user("owner", None, ServerRole::User, 0)
        .await
        .unwrap();

    // cur = post-apply doc: owner set, name present, /system/name now OwnerOrGm.
    let mut d = doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({ "name": "Goblin Skirmisher", "displayName": "Goblin" }),
    );
    d.scope = Scope::World { world_id: w.id };
    d.owner = Some(owner);
    d.permissions
        .property_overrides
        .insert("/system/name".into(), Visibility::OwnerOrGm);
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create { doc: d.clone() }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // The broadcast Update that tightened permissions (adds the name override).
    let cmd = Command {
        seq: 2,
        world_id: w.id,
        author: gm,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/permissions/property_overrides".into(),
                old: serde_json::json!({}),
                new: serde_json::json!({ "/system/name": "owner_or_gm" }),
            }],
        }],
    };

    let current = load_current_docs(&r, &cmd).await;
    let snapshot = immediate_snapshot(&cmd, &current, &[gm], &|_| None);

    // Non-owner player: keeps the permission change PLUS a null retraction of /system/name.
    let other = PermissionContext {
        user_id: Uuid::from_u128(77),
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &other,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out.ops[0] else {
        panic!("expected Update");
    };
    let retract = changes
        .iter()
        .find(|c| c.path == "/system/name")
        .expect("name retracted");
    assert_eq!(retract.new, serde_json::Value::Null);
    assert_eq!(retract.old, serde_json::Value::Null); // pre-image must not leak the real name

    // Owner: keeps the name (OwnerOrGm is visible) — no /system/name retraction.
    let owner_ctx = PermissionContext {
        user_id: owner,
        world_role: WorldRole::Player,
    };
    let out_owner = filter_command(
        &cmd,
        &snapshot,
        &owner_ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out_owner.ops[0] else {
        panic!("expected Update");
    };
    assert!(!changes.iter().any(|c| c.path == "/system/name"));

    // GM: sees everything; no synthesized retraction.
    let out_gm = filter_command(
        &cmd,
        &snapshot,
        &gm_ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out_gm.ops[0] else {
        panic!("expected Update");
    };
    assert!(!changes.iter().any(|c| c.path == "/system/name"));
}

#[tokio::test]
async fn permission_tightening_retracts_embedded_owner_or_gm_for_non_owner() {
    use crate::auth::role::ServerRole;
    use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
    use crate::data::membership::PermissionContext;
    use crate::data::sqlite::SqliteRepository;

    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let owner = r
        .create_user("owner", None, ServerRole::User, 0)
        .await
        .unwrap();

    // Parent (owner set) embeds an actor copy whose name is OwnerOrGm-hidden.
    let mut child = doc(
        PermissionSet::default(),
        serde_json::json!({ "name": "Goblin Skirmisher", "displayName": "Goblin" }),
    );
    child
        .permissions
        .property_overrides
        .insert("/system/name".into(), Visibility::OwnerOrGm);
    let mut parent = doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({ "public": 0 }),
    );
    parent.scope = Scope::World { world_id: w.id };
    parent.owner = Some(owner);
    parent.embedded.insert("actor".into(), vec![child]);
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create {
            doc: parent.clone(),
        }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Tighten the embedded child's permissions (adds the name override).
    let cmd = Command {
        seq: 2,
        world_id: w.id,
        author: gm,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: parent.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/embedded/actor/0/permissions/property_overrides".into(),
                old: serde_json::json!({}),
                new: serde_json::json!({ "/system/name": "owner_or_gm" }),
            }],
        }],
    };

    let current = load_current_docs(&r, &cmd).await;
    let snapshot = immediate_snapshot(&cmd, &current, &[gm], &|_| None);

    // Non-owner player: the embedded name is retracted with a null pre-image.
    let other = PermissionContext {
        user_id: Uuid::from_u128(77),
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &other,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out.ops[0] else {
        panic!("expected Update");
    };
    let retract = changes
        .iter()
        .find(|c| c.path == "/embedded/actor/0/system/name")
        .expect("embedded name retracted");
    assert_eq!(retract.new, serde_json::Value::Null);
    assert_eq!(retract.old, serde_json::Value::Null);

    // Owner: the embedded OwnerOrGm name stays visible — no retraction.
    let owner_ctx = PermissionContext {
        user_id: owner,
        world_role: WorldRole::Player,
    };
    let out_owner = filter_command(
        &cmd,
        &snapshot,
        &owner_ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out_owner.ops[0] else {
        panic!("expected Update");
    };
    assert!(!changes
        .iter()
        .any(|c| c.path == "/embedded/actor/0/system/name"));

    // GM: sees everything — no retraction.
    let out_gm = filter_command(
        &cmd,
        &snapshot,
        &gm_ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out_gm.ops[0] else {
        panic!("expected Update");
    };
    assert!(!changes
        .iter()
        .any(|c| c.path == "/embedded/actor/0/system/name"));
}

#[tokio::test]
async fn filter_command_update_redacts_embedded_child_gm_only() {
    use crate::auth::role::ServerRole;
    use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
    use crate::data::membership::PermissionContext;
    use crate::data::sqlite::SqliteRepository;

    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };

    let mut child = doc(
        PermissionSet::default(),
        serde_json::json!({ "secret": 1, "shown": 2 }),
    );
    child
        .permissions
        .property_overrides
        .insert("/system/secret".into(), Visibility::GmOnly);
    let mut parent = doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({ "public": 0 }),
    );
    parent.scope = Scope::World { world_id: w.id };
    parent.embedded.insert("items".into(), vec![child]);
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create {
            doc: parent.clone(),
        }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let cmd = Command {
        seq: 2,
        world_id: w.id,
        author: gm,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: parent.id,
            changes: vec![
                // Direct write of the embedded child's gm-only field → dropped.
                FieldChange {
                    remove: false,
                    path: "/embedded/items/0/system/secret".into(),
                    old: serde_json::json!(1),
                    new: serde_json::json!(9),
                },
                // Wholesale rewrite of the child's /system (ancestor of the gm-only
                // leaf) → the hidden leaf is stripped from old + new, sibling kept.
                FieldChange {
                    remove: false,
                    path: "/embedded/items/0/system".into(),
                    old: serde_json::json!({ "secret": 1, "shown": 2 }),
                    new: serde_json::json!({ "secret": 9, "shown": 3 }),
                },
                // Unrelated public parent field → kept.
                FieldChange {
                    remove: false,
                    path: "/system/public".into(),
                    old: serde_json::json!(0),
                    new: serde_json::json!(5),
                },
            ],
        }],
    };

    let current = load_current_docs(&r, &cmd).await;
    let snapshot = immediate_snapshot(&cmd, &current, &[gm], &|_| None);
    let player = PermissionContext {
        user_id: Uuid::from_u128(77),
        world_role: WorldRole::Player,
    };
    let filtered = filter_command(
        &cmd,
        &snapshot,
        &player,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &filtered.ops[0] else {
        panic!("expected Update");
    };
    assert_eq!(
        changes.len(),
        2,
        "the direct gm-only embedded change is dropped"
    );
    let sys = changes
        .iter()
        .find(|c| c.path == "/embedded/items/0/system")
        .unwrap();
    assert!(sys.new.get("secret").is_none(), "secret stripped from new");
    assert!(sys.old.get("secret").is_none(), "secret stripped from old");
    assert_eq!(sys.new["shown"], serde_json::json!(3));
    assert!(changes.iter().any(|c| c.path == "/system/public"));

    // GM sees all three unredacted.
    let gm_view = filter_command(
        &cmd,
        &snapshot,
        &gm_ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &gm_view.ops[0] else {
        panic!("expected Update");
    };
    assert_eq!(changes.len(), 3);
}

#[tokio::test]
async fn filter_command_nulls_a_gm_only_array_element_inside_an_ancestor_change() {
    // The delta path and whole-document egress must reach the same verdict on an
    // array-index override: a change writing the whole array carries the hidden element
    // in both `old` and `new`, so `redact_change` must null it in place there exactly as
    // `filter_properties` does on the whole document. Length and sibling positions are
    // preserved, so the recipient's indices still agree with the authoritative array.
    use crate::auth::role::ServerRole;
    use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
    use crate::data::membership::PermissionContext;
    use crate::data::sqlite::SqliteRepository;

    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };

    let mut d = doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({ "inventory": ["MOCK_SECRET_A", "visible"] }),
    );
    d.scope = Scope::World { world_id: w.id };
    d.permissions
        .property_overrides
        .insert("/system/inventory/0".into(), Visibility::GmOnly);
    // Ingress accepts the override, which is what obliges egress to act on it.
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create { doc: d.clone() }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let cmd = Command {
        seq: 2,
        world_id: w.id,
        author: gm,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/system/inventory".into(),
                old: serde_json::json!(["MOCK_SECRET_A", "visible"]),
                new: serde_json::json!(["MOCK_SECRET_B", "also visible"]),
            }],
        }],
    };

    let current = load_current_docs(&r, &cmd).await;
    let snapshot = immediate_snapshot(&cmd, &current, &[gm], &|_| None);
    let player = PermissionContext {
        user_id: Uuid::from_u128(77),
        world_role: WorldRole::Player,
    };
    let filtered = filter_command(
        &cmd,
        &snapshot,
        &player,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &filtered.ops[0] else {
        panic!("expected Update");
    };
    assert_eq!(changes[0].new, serde_json::json!([null, "also visible"]));
    assert_eq!(changes[0].old, serde_json::json!([null, "visible"]));

    let gm_view = filter_command(
        &cmd,
        &snapshot,
        &gm_ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &gm_view.ops[0] else {
        panic!("expected Update");
    };
    assert_eq!(
        changes[0].new,
        serde_json::json!(["MOCK_SECRET_B", "also visible"])
    );
}

#[test]
fn gm_role_denies_gm_unless_individually_granted() {
    let owner = Uuid::from_u128(1);
    let gm = Uuid::from_u128(2);
    let mut perms = PermissionSet {
        default: DocRole::None,
        gm_role: Some(DocRole::None),
        ..Default::default()
    };
    perms.users.insert(owner, DocRole::Owner);
    let d = doc(perms, serde_json::json!({}));

    // A GM not individually listed gets nothing — gm_role caps them like any other actor.
    let a_gm = resolve_access(gm, WorldRole::Gm, &d, d.owner);
    assert!(
        !a_gm.has(cap::READ),
        "unlisted GM must not read a gm_role:None document"
    );
    assert!(
        !a_gm.all,
        "gm_role:Some(_) must not grant the unconditional short-circuit"
    );

    // The owner is unaffected.
    let a_owner = resolve_access(owner, WorldRole::Player, &d, d.owner);
    assert!(a_owner.has(cap::READ));
}

#[test]
fn gm_role_denies_but_admits_a_gm_individually_listed() {
    let owner = Uuid::from_u128(1);
    let gm = Uuid::from_u128(2);
    let mut perms = PermissionSet {
        default: DocRole::None,
        gm_role: Some(DocRole::None),
        ..Default::default()
    };
    perms.users.insert(owner, DocRole::Owner);
    perms.users.insert(gm, DocRole::Observer); // e.g. a whisper naming the GM
    let d = doc(perms, serde_json::json!({}));

    let a_gm = resolve_access(gm, WorldRole::Gm, &d, d.owner);
    assert!(
        a_gm.has(cap::READ),
        "a GM individually listed in `users` must read despite gm_role:None"
    );
    assert!(
        !a_gm.all,
        "still not the unconditional short-circuit — just an ordinary Observer grant"
    );
}

#[test]
fn gm_role_option_none_default_preserves_unconditional_gm_access() {
    let owner = Uuid::from_u128(1);
    let gm = Uuid::from_u128(2);
    let mut perms = PermissionSet {
        default: DocRole::None,
        gm_role: None, // the field's actual default — Option::None, not Some(DocRole::None)
        ..Default::default()
    };
    perms.users.insert(owner, DocRole::Owner);
    let d = doc(perms, serde_json::json!({}));

    let a_gm = resolve_access(gm, WorldRole::Gm, &d, d.owner);
    assert!(
        a_gm.all,
        "gm_role: None (the default) must preserve the unconditional GM short-circuit \
         even when the document's own default/users would otherwise deny access"
    );
}

#[test]
fn gm_role_observer_grants_any_gm_without_explicit_listing() {
    let owner = Uuid::from_u128(1);
    let gm = Uuid::from_u128(2);
    let stranger = Uuid::from_u128(3);
    let mut perms = PermissionSet {
        default: DocRole::None,
        gm_role: Some(DocRole::Observer),
        ..Default::default()
    };
    perms.users.insert(owner, DocRole::Owner);
    let d = doc(perms, serde_json::json!({}));

    // Any GM reads, even without being individually listed (dynamic resolution).
    let a_gm = resolve_access(gm, WorldRole::Gm, &d, d.owner);
    assert!(a_gm.has(cap::READ));
    assert!(a_gm.see_gm_only, "still a GM for property-tier purposes");

    // A non-owner, non-GM Player reads nothing.
    let a_stranger = resolve_access(stranger, WorldRole::Player, &d, d.owner);
    assert!(!a_stranger.has(cap::READ));
}

#[test]
fn resolve_access_world_layers_world_grants_using_the_gm_role_fallback() {
    use crate::data::document::CapabilityGrants;
    let owner = Uuid::from_u128(1);
    let gm = Uuid::from_u128(2);
    let mut perms = PermissionSet {
        default: DocRole::None,
        gm_role: Some(DocRole::Observer),
        ..Default::default()
    };
    perms.users.insert(owner, DocRole::Owner);
    let d = doc(perms, serde_json::json!({}));

    let mut world_grants = CapabilityGrants::default();
    world_grants
        .by_role
        .entry(DocRole::Observer)
        .or_default()
        .insert("dnd5e:extra".to_string());

    // A GM not individually listed still resolves via the gm_role (Observer)
    // fallback, so world-level Observer grants must layer on top of it too —
    // not just `doc.permissions.default` (None here, which carries no such
    // grant). Proves resolve_access_world uses the SAME effective role as
    // resolve_access rather than recomputing it independently.
    let a_gm = resolve_access_world(gm, WorldRole::Gm, &d, &world_grants, d.owner);
    assert!(
        a_gm.has("dnd5e:extra"),
        "world grant for the gm_role fallback role must apply"
    );
}

#[tokio::test]
async fn filter_command_redacts_nested_gm_only_paths() {
    use crate::auth::role::ServerRole;
    use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
    use crate::data::membership::PermissionContext;
    use crate::data::sqlite::SqliteRepository;

    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };

    let mut d = doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({
            "secret": { "value": 1 },
            "sheet": { "hidden": 2, "shown": 3 },
            "public": 4
        }),
    );
    d.scope = Scope::World { world_id: w.id };
    // A GM-only object and a GM-only nested leaf.
    d.permissions
        .property_overrides
        .insert("/system/secret".into(), Visibility::GmOnly);
    d.permissions
        .property_overrides
        .insert("/system/sheet/hidden".into(), Visibility::GmOnly);
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create { doc: d.clone() }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let cmd = Command {
        seq: 2,
        world_id: w.id,
        author: gm,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![
                // Descendant of a GM-only pointer → dropped entirely.
                FieldChange {
                    remove: false,
                    path: "/system/secret/value".into(),
                    old: serde_json::json!(1),
                    new: serde_json::json!(9),
                },
                // Ancestor of a GM-only pointer → hidden child stripped from
                // both pre-image and new value, siblings preserved.
                FieldChange {
                    remove: false,
                    path: "/system/sheet".into(),
                    old: serde_json::json!({ "hidden": 2, "shown": 3 }),
                    new: serde_json::json!({ "hidden": 20, "shown": 30 }),
                },
                // Unrelated public field → kept whole.
                FieldChange {
                    remove: false,
                    path: "/system/public".into(),
                    old: serde_json::json!(4),
                    new: serde_json::json!(40),
                },
            ],
        }],
    };

    let current = load_current_docs(&r, &cmd).await;
    let snapshot = immediate_snapshot(&cmd, &current, &[gm], &|_| None);
    let player = PermissionContext {
        user_id: Uuid::from_u128(77),
        world_role: WorldRole::Player,
    };
    let filtered = filter_command(
        &cmd,
        &snapshot,
        &player,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &filtered.ops[0] else {
        panic!("expected Update");
    };
    assert_eq!(changes.len(), 2, "the GM-only descendant change is dropped");
    let sheet = changes.iter().find(|c| c.path == "/system/sheet").unwrap();
    assert!(
        sheet.new.get("hidden").is_none(),
        "hidden child stripped from new"
    );
    assert!(
        sheet.old.get("hidden").is_none(),
        "hidden child stripped from old"
    );
    assert_eq!(sheet.new["shown"], serde_json::json!(30));
    let public = changes.iter().find(|c| c.path == "/system/public").unwrap();
    assert_eq!(public.new, serde_json::json!(40));

    // The GM sees every change unredacted.
    let gm_view = filter_command(
        &cmd,
        &snapshot,
        &gm_ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &gm_view.ops[0] else {
        panic!("expected Update");
    };
    assert_eq!(changes.len(), 3);
}

// ---- effective_owner: the single token-ownership rule ----

fn token_linked_to(actor_id: Option<Uuid>) -> Document {
    let mut d = doc(PermissionSet::default(), serde_json::json!({}));
    d.id = Uuid::from_u128(100);
    d.doc_type = "token".into();
    d.engine = Some(match actor_id {
        Some(a) => serde_json::json!({
            "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0,
            "actor_id": a.to_string()
        }),
        None => serde_json::json!({
            "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0
        }),
    });
    d
}

fn actor_owned_by(id: Uuid, owner: Option<Uuid>) -> Document {
    let mut d = doc(PermissionSet::default(), serde_json::json!({}));
    d.id = id;
    d.owner = owner;
    d
}

#[test]
fn token_actor_link_reads_only_a_tokens_engine_actor_id() {
    let a = Uuid::from_u128(42);
    assert_eq!(token_actor_link(&token_linked_to(Some(a))), Some(a));
    // A raw/instanced token carries no link.
    assert_eq!(token_actor_link(&token_linked_to(None)), None);
    // A non-token doc_type never links, even with a stray `actor_id` key.
    let mut impostor = token_linked_to(Some(a));
    impostor.doc_type = "actor".into();
    assert_eq!(token_actor_link(&impostor), None);
}

#[test]
fn effective_owner_prefers_the_per_token_override() {
    let actor_id = Uuid::from_u128(42);
    let inheritor = Uuid::from_u128(1);
    let override_user = Uuid::from_u128(2);
    let actor = actor_owned_by(actor_id, Some(inheritor));

    // No override: inherits the linked actor's owner.
    let plain = token_linked_to(Some(actor_id));
    assert_eq!(effective_owner(&plain, Some(&actor)), Some(inheritor));

    // Override set: it wins over the same actor, same link.
    let mut overridden = token_linked_to(Some(actor_id));
    overridden.owner = Some(override_user);
    assert_eq!(
        effective_owner(&overridden, Some(&actor)),
        Some(override_user)
    );
}

#[test]
fn effective_owner_fails_closed_on_degenerate_links() {
    let actor_id = Uuid::from_u128(42);
    let player = Uuid::from_u128(1);

    // No link, no override.
    assert_eq!(effective_owner(&token_linked_to(None), None), None);
    // Dangling link: the actor row does not exist.
    assert_eq!(
        effective_owner(&token_linked_to(Some(actor_id)), None),
        None
    );
    // Linked to an actor that nobody owns.
    assert_eq!(
        effective_owner(
            &token_linked_to(Some(actor_id)),
            Some(&actor_owned_by(actor_id, None))
        ),
        None
    );
    // A `linked_actor` that is NOT the document the link names is rejected
    // rather than trusted — a mis-joined caller under-permits, never leaks
    // write authority to the wrong actor's owner.
    assert_eq!(
        effective_owner(
            &token_linked_to(Some(actor_id)),
            Some(&actor_owned_by(Uuid::from_u128(999), Some(player)))
        ),
        None
    );
    // Same, for a correctly-identified document of the wrong doc_type.
    let mut wrong_type = actor_owned_by(actor_id, Some(player));
    wrong_type.doc_type = "token".into();
    assert_eq!(
        effective_owner(&token_linked_to(Some(actor_id)), Some(&wrong_type)),
        None
    );
    // Control: the same call with the correctly-joined owned actor resolves,
    // so the rejections above are the guards, not a constant `None`.
    assert_eq!(
        effective_owner(
            &token_linked_to(Some(actor_id)),
            Some(&actor_owned_by(actor_id, Some(player)))
        ),
        Some(player)
    );
}

#[test]
fn effective_owner_rejects_a_cross_scope_actor() {
    // A candidate from another scope is an illegitimate join, same class as a
    // wrong-id or wrong-type candidate: fail closed to no owner.
    let actor_id = Uuid::from_u128(42);
    let mut token = token_linked_to(Some(actor_id));
    token.scope = Scope::World {
        world_id: Uuid::from_u128(1000),
    };
    let mut foreign = actor_owned_by(actor_id, Some(Uuid::from_u128(1)));
    foreign.scope = Scope::World {
        world_id: Uuid::from_u128(2000),
    };
    assert_eq!(effective_owner(&token, Some(&foreign)), None);

    // Same scope still resolves.
    let mut same = actor_owned_by(actor_id, Some(Uuid::from_u128(1)));
    same.scope = token.scope.clone();
    assert_eq!(
        effective_owner(&token, Some(&same)),
        Some(Uuid::from_u128(1))
    );
}

#[test]
fn a_non_token_never_inherits_ownership() {
    let actor_id = Uuid::from_u128(42);
    let player = Uuid::from_u128(1);
    let mut not_a_token = token_linked_to(Some(actor_id));
    not_a_token.doc_type = "drawing".into();
    assert_eq!(
        effective_owner(&not_a_token, Some(&actor_owned_by(actor_id, Some(player)))),
        None,
        "inheritance is token-scoped: no other doc_type joins an actor"
    );
}

#[test]
fn effective_ownership_grants_the_owner_floor_and_the_owner_or_gm_tier() {
    let actor_id = Uuid::from_u128(42);
    let player = Uuid::from_u128(1);
    let stranger = Uuid::from_u128(2);
    let mut token = token_linked_to(Some(actor_id));
    // The shipping `buildTokenDoc` default: READ-only for everyone.
    token.permissions.default = DocRole::Observer;
    let actor = actor_owned_by(actor_id, Some(player));
    let owner = effective_owner(&token, Some(&actor));

    let a_player = resolve_access(player, WorldRole::Player, &token, owner);
    assert!(
        a_player.has(cap::READ) && a_player.has(cap::WRITE_FIELDS),
        "an effective owner holds the DocRole::Owner floor"
    );
    assert!(
        !a_player.has(cap::EDIT_PERMISSIONS) && !a_player.has(cap::DELETE),
        "the BUILT-IN floor stops at WRITE_FIELDS — no re-assigning or deleting. \
         Additive `by_role[Owner]` grants can widen past it; see \
         `world_by_role_owner_grants_reach_an_inheriting_owner`"
    );
    assert!(
        a_player.is_owner && a_player.can_see(Visibility::OwnerOrGm),
        "redaction's OwnerOrGm tier admits the same effective owner"
    );
    assert!(
        !a_player.can_see(Visibility::GmOnly),
        "an owner is not a GM"
    );

    // Non-vacuity: same token, same call, different user.
    let a_stranger = resolve_access(stranger, WorldRole::Player, &token, owner);
    assert!(a_stranger.has(cap::READ) && !a_stranger.has(cap::WRITE_FIELDS));
    assert!(!a_stranger.is_owner);
}

#[test]
fn the_owner_floor_never_downgrades_a_stronger_document_grant() {
    // A doc that already grants a user Owner keeps it when they are NOT the
    // effective owner: the floor only ever strengthens.
    let player = Uuid::from_u128(1);
    let mut token = token_linked_to(None);
    token.permissions.users.insert(player, DocRole::Owner);
    let a = resolve_access(player, WorldRole::Player, &token, None);
    assert!(a.has(cap::WRITE_FIELDS));
    assert!(
        !a.is_owner,
        "no effective owner => not the OwnerOrGm subject"
    );
}

#[test]
fn world_by_role_owner_grants_reach_an_inheriting_owner() {
    use crate::data::document::CapabilityGrants;
    // The owner floor sets the ROLE, and that role also selects additive
    // capability grants — so an INHERITING owner receives `by_role[Owner]`
    // exactly as a stamped `permissions.users[user] = Owner` would. That
    // equivalence is the point: inherited and stamped ownership must not
    // diverge. A deployment that puts EDIT_PERMISSIONS in `by_role[Owner]`
    // is choosing to hand it to every Owner, inheriting ones included; this
    // test documents that as intended, not as an accident of the floor.
    let actor_id = Uuid::from_u128(42);
    let player = Uuid::from_u128(1);
    let stranger = Uuid::from_u128(2);
    let mut token = token_linked_to(Some(actor_id));
    token.permissions.default = DocRole::Observer;
    let actor = actor_owned_by(actor_id, Some(player));
    let owner = effective_owner(&token, Some(&actor));

    let mut world_grants = CapabilityGrants::default();
    world_grants
        .by_role
        .entry(DocRole::Owner)
        .or_default()
        .insert(cap::EDIT_PERMISSIONS.to_string());

    let inheriting = resolve_access_world(player, WorldRole::Player, &token, &world_grants, owner);
    assert!(
        inheriting.has(cap::EDIT_PERMISSIONS),
        "a world by_role[Owner] grant reaches an owner who inherits through the actor link"
    );

    // Equivalence leg: a STAMPED owner on the same doc, with no effective
    // owner at all, resolves the identical grant — the two paths agree.
    let mut stamped = token_linked_to(None);
    stamped.permissions.default = DocRole::Observer;
    stamped.permissions.users.insert(player, DocRole::Owner);
    assert!(resolve_access_world(
        player,
        WorldRole::Player,
        &stamped,
        &world_grants,
        stamped.owner
    )
    .has(cap::EDIT_PERMISSIONS));

    // Non-vacuity: the grant is role-selected, not unconditional — a
    // non-owner on the same document with the same world grants gets nothing.
    assert!(
        !resolve_access_world(stranger, WorldRole::Player, &token, &world_grants, owner)
            .has(cap::EDIT_PERMISSIONS)
    );
}

// ---- filter_command joins the effective owner (egress hot path) ----

#[tokio::test]
async fn filter_command_admits_the_inheriting_owner_of_a_linked_token() {
    // token: permissions.default = None, owner = None, linked to an actor owned
    // by P. Literal-owner egress treated P as a stranger (op dropped); the
    // effective join must now deliver Create/Update/Delete AND OwnerOrGm-tier
    // content to P, while a true stranger still receives nothing. A document
    // P can write (owner floor at apply_intent) is one P receives.
    let p = Uuid::from_u128(1);
    let stranger = Uuid::from_u128(2);
    let actor_id = Uuid::from_u128(42);
    let actor = actor_owned_by(actor_id, Some(p));
    let mut token = token_linked_to(Some(actor_id));
    token.permissions.default = DocRole::None;
    token
        .permissions
        .property_overrides
        .insert("/system/notes".into(), Visibility::OwnerOrGm);

    let cmd = Command {
        seq: 1,
        world_id: Uuid::from_u128(7),
        author: Uuid::from_u128(9),
        ts: 0,
        ops: vec![Operation::Create { doc: token.clone() }],
    };
    let lookup = |id: &Uuid| (id == &actor.id).then_some(&actor);
    let current: HashMap<Uuid, CurrentDoc> = HashMap::new();
    let snapshot = immediate_snapshot(&cmd, &current, &[], &lookup);

    let p_ctx = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &p_ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert_eq!(out.ops.len(), 1, "inheriting owner must RECEIVE the create");

    let s_ctx = PermissionContext {
        user_id: stranger,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &s_ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert!(
        out.ops.is_empty(),
        "a stranger still receives nothing (fail closed)"
    );

    // Without the actor join (dangling source) the op is withheld even from P:
    // degenerate input under-permits, never over-permits.
    let out = filter_command(
        &cmd,
        &snapshot,
        &p_ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert!(out.ops.is_empty());
}

#[tokio::test]
async fn filter_command_update_keeps_owner_or_gm_changes_for_the_inheriting_owner() {
    use crate::auth::role::ServerRole;
    use crate::data::command::{Command, FieldChange, Operation, WriteOrigin};
    use crate::data::membership::PermissionContext;
    use crate::data::sqlite::SqliteRepository;

    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let p = r.create_user("p", None, ServerRole::User, 0).await.unwrap();

    let mut actor = doc(PermissionSet::default(), serde_json::json!({}));
    actor.id = Uuid::from_u128(42);
    actor.scope = Scope::World { world_id: w.id };
    actor.owner = Some(p);

    let mut token = doc(
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
        serde_json::json!({ "notes": "secret plan" }),
    );
    token.doc_type = "token".into();
    token.id = Uuid::from_u128(100);
    token.scope = Scope::World { world_id: w.id };
    token.engine = Some(serde_json::json!({
        "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0,
        "actor_id": actor.id.to_string()
    }));
    token
        .permissions
        .property_overrides
        .insert("/system/notes".into(), Visibility::OwnerOrGm);

    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![
            Operation::Create { doc: actor.clone() },
            Operation::Create { doc: token.clone() },
        ],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let cmd = Command {
        seq: 2,
        world_id: w.id,
        author: gm,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: token.id,
            changes: vec![
                FieldChange {
                    remove: false,
                    path: "/system/notes".into(),
                    old: serde_json::json!("secret plan"),
                    new: serde_json::json!("new plan"),
                },
                FieldChange {
                    remove: false,
                    path: "/base".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!({ "system": { "notes": "template" } }),
                },
            ],
        }],
    };

    let p_ctx = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };
    let current = load_current_docs(&r, &cmd).await;
    let lookup = |id: &Uuid| (id == &actor.id).then_some(&actor);
    let snapshot = immediate_snapshot(&cmd, &current, &[], &lookup);
    let out = filter_command(
        &cmd,
        &snapshot,
        &p_ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    let Operation::Update { changes, .. } = &out.ops[0] else {
        panic!("expected Update");
    };
    assert_eq!(
        changes.len(),
        2,
        "the inheriting owner keeps both the OwnerOrGm /system/notes change and /base"
    );

    let stranger_ctx = PermissionContext {
        user_id: Uuid::from_u128(999),
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &stranger_ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert!(
        out.ops.is_empty(),
        "a stranger receives no READ on a default:none token, even via the actor join"
    );
}

// ---- write-receive parity + adversarial egress ownership ----

#[tokio::test]
async fn a_document_you_can_write_is_a_document_you_receive() {
    // A document a user can WRITE (the owner floor grants WRITE_FIELDS at
    // `apply_intent`) must also be a document that user RECEIVES at egress
    // (the same owner floor, joined through the same `effective_owner`
    // rule at `filter_command`) — write authz and read authz resolve
    // ownership through one shared join, never two. Reuses the persisted
    // actor+linked-token arrangement from
    // `filter_command_update_keeps_owner_or_gm_changes_for_the_inheriting_owner`.
    use crate::auth::role::ServerRole;
    use crate::data::command::{FieldChange, Operation, WriteOrigin};
    use crate::data::membership::PermissionContext;
    use crate::data::sqlite::SqliteRepository;

    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let p = r.create_user("p", None, ServerRole::User, 0).await.unwrap();

    let mut actor = doc(PermissionSet::default(), serde_json::json!({}));
    actor.id = Uuid::from_u128(42);
    actor.scope = Scope::World { world_id: w.id };
    actor.owner = Some(p);

    let mut token = doc(
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
        serde_json::json!({ "notes": "secret plan" }),
    );
    token.doc_type = "token".into();
    token.id = Uuid::from_u128(100);
    token.scope = Scope::World { world_id: w.id };
    token.engine = Some(serde_json::json!({
        "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0,
        "actor_id": actor.id.to_string()
    }));
    token
        .permissions
        .property_overrides
        .insert("/system/notes".into(), Visibility::OwnerOrGm);

    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![
            Operation::Create { doc: actor.clone() },
            Operation::Create { doc: token.clone() },
        ],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let p_ctx = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };

    // 1. apply_intent as P: patches /system/notes on a `default: None` token
    //    it does not literally own. Must SUCCEED — the owner floor (via the
    //    actor link) grants WRITE_FIELDS.
    let cmd = r
        .apply_intent(
            &p_ctx,
            w.id,
            vec![Operation::Update {
                doc_id: token.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/system/notes".into(),
                    old: serde_json::json!("secret plan"),
                    new: serde_json::json!("new plan"),
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await
        .expect("owner floor grants WRITE_FIELDS: the patch must succeed")
        .command;

    // 2. filter_command of the returned command for P, joined through the
    //    same actor link, must RETAIN the op — the owner floor also grants
    //    READ at egress through the same owner value.
    let current = load_current_docs(&r, &cmd).await;
    let lookup = |id: &Uuid| (id == &actor.id).then_some(&actor);
    let snapshot = immediate_snapshot(&cmd, &current, &[], &lookup);
    let out_p = filter_command(
        &cmd,
        &snapshot,
        &p_ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert_eq!(
        out_p.ops.len(),
        1,
        "the writer must also receive the write it just made"
    );

    // 3. A true stranger receives nothing.
    let stranger_ctx = PermissionContext {
        user_id: Uuid::from_u128(999),
        world_role: WorldRole::Player,
    };
    let out_stranger = filter_command(
        &cmd,
        &snapshot,
        &stranger_ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert!(
        out_stranger.ops.is_empty(),
        "a stranger receives nothing (fail closed)"
    );
}

#[test]
fn egress_ownership_ignores_a_cross_scope_actor() {
    // The scope check in `effective_owner`, exercised through the egress
    // join: a linked actor from a DIFFERENT scope must not be treated as
    // the token's owner at `filter_command`, even though the linked id
    // matches.
    use crate::data::command::{Command, Operation};
    use crate::data::membership::PermissionContext;

    let p = Uuid::from_u128(1);
    let actor_id = Uuid::from_u128(42);
    let mut token = token_linked_to(Some(actor_id));
    token.permissions.default = DocRole::None;
    token.scope = Scope::World {
        world_id: Uuid::from_u128(1000),
    };

    let mut foreign_actor = actor_owned_by(actor_id, Some(p));
    foreign_actor.scope = Scope::World {
        world_id: Uuid::from_u128(2000),
    };

    let cmd = Command {
        seq: 1,
        world_id: Uuid::from_u128(7),
        author: Uuid::from_u128(9),
        ts: 0,
        ops: vec![Operation::Create { doc: token.clone() }],
    };
    let lookup = |id: &Uuid| (id == &foreign_actor.id).then_some(&foreign_actor);
    let current: HashMap<Uuid, CurrentDoc> = HashMap::new();
    let snapshot = immediate_snapshot(&cmd, &current, &[], &lookup);

    let p_ctx = PermissionContext {
        user_id: p,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &p_ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert!(
        out.ops.is_empty(),
        "a cross-scope actor join must not be treated as the owner at egress"
    );
}

#[test]
fn egress_ownership_honors_the_per_token_override() {
    // token.owner = A, linked actor owned by B: the per-token override wins
    // over the actor join, the same precedence the write path
    // (`effective_owner`) uses — A receives, B does not.
    use crate::data::command::{Command, Operation};
    use crate::data::membership::PermissionContext;

    let a = Uuid::from_u128(1);
    let b = Uuid::from_u128(2);
    let actor_id = Uuid::from_u128(42);
    let actor = actor_owned_by(actor_id, Some(b));
    let mut token = token_linked_to(Some(actor_id));
    token.permissions.default = DocRole::None;
    token.owner = Some(a);

    let cmd = Command {
        seq: 1,
        world_id: Uuid::from_u128(7),
        author: Uuid::from_u128(9),
        ts: 0,
        ops: vec![Operation::Create { doc: token.clone() }],
    };
    let lookup = |id: &Uuid| (id == &actor.id).then_some(&actor);
    let current: HashMap<Uuid, CurrentDoc> = HashMap::new();
    let snapshot = immediate_snapshot(&cmd, &current, &[], &lookup);

    let a_ctx = PermissionContext {
        user_id: a,
        world_role: WorldRole::Player,
    };
    let out_a = filter_command(
        &cmd,
        &snapshot,
        &a_ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert_eq!(
        out_a.ops.len(),
        1,
        "the per-token override wins over the actor join: A receives"
    );

    let b_ctx = PermissionContext {
        user_id: b,
        world_role: WorldRole::Player,
    };
    let out_b = filter_command(
        &cmd,
        &snapshot,
        &b_ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert!(
        out_b.ops.is_empty(),
        "the override wins over the actor: B (the linked actor's literal owner) does not receive"
    );
}

#[test]
fn egress_gm_and_gm_role_cap_are_unchanged() {
    // The owner-join plumbing through `filter_command` must not widen the
    // `gm_role` cap: a plain doc still delivers everything to
    // the GM, but a `gm_role: Some(DocRole::None)` doc (message-style,
    // e.g. a whisper) still drops the capped GM's op entirely.
    use crate::data::command::{Command, Operation};
    use crate::data::membership::PermissionContext;

    let gm = Uuid::from_u128(1);
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let current: HashMap<Uuid, CurrentDoc> = HashMap::new();

    // Plain doc: the GM still receives everything unconditionally.
    let plain = doc(PermissionSet::default(), serde_json::json!({ "hp": 10 }));
    let cmd_plain = Command {
        seq: 1,
        world_id: Uuid::from_u128(7),
        author: Uuid::from_u128(9),
        ts: 0,
        ops: vec![Operation::Create { doc: plain.clone() }],
    };
    let snapshot_plain = immediate_snapshot(&cmd_plain, &current, &[gm], &|_| None);
    let out_plain = filter_command(
        &cmd_plain,
        &snapshot_plain,
        &gm_ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert_eq!(
        out_plain.ops.len(),
        1,
        "an uncapped GM still receives everything"
    );

    // `gm_role: Some(DocRole::None)` doc: the capped GM's op is still
    // dropped entirely — the owner plumb must not have widened this cap.
    let mut capped = doc(PermissionSet::default(), serde_json::json!({}));
    capped.permissions.gm_role = Some(DocRole::None);
    let cmd_capped = Command {
        seq: 2,
        world_id: Uuid::from_u128(7),
        author: Uuid::from_u128(9),
        ts: 0,
        ops: vec![Operation::Create {
            doc: capped.clone(),
        }],
    };
    let snapshot_capped = immediate_snapshot(&cmd_capped, &current, &[gm], &|_| None);
    let out_capped = filter_command(
        &cmd_capped,
        &snapshot_capped,
        &gm_ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert!(
        out_capped.ops.is_empty(),
        "a gm_role-capped GM must still be denied — the owner plumb must not widen the cap"
    );
}

#[test]
fn redaction_target_classifies_each_whole_band() {
    // The expectation is a HARDCODED list, never `REDACTABLE_BANDS` itself. Deriving the
    // expected value from the constant under test makes the assertion definitionally true
    // for any array contents — it would stay green if a band were renamed, which is the
    // exact "both paths wrong the same way" shape this suite exists to refuse.
    for band in ["name", "engine", "system", "base"] {
        let pointer = format!("/{band}");
        assert_eq!(
            redaction_target(&pointer),
            Some(RedactionTarget::Band),
            "{pointer} must classify as a whole band"
        );
    }
    // Pins the constant's contents independently, so a band added or renamed fails HERE
    // with a message naming the obligation, rather than silently widening what egress
    // is willing to remove.
    assert_eq!(
        REDACTABLE_BANDS,
        ["name", "engine", "system", "base"],
        "the band list changed: re-audit every redaction call site and this suite"
    );
}

#[test]
fn redaction_target_classifies_within_a_band() {
    for pointer in [
        "/system/hp",
        "/system/a/b/c",
        "/engine/vision",
        "/base/system/hp",
        // An empty middle segment still lands inside the untyped body.
        "/system//hp",
        // An index segment is indistinguishable from an object key named "0" from the
        // pointer alone, so it classifies as `Within` and egress must be able to act on
        // it — narrowing the classifier to refuse index-shaped segments would hide an
        // array element from redaction instead of redacting it.
        "/system/inventory/0",
        "/system/inventory/0/secret",
    ] {
        assert_eq!(
            redaction_target(pointer),
            Some(RedactionTarget::Within),
            "{pointer} must classify as within a band"
        );
    }
}

#[test]
fn redaction_target_refuses_every_structural_envelope_field() {
    // The eleven non-content fields of `Document`. Nothing may redact these: a
    // whole-key strip either substitutes a defaulted value or leaves a shape that
    // cannot deserialize.
    for field in [
        "id",
        "scope",
        "doc_type",
        "schema_version",
        "source",
        "owner",
        "permissions",
        "parent_id",
        "embedded",
        "created_at",
        "updated_at",
    ] {
        assert_eq!(redaction_target(&format!("/{field}")), None, "/{field}");
        assert_eq!(
            redaction_target(&format!("/{field}/anything")),
            None,
            "/{field}/anything"
        );
    }
}

#[test]
fn redaction_target_refuses_permissions_subpaths_lacking_serde_default() {
    // A nested pointer into `permissions` strips a field carrying no serde default,
    // leaving a value that cannot deserialize as a `PermissionSet`.
    for pointer in [
        "/permissions",
        "/permissions/default",
        "/permissions/users",
        "/permissions/property_overrides",
    ] {
        assert_eq!(redaction_target(pointer), None, "{pointer}");
    }
}

#[test]
fn redaction_target_refuses_malformed_and_unknown_pointers() {
    for pointer in [
        "",
        "/",
        "system/hp",
        "/unknown",
        "/systemx",
        "/nameless",
        // A band name followed by a non-separator character is a collision, not a
        // match, for every band the shared prefix path handles — not just `system`.
        "/enginex",
        "/basex",
        // A band name plus a trailing separator leaves an empty residual segment,
        // which the guard refuses rather than treating as `Within`.
        "/system/",
    ] {
        assert_eq!(redaction_target(pointer), None, "{pointer:?}");
    }
}

#[test]
fn name_is_a_leaf_band_with_no_interior() {
    // `/name` is a display string, not a container — mirrors the same rule in
    // `required_cap_for_path`.
    assert_eq!(redaction_target("/name"), Some(RedactionTarget::Band));
    assert_eq!(redaction_target("/name/first"), None);
}

// -------------------------------------------------------------------
// Commit-time snapshot redaction — pure `filter_command` unit tests.
// Hand-built `CommandSnapshot`/`CurrentDoc` inputs; no repository round trip.
// -------------------------------------------------------------------

/// A `CurrentDoc` wrapping `doc` at generation `created_seq`.
fn current_doc(doc: Document, created_seq: i64) -> CurrentDoc {
    CurrentDoc { doc, created_seq }
}

/// An `OpSnapshot` for an `Update` op: commit-time owner/gm/permissions plus a pruned
/// override set, no retraction, no created_seq mismatch (matches the current generation).
fn op_snapshot_update(
    owner_at_commit: Option<Uuid>,
    overrides_at_commit: Vec<(&str, Visibility)>,
    permissions_at_commit: PermissionSet,
) -> OpSnapshot {
    OpSnapshot {
        owner_at_commit,
        doc_type: "actor".into(),
        overrides_at_commit: overrides_at_commit
            .into_iter()
            .map(|(p, v)| (p.to_string(), v))
            .collect(),
        retraction_hidden_at_commit: None,
        created_seq_at_commit: None,
        permissions_at_commit: Some(permissions_at_commit),
        permissions_before_commit: None,
    }
}

fn snapshot_one_op(op: OpSnapshot, world_gm_at_commit: HashMap<Uuid, bool>) -> CommandSnapshot {
    CommandSnapshot {
        per_op: vec![Some(op)],
        world_gm_at_commit,
    }
}

fn permissions_default_observer() -> PermissionSet {
    PermissionSet {
        default: DocRole::Observer,
        ..Default::default()
    }
}

fn field_change_update_cmd(
    world: Uuid,
    author: Uuid,
    doc_id: Uuid,
    path: &str,
    old: serde_json::Value,
    new: serde_json::Value,
) -> Command {
    Command {
        seq: 1,
        world_id: world,
        author,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id,
            changes: vec![FieldChange {
                remove: false,
                path: path.into(),
                old,
                new,
            }],
        }],
    }
}

#[test]
fn filter_command_drops_an_op_with_no_recorded_snapshot() {
    let world = Uuid::from_u128(9);
    let author = Uuid::from_u128(1);
    let doc_id = Uuid::from_u128(2);
    let cmd = field_change_update_cmd(
        world,
        author,
        doc_id,
        "/system/x",
        serde_json::json!(1),
        serde_json::json!(2),
    );
    let snapshot = CommandSnapshot {
        per_op: vec![None],
        world_gm_at_commit: HashMap::new(),
    };
    let cur = doc(
        permissions_default_observer(),
        serde_json::json!({ "x": 2 }),
    );
    let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
    let ctx = PermissionContext {
        user_id: Uuid::from_u128(3),
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert!(
        out.ops.is_empty(),
        "a None op-snapshot must drop the op on replay"
    );
}

#[test]
fn world_role_promotion_does_not_disclose_pre_promotion_gm_only_or_owner_or_gm_history() {
    // A player, hidden from a GmOnly field and a separate OwnerOrGm field while a non-GM
    // non-owner, is later promoted to GM and resyncs from before the promotion — both
    // fields must stay hidden. INVARIANT: `Access::can_see(OwnerOrGm)` is a disjunction
    // (`see_gm_only || is_owner`), so resolving the commit-time half's `see_gm_only` from
    // the recipient's CURRENT world role would defeat `owner_at_commit` for the OwnerOrGm
    // tier too, not just leak the GmOnly field — both must come from the snapshot alone.
    let world = Uuid::from_u128(9);
    let author = Uuid::from_u128(1);
    let doc_id = Uuid::from_u128(2);
    let owner = Uuid::from_u128(10);
    let recipient = Uuid::from_u128(20);
    let cmd = Command {
        seq: 1,
        world_id: world,
        author,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id,
            changes: vec![
                FieldChange {
                    remove: false,
                    path: "/system/secret".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!("gm secret"),
                },
                FieldChange {
                    remove: false,
                    path: "/system/owner_note".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!("owner note"),
                },
            ],
        }],
    };
    let op = op_snapshot_update(
        Some(owner),
        vec![
            ("/system/secret", Visibility::GmOnly),
            ("/system/owner_note", Visibility::OwnerOrGm),
        ],
        permissions_default_observer(),
    );
    // The recipient was NOT GM at commit.
    let snapshot = snapshot_one_op(op, HashMap::from([(recipient, false)]));
    let mut cur = doc(permissions_default_observer(), serde_json::json!({}));
    cur.owner = Some(owner);
    let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
    // The recipient IS currently GM (post-promotion) — this is the defect scenario.
    let ctx = PermissionContext {
        user_id: recipient,
        world_role: WorldRole::Gm,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out.ops[0] else {
        panic!("expected an Update op");
    };
    assert!(
        changes.is_empty(),
        "both the GmOnly and OwnerOrGm fields must stay hidden: {changes:?}"
    );
}

#[test]
fn reused_id_drops_a_stale_update_against_the_new_generation() {
    let world = Uuid::from_u128(9);
    let author = Uuid::from_u128(1);
    let doc_id = Uuid::from_u128(2);
    let cmd = field_change_update_cmd(
        world,
        author,
        doc_id,
        "/system/x",
        serde_json::json!(1),
        serde_json::json!(2),
    );
    let mut op = op_snapshot_update(None, vec![], permissions_default_observer());
    op.created_seq_at_commit = Some(5); // the OLD generation's created_seq
    let snapshot = snapshot_one_op(op, HashMap::new());
    let cur = doc(
        permissions_default_observer(),
        serde_json::json!({ "x": 2 }),
    );
    // The CURRENT document at this id is generation 9 (a later Create reused the id).
    let current = HashMap::from([(doc_id, current_doc(cur, 9))]);
    let ctx = PermissionContext {
        user_id: Uuid::from_u128(3),
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert!(
        out.ops.is_empty(),
        "a created_seq mismatch must drop the stale Update"
    );
}

#[test]
fn cross_op_existence_consistency_drops_an_update_denied_at_create_commit_time() {
    // A recipient denied commit-time access to a document's Create, later granted current
    // access, must ALSO have every subsequent Update to that doc_id dropped by the SAME
    // whole-document gate — not just the Create.
    let world = Uuid::from_u128(9);
    let author = Uuid::from_u128(1);
    let doc_id = Uuid::from_u128(2);
    let recipient = Uuid::from_u128(3);
    let cmd = field_change_update_cmd(
        world,
        author,
        doc_id,
        "/system/x",
        serde_json::json!(1),
        serde_json::json!(2),
    );
    // Commit-time permissions: default = None (nobody without an explicit grant may read).
    let denied_at_commit = PermissionSet {
        default: DocRole::None,
        ..Default::default()
    };
    let op = op_snapshot_update(None, vec![], denied_at_commit);
    let snapshot = snapshot_one_op(op, HashMap::new());
    // Current permissions: default = Observer (now anyone may read) — the asymmetry.
    let cur = doc(
        permissions_default_observer(),
        serde_json::json!({ "x": 2 }),
    );
    let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
    let ctx = PermissionContext {
        user_id: recipient,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert!(
        out.ops.is_empty(),
        "commit-time denial must drop the Update even though current access now permits it"
    );
}

#[test]
fn retraction_uses_the_commands_own_commit_moment_not_whatever_is_live() {
    // (a) A command that narrows visibility, replayed long after a LATER command has
    // narrowed it further — the retraction pass must reflect what the CHOSEN command
    // itself hid at ITS OWN commit, not whatever is live now.
    let world = Uuid::from_u128(9);
    let author = Uuid::from_u128(1);
    let doc_id = Uuid::from_u128(2);
    let recipient = Uuid::from_u128(3);
    let cmd = Command {
        seq: 1,
        world_id: world,
        author,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id,
            changes: vec![FieldChange {
                remove: false,
                path: "/permissions/property_overrides/~1system~1a".into(),
                old: serde_json::Value::Null,
                new: serde_json::json!("gm_only"),
            }],
        }],
    };
    let mut op = op_snapshot_update(None, vec![], permissions_default_observer());
    // This command's OWN narrowing: only "/system/a" became hidden at ITS commit.
    op.retraction_hidden_at_commit = Some(vec![("/system/a".to_string(), Visibility::GmOnly)]);
    let snapshot = snapshot_one_op(op, HashMap::from([(recipient, false)]));
    let cur = doc(
        permissions_default_observer(),
        serde_json::json!({ "a": 1, "b": 2 }),
    );
    let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
    let ctx = PermissionContext {
        user_id: recipient,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out.ops[0] else {
        panic!("expected an Update op");
    };
    assert!(
        changes
            .iter()
            .any(|c| c.path == "/system/a" && c.new.is_null()),
        "retraction must null the field THIS command hid: {changes:?}"
    );
    assert!(
        !changes.iter().any(|c| c.path == "/system/b"),
        "retraction must not touch a field this command never hid: {changes:?}"
    );
}

#[test]
fn retraction_does_not_null_the_owners_own_owner_or_gm_fields() {
    // (b) The SAME retracting command, replayed to the document's own OWNER — the owner's
    // legitimately-visible OwnerOrGm fields must NOT be nulled by retraction.
    let world = Uuid::from_u128(9);
    let author = Uuid::from_u128(1);
    let doc_id = Uuid::from_u128(2);
    let owner = Uuid::from_u128(3);
    let cmd = Command {
        seq: 1,
        world_id: world,
        author,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id,
            changes: vec![FieldChange {
                remove: false,
                path: "/permissions/property_overrides/~1system~1name".into(),
                old: serde_json::Value::Null,
                new: serde_json::json!("owner_or_gm"),
            }],
        }],
    };
    let mut op = op_snapshot_update(Some(owner), vec![], permissions_default_observer());
    op.retraction_hidden_at_commit =
        Some(vec![("/system/name".to_string(), Visibility::OwnerOrGm)]);
    let snapshot = snapshot_one_op(op, HashMap::new());
    let mut cur = doc(
        permissions_default_observer(),
        serde_json::json!({ "name": "PC" }),
    );
    cur.owner = Some(owner);
    let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
    let ctx = PermissionContext {
        user_id: owner,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out.ops[0] else {
        panic!("expected an Update op");
    };
    assert!(
        !changes.iter().any(|c| c.path == "/system/name"),
        "the owner's own OwnerOrGm field must not be retracted: {changes:?}"
    );
}

#[test]
fn multi_op_leak_within_one_command_is_closed_by_the_post_loop_accumulator() {
    // Within ONE command, an Update that sets a secret value followed by an Update that
    // adds a gm_only override on the SAME pointer must have BOTH ops' snapshots reflect the
    // FINAL (post-loop) override tree: BOTH ops' `overrides_at_commit` carry the gm_only
    // override here, even though only the SECOND op is the one that added it. A snapshot
    // built from each op's own per-iteration local state instead of the whole command's
    // final post-image would leave the FIRST op's `overrides_at_commit` empty, and this
    // test would then fail (the secret would leak to a non-GM/non-owner recipient on the
    // first op).
    let world = Uuid::from_u128(9);
    let author = Uuid::from_u128(1);
    let doc_id = Uuid::from_u128(2);
    let recipient = Uuid::from_u128(3);
    let cmd = Command {
        seq: 1,
        world_id: world,
        author,
        ts: 0,
        ops: vec![
            Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/system/secret".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!("X"),
                }],
            },
            Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/permissions/property_overrides/~1system~1secret".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!("gm_only"),
                }],
            },
        ],
    };
    let final_overrides = vec![("/system/secret", Visibility::GmOnly)];
    let op0 = op_snapshot_update(
        None,
        final_overrides.clone(),
        permissions_default_observer(),
    );
    let op1 = op_snapshot_update(None, final_overrides, permissions_default_observer());
    let snapshot = CommandSnapshot {
        per_op: vec![Some(op0), Some(op1)],
        world_gm_at_commit: HashMap::new(),
    };
    let cur = doc(
        permissions_default_observer(),
        serde_json::json!({ "secret": "X" }),
    );
    let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
    let ctx = PermissionContext {
        user_id: recipient,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out.ops[0] else {
        panic!("expected an Update op for the FIRST op");
    };
    assert!(
        changes.is_empty(),
        "the first op's own snapshot must already reflect the LATER op's override: {changes:?}"
    );
}

#[test]
fn behavioural_mutation_current_output_unaffected_by_history_commit_output_unaffected_by_live() {
    // Mutate each live input independently — the target's overrides, its default, the
    // linked actor's owner, an embedded child's index, the recipient's world role — and
    // assert `filter_command`'s CURRENT-time output is unaffected by history and its
    // COMMIT-time output is unaffected by anything live.
    let world = Uuid::from_u128(9);
    let author = Uuid::from_u128(1);
    let doc_id = Uuid::from_u128(2);
    let recipient = Uuid::from_u128(3);
    let cmd = field_change_update_cmd(
        world,
        author,
        doc_id,
        "/system/x",
        serde_json::json!(1),
        serde_json::json!(2),
    );
    // Baseline: nothing hidden at commit, nothing hidden currently.
    let op = op_snapshot_update(None, vec![], permissions_default_observer());
    let snapshot = snapshot_one_op(op, HashMap::from([(recipient, false)]));
    let cur = doc(
        permissions_default_observer(),
        serde_json::json!({ "x": 2 }),
    );
    let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
    let ctx = PermissionContext {
        user_id: recipient,
        world_role: WorldRole::Player,
    };
    let baseline = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &baseline.ops[0] else {
        panic!("expected Update")
    };
    assert_eq!(changes.len(), 1, "baseline: field visible to everyone");

    // Mutate ONLY the live default (current-time) — commit-time snapshot untouched.
    let denied_current = PermissionSet {
        default: DocRole::None,
        ..Default::default()
    };
    let mut cur2 = doc(denied_current, serde_json::json!({ "x": 2 }));
    cur2.owner = None;
    let current2 = HashMap::from([(doc_id, current_doc(cur2, 0))]);
    let out_live_mutated = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current2,
        |_| None,
    );
    assert!(
        out_live_mutated.ops.is_empty(),
        "mutating ONLY the live default must change the CURRENT-time gate outcome"
    );

    // Mutate ONLY the commit-time permissions (recipient denied at commit) — live unchanged.
    let mut op_denied_commit = op_snapshot_update(
        None,
        vec![],
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
    );
    op_denied_commit.doc_type = "actor".into();
    let snapshot_denied_commit =
        snapshot_one_op(op_denied_commit, HashMap::from([(recipient, false)]));
    let out_commit_mutated = filter_command(
        &cmd,
        &snapshot_denied_commit,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert!(
        out_commit_mutated.ops.is_empty(),
        "mutating ONLY the commit-time permissions must change the COMMIT-time gate outcome"
    );
}

#[test]
fn embedded_child_index_in_the_commit_time_snapshot_is_independent_of_the_current_embedded_array() {
    // The commit-time override set is a flat, ALREADY-ADDRESSED pointer list
    // (`OpSnapshot::overrides_at_commit`), never re-derived from the CURRENT document's
    // embedded array. Mutating ONLY the current embedded structure (here: inserting
    // siblings so the commit-time secret child now sits at a different position) must not
    // change what the commit-time half redacts at a pointer the snapshot already names.
    let world = Uuid::from_u128(9);
    let author = Uuid::from_u128(1);
    let doc_id = Uuid::from_u128(2);
    let recipient = Uuid::from_u128(3);
    let cmd = Command {
        seq: 1,
        world_id: world,
        author,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id,
            changes: vec![FieldChange {
                remove: false,
                path: "/embedded/actor/1/system/name".into(),
                old: serde_json::Value::Null,
                new: serde_json::json!("Hidden At Commit"),
            }],
        }],
    };
    let op = op_snapshot_update(
        None,
        vec![("/embedded/actor/1/system/name", Visibility::GmOnly)],
        permissions_default_observer(),
    );
    let snapshot = snapshot_one_op(op, HashMap::from([(recipient, false)]));
    // CURRENT structure: THREE children under "actor" — none carries the override (the
    // override lives only in the snapshot), so hidden_current alone would NOT redact this
    // pointer; only hidden_commit does.
    let mut cur = doc(permissions_default_observer(), serde_json::json!({}));
    cur.embedded.insert(
        "actor".into(),
        vec![
            doc(permissions_default_observer(), serde_json::json!({})),
            doc(permissions_default_observer(), serde_json::json!({})),
            doc(permissions_default_observer(), serde_json::json!({})),
        ],
    );
    let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
    let ctx = PermissionContext {
        user_id: recipient,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out.ops[0] else {
        panic!("expected Update")
    };
    assert!(
        changes.is_empty(),
        "the commit-time snapshot's own recorded pointer must still redact, regardless of \
         what the CURRENT embedded array now holds at that index: {changes:?}"
    );
}

#[test]
fn linked_token_actor_owner_mutation_only_affects_the_current_time_half() {
    // `effective_owner_via` joins the CURRENT actor table via the caller-supplied closure;
    // mutating what it returns changes ONLY the current-time half's ownership resolution,
    // never the commit-time half's (`OpSnapshot::owner_at_commit`, frozen at commit).
    let world = Uuid::from_u128(9);
    let author = Uuid::from_u128(1);
    let token_id = Uuid::from_u128(2);
    let actor_id = Uuid::from_u128(50);
    let recipient = Uuid::from_u128(3);
    let mut perms = permissions_default_observer();
    perms
        .property_overrides
        .insert("/system/name".into(), Visibility::OwnerOrGm);
    let cmd = Command {
        seq: 1,
        world_id: world,
        author,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: token_id,
            changes: vec![FieldChange {
                remove: false,
                path: "/system/name".into(),
                old: serde_json::Value::Null,
                new: serde_json::json!("Owner-visible name"),
            }],
        }],
    };
    // Commit-time: the recipient WAS the effective owner at commit (owner_at_commit is
    // Some(recipient)) — so the commit-time half admits this pointer regardless of what
    // the CURRENT actor_lookup later resolves. Isolates the mutation to the current-time
    // half only: union semantics mean a commit-time admission is never overridden into a
    // reveal, but a current-time denial always adds further hiding on top of it.
    let mut op = op_snapshot_update(
        Some(recipient),
        vec![("/system/name", Visibility::OwnerOrGm)],
        perms.clone(),
    );
    op.doc_type = "token".into();
    let snapshot = snapshot_one_op(op, HashMap::new());
    let mut token_doc = doc(perms, serde_json::json!({ "name": "Token PC" }));
    token_doc.doc_type = "token".into();
    token_doc.engine = Some(serde_json::json!({ "actor_id": actor_id.to_string() }));
    let current = HashMap::from([(token_id, current_doc(token_doc, 0))]);
    let ctx = PermissionContext {
        user_id: recipient,
        world_role: WorldRole::Player,
    };

    // actor_lookup resolves the recipient as the CURRENT linked actor's owner.
    let mut owning_actor = doc(PermissionSet::default(), serde_json::json!({}));
    owning_actor.id = actor_id;
    owning_actor.doc_type = "actor".into();
    owning_actor.owner = Some(recipient);
    let out_owner_now = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |id| {
            if *id == actor_id {
                Some(&owning_actor)
            } else {
                None
            }
        },
    );
    let Operation::Update { changes, .. } = &out_owner_now.ops[0] else {
        panic!("expected Update")
    };
    assert_eq!(
        changes.len(),
        1,
        "current-time ownership (via the actor_lookup closure) must admit OwnerOrGm now: {changes:?}"
    );

    // Same command, same snapshot — actor_lookup now resolves NO owner (mutate ONLY the
    // live input). The commit-time half still admits the pointer (unchanged), but the
    // current-time half now denies it, and denial from EITHER half hides a pointer.
    let out_no_owner = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    let Operation::Update { changes, .. } = &out_no_owner.ops[0] else {
        panic!("expected Update")
    };
    assert!(
        changes.is_empty(),
        "mutating ONLY the actor_lookup closure must change the CURRENT-time outcome: {changes:?}"
    );
}

#[test]
fn traversal_split_produces_byte_identical_output_for_the_same_document() {
    // The shared `(doc, prefix) -> Vec<(String, Visibility)>` traversal used by both the
    // live path (`collect_hidden`, via `hidden_from_overrides`) and snapshot construction
    // must be exactly the traversal `collect_overrides` performs — pinned here so a future
    // change to one cannot silently diverge from the other.
    let child = doc(
        perms_with(&[("/system/name", Visibility::OwnerOrGm)]),
        serde_json::json!({ "name": "Hidden" }),
    );
    let mut parent = doc(
        perms_with(&[("/system/secret", Visibility::GmOnly)]),
        serde_json::json!({ "secret": "S" }),
    );
    parent.embedded.insert("actor".into(), vec![child]);

    let mut overrides = Vec::new();
    collect_overrides(&parent, "", &mut overrides).unwrap();
    let pointers: std::collections::BTreeSet<&str> =
        overrides.iter().map(|(p, _)| p.as_str()).collect();
    assert!(pointers.contains("/system/secret"));
    assert!(pointers.contains("/base"));
    assert!(pointers.contains("/embedded/actor/0/system/name"));
    assert!(pointers.contains("/embedded/actor/0/base"));

    // hidden_from_overrides + collect_overrides together must reproduce collect_hidden's
    // own output exactly, for the same (doc, access).
    let mut via_collect_hidden = Vec::new();
    collect_hidden(&parent, &non_gm(), "", &mut via_collect_hidden).unwrap();
    let via_split = hidden_from_overrides(&overrides, &non_gm());
    let mut a: Vec<&str> = via_collect_hidden.iter().map(String::as_str).collect();
    let mut b: Vec<&str> = via_split.iter().map(String::as_str).collect();
    a.sort();
    b.sort();
    assert_eq!(
        a, b,
        "collect_hidden must equal collect_overrides + hidden_from_overrides"
    );
}

#[test]
fn world_cap_default_grant_rescues_read_at_both_halves_of_the_commit_current_conjunction() {
    // A recipient with NO document-level READ (default: DocRole::None, not owner, not
    // individually listed) but a world `WorldCapDefaults` grant of `cap::READ` for their
    // floored role must still receive the op — at BOTH the commit-time and current-time
    // halves of `filter_command`'s READ conjunction, since world capability GRANTS stay
    // current-only at both halves (never commit-snapshotted).
    let world = Uuid::from_u128(9);
    let author = Uuid::from_u128(1);
    let doc_id = Uuid::from_u128(2);
    let recipient = Uuid::from_u128(20);
    let denied = PermissionSet {
        default: DocRole::None,
        ..Default::default()
    };
    let cmd = field_change_update_cmd(
        world,
        author,
        doc_id,
        "/system/x",
        serde_json::json!(1),
        serde_json::json!(2),
    );
    let op = op_snapshot_update(None, vec![], denied.clone());
    let snapshot = snapshot_one_op(op, HashMap::new());
    let cur = doc(denied, serde_json::json!({ "x": 2 }));
    let current = HashMap::from([(doc_id, current_doc(cur, 0))]);
    let ctx = PermissionContext {
        user_id: recipient,
        world_role: WorldRole::Player,
    };

    let mut world_defaults = WorldCapDefaults::default();
    world_defaults
        .all
        .by_role
        .entry(DocRole::None)
        .or_default()
        .insert(cap::READ.to_string());

    let out_with_grant = filter_command(&cmd, &snapshot, &ctx, &world_defaults, &current, |_| None);
    assert!(
        !out_with_grant.ops.is_empty(),
        "a world-cap-default READ grant must rescue an op the document's own \
         permissions deny, at both the commit-time and current-time halves"
    );

    // Negative control: the same recipient, same document, with NO world grant — the op
    // must still be dropped (confirms the rescue above is attributable to the grant, not
    // a change to the document/recipient setup).
    let out_no_grant = filter_command(
        &cmd,
        &snapshot,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        |_| None,
    );
    assert!(
        out_no_grant.ops.is_empty(),
        "with neither document access nor a world grant, the op must still be dropped"
    );
}

// -------------------------------------------------------------------
// READ-transition synthesis: a permission change that grants or revokes a recipient's
// whole-document READ cannot travel as a field delta — see `filter_command`'s own comment.
// -------------------------------------------------------------------

/// `immediate_snapshot` with every Update op's pre-image permissions set to `before`.
fn snapshot_with_before<'a>(
    cmd: &Command,
    current: &HashMap<Uuid, CurrentDoc>,
    gm_at_commit: &[Uuid],
    actor_lookup: &impl Fn(&Uuid) -> Option<&'a Document>,
    before: PermissionSet,
) -> CommandSnapshot {
    let mut s = immediate_snapshot(cmd, current, gm_at_commit, actor_lookup);
    for op in s.per_op.iter_mut().flatten() {
        op.permissions_before_commit = Some(before.clone());
    }
    s
}

#[tokio::test]
async fn reveal_by_permissions_synthesizes_a_create_for_the_newly_readable_recipient() {
    let player = Uuid::from_u128(1);
    let mut d = doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({ "hp": 3 }),
    );
    d.doc_type = "combatant".into();
    d.engine = crate::data::document::tests::default_test_engine("combatant");
    let cmd = Command {
        seq: 9,
        world_id: world_of(&d).unwrap(),
        author: Uuid::from_u128(99),
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/permissions/default".into(),
                old: serde_json::json!("none"),
                new: serde_json::json!("observer"),
            }],
        }],
    };
    let current = HashMap::from([(
        d.id,
        CurrentDoc {
            doc: d.clone(),
            created_seq: 1,
        },
    )]);
    let lookup = |_: &Uuid| None;
    let snap = snapshot_with_before(
        &cmd,
        &current,
        &[],
        &lookup,
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
    );
    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snap,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert_eq!(out.seq, 9);
    assert_eq!(out.ops.len(), 1);
    match &out.ops[0] {
        Operation::Create { doc } => {
            assert_eq!(doc.id, d.id);
            assert_eq!(doc.doc_type, "combatant");
            assert_eq!(doc.system["hp"], 3);
        }
        other => panic!("expected a synthesized Create, got {other:?}"),
    }
}

#[tokio::test]
async fn hide_by_permissions_synthesizes_a_stub_delete_for_the_recipient_losing_read() {
    let player = Uuid::from_u128(1);
    let mut d = doc(
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
        serde_json::json!({ "hp": 3 }),
    );
    d.doc_type = "combatant".into();
    d.engine = crate::data::document::tests::default_test_engine("combatant");
    d.name = Some("MOCK_NAME_A".into());
    let cmd = Command {
        seq: 10,
        world_id: world_of(&d).unwrap(),
        author: Uuid::from_u128(99),
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/permissions/default".into(),
                old: serde_json::json!("observer"),
                new: serde_json::json!("none"),
            }],
        }],
    };
    let current = HashMap::from([(
        d.id,
        CurrentDoc {
            doc: d.clone(),
            created_seq: 1,
        },
    )]);
    let lookup = |_: &Uuid| None;
    let snap = snapshot_with_before(
        &cmd,
        &current,
        &[],
        &lookup,
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
    );
    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snap,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert_eq!(out.ops.len(), 1);
    match &out.ops[0] {
        Operation::Delete { doc } => {
            assert_eq!(doc.id, d.id);
            assert_eq!(doc.doc_type, "combatant");
            // Stub: nothing the recipient may no longer see rides the Delete.
            assert!(doc.name.is_none());
            assert!(doc.engine.is_none());
            assert_eq!(doc.system, serde_json::json!({}));
            assert!(doc.embedded.is_empty());
            assert_eq!(doc.permissions, PermissionSet::default());
        }
        other => panic!("expected a synthesized Delete, got {other:?}"),
    }
}

#[tokio::test]
async fn a_gm_recipient_sees_the_plain_update_on_hide_and_reveal() {
    let gm = Uuid::from_u128(2);
    let mut d = doc(
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
        serde_json::json!({}),
    );
    d.doc_type = "combatant".into();
    d.engine = crate::data::document::tests::default_test_engine("combatant");
    let cmd = Command {
        seq: 11,
        world_id: world_of(&d).unwrap(),
        author: gm,
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/permissions/default".into(),
                old: serde_json::json!("observer"),
                new: serde_json::json!("none"),
            }],
        }],
    };
    let current = HashMap::from([(
        d.id,
        CurrentDoc {
            doc: d.clone(),
            created_seq: 1,
        },
    )]);
    let lookup = |_: &Uuid| None;
    let snap = snapshot_with_before(
        &cmd,
        &current,
        &[gm],
        &lookup,
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
    );
    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let out = filter_command(
        &cmd,
        &snap,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert!(matches!(&out.ops[0], Operation::Update { .. }));
}

#[tokio::test]
async fn a_reveal_is_not_synthesized_when_current_access_denies_read() {
    // Revealed at commit, hidden again later: the replayed earlier command must not
    // hand the recipient a document they may not currently see.
    let player = Uuid::from_u128(1);
    let mut d = doc(
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
        serde_json::json!({}),
    );
    d.doc_type = "combatant".into();
    d.engine = crate::data::document::tests::default_test_engine("combatant");
    let cmd = Command {
        seq: 12,
        world_id: world_of(&d).unwrap(),
        author: Uuid::from_u128(99),
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/permissions/default".into(),
                old: serde_json::json!("none"),
                new: serde_json::json!("observer"),
            }],
        }],
    };
    let current = HashMap::from([(
        d.id,
        CurrentDoc {
            doc: d.clone(),
            created_seq: 1,
        },
    )]);
    let lookup = |_: &Uuid| None;
    let mut snap = snapshot_with_before(
        &cmd,
        &current,
        &[],
        &lookup,
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
    );
    // Commit-time permissions were the revealing post-image (observer), current is none.
    for op in snap.per_op.iter_mut().flatten() {
        op.permissions_at_commit = Some(PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        });
    }
    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snap,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert!(out.ops.is_empty());
}

#[tokio::test]
async fn a_legacy_snapshot_without_before_permissions_keeps_the_old_behaviour() {
    let player = Uuid::from_u128(1);
    let mut d = doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({}),
    );
    d.doc_type = "combatant".into();
    d.engine = crate::data::document::tests::default_test_engine("combatant");
    let cmd = Command {
        seq: 13,
        world_id: world_of(&d).unwrap(),
        author: Uuid::from_u128(99),
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/permissions/default".into(),
                old: serde_json::json!("none"),
                new: serde_json::json!("observer"),
            }],
        }],
    };
    let current = HashMap::from([(
        d.id,
        CurrentDoc {
            doc: d.clone(),
            created_seq: 1,
        },
    )]);
    let lookup = |_: &Uuid| None;
    let snap = immediate_snapshot(&cmd, &current, &[], &lookup); // permissions_before_commit: None
    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snap,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert!(matches!(&out.ops[0], Operation::Update { .. }));
}

#[tokio::test]
async fn a_same_op_owner_and_permissions_change_still_reveals_to_the_new_owner() {
    // `OpSnapshot::owner_at_commit` carries only the POST-image owner; an Update that
    // changes `/owner` and `/permissions/default` in the same op resolves `access_before`
    // against the new owner rather than the true pre-image owner. That is the safe
    // direction: the post-image owner is who must now see the document, so a Create still
    // synthesizes correctly for them.
    let new_owner = Uuid::from_u128(1);
    // `d` carries the POST-image permissions (matching this Update's `new` values), the
    // same convention `reveal_by_permissions_synthesizes_a_create_for_the_newly_readable_recipient`
    // uses: `immediate_snapshot` derives `permissions_at_commit` from `d` itself.
    let mut d = doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({ "hp": 3 }),
    );
    d.doc_type = "combatant".into();
    d.engine = crate::data::document::tests::default_test_engine("combatant");
    d.owner = Some(new_owner);
    let cmd = Command {
        seq: 14,
        world_id: world_of(&d).unwrap(),
        author: Uuid::from_u128(99),
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: d.id,
            changes: vec![
                FieldChange {
                    remove: false,
                    path: "/owner".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!(new_owner),
                },
                FieldChange {
                    remove: false,
                    path: "/permissions/default".into(),
                    old: serde_json::json!("none"),
                    new: serde_json::json!("observer"),
                },
            ],
        }],
    };
    let current = HashMap::from([(
        d.id,
        CurrentDoc {
            doc: d.clone(),
            created_seq: 1,
        },
    )]);
    let lookup = |_: &Uuid| None;
    let mut snap = snapshot_with_before(
        &cmd,
        &current,
        &[],
        &lookup,
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
    );
    for op in snap.per_op.iter_mut().flatten() {
        op.owner_at_commit = Some(new_owner);
    }
    let ctx = PermissionContext {
        user_id: new_owner,
        world_role: WorldRole::Player,
    };
    let out = filter_command(
        &cmd,
        &snap,
        &ctx,
        &WorldCapDefaults::default(),
        &current,
        lookup,
    );
    assert_eq!(out.ops.len(), 1);
    assert!(matches!(&out.ops[0], Operation::Create { .. }));
}
