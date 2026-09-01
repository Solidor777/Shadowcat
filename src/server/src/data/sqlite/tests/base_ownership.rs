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
    let expected =
        serde_json::to_value(crate::merge::snapshot_base(&stored)).expect("MergeBase serializes");
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
    // A row predating the base walk: its snapshot holds an engine shape that
    // is invalid under the doc's CURRENT schema. Seed it raw
    // (`seed_document_unvalidated` bypasses every ingress gate by design).
    let mut doc = world_doc(1, world, serde_json::json!({}));
    doc.doc_type = "wall".into();
    doc.engine = crate::data::document::tests::default_test_engine("wall");
    doc.source = Some(Source {
        id: Uuid::from_u128(900),
        pack: None,
        version: 1,
    });
    doc.base = Some(serde_json::json!({
        "name": "Old",
        "engine": { "seg": { "x1": "not-a-number" } },
        "system": {},
        "embedded": {}
    }));
    r.seed_document_unvalidated(&doc).await.unwrap();

    // Reads never validate: the stale snapshot comes back verbatim.
    let loaded = r.get_document(Uuid::from_u128(1)).await.unwrap().unwrap();
    assert_eq!(loaded.base, doc.base);

    // Rewriting the row re-validates the post-image — and fails closed.
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
        matches!(err, DataError::BadEngine(_)),
        "a stale-schema base fails re-validation on rewrite, got {err:?}"
    );
}
