use super::*;
use crate::auth::role::ServerRole;
use crate::data::command::{Operation, UnsequencedCommand};
use crate::data::document::{Document, PermissionSet, Scope};
use crate::data::sqlite::SqliteRepository;
use uuid::Uuid;

/// A minimal actor doc, seeded directly via `apply_command` (bypasses
/// permission checks — these tests exercise host resolution, not the
/// token/actor create paths). The `engine` body must be well-formed for the
/// doc type, since `apply_command` normalizes it.
fn actor_doc(id: Uuid, world: Uuid) -> Document {
    Document {
        id,
        scope: Scope::World { world_id: world },
        doc_type: "actor".into(),
        schema_version: 1,
        name: Some("A".into()),
        source: None,
        base: None,
        owner: None,
        permissions: PermissionSet::default(),
        embedded: Default::default(),
        parent_id: None,
        engine: Some(serde_json::json!({
            "displayName": "A",
            "visual": { "kind": "image", "asset": "a.png" },
            "size": { "w": 1.0, "h": 1.0 },
            "shape": "square",
            "faction": null,
            "conditions": [],
            "prototype": true
        })),
        system: serde_json::json!({ "stats": { "str": 3 } }),
        created_at: 0,
        updated_at: 0,
    }
}

/// A minimal token doc: `actor_id` is the linked actor; `embedded_copy` is an
/// optional actor doc embedded under `embedded["actor"]`.
fn token_doc(
    id: Uuid,
    world: Uuid,
    actor_id: Option<Uuid>,
    embedded_copy: Option<Document>,
) -> Document {
    let mut embedded: std::collections::BTreeMap<String, Vec<Document>> = Default::default();
    if let Some(copy) = embedded_copy {
        embedded.insert("actor".into(), vec![copy]);
    }
    Document {
        id,
        scope: Scope::World { world_id: world },
        doc_type: "token".into(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: None,
        permissions: PermissionSet::default(),
        embedded,
        parent_id: None,
        engine: Some(serde_json::json!({
            "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0,
            "actor_id": actor_id,
        })),
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    }
}

/// An in-memory repo with one world and the given docs committed to it.
async fn repo_with(build: impl FnOnce(Uuid) -> Vec<Document>) -> SqliteRepository {
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    let docs = build(w.id);
    if !docs.is_empty() {
        repo.apply_command(UnsequencedCommand {
            world_id: w.id,
            author: gm,
            ts: 0,
            ops: docs
                .into_iter()
                .map(|doc| Operation::Create { doc })
                .collect(),
        })
        .await
        .unwrap();
    }
    repo
}

#[tokio::test]
async fn an_actor_ref_resolves_to_the_actor_itself() {
    let actor_id = Uuid::new_v4();
    let repo = repo_with(|w| vec![actor_doc(actor_id, w)]).await;
    let host = host_for_actor_owner(&repo, &ActorOwnerRef::Actor { actor_id })
        .await
        .unwrap();
    assert_eq!(host.map(|d| d.id), Some(actor_id));
}

#[tokio::test]
async fn a_token_ref_prefers_the_embedded_actor_copy() {
    let token_id = Uuid::new_v4();
    let linked_id = Uuid::new_v4();
    let copy_id = Uuid::new_v4();
    let repo = repo_with(|w| {
        vec![
            actor_doc(linked_id, w),
            token_doc(token_id, w, Some(linked_id), Some(actor_doc(copy_id, w))),
        ]
    })
    .await;
    let host = host_for_actor_owner(&repo, &ActorOwnerRef::TokenInstance { token_id })
        .await
        .unwrap();
    assert_eq!(host.map(|d| d.id), Some(copy_id));
}

#[tokio::test]
async fn a_token_ref_without_an_embedded_copy_falls_to_the_linked_actor() {
    let token_id = Uuid::new_v4();
    let linked_id = Uuid::new_v4();
    let repo = repo_with(|w| {
        vec![
            actor_doc(linked_id, w),
            token_doc(token_id, w, Some(linked_id), None),
        ]
    })
    .await;
    let host = host_for_actor_owner(&repo, &ActorOwnerRef::TokenInstance { token_id })
        .await
        .unwrap();
    assert_eq!(host.map(|d| d.id), Some(linked_id));
}

#[tokio::test]
async fn a_token_ref_with_no_actor_anywhere_has_no_host() {
    let token_id = Uuid::new_v4();
    let repo = repo_with(|w| vec![token_doc(token_id, w, None, None)]).await;
    let host = host_for_actor_owner(&repo, &ActorOwnerRef::TokenInstance { token_id })
        .await
        .unwrap();
    assert!(host.is_none());
}

#[tokio::test]
async fn an_absent_document_has_no_host() {
    let repo = repo_with(|_| vec![]).await;
    let host = host_for_actor_owner(
        &repo,
        &ActorOwnerRef::Actor {
            actor_id: Uuid::new_v4(),
        },
    )
    .await
    .unwrap();
    assert!(host.is_none());
    let host = host_for_actor_owner(
        &repo,
        &ActorOwnerRef::TokenInstance {
            token_id: Uuid::new_v4(),
        },
    )
    .await
    .unwrap();
    assert!(host.is_none());
}
