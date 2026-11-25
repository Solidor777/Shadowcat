//! Tests that a table draw nested to `MAX_DRAW_DEPTH`, with the deepest roll formula the dice
//! parser accepts at every level, is stored and read back through every path a message document
//! is read from.

use super::*;
use crate::auth::role::ServerRole;
use crate::chat::Audience;
use crate::data::command::UnsequencedCommand;
use crate::data::document::{DocRole, Document, PermissionSet, Scope, WorldRole};
use crate::data::snapshot::StoredCommand;
use crate::data::sqlite::SqliteRepository;
use crate::dice::notation::MAX_EXPR_JSON_DEPTH;
use crate::ws::room::RoomRegistry;

/// A table whose only row matches the total of `formula` (`terms` ones summed) and, unless it is
/// the last of the chain, draws once from `next`.
fn table(id: Uuid, world: Uuid, terms: usize, next: Option<Uuid>) -> Document {
    let results: Vec<serde_json::Value> = next
        .into_iter()
        .map(|t| serde_json::json!({ "kind": "draw", "table_id": t, "count": 1 }))
        .collect();
    Document {
        id,
        scope: Scope::World { world_id: world },
        doc_type: "table".into(),
        schema_version: 1,
        name: Some("T".into()),
        source: None,
        base: None,
        owner: None,
        permissions: PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        embedded: Default::default(),
        parent_id: None,
        engine: Some(serde_json::json!({
            "draw": { "kind": "formula", "notation": vec!["1"; terms].join("+") },
            "rows": [{
                "weight": 1,
                "range": { "lo": terms, "hi": terms },
                "label": "r",
                "results": results,
            }],
            "description": ""
        })),
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    }
}

// Fails if the parser lets a table's roll serialize deeper than the cap leaves room for: the
// nested draw nests three JSON levels per level of `MAX_DRAW_DEPTH`, the send succeeds, then
// every read of the message row errors.
#[tokio::test]
async fn a_draw_nested_to_the_depth_cap_with_the_deepest_formula_reads_back() {
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    crate::data::world_seed::seed_test_channel_registry(&repo, w.id, &[]).await;

    // The formula the depth cap accepts with the most terms.
    let terms = (MAX_EXPR_JSON_DEPTH - 2) / 2 + 1;
    let ids: Vec<Uuid> = (0..=MAX_DRAW_DEPTH).map(|_| Uuid::new_v4()).collect();
    let ops = ids
        .iter()
        .enumerate()
        .map(|(i, id)| Operation::Create {
            doc: table(*id, w.id, terms, ids.get(i + 1).copied()),
        })
        .collect();
    repo.apply_command(UnsequencedCommand {
        world_id: w.id,
        author: gm,
        ts: 0,
        ops,
    })
    .await
    .unwrap();

    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let room = RoomRegistry::new()
        .get_or_create(&repo, w.id)
        .await
        .unwrap()
        .unwrap();
    let (cmd, _) = handle_draw_table(
        DrawTableRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &crate::ws::PingRateLimiter::new(),
            preview: crate::chat::LinkPreviewDeps {
                client: &crate::chat::build_link_preview_client(),
                cache: &crate::chat::LinkPreviewCache::new(),
                rate: &crate::chat::PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
            seed: None,
        },
        ids[0],
        "general".into(),
        1,
        None,
        Audience::Public,
    )
    .await
    .unwrap();
    let id = match &cmd.ops[0] {
        Operation::Create { doc } => doc.id,
        other => panic!("expected Create, got {other:?}"),
    };

    repo.get_document(id).await.unwrap().expect("stored");
    let listed = repo
        .query_documents(w.id, chat::MESSAGE_DOC_TYPE)
        .await
        .unwrap();
    assert!(listed.iter().any(|d| d.id == id));
    let events = repo.events_since(w.id, 0).await.unwrap();
    for ev in &events {
        StoredCommand::from_stored_json(&serde_json::to_string(ev).unwrap())
            .expect("a stored event reads back");
    }
}
