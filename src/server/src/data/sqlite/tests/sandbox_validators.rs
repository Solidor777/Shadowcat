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
    let registry = r.validator_registry().await;
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

// ---------- chokepoint hardening pins ----------

/// Refuses with `reason` iff `needle` appears anywhere in the serialized input
/// JSON — a content-dependent verdict for tests that must prove WHICH input a
/// validator was (or was not) shown. The scan is a naive two-loop substring
/// search, the same shape the example validator's own scan takes.
fn scan_one_refusing_wat(needle: &str, reason: &str) -> String {
    format!(
        r#"
  (module
    (memory (export "memory") 1)
    (data (i32.const 2048) "{needle_esc}")
    (data (i32.const 4096) "{reason_esc}")
    (func (export "alloc") (param i32) (result i32) (i32.const 1024))
    (func $contains (param $hp i32) (param $hl i32) (param $np i32) (param $nl i32) (result i32)
      (local $i i32)
      (local $j i32)
      (local.set $i (i32.const 0))
      (block $notfound
        (block $found
          (loop $outer
            (br_if $notfound (i32.gt_u (local.get $i) (i32.sub (local.get $hl) (local.get $nl))))
            (local.set $j (i32.const 0))
            (block $mismatch
              (loop $inner
                (br_if $found (i32.eq (local.get $j) (local.get $nl)))
                (br_if $mismatch
                  (i32.ne
                    (i32.load8_u (i32.add (local.get $hp) (i32.add (local.get $i) (local.get $j))))
                    (i32.load8_u (i32.add (local.get $np) (local.get $j)))))
                (local.set $j (i32.add (local.get $j) (i32.const 1)))
                (br $inner)))
            (local.set $i (i32.add (local.get $i) (i32.const 1)))
            (br $outer)))
        (return (i32.const 1)))
      (i32.const 0))
    (func (export "validate") (param $p i32) (param $l i32) (result i32)
      (if (call $contains (local.get $p) (local.get $l) (i32.const 2048) (i32.const {nl}))
        (then (return (i32.const 1))))
      (i32.const 0))
    (func (export "reason_ptr") (result i32) (i32.const 4096))
    (func (export "reason_len") (result i32) (i32.const {lr})))
"#,
        nl = needle.len(),
        lr = reason.len(),
        needle_esc = needle.replace('"', "\\\""),
        reason_esc = reason.replace('"', "\\\"")
    )
}

/// Refuses iff BOTH needles appear anywhere in the serialized input JSON —
/// pairs a verdict to a COMBINATION of values no single pre-image carries.
fn scan_two_refusing_wat(n1: &str, n2: &str, reason: &str) -> String {
    let o2 = 2048 + n1.len();
    format!(
        r#"
  (module
    (memory (export "memory") 1)
    (data (i32.const 2048) "{n1_esc}")
    (data (i32.const {o2}) "{n2_esc}")
    (data (i32.const 4096) "{reason_esc}")
    (func (export "alloc") (param i32) (result i32) (i32.const 1024))
    (func $contains (param $hp i32) (param $hl i32) (param $np i32) (param $nl i32) (result i32)
      (local $i i32)
      (local $j i32)
      (local.set $i (i32.const 0))
      (block $notfound
        (block $found
          (loop $outer
            (br_if $notfound (i32.gt_u (local.get $i) (i32.sub (local.get $hl) (local.get $nl))))
            (local.set $j (i32.const 0))
            (block $mismatch
              (loop $inner
                (br_if $found (i32.eq (local.get $j) (local.get $nl)))
                (br_if $mismatch
                  (i32.ne
                    (i32.load8_u (i32.add (local.get $hp) (i32.add (local.get $i) (local.get $j))))
                    (i32.load8_u (i32.add (local.get $np) (local.get $j)))))
                (local.set $j (i32.add (local.get $j) (i32.const 1)))
                (br $inner)))
            (local.set $i (i32.add (local.get $i) (i32.const 1)))
            (br $outer)))
        (return (i32.const 1)))
      (i32.const 0))
    (func (export "validate") (param $p i32) (param $l i32) (result i32)
      (if (i32.and
            (call $contains (local.get $p) (local.get $l) (i32.const 2048) (i32.const {l1}))
            (call $contains (local.get $p) (local.get $l) (i32.const {o2}) (i32.const {l2})))
        (then (return (i32.const 1))))
      (i32.const 0))
    (func (export "reason_ptr") (result i32) (i32.const 4096))
    (func (export "reason_len") (result i32) (i32.const {lr})))
"#,
        l1 = n1.len(),
        l2 = n2.len(),
        lr = reason.len(),
        n1_esc = n1.replace('"', "\\\""),
        n2_esc = n2.replace('"', "\\\""),
        reason_esc = reason.replace('"', "\\\"")
    )
}

#[tokio::test]
async fn an_embedded_only_system_update_reaches_the_child_validator() {
    let dir = tempfile::tempdir().unwrap();
    // Refuses iff the judged document carries `"hp":5` — absent from the
    // Create below, present only in the Update's merged post-image.
    write_validator_module(
        dir.path(),
        "mod-x",
        "widget",
        &scan_one_refusing_wat("\"hp\":5", "widget refused"),
    );
    let (r, gm_ctx, w) = sandbox_world(dir.path()).await;
    enable(&r, w.id, "mod-x", true).await;

    // An `item` parent with one embedded `widget` child.
    let mut parent = sandbox_item(1, w.id, serde_json::json!({}));
    let mut child = sandbox_item(2, w.id, serde_json::json!({}));
    child.doc_type = "widget".into();
    parent.embedded.insert("widgets".into(), vec![child]);
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create { doc: parent }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // An Update touching ONLY an embedded child's `system` band — never the
    // top-level `/system` path — must still reach the child's validator, and
    // its refusal rejects the write.
    let err = r
        .apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Update {
                doc_id: Uuid::from_u128(1),
                changes: vec![FieldChange {
                    path: "/embedded/widgets/0/system/hp".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!(5),
                    remove: false,
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    let DataError::OpFailed(msg) = err else {
        panic!("expected OpFailed, got {err:?}");
    };
    assert!(
        msg.contains("widget refused"),
        "an embedded-only system Update must reach the child's validator: {msg}"
    );
}

#[tokio::test]
async fn a_concurrent_cross_field_change_revalidates_the_in_transaction_post_image() {
    let dir = tempfile::tempdir().unwrap();
    // Refuses only when the input carries BOTH `"a":2` AND `"b":2` — true of no
    // single pre-transaction merge below, but true of the second op's
    // in-transaction post-image.
    write_validator_module(
        dir.path(),
        "mod-x",
        "item",
        &scan_two_refusing_wat("\"a\":2", "\"b\":2", "cross-field sum too large"),
    );
    let (r, gm_ctx, w) = sandbox_world(dir.path()).await;
    enable(&r, w.id, "mod-x", true).await;

    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create {
            doc: sandbox_item(1, w.id, serde_json::json!({ "a": 1, "b": 1 })),
        }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Two Updates to the SAME document in one batch. The pre-transaction pass
    // merges BOTH against the same pre-image ({a:1,b:1}): op1's merge is
    // {a:2,b:1} (first needle only), op2's is {a:1,b:2} (second needle only) —
    // both accepted. Inside the transaction op1 lands first, so op2's
    // in-transaction pre-image is {a:2,b:1} and its post-image {a:2,b:2} —
    // which no pre-transaction validation ever saw. The re-validation against
    // the in-transaction merge is what refuses this batch.
    let err = r
        .apply_intent(
            &gm_ctx,
            w.id,
            vec![
                Operation::Update {
                    doc_id: Uuid::from_u128(1),
                    changes: vec![FieldChange {
                        path: "/system/a".into(),
                        old: serde_json::json!(1),
                        new: serde_json::json!(2),
                        remove: false,
                    }],
                },
                Operation::Update {
                    doc_id: Uuid::from_u128(1),
                    changes: vec![FieldChange {
                        path: "/system/b".into(),
                        old: serde_json::json!(1),
                        new: serde_json::json!(2),
                        remove: false,
                    }],
                },
            ],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    let DataError::OpFailed(msg) = err else {
        panic!("expected OpFailed, got {err:?}");
    };
    assert!(
        msg.contains("cross-field sum too large"),
        "the in-transaction post-image must be what the validator judges: {msg}"
    );
    // Whole-batch rollback: the document is still {a:1,b:1}, and each op on its
    // own (validated against the true pre-image) is accepted.
    assert_eq!(
        r.get_document(Uuid::from_u128(1))
            .await
            .unwrap()
            .unwrap()
            .system,
        serde_json::json!({ "a": 1, "b": 1 })
    );
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Update {
            doc_id: Uuid::from_u128(1),
            changes: vec![FieldChange {
                path: "/system/a".into(),
                old: serde_json::json!(1),
                new: serde_json::json!(2),
                remove: false,
            }],
        }],
        2,
        WriteOrigin::Client,
    )
    .await
    .expect("a single cross-field-safe update is accepted");
}

#[tokio::test]
async fn an_unauthorized_intent_is_forbidden_before_and_without_faulting_any_validator() {
    let dir = tempfile::tempdir().unwrap();
    write_validator_module(dir.path(), "mod-x", "item", SANDBOX_FAULTING_WAT);
    let (r, _gm_ctx, w) = sandbox_world(dir.path()).await;
    enable(&r, w.id, "mod-x", true).await;
    let player = r.create_user("p", None, ServerRole::User, 0).await.unwrap();
    r.add_member(w.id, player, WorldRole::Player).await.unwrap();
    let p_ctx = crate::data::membership::PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };

    // The player holds no core:create and no WRITE_FIELDS floor — Phase 1
    // would refuse this Create with `Forbidden`, so the answer must be
    // `Forbidden`, never the validator's own error...
    let err = r
        .apply_intent(
            &p_ctx,
            w.id,
            vec![Operation::Create {
                doc: sandbox_item(1, w.id, serde_json::json!({})),
            }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DataError::Forbidden),
        "an unauthorized intent must get Forbidden, not a validator error: {err:?}"
    );
    // ...and the validator must never have run at all: the streak is still
    // zero (`record_fault` increments and returns the new total, so a fresh
    // `1` proves no fault was ever recorded).
    let registry = r.validator_registry().await;
    assert_eq!(registry.record_fault(w.id, "mod-x"), 1);
}

#[tokio::test]
async fn prior_is_withheld_from_a_writer_without_whole_document_read() {
    let dir = tempfile::tempdir().unwrap();
    // Refuses iff the input contains the OLD value — i.e. iff the validator
    // could see the stored pre-image at all (through `prior` or otherwise).
    write_validator_module(
        dir.path(),
        "mod-x",
        "item",
        &scan_one_refusing_wat("\"hp\":1", "saw stored content"),
    );
    let (r, gm_ctx, w) = sandbox_world(dir.path()).await;
    enable(&r, w.id, "mod-x", true).await;

    // The stored document is seeded through the trusted replay path (no
    // validators run there) — the needle lives in its `system` band, so an
    // intent-path Create of the same value would itself be refused.
    r.apply_command(UnsequencedCommand {
        world_id: w.id,
        author: gm_ctx.user_id,
        ts: 1,
        ops: vec![Operation::Create {
            doc: sandbox_item(1, w.id, serde_json::json!({ "hp": 1 })),
        }],
    })
    .await
    .unwrap();

    // Control FIRST, while the stored value still carries the needle: the GM
    // holds READ, so `prior` IS supplied — the validator sees the old value
    // and refuses, proving the validator genuinely judges `prior` when it is
    // permitted to see it.
    let err = r
        .apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Update {
                doc_id: Uuid::from_u128(1),
                changes: vec![FieldChange {
                    path: "/system/hp".into(),
                    old: serde_json::json!(1),
                    new: serde_json::json!(2),
                    remove: false,
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    let DataError::OpFailed(msg) = err else {
        panic!("expected OpFailed, got {err:?}");
    };
    assert!(
        msg.contains("saw stored content"),
        "a READ-holding writer's validator must receive `prior`: {msg}"
    );

    // A player granted `core:write_fields` by user (and nothing else — no READ
    // on this document) may WRITE `/system` but must never be shown the stored
    // pre-image, so the validator runs with `prior` withheld and ACCEPTS
    // (it never sees `"hp":1`).
    let player = r.create_user("p", None, ServerRole::User, 0).await.unwrap();
    r.add_member(w.id, player, WorldRole::Player).await.unwrap();
    let mut wd = WorldCapDefaults::default();
    wd.all.by_user.insert(
        player,
        ["core:write_fields".to_string()].into_iter().collect(),
    );
    r.set_world_cap_defaults(w.id, &wd).await.unwrap();
    let p_ctx = crate::data::membership::PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    r.apply_intent(
        &p_ctx,
        w.id,
        vec![Operation::Update {
            doc_id: Uuid::from_u128(1),
            changes: vec![FieldChange {
                path: "/system/hp".into(),
                old: serde_json::json!(1),
                new: serde_json::json!(2),
                remove: false,
            }],
        }],
        3,
        WriteOrigin::Client,
    )
    .await
    .expect("a write-without-read writer's validator never sees the pre-image");
}
