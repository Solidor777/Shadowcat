//! End-to-end sandbox validator lifecycle: builds `examples/validator-rust/` for real (never
//! skips — a missing `wasm32-unknown-unknown` target fails loudly, naming the fix), installs it
//! into a temp modules dir, has the GM enable the module and opt into validators, and asserts a
//! negative-`hp` actor Create is refused with the reason while a non-negative one is accepted.
use shadowcat_test_support as common;
use std::path::PathBuf;

use futures_util::SinkExt;

/// The example crate's own directory, resolved relative to this workspace member's manifest
/// dir (`CARGO_MANIFEST_DIR` is `src/server/`), never the process's current directory.
fn example_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("validator-rust")
}

/// Builds `examples/validator-rust/` for `wasm32-unknown-unknown` in release mode, returning
/// the built `.wasm` bytes. Fails loudly (never skips) when the target is missing, naming the
/// exact `rustup` command to run.
fn build_example_wasm() -> Vec<u8> {
    let dir = example_dir();
    let status = std::process::Command::new("cargo")
        .args(["build", "--target", "wasm32-unknown-unknown", "--release"])
        .current_dir(&dir)
        .status()
        .expect("cargo invocation itself must succeed (cargo must be on PATH)");
    assert!(
        status.success(),
        "building examples/validator-rust for wasm32-unknown-unknown failed — if this is a \
         missing-target error, run `rustup target add wasm32-unknown-unknown` and retry; this \
         test NEVER skips on a missing target"
    );
    let wasm_path = dir
        .join("target")
        .join("wasm32-unknown-unknown")
        .join("release")
        .join("shadowcat_example_validator.wasm");
    std::fs::read(&wasm_path)
        .unwrap_or_else(|e| panic!("built wasm not found at {}: {e}", wasm_path.display()))
}

/// Copies the example's `module.json` plus the freshly-built `.wasm` into
/// `<modules_dir>/example-validator/`.
fn install_example(modules_dir: &std::path::Path) {
    let dest = modules_dir.join("example-validator");
    std::fs::create_dir_all(&dest).unwrap();
    let manifest = std::fs::read_to_string(example_dir().join("module.json")).unwrap();
    std::fs::write(dest.join("module.json"), manifest).unwrap();
    std::fs::write(dest.join("validator.wasm"), build_example_wasm()).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_negative_hp_actor_create_is_refused_a_non_negative_one_is_accepted() {
    let modules_dir =
        std::env::temp_dir().join(format!("shadowcat-sandbox-e2e-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&modules_dir).unwrap();
    install_example(&modules_dir);

    let h = common::spawn_with(|cfg| {
        cfg.modules_dir = Some(modules_dir.to_string_lossy().into_owned());
    })
    .await;

    // GM enables the module and opts into validators.
    let entries = serde_json::json!([{ "id": "example-validator", "validators_enabled": true }]);
    let res = h
        .client
        .put(format!(
            "http://{}/api/worlds/{}/enabled-modules",
            h.addr, h.world
        ))
        .json(&entries)
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        204,
        "enable request failed: {:?}",
        res.text().await
    );

    let mut ws = h.connect().await;
    common::drain_until_type(&mut ws, "welcome").await;

    // A negative-hp actor create is refused. "actor" is engine-defined, so the
    // intent must carry a valid typed `engine` body — the validator judges only
    // the opaque `system` band beside it.
    use tokio_tungstenite::tungstenite::Message;
    let refused_doc = serde_json::json!({
        "op": "create",
        "doc": {
            "id": uuid::Uuid::new_v4(),
            "scope": { "kind": "world", "world_id": h.world },
            "doc_type": "actor",
            "schema_version": 1,
            "engine": {
                "displayName": "Goblin", "visual": { "kind": "image", "asset": "a.png" },
                "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
                "faction": null, "conditions": [], "prototype": true
            },
            "system": { "hp": -1 },
            "created_at": 0,
            "updated_at": 0,
        }
    });
    let intent_id = uuid::Uuid::new_v4();
    ws.send(Message::Text(
        serde_json::json!({ "type": "intent", "intent_id": intent_id, "ops": [refused_doc] })
            .to_string(),
    ))
    .await
    .unwrap();
    let reject = common::drain_until_type(&mut ws, "reject").await;
    assert_eq!(reject["intent_id"], intent_id.to_string());
    assert!(
        reject["detail"]
            .as_str()
            .is_some_and(|d| d.contains("hp must not be negative")),
        "unexpected reject frame: {reject:?}"
    );

    // A non-negative-hp actor create is accepted.
    let accepted_doc = serde_json::json!({
        "op": "create",
        "doc": {
            "id": uuid::Uuid::new_v4(),
            "scope": { "kind": "world", "world_id": h.world },
            "doc_type": "actor",
            "schema_version": 1,
            "engine": {
                "displayName": "Goblin", "visual": { "kind": "image", "asset": "a.png" },
                "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
                "faction": null, "conditions": [], "prototype": true
            },
            "system": { "hp": 3 },
            "created_at": 0,
            "updated_at": 0,
        }
    });
    let intent_id_2 = uuid::Uuid::new_v4();
    ws.send(Message::Text(
        serde_json::json!({ "type": "intent", "intent_id": intent_id_2, "ops": [accepted_doc] })
            .to_string(),
    ))
    .await
    .unwrap();
    let event = common::drain_until_event(&mut ws).await;
    assert_eq!(event["command"]["ops"][0]["doc"]["system"]["hp"], 3);

    // The scan is anchored to the `system` value's span: a document whose NAME
    // carries a lookalike `"hp":-1` substring but whose `system.hp` is
    // non-negative must be accepted — the needle outside `system` is never
    // mistaken for the judged field.
    let named_doc = serde_json::json!({
        "op": "create",
        "doc": {
            "id": uuid::Uuid::new_v4(),
            "scope": { "kind": "world", "world_id": h.world },
            "doc_type": "actor",
            "schema_version": 1,
            "name": "the \"hp\":-1 monster",
            "engine": {
                "displayName": "Goblin", "visual": { "kind": "image", "asset": "a.png" },
                "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
                "faction": null, "conditions": [], "prototype": true
            },
            "system": { "hp": 7 },
            "created_at": 0,
            "updated_at": 0,
        }
    });
    let intent_id_3 = uuid::Uuid::new_v4();
    ws.send(Message::Text(
        serde_json::json!({ "type": "intent", "intent_id": intent_id_3, "ops": [named_doc] })
            .to_string(),
    ))
    .await
    .unwrap();
    let event = common::drain_until_event(&mut ws).await;
    assert_eq!(event["command"]["ops"][0]["doc"]["system"]["hp"], 7);
}
