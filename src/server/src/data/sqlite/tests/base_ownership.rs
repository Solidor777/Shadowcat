//! `Document.base` server ownership at the write path: Create-time
//! derivation (`merge::bands::derive_create_base`), the client `/base`
//! write rejection, `WriteOrigin::TemplateMerge`'s apply-level gating, and
//! legacy-row read tolerance (validation is ingest-time only).

use super::*;
use crate::data::membership::PermissionContext;

/// A fresh repo + world + GM context; every test in this file writes as the GM.
async fn gm_setup() -> (SqliteRepository, Uuid, PermissionContext) {
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    (r, w.id, ctx)
}

/// A complete-but-WRONG snapshot a client might submit on Create: the
/// derivation must discard it wholesale in favour of the document's own
/// bands.
fn forged_base() -> serde_json::Value {
    serde_json::json!({
        "name": "FORGED", "engine": null, "system": { "hp": 999 }, "embedded": {}
    })
}

#[tokio::test]
async fn create_derives_base_for_an_instance_discarding_a_client_supplied_value() {
    let (r, world, gm_ctx) = gm_setup().await;
    let mut doc = world_doc(1, world, serde_json::json!({ "hp": 7 }));
    doc.name = Some("Goblin".into());
    doc.source = Some(Source {
        id: Uuid::from_u128(900),
        pack: None,
        version: 1,
    });
    doc.base = Some(forged_base());
    r.apply_intent(
        &gm_ctx,
        world,
        vec![Operation::Create { doc }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let stored = r.get_document(Uuid::from_u128(1)).await.unwrap().unwrap();
    let base = stored.base.clone().expect("an instance carries a base");
    // The stored value is the SERVER's snapshot of the document's own
    // (validated, normalized) bands — nothing of the forged value survives.
    // The named template (900) does not exist, so the derived standing is
    // `Stranger` (`permission::owner_standing` fails closed with no template
    // to resolve READ against).
    let expected = serde_json::to_value(crate::merge::StoredBase {
        snapshot: crate::merge::snapshot_base(&stored),
        owner_standing: crate::data::document::OwnerStanding::Stranger,
    })
    .expect("StoredBase serializes");
    assert_eq!(base, expected);
    assert_eq!(base["name"], serde_json::json!("Goblin"));
    assert_eq!(base["system"], serde_json::json!({ "hp": 7 }));
}

#[tokio::test]
async fn create_without_source_stores_no_base() {
    let (r, world, gm_ctx) = gm_setup().await;
    let mut doc = world_doc(1, world, serde_json::json!({}));
    doc.base = Some(forged_base());
    r.apply_intent(
        &gm_ctx,
        world,
        vec![Operation::Create { doc }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let stored = r.get_document(Uuid::from_u128(1)).await.unwrap().unwrap();
    assert!(stored.base.is_none(), "a non-instance never carries a base");
}

#[tokio::test]
async fn create_clears_base_on_embedded_children_recursively() {
    let (r, world, gm_ctx) = gm_setup().await;
    let mut doc = world_doc(1, world, serde_json::json!({}));
    doc.source = Some(Source {
        id: Uuid::from_u128(900),
        pack: None,
        version: 1,
    });
    let mut child = world_doc(2, world, serde_json::json!({}));
    child.base = Some(forged_base());
    let mut grandchild = world_doc(3, world, serde_json::json!({}));
    grandchild.base = Some(forged_base());
    child.embedded.insert("nested".into(), vec![grandchild]);
    doc.embedded.insert("items".into(), vec![child]);
    r.apply_intent(
        &gm_ctx,
        world,
        vec![Operation::Create { doc }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let stored = r.get_document(Uuid::from_u128(1)).await.unwrap().unwrap();
    let child = &stored.embedded["items"][0];
    assert!(
        child.base.is_none(),
        "an embedded child never carries a base"
    );
    assert!(
        child.embedded["nested"][0].base.is_none(),
        "base is cleared at every embedded depth"
    );
    // The root's derived snapshot keys the child record by the child's own
    // id (no `source` on the child — `snapshot_base`'s fallback keying).
    let base = stored.base.clone().unwrap();
    assert_eq!(
        base["embedded"]["items"][0]["sourceId"],
        serde_json::json!(child.id.to_string())
    );
}

#[tokio::test]
async fn client_update_to_base_is_forbidden_even_for_a_gm() {
    let (r, world, gm_ctx) = gm_setup().await;
    let doc = world_doc(1, world, serde_json::json!({}));
    r.apply_intent(
        &gm_ctx,
        world,
        vec![Operation::Create { doc }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let err = r
        .apply_intent(
            &gm_ctx,
            world,
            vec![Operation::Update {
                doc_id: Uuid::from_u128(1),
                changes: vec![FieldChange {
                    remove: false,
                    path: "/base".into(),
                    old: serde_json::Value::Null,
                    new: forged_base(),
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DataError::Forbidden),
        "/base maps to no capability: rejected for every client origin, got {err:?}"
    );
}

/// An `actor`-typed instance created through the ordinary client write path,
/// returned with its stored post-image (the derived base included).
async fn seeded_instance(
    r: &SqliteRepository,
    world: Uuid,
    gm_ctx: &PermissionContext,
) -> Document {
    let mut doc = world_doc(1, world, serde_json::json!({ "hp": 7 }));
    doc.name = Some("Goblin".into());
    doc.source = Some(Source {
        id: Uuid::from_u128(900),
        pack: None,
        version: 1,
    });
    r.apply_intent(
        gm_ctx,
        world,
        vec![Operation::Create { doc }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    r.get_document(Uuid::from_u128(1)).await.unwrap().unwrap()
}

#[tokio::test]
async fn template_merge_origin_writes_base_while_occ_still_applies() {
    let (r, world, gm_ctx) = gm_setup().await;
    let stored = seeded_instance(&r, world, &gm_ctx).await;
    let base = stored.base.clone().unwrap();

    // A write the client origin may not perform succeeds under
    // `WriteOrigin::TemplateMerge` with an honest OCC pre-image.
    let mut new_base = base.clone();
    new_base["system"] = serde_json::json!({ "hp": 8 });
    r.apply_intent(
        &gm_ctx,
        world,
        vec![Operation::Update {
            doc_id: stored.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/base".into(),
                old: base.clone(),
                new: new_base.clone(),
            }],
        }],
        2,
        WriteOrigin::TemplateMerge,
    )
    .await
    .unwrap();
    let after = r.get_document(stored.id).await.unwrap().unwrap();
    assert_eq!(after.base.unwrap(), new_base);

    // OCC is NOT waived for the origin: a stale pre-image conflicts.
    let err = r
        .apply_intent(
            &gm_ctx,
            world,
            vec![Operation::Update {
                doc_id: stored.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/base".into(),
                    old: base,
                    new: new_base,
                }],
            }],
            3,
            WriteOrigin::TemplateMerge,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::Conflict(_)), "got {err:?}");

    // `/base/...` sub-paths stay rejected even under the merge origin:
    // merge emission is whole-band only.
    let current = r.get_document(stored.id).await.unwrap().unwrap();
    let err = r
        .apply_intent(
            &gm_ctx,
            world,
            vec![Operation::Update {
                doc_id: stored.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/base/system".into(),
                    old: current.base.clone().unwrap()["system"].clone(),
                    new: serde_json::json!({ "hp": 9 }),
                }],
            }],
            4,
            WriteOrigin::TemplateMerge,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::Forbidden), "got {err:?}");
}

#[tokio::test]
async fn template_merge_origin_still_runs_engine_and_scope_checks() {
    let (r, world, gm_ctx) = gm_setup().await;
    let stored = seeded_instance(&r, world, &gm_ctx).await;

    // Engine validation is not waived: an invalid `/engine` post-image fails
    // under the merge origin exactly as under any other.
    let err = r
        .apply_intent(
            &gm_ctx,
            world,
            vec![Operation::Update {
                doc_id: stored.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/engine".into(),
                    old: stored.engine.clone().unwrap(),
                    new: serde_json::json!({ "bogus": true }),
                }],
            }],
            2,
            WriteOrigin::TemplateMerge,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::BadEngine(_)), "got {err:?}");

    // Scope is not waived either: the document lives in another world.
    let other_world = r
        .create_world_owned("Other", gm_ctx.user_id, 0)
        .await
        .unwrap();
    let current = r.get_document(stored.id).await.unwrap().unwrap();
    let err = r
        .apply_intent(
            &gm_ctx,
            other_world.id,
            vec![Operation::Update {
                doc_id: stored.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/name".into(),
                    old: current
                        .name
                        .as_deref()
                        .map_or(serde_json::Value::Null, |n| {
                            serde_json::Value::String(n.to_string())
                        }),
                    new: serde_json::json!("Renamed"),
                }],
            }],
            3,
            WriteOrigin::TemplateMerge,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::OpFailed(_)), "got {err:?}");
}

#[tokio::test]
async fn legacy_row_with_a_stale_schema_base_still_reads() {
    let (r, world, gm_ctx) = gm_setup().await;
    // A row predating both the base walk and the owner-standing key: its
    // snapshot holds an engine shape invalid under the doc's CURRENT schema
    // AND lacks `owner_standing`. Seed it raw (`seed_document_unvalidated`
    // bypasses every ingress gate by design).
    let mut doc = world_doc(1, world, serde_json::json!({}));
    doc.doc_type = "wall".into();
    doc.engine = crate::data::document::tests::default_test_engine("wall");
    doc.source = Some(Source {
        id: Uuid::from_u128(900),
        pack: None,
        version: 1,
    });
    // Missing `owner_standing` too — this row genuinely predates that key
    // (`check_base_node_shape`'s requirement), not a fixture papered over to
    // dodge it.
    doc.base = Some(serde_json::json!({
        "name": "Old",
        "engine": { "seg": { "x1": "not-a-number" } },
        "system": {},
        "embedded": {},
        "property_overrides": {}
    }));
    r.seed_document_unvalidated(&doc).await.unwrap();

    // Reads never validate: the stale snapshot comes back verbatim.
    let loaded = r.get_document(Uuid::from_u128(1)).await.unwrap().unwrap();
    assert_eq!(loaded.base, doc.base);

    // Rewriting the row re-validates the post-image — and fails closed on
    // the FIRST shape defect `check_base_node_shape` finds: the missing
    // `owner_standing` key, before ever reaching the stale `engine` content
    // the fixture also carries. Asserting the actual first failure (rather
    // than adding `owner_standing` to reach a deeper `BadEngine` failure)
    // keeps this test honest about what a genuinely legacy row hits.
    let err = r
        .apply_intent(
            &gm_ctx,
            world,
            vec![Operation::Update {
                doc_id: Uuid::from_u128(1),
                changes: vec![FieldChange {
                    remove: false,
                    path: "/name".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!("New"),
                }],
            }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, DataError::SchemaViolation { pointer, reason }
            if pointer == "/base" && reason.contains("owner_standing")),
        "a stale-schema base fails re-validation on rewrite, got {err:?}"
    );
}

/// A doc with one embedded item, created through the ordinary client write
/// path (whose Create arm strips any child `base`). Returns the stored root.
async fn seeded_doc_with_embedded_item(
    r: &SqliteRepository,
    world: Uuid,
    gm_ctx: &PermissionContext,
) -> Document {
    let mut doc = world_doc(1, world, serde_json::json!({}));
    let child = world_doc(2, world, serde_json::json!({ "qty": 1 }));
    doc.embedded.insert("items".into(), vec![child]);
    r.apply_intent(
        gm_ctx,
        world,
        vec![Operation::Create { doc }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    r.get_document(Uuid::from_u128(1)).await.unwrap().unwrap()
}

/// A client-origin Update that would leave a `base` on an embedded child —
/// here a direct `/embedded/items/0/base` leaf write — is rejected fail-closed
/// even for a GM: the capability gate passes (`/embedded` is writable), the
/// POST-IMAGE check refuses.
#[tokio::test]
async fn client_update_cannot_write_base_onto_an_embedded_child_leaf() {
    let (r, world, gm_ctx) = gm_setup().await;
    let stored = seeded_doc_with_embedded_item(&r, world, &gm_ctx).await;
    assert!(stored.embedded["items"][0].base.is_none());

    let err = r
        .apply_intent(
            &gm_ctx,
            world,
            vec![Operation::Update {
                doc_id: stored.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/embedded/items/0/base".into(),
                    old: serde_json::Value::Null,
                    new: forged_base(),
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DataError::Forbidden),
        "an embedded child never carries a base, got {err:?}"
    );
    assert!(
        r.get_document(stored.id).await.unwrap().unwrap().embedded["items"][0]
            .base
            .is_none(),
        "nothing was written"
    );
}

/// The wholesale form of the same smuggle: a client-origin whole-collection
/// `/embedded/items` replacement carrying a base-bearing child is rejected on
/// the post-image, same as the leaf write.
#[tokio::test]
async fn client_update_cannot_replace_a_collection_with_a_base_bearing_child() {
    let (r, world, gm_ctx) = gm_setup().await;
    let stored = seeded_doc_with_embedded_item(&r, world, &gm_ctx).await;
    let before = serde_json::to_value(&stored.embedded["items"]).unwrap();
    let mut smuggled = before.clone();
    smuggled[0]["base"] = forged_base();

    let err = r
        .apply_intent(
            &gm_ctx,
            world,
            vec![Operation::Update {
                doc_id: stored.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/embedded/items".into(),
                    old: before,
                    new: smuggled,
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DataError::Forbidden),
        "a whole-collection replacement may not smuggle a child base, got {err:?}"
    );
}

/// Control: ordinary client embedded edits — none of which carry a `base` —
/// pass the same post-image check untouched.
#[tokio::test]
async fn client_update_of_an_embedded_child_without_base_passes() {
    let (r, world, gm_ctx) = gm_setup().await;
    let stored = seeded_doc_with_embedded_item(&r, world, &gm_ctx).await;

    r.apply_intent(
        &gm_ctx,
        world,
        vec![Operation::Update {
            doc_id: stored.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/embedded/items/0/system".into(),
                old: serde_json::json!({ "qty": 1 }),
                new: serde_json::json!({ "qty": 2 }),
            }],
        }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let after = r.get_document(stored.id).await.unwrap().unwrap();
    assert_eq!(
        after.embedded["items"][0].system,
        serde_json::json!({ "qty": 2 })
    );
    assert!(after.embedded["items"][0].base.is_none());
}

#[tokio::test]
async fn create_propagates_the_templates_policy_and_records_it_in_the_base() {
    // The stamp arrives from a client with no overrides at all; the write
    // path loads the template, lands its content-band policy on the new
    // instance (so a later merge that moves the hidden value lands it
    // hidden) and records that policy in the derived base.
    let (r, world, gm_ctx) = gm_setup().await;
    let mut template = world_doc(1, world, serde_json::json!({ "hp": 7, "secret": "S" }));
    template.permissions.property_overrides.insert(
        "/system/secret".into(),
        crate::data::document::Visibility::GmOnly,
    );
    r.apply_intent(
        &gm_ctx,
        world,
        vec![Operation::Create { doc: template }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut instance = world_doc(2, world, serde_json::json!({ "hp": 7 }));
    instance.source = Some(Source {
        id: Uuid::from_u128(1),
        pack: None,
        version: 1,
    });
    r.apply_intent(
        &gm_ctx,
        world,
        vec![Operation::Create { doc: instance }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let stored = r.get_document(Uuid::from_u128(2)).await.unwrap().unwrap();
    assert_eq!(
        stored.permissions.property_overrides.get("/system/secret"),
        Some(&crate::data::document::Visibility::GmOnly),
        "the template's policy propagated onto the instance"
    );
    let base = stored.base.expect("an instance carries a base");
    assert_eq!(
        base["property_overrides"]["/system/secret"],
        serde_json::json!("gm_only"),
        "and is recorded in the snapshot"
    );
    assert_eq!(base["system"], serde_json::json!({ "hp": 7 }));

    // A stamp whose template is not loadable propagates nothing and still
    // derives a base recording its own (empty) policy.
    let mut orphan = world_doc(3, world, serde_json::json!({ "hp": 1 }));
    orphan.source = Some(Source {
        id: Uuid::from_u128(900),
        pack: None,
        version: 1,
    });
    r.apply_intent(
        &gm_ctx,
        world,
        vec![Operation::Create { doc: orphan }],
        3,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let stored = r.get_document(Uuid::from_u128(3)).await.unwrap().unwrap();
    assert!(stored.permissions.property_overrides.is_empty());
    assert_eq!(
        stored.base.unwrap()["property_overrides"],
        serde_json::json!({})
    );
}
