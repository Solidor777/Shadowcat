use super::*;
use crate::data::document::{PermissionSet, Scope};

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
        user_id: Some(uuid::Uuid::from_u128(0xB)),
    };
    assert!(filter_properties(&d, &gm).is_ok());
}

#[test]
fn owner_or_gm_visible_to_owner_and_gm_not_other_player() {
    let owner = crate::test_ids::test_uuid(1);
    let other = crate::test_ids::test_uuid(2);
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
    let owner = crate::test_ids::test_uuid(1);
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
    let owner = crate::test_ids::test_uuid(1);
    let other = crate::test_ids::test_uuid(2);
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
    let me = crate::test_ids::test_uuid(1);
    let other = crate::test_ids::test_uuid(2);
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
        crate::test_ids::test_uuid(5),
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
    perms
        .users
        .insert(crate::test_ids::test_uuid(1), DocRole::Owner);
    perms
        .users
        .insert(crate::test_ids::test_uuid(2), DocRole::Observer);
    let d = doc(perms, serde_json::json!({}));
    // Owner: read + write fields, but NOT manage embedded by default.
    let owner = resolve_access(
        crate::test_ids::test_uuid(1),
        WorldRole::Player,
        &d,
        d.owner,
    );
    assert!(owner.has(cap::READ) && owner.has(cap::WRITE_FIELDS));
    assert!(!owner.has(cap::MANAGE_EMBEDDED) && !owner.has(cap::DELETE));
    // Observer: read only.
    let obs = resolve_access(
        crate::test_ids::test_uuid(2),
        WorldRole::Player,
        &d,
        d.owner,
    );
    assert!(obs.has(cap::READ) && !obs.has(cap::WRITE_FIELDS));
    // Stranger falls to default (None): nothing.
    let other = resolve_access(
        crate::test_ids::test_uuid(3),
        WorldRole::Player,
        &d,
        d.owner,
    );
    assert!(!other.has(cap::READ));
}

#[test]
fn additive_grants_widen_the_floor() {
    use crate::data::document::CapabilityGrants;
    let mut perms = PermissionSet::default();
    perms
        .users
        .insert(crate::test_ids::test_uuid(1), DocRole::Owner);
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
        .entry(crate::test_ids::test_uuid(1))
        .or_default()
        .insert("dnd5e:cast".to_string());
    perms.capabilities = grants;
    let d = doc(perms, serde_json::json!({}));
    let a = resolve_access(
        crate::test_ids::test_uuid(1),
        WorldRole::Player,
        &d,
        d.owner,
    );
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

    let player = resolve_access(
        crate::test_ids::test_uuid(7),
        WorldRole::Player,
        &d,
        d.owner,
    );
    let view = filter_properties(&d, &player).unwrap();
    assert_eq!(view.system.get("secret"), None);
    assert_eq!(view.system["public"], serde_json::json!(1));

    let gm = resolve_access(crate::test_ids::test_uuid(7), WorldRole::Gm, &d, d.owner);
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

    let player = resolve_access(
        crate::test_ids::test_uuid(7),
        WorldRole::Player,
        &d,
        d.owner,
    );
    let view = filter_properties(&d, &player).unwrap();
    assert_eq!(view.system, serde_json::Value::Null);

    let gm = resolve_access(crate::test_ids::test_uuid(7), WorldRole::Gm, &d, d.owner);
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

    let player = resolve_access(
        crate::test_ids::test_uuid(7),
        WorldRole::Player,
        &d,
        d.owner,
    );
    let view = filter_properties(&d, &player).unwrap();
    assert_eq!(view.engine, None);

    let gm = resolve_access(crate::test_ids::test_uuid(7), WorldRole::Gm, &d, d.owner);
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

    let player = resolve_access(
        crate::test_ids::test_uuid(7),
        WorldRole::Player,
        &d,
        d.owner,
    );
    let view = filter_properties(&d, &player).unwrap();
    assert!(view.engine.as_ref().unwrap().get("vision").is_none());
    assert_eq!(view.engine.as_ref().unwrap()["visionmode"], "dark");

    let gm = resolve_access(crate::test_ids::test_uuid(7), WorldRole::Gm, &d, d.owner);
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

    let player = resolve_access(
        crate::test_ids::test_uuid(7),
        WorldRole::Player,
        &d,
        d.owner,
    );
    let view = filter_properties(&d, &player).unwrap();
    assert_eq!(
        view.system["inventory"],
        serde_json::json!([null, "visible"]),
        "the hidden element must be nulled without shifting its siblings"
    );

    let gm = resolve_access(crate::test_ids::test_uuid(7), WorldRole::Gm, &d, d.owner);
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

    let player = resolve_access(
        crate::test_ids::test_uuid(7),
        WorldRole::Player,
        &d,
        d.owner,
    );
    let view = filter_properties(&d, &player).unwrap();
    assert_eq!(
        view.system["inventory"],
        serde_json::json!([{ "public": 1 }, { "secret": "MOCK_SECRET_B" }]),
        "only the pointed-at key is removed; the sibling element is untouched"
    );

    let gm = resolve_access(crate::test_ids::test_uuid(7), WorldRole::Gm, &d, d.owner);
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

    let gm = resolve_access(crate::test_ids::test_uuid(7), WorldRole::Gm, &d, d.owner);
    assert_eq!(filter_properties(&d, &gm).unwrap(), d);
}

#[test]
fn owner_or_gm_name_visible_to_owner_and_gm_not_other_player() {
    // `/name` mirrors the `/system/name` OwnerOrGm tier: an owner and the
    // GM see it; another player is redacted to `null` (not stripped, since
    // `name` is a top-level `Option` envelope field).
    let owner = crate::test_ids::test_uuid(1);
    let other = crate::test_ids::test_uuid(2);
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

    let player = resolve_access(
        crate::test_ids::test_uuid(7),
        WorldRole::Player,
        &d,
        d.owner,
    );
    assert_eq!(filter_properties(&d, &player).unwrap().name, None);

    let gm = resolve_access(crate::test_ids::test_uuid(7), WorldRole::Gm, &d, d.owner);
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
    let owner = crate::test_ids::test_uuid(1);
    let other = crate::test_ids::test_uuid(2);
    let mut d = doc(PermissionSet::default(), serde_json::json!({ "hp": 10 }));
    d.owner = Some(owner);
    d.base = Some(serde_json::json!({
        "name": "Goblin", "engine": null, "system": { "hp": 10 }, "embedded": {},
        "property_overrides": {}, "owner_standing": "owner",
    }));

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

    r.add_member(w.id, owner, WorldRole::Player).await.unwrap();

    // The template the instance is stamped from — readable by, and owned by,
    // `owner`, so the derived `owner_standing` is `Owner` and `/base` is not
    // hidden from the owner by the standing gate alone.
    let mut template = doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({ "hp": 10 }),
    );
    template.id = crate::test_ids::test_uuid(55);
    template.scope = Scope::World { world_id: w.id };
    template.owner = Some(owner);
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create {
            doc: template.clone(),
        }],
        1,
        WriteOrigin::Client,
    )
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
    // A stamped instance: the Create write path derives `base` from the
    // document's own bands (`merge::bands::derive_create_base`), so the
    // stored row genuinely carries a snapshot for the redaction below to
    // gate.
    d.source = Some(crate::data::document::Source {
        id: crate::test_ids::test_uuid(55),
        pack: None,
        version: 1,
    });
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create { doc: d.clone() }],
        2,
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
        user_id: crate::test_ids::test_uuid(77),
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

    let player = resolve_access(
        crate::test_ids::test_uuid(7),
        WorldRole::Player,
        &parent,
        parent.owner,
    );
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

    let player = resolve_access(
        crate::test_ids::test_uuid(7),
        WorldRole::Player,
        &parent,
        parent.owner,
    );
    let view = filter_properties(&parent, &player).unwrap();
    let child_view = &view.embedded.get("items").unwrap()[0];
    assert_eq!(
        child_view.system.get("secret"),
        None,
        "child gm-only stripped"
    );
    assert_eq!(child_view.system["shown"], serde_json::json!(2));

    // The GM sees the embedded child's gm-only field.
    let gm = resolve_access(
        crate::test_ids::test_uuid(7),
        WorldRole::Gm,
        &parent,
        parent.owner,
    );
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
        user_id: crate::test_ids::test_uuid(77),
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
        user_id: crate::test_ids::test_uuid(77),
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
        user_id: crate::test_ids::test_uuid(77),
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
        user_id: crate::test_ids::test_uuid(77),
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
        user_id: crate::test_ids::test_uuid(77),
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
        user_id: crate::test_ids::test_uuid(77),
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
        user_id: crate::test_ids::test_uuid(77),
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
        user_id: crate::test_ids::test_uuid(77),
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
        user_id: crate::test_ids::test_uuid(77),
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
    let owner = crate::test_ids::test_uuid(1);
    let gm = crate::test_ids::test_uuid(2);
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
    let owner = crate::test_ids::test_uuid(1);
    let gm = crate::test_ids::test_uuid(2);
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
    let owner = crate::test_ids::test_uuid(1);
    let gm = crate::test_ids::test_uuid(2);
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
    let owner = crate::test_ids::test_uuid(1);
    let gm = crate::test_ids::test_uuid(2);
    let stranger = crate::test_ids::test_uuid(3);
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
    let owner = crate::test_ids::test_uuid(1);
    let gm = crate::test_ids::test_uuid(2);
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
        user_id: crate::test_ids::test_uuid(77),
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
