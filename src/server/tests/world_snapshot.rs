//! `GET /api/worlds/{id}/snapshot`: the cold-start bootstrap endpoint that replaces full
//! event replay from seq 1. Exercises the HTTP surface directly (not `data::repository`) so
//! a redaction or seq-reporting regression at the route itself is caught here, mirroring
//! `assets.rs`'s rationale for testing through the route rather than the repository.

use shadowcat_test_support as common;

/// A `create` op JSON for a document of `doc_type`, with an optional `property_overrides`
/// map layered onto its permissions. Mirrors `common::create_doc_op`'s shape but exposes the
/// `permissions`/`system` fields this suite needs to exercise redaction.
fn doc_json(
    world: uuid::Uuid,
    id: u128,
    doc_type: &str,
    engine: Option<serde_json::Value>,
    system: serde_json::Value,
    property_overrides: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "id": uuid::Uuid::from_u128(id),
        "scope": { "kind": "world", "world_id": world },
        "doc_type": doc_type,
        "schema_version": 1,
        "permissions": {
            "default": "observer",
            "users": {},
            "property_overrides": property_overrides
        },
        "engine": engine,
        "system": system,
        "created_at": 0,
        "updated_at": 0,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshot_spans_multiple_doc_types_and_redacts_per_recipient() {
    let h = common::spawn().await;
    let (_player_id, player_cookie) = h.add_player("p").await;

    // GM creates two doc types in one world: an actor with a GM-only property, and a scene
    // with no such restriction.
    let actor = doc_json(
        h.world,
        1,
        "actor",
        Some(serde_json::json!({
            "displayName": "Test", "visual": { "kind": "image", "asset": "a.png" },
            "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
            "faction": null, "conditions": [], "prototype": true
        })),
        serde_json::json!({ "secret": 42, "public": 7 }),
        serde_json::json!({ "/system/secret": "gm_only" }),
    );
    let scene = doc_json(
        h.world,
        2,
        "scene",
        Some(serde_json::json!({
            "grid": { "kind": "square", "size": 100.0 }, "background": null
        })),
        serde_json::json!({}),
        serde_json::json!({}),
    );

    for doc in [&actor, &scene] {
        let res = h
            .client
            .post(format!(
                "http://{}/api/worlds/{}/documents",
                h.addr, h.world
            ))
            .json(doc)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 200, "body: {:?}", res.text().await);
    }

    // The GM sees both doc types, secret included.
    let gm_res = h
        .client
        .get(format!("http://{}/api/worlds/{}/snapshot", h.addr, h.world))
        .send()
        .await
        .unwrap();
    assert_eq!(gm_res.status(), 200);
    let gm_body: serde_json::Value = gm_res.json().await.unwrap();
    let gm_docs = gm_body["documents"].as_array().unwrap();
    assert_eq!(gm_docs.len(), 2, "GM sees every doc_type in one call");
    let gm_actor = gm_docs
        .iter()
        .find(|d| d["doc_type"] == "actor")
        .expect("actor present for GM");
    assert_eq!(
        gm_actor["system"]["secret"], 42,
        "GM keeps the GM-only field"
    );

    // A player sees the same two documents but with the GM-only property redacted from
    // the actor. `h.add_player` already logged the player in; reuse its cookie directly.
    let player_res = h
        .client
        .get(format!("http://{}/api/worlds/{}/snapshot", h.addr, h.world))
        .header("cookie", &player_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(player_res.status(), 200);
    let player_body: serde_json::Value = player_res.json().await.unwrap();
    let player_docs = player_body["documents"].as_array().unwrap();
    assert_eq!(
        player_docs.len(),
        2,
        "player still sees both doc_types (neither is fully hidden)"
    );
    let player_actor = player_docs
        .iter()
        .find(|d| d["doc_type"] == "actor")
        .expect("actor present for player");
    assert!(
        player_actor["system"]
            .as_object()
            .map(|m| !m.contains_key("secret"))
            .unwrap_or(true),
        "GM-only property must be redacted from a player's snapshot: {player_actor:?}"
    );
    assert_eq!(
        player_actor["system"]["public"], 7,
        "non-restricted properties survive redaction"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshot_seq_reflects_the_latest_commit() {
    let h = common::spawn().await;

    let before = h
        .client
        .get(format!("http://{}/api/worlds/{}/snapshot", h.addr, h.world))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(before["seq"], 0, "no commits yet");

    let doc = doc_json(
        h.world,
        3,
        "scene",
        Some(serde_json::json!({
            "grid": { "kind": "square", "size": 100.0 }, "background": null
        })),
        serde_json::json!({}),
        serde_json::json!({}),
    );
    let res = h
        .client
        .post(format!(
            "http://{}/api/worlds/{}/documents",
            h.addr, h.world
        ))
        .json(&doc)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "body: {:?}", res.text().await);

    let after = h
        .client
        .get(format!("http://{}/api/worlds/{}/snapshot", h.addr, h.world))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(after["seq"], 1, "seq reflects the single commit just made");
}
