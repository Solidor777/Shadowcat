//! The sandboxed-validator chokepoint: `apply_intent`'s pre-transaction pass
//! over the `system` band (opt-in per world per module), its interaction with
//! Phase 1's structural validators and the OCC pre-image check, and the
//! `apply_command` exemption.

use super::*;

// ---------- sandboxed-validator chokepoint (the pre-transaction pass) ----------

/// Writes one installed-module folder under `dir/<id>/` declaring a validator
/// for `doc_type`, compiled from `wat_src` at test time.
fn write_validator_module(dir: &std::path::Path, id: &str, doc_type: &str, wat_src: &str) {
    let module_dir = dir.join(id);
    std::fs::create_dir_all(&module_dir).unwrap();
    std::fs::write(
        module_dir.join("module.json"),
        serde_json::json!({
            "id": id,
            "version": "1.0.0",
            "engines": { "shadowcat": "*" },
            "validators": [{ "docType": doc_type, "wasm": "v.wasm" }],
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        module_dir.join("v.wasm"),
        wat::parse_str(wat_src).expect("valid WAT fixture"),
    )
    .unwrap();
}

/// Always accepts.
const SANDBOX_ACCEPTING_WAT: &str = r#"
  (module
    (memory (export "memory") 1)
    (func (export "alloc") (param i32) (result i32) (i32.const 0))
    (func (export "validate") (param i32 i32) (result i32) (i32.const 0)))
"#;

/// No `validate` export at all — every call faults with `FaultKind::BadAbi`, the
/// cheapest deterministic fault.
const SANDBOX_FAULTING_WAT: &str = r#"
  (module
    (memory (export "memory") 1)
    (func (export "alloc") (param i32) (result i32) (i32.const 0)))
"#;

/// Always refuses with `reason` stored at a static offset.
fn sandbox_refusing_wat(reason: &str) -> String {
    format!(
        r#"
  (module
    (memory (export "memory") 1)
    (data (i32.const 2048) "{reason}")
    (func (export "alloc") (param i32) (result i32) (i32.const 1024))
    (func (export "validate") (param i32 i32) (result i32) (i32.const 1))
    (func (export "reason_ptr") (result i32) (i32.const 2048))
    (func (export "reason_len") (result i32) (i32.const {len})))
"#,
        len = reason.len()
    )
}

/// A fresh repository wired to `modules_dir`, one world with its creating GM,
/// and the GM's context — the shared setup for every validator-chokepoint test.
async fn sandbox_world(
    dir: &std::path::Path,
) -> (
    SqliteRepository,
    crate::data::membership::PermissionContext,
    crate::data::document::World,
) {
    let r = SqliteRepository::connect("sqlite::memory:")
        .await
        .unwrap()
        .with_modules_dir(dir);
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = crate::data::membership::PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    (r, gm_ctx, w)
}

/// An `item` document (a client-side doc_type the server treats structurally —
/// no engine body) with the given `system` band.
fn sandbox_item(id: u128, world: Uuid, system: serde_json::Value) -> Document {
    let mut d = world_doc(id, world, system);
    d.doc_type = "item".into();
    d.engine = None;
    d
}

/// Enables `id` for `world` with `validators_enabled` set as given.
async fn enable(r: &SqliteRepository, world: Uuid, id: &str, validators_enabled: bool) {
    r.set_world_enabled_modules(
        world,
        &[crate::modules::WorldModuleEntry {
            id: id.into(),
            validators_enabled,
        }],
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_validator_refused_create_is_rejected_with_the_modules_reason() {
    let dir = tempfile::tempdir().unwrap();
    write_validator_module(
        dir.path(),
        "mod-x",
        "item",
        &sandbox_refusing_wat("hp must stay positive"),
    );
    let (r, gm_ctx, w) = sandbox_world(dir.path()).await;
    enable(&r, w.id, "mod-x", true).await;

    let err = r
        .apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Create {
                doc: sandbox_item(1, w.id, serde_json::json!({ "hp": -1 })),
            }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    let DataError::OpFailed(msg) = err else {
        panic!("expected OpFailed, got {err:?}");
    };
    assert!(
        msg.contains("validator ") && msg.contains("hp must stay positive"),
        "the refusal must name the validator channel and carry the reason: {msg}"
    );
    assert!(r.get_document(Uuid::from_u128(1)).await.unwrap().is_none());
}

#[tokio::test]
async fn a_faulting_validator_rejects_with_dataerror_validator() {
    let dir = tempfile::tempdir().unwrap();
    write_validator_module(dir.path(), "mod-x", "item", SANDBOX_FAULTING_WAT);
    let (r, gm_ctx, w) = sandbox_world(dir.path()).await;
    enable(&r, w.id, "mod-x", true).await;

    let err = r
        .apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Create {
                doc: sandbox_item(1, w.id, serde_json::json!({})),
            }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    let DataError::Validator(fault) = err else {
        panic!("expected DataError::Validator, got {err:?}");
    };
    assert_eq!(fault.module, "mod-x");
    assert_eq!(fault.consecutive, 1);
}

#[tokio::test]
async fn validators_disabled_for_the_world_accept_regardless_of_the_verdict() {
    let dir = tempfile::tempdir().unwrap();
    write_validator_module(dir.path(), "mod-x", "item", &sandbox_refusing_wat("always"));
    let (r, gm_ctx, w) = sandbox_world(dir.path()).await;
    enable(&r, w.id, "mod-x", false).await;

    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create {
            doc: sandbox_item(1, w.id, serde_json::json!({ "hp": -1 })),
        }],
        1,
        WriteOrigin::Client,
    )
    .await
    .expect("validators_enabled: false means the module's verdict is never consulted");
}

#[tokio::test]
async fn a_module_not_enabled_for_the_world_never_runs() {
    let dir = tempfile::tempdir().unwrap();
    write_validator_module(dir.path(), "mod-x", "item", &sandbox_refusing_wat("always"));
    let (r, gm_ctx, w) = sandbox_world(dir.path()).await;
    // Installed, but the world's enabled set is empty.

    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create {
            doc: sandbox_item(1, w.id, serde_json::json!({ "hp": -1 })),
        }],
        1,
        WriteOrigin::Client,
    )
    .await
    .expect("a module absent from the enabled set must not run");
}

#[tokio::test]
async fn an_embedded_child_is_validated_under_its_own_doc_type() {
    let dir = tempfile::tempdir().unwrap();
    // The validator judges ONLY "widget" documents; the parent is an "item".
    write_validator_module(
        dir.path(),
        "mod-x",
        "widget",
        &sandbox_refusing_wat("widget refused"),
    );
    let (r, gm_ctx, w) = sandbox_world(dir.path()).await;
    enable(&r, w.id, "mod-x", true).await;

    let mut parent = sandbox_item(1, w.id, serde_json::json!({}));
    let mut child = sandbox_item(2, w.id, serde_json::json!({}));
    child.doc_type = "widget".into();
    parent.embedded.insert("widgets".into(), vec![child]);

    let err = r
        .apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Create { doc: parent }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    let DataError::OpFailed(msg) = err else {
        panic!("expected OpFailed, got {err:?}");
    };
    assert!(
        msg.contains("widget refused"),
        "the embedded child's own doc_type must select the validator: {msg}"
    );
}

#[tokio::test]
async fn apply_command_never_invokes_a_validator() {
    let dir = tempfile::tempdir().unwrap();
    write_validator_module(dir.path(), "mod-x", "item", &sandbox_refusing_wat("always"));
    let (r, gm_ctx, w) = sandbox_world(dir.path()).await;
    enable(&r, w.id, "mod-x", true).await;

    // The trusted undo/replay substrate applies the SAME document the intent
    // path just refused, without consulting any validator.
    let cmd = UnsequencedCommand {
        world_id: w.id,
        author: gm_ctx.user_id,
        ts: 1,
        ops: vec![Operation::Create {
            doc: sandbox_item(1, w.id, serde_json::json!({ "hp": -1 })),
        }],
    };
    r.apply_command(cmd)
        .await
        .expect("apply_command is the trusted replay path: no validator runs");
    assert!(r.get_document(Uuid::from_u128(1)).await.unwrap().is_some());
}

#[tokio::test]
async fn two_refusing_modules_run_in_module_id_order_regardless_of_enable_order() {
    let dir = tempfile::tempdir().unwrap();
    write_validator_module(
        dir.path(),
        "module-a",
        "item",
        &sandbox_refusing_wat("from-a"),
    );
    write_validator_module(
        dir.path(),
        "module-b",
        "item",
        &sandbox_refusing_wat("from-b"),
    );
    let (r, gm_ctx, w) = sandbox_world(dir.path()).await;
    // Enabled in REVERSE-alphabetical order.
    r.set_world_enabled_modules(
        w.id,
        &[
            crate::modules::WorldModuleEntry {
                id: "module-b".into(),
                validators_enabled: true,
            },
            crate::modules::WorldModuleEntry {
                id: "module-a".into(),
                validators_enabled: true,
            },
        ],
    )
    .await
    .unwrap();

    let err = r
        .apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Create {
                doc: sandbox_item(1, w.id, serde_json::json!({})),
            }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    let DataError::OpFailed(msg) = err else {
        panic!("expected OpFailed, got {err:?}");
    };
    assert!(
        msg.contains("from-a"),
        "the alphabetically-first module's refusal must win: {msg}"
    );
}

#[tokio::test]
async fn a_structural_failure_rejects_before_and_without_faulting_any_validator() {
    let dir = tempfile::tempdir().unwrap();
    write_validator_module(dir.path(), "mod-x", "item", SANDBOX_FAULTING_WAT);
    let (r, gm_ctx, w) = sandbox_world(dir.path()).await;
    enable(&r, w.id, "mod-x", true).await;
    r.set_world_schema_declarations(
        w.id,
        &[crate::data::document::SchemaDeclaration {
            module_id: "example-system".into(),
            version: "1".into(),
            schema_format: 1,
            doc_type: "item".into(),
            subtree_pointer: "/system/hp".into(),
            schema: serde_json::from_value(serde_json::json!({ "type": "number" })).unwrap(),
        }],
    )
    .await
    .unwrap();

    let err = r
        .apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Create {
                doc: sandbox_item(1, w.id, serde_json::json!({ "hp": "not-a-number" })),
            }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DataError::SchemaViolation { .. }),
        "a tier-2 violation must surface as Phase 1's own error, never DataError::Validator: {err:?}"
    );

    // The registry's streak for (world, mod-x) is still zero: `record_fault`
    // increments from the stored value and returns the new total, so a fresh
    // `1` proves the rejected submission never reached the validator.
    let registry = r.validator_registry(dir.path()).await;
    assert_eq!(registry.record_fault(w.id, "mod-x"), 1);
}

#[tokio::test]
async fn a_document_changed_between_the_pre_image_read_and_the_transaction_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    write_validator_module(dir.path(), "mod-x", "item", SANDBOX_ACCEPTING_WAT);
    let (r, gm_ctx, w) = sandbox_world(dir.path()).await;
    enable(&r, w.id, "mod-x", true).await;

    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create {
            doc: sandbox_item(1, w.id, serde_json::json!({ "hp": 1 })),
        }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // A first Update moves the stored value from its original pre-image (hp: 1)
    // to a newer one (hp: 2); the validator accepts both.
    let first_old = serde_json::json!(1);
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Update {
            doc_id: Uuid::from_u128(1),
            changes: vec![FieldChange {
                path: "/system/hp".into(),
                old: first_old.clone(),
                new: serde_json::json!(2),
                remove: false,
            }],
        }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // A second Update captured against the original pre-image: the validator's
    // own pre-tx read sees the newer stored value and validates the merged
    // post-image, but Phase 1's ordinary OCC pre-image check refuses the
    // write — the guarantee that a stale validator read can never let a
    // superseded post-image commit.
    let err = r
        .apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Update {
                doc_id: Uuid::from_u128(1),
                changes: vec![FieldChange {
                    path: "/system/hp".into(),
                    old: first_old,
                    new: serde_json::json!(3),
                    remove: false,
                }],
            }],
            3,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DataError::Conflict(_)),
        "expected Conflict, got {err:?}"
    );
    assert_eq!(
        r.get_document(Uuid::from_u128(1))
            .await
            .unwrap()
            .unwrap()
            .system["hp"],
        serde_json::json!(2),
        "the conflicting write must not have landed"
    );
}
