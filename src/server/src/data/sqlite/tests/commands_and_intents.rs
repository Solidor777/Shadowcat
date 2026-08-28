//! Role/capability gating on `apply_command`/`apply_intent`, command
//! create/update/delete round trips and their commit-time snapshots, OCC
//! conflict and `remove` semantics, `WriteOrigin`-scoped exemptions, path/
//! capability-gated Update, tier-2 system-schema enforcement, and the
//! singleton-`doc_type` create-gate (cross-call and intra-batch).

use super::*;

#[tokio::test]
async fn non_gm_create_denied_by_default() {
    use crate::data::document::DocRole;
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = r.create_user("p", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    r.add_member(w.id, player, WorldRole::Player).await.unwrap();
    let p_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    // Player owns the doc (passes the WRITE_FIELDS floor) but the world grants
    // no core:create, so creation is denied — isolating the create gate.
    let mut doc = world_doc(1, w.id, serde_json::json!({}));
    doc.permissions.users.insert(player, DocRole::Owner);
    let err = r
        .apply_intent(
            &p_ctx,
            w.id,
            vec![Operation::Create { doc }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::Forbidden));
}

#[tokio::test]
async fn non_gm_create_allowed_with_role_grant() {
    use crate::data::document::DocRole;
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = r.create_user("p", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    r.add_member(w.id, player, WorldRole::Player).await.unwrap();
    let p_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let mut wd = WorldCapDefaults::default();
    wd.role_caps
        .all
        .entry(WorldRole::Player)
        .or_default()
        .insert("core:create".into());
    r.set_world_cap_defaults(w.id, &wd).await.unwrap();

    let mut doc = world_doc(1, w.id, serde_json::json!({}));
    doc.permissions.users.insert(player, DocRole::Owner);
    assert!(r
        .apply_intent(
            &p_ctx,
            w.id,
            vec![Operation::Create { doc }],
            1,
            WriteOrigin::Client
        )
        .await
        .is_ok());
}

#[tokio::test]
async fn role_grant_is_type_scoped() {
    use crate::data::document::DocRole;
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = r.create_user("p", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    r.add_member(w.id, player, WorldRole::Player).await.unwrap();
    let p_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    // Players may create tokens only.
    let mut wd = WorldCapDefaults::default();
    wd.role_caps
        .by_type
        .entry("token".into())
        .or_default()
        .entry(WorldRole::Player)
        .or_default()
        .insert("core:create".into());
    r.set_world_cap_defaults(w.id, &wd).await.unwrap();

    let mut tok = world_doc(1, w.id, serde_json::json!({}));
    tok.doc_type = "token".into();
    tok.engine = crate::data::document::tests::default_test_engine("token");
    tok.permissions.users.insert(player, DocRole::Owner);
    assert!(r
        .apply_intent(
            &p_ctx,
            w.id,
            vec![Operation::Create { doc: tok }],
            1,
            WriteOrigin::Client
        )
        .await
        .is_ok());

    let mut act = world_doc(2, w.id, serde_json::json!({}));
    act.permissions.users.insert(player, DocRole::Owner);
    let err = r
        .apply_intent(
            &p_ctx,
            w.id,
            vec![Operation::Create { doc: act }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::Forbidden));
}

#[tokio::test]
async fn player_may_create_message_but_not_other_types() {
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = r
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    r.add_member(w.id, player, WorldRole::Player).await.unwrap();
    let pl_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };

    // A server-shaped message doc (author owns it) — Player create allowed.
    let msg = crate::chat::build_message_doc(
        w.id,
        player,
        crate::chat::MessageDraft {
            channel: "all".into(),
            actor_owner: None,
            audience: crate::chat::Audience::Public,
            kind: crate::chat::MessageKind::Normal,
            content: crate::chat::plain_text_content("hi"),
            source: None,
        },
        1,
    );
    r.apply_intent(
        &pl_ctx,
        w.id,
        vec![Operation::Create { doc: msg }],
        1,
        WriteOrigin::Client,
    )
    .await
    .expect("player may post a message");

    // A non-message doc the player owns — still denied (core:create GM-only).
    let mut other = crate::chat::build_message_doc(
        w.id,
        player,
        crate::chat::MessageDraft {
            channel: "all".into(),
            actor_owner: None,
            audience: crate::chat::Audience::Public,
            kind: crate::chat::MessageKind::Normal,
            content: vec![],
            source: None,
        },
        2,
    );
    other.doc_type = "note".into();
    // "note" is not engine-defined (unlike "message"); the engine body
    // `build_message_doc` set must not follow the doc_type override.
    other.engine = None;
    let err = r
        .apply_intent(
            &pl_ctx,
            w.id,
            vec![Operation::Create { doc: other }],
            2,
            WriteOrigin::Client,
        )
        .await;
    assert!(
        matches!(err, Err(DataError::Forbidden)),
        "non-message create must stay GM-gated"
    );
}

#[tokio::test]
async fn spectator_may_not_create_message() {
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let spec = r
        .create_user("sp", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    r.add_member(w.id, spec, WorldRole::Spectator)
        .await
        .unwrap();
    let sp_ctx = PermissionContext {
        user_id: spec,
        world_role: WorldRole::Spectator,
    };
    let msg = crate::chat::build_message_doc(
        w.id,
        spec,
        crate::chat::MessageDraft {
            channel: "all".into(),
            actor_owner: None,
            audience: crate::chat::Audience::Public,
            kind: crate::chat::MessageKind::Normal,
            content: vec![],
            source: None,
        },
        1,
    );
    let err = r
        .apply_intent(
            &sp_ctx,
            w.id,
            vec![Operation::Create { doc: msg }],
            1,
            WriteOrigin::Client,
        )
        .await;
    assert!(matches!(err, Err(DataError::Forbidden)));
}

#[tokio::test]
async fn player_may_not_forge_message_owner_via_baseline_exemption() {
    use crate::data::document::DocRole;
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = r
        .create_user("pl2", None, ServerRole::User, 0)
        .await
        .unwrap();
    let other = r
        .create_user("other", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    r.add_member(w.id, player, WorldRole::Player).await.unwrap();
    let pl_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };

    // Build a server-shaped message doc for `player`, then forge its owner to
    // `other` while keeping `player`'s Owner grant in permissions.users (so the
    // WRITE_FIELDS floor would otherwise pass). The baseline exemption must not
    // fire for a non-self-owned message.
    let mut msg = crate::chat::build_message_doc(
        w.id,
        player,
        crate::chat::MessageDraft {
            channel: "all".into(),
            actor_owner: None,
            audience: crate::chat::Audience::Public,
            kind: crate::chat::MessageKind::Normal,
            content: crate::chat::plain_text_content("hi"),
            source: None,
        },
        1,
    );
    msg.owner = Some(other);
    msg.permissions.users.insert(player, DocRole::Owner);

    let err = r
        .apply_intent(
            &pl_ctx,
            w.id,
            vec![Operation::Create { doc: msg }],
            1,
            WriteOrigin::Client,
        )
        .await;
    assert!(
        matches!(err, Err(DataError::Forbidden)),
        "forged owner must not benefit from the baseline message-create exemption"
    );
}

#[tokio::test]
async fn player_may_not_update_own_message() {
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = r
        .create_user("pl3", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    r.add_member(w.id, player, WorldRole::Player).await.unwrap();
    let pl_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };

    // Player posts a legitimate message via the baseline create exemption.
    let msg = crate::chat::build_message_doc(
        w.id,
        player,
        crate::chat::MessageDraft {
            channel: "all".into(),
            actor_owner: None,
            audience: crate::chat::Audience::Public,
            kind: crate::chat::MessageKind::Normal,
            content: crate::chat::plain_text_content("hi"),
            source: None,
        },
        1,
    );
    let msg_id = msg.id;
    r.apply_intent(
        &pl_ctx,
        w.id,
        vec![Operation::Create { doc: msg }],
        1,
        WriteOrigin::Client,
    )
    .await
    .expect("player may post a message");

    // The owning Player's DocRole::Owner grants WRITE_FIELDS on their own
    // message (the capability check alone passes), so this Update would otherwise
    // let them forge `kind`/`content` post-hoc. Must be rejected outright:
    // a `Client`-origin write has no legitimate message-edit path.
    let err = r
        .apply_intent(
            &pl_ctx,
            w.id,
            vec![Operation::Update {
                doc_id: msg_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/system/kind".into(),
                    old: serde_json::json!("normal"),
                    new: serde_json::json!("system"),
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await;
    assert!(
        matches!(err, Err(DataError::Forbidden)),
        "message docs must be immutable to clients via Update"
    );
}

/// Seeds a world + Player-owned stored message via the baseline create
/// exemption; returns (repo, world_id, owner_ctx, msg_id) for tests that
/// exercise the Update path against it.
async fn seed_owned_message() -> (
    SqliteRepository,
    Uuid,
    crate::data::membership::PermissionContext,
    Uuid,
) {
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = r
        .create_user("pl4", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    r.add_member(w.id, player, WorldRole::Player).await.unwrap();
    let owner_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let msg = crate::chat::build_message_doc(
        w.id,
        player,
        crate::chat::MessageDraft {
            channel: "all".into(),
            actor_owner: None,
            audience: crate::chat::Audience::Public,
            kind: crate::chat::MessageKind::Normal,
            content: crate::chat::plain_text_content("hi"),
            source: None,
        },
        1,
    );
    let msg_id = msg.id;
    r.apply_intent(
        &owner_ctx,
        w.id,
        vec![Operation::Create { doc: msg }],
        1,
        WriteOrigin::Client,
    )
    .await
    .expect("player may post a message");
    (r, w.id, owner_ctx, msg_id)
}

#[tokio::test]
async fn message_update_rejected_for_client_allowed_for_server_revision() {
    let (repo, world, owner_ctx, msg_id) = seed_owned_message().await;
    let change = FieldChange {
        remove: false,
        path: "/engine/content".into(),
        old: serde_json::json!([{ "kind": "text", "text": "hi" }]),
        new: serde_json::json!([{ "kind": "text", "text": "edited" }]),
    };
    let ops = || {
        vec![Operation::Update {
            doc_id: msg_id,
            changes: vec![change.clone()],
        }]
    };

    // Client origin: blanket-rejected.
    let client = repo
        .apply_intent(&owner_ctx, world, ops(), 2, WriteOrigin::Client)
        .await;
    assert!(
        matches!(client, Err(DataError::Forbidden)),
        "client update must be forbidden"
    );

    // Server revision origin: permitted (owner holds WRITE_FIELDS via DocRole::Owner).
    let server = repo
        .apply_intent(
            &owner_ctx,
            world,
            ops(),
            3,
            WriteOrigin::ServerMessageRevision,
        )
        .await;
    assert!(
        server.is_ok(),
        "server revision update must be allowed: {server:?}"
    );
}

#[tokio::test]
async fn create_update_delete_round_trip_via_invert() {
    let r = repo().await;
    let w = r.create_world("W", 0).await.unwrap();
    let author = r
        .create_user("author", None, ServerRole::User, 0)
        .await
        .unwrap();

    // Create
    let create = UnsequencedCommand {
        world_id: w.id,
        author,
        ts: 1,
        ops: vec![Operation::Create {
            doc: world_doc(1, w.id, serde_json::json!({ "hp": 10 })),
        }],
    };
    let c1 = r.apply_command(create.clone()).await.unwrap();
    assert_eq!(c1.command.seq, 1);
    assert!(r.get_document(Uuid::from_u128(1)).await.unwrap().is_some());

    // Update
    let update = UnsequencedCommand {
        world_id: w.id,
        author,
        ts: 2,
        ops: vec![Operation::Update {
            doc_id: Uuid::from_u128(1),
            changes: vec![FieldChange {
                remove: false,
                path: "/system/hp".into(),
                old: serde_json::json!(10),
                new: serde_json::json!(3),
            }],
        }],
    };
    let c2 = r.apply_command(update.clone()).await.unwrap();
    assert_eq!(c2.command.seq, 2);
    assert_eq!(
        r.get_document(Uuid::from_u128(1))
            .await
            .unwrap()
            .unwrap()
            .system["hp"],
        serde_json::json!(3)
    );

    // Invert the update — hp returns to 10
    r.apply_command(c2.command.invert()).await.unwrap();
    assert_eq!(
        r.get_document(Uuid::from_u128(1))
            .await
            .unwrap()
            .unwrap()
            .system["hp"],
        serde_json::json!(10)
    );

    // Invert the create — document gone
    r.apply_command(c1.command.invert()).await.unwrap();
    assert!(r.get_document(Uuid::from_u128(1)).await.unwrap().is_none());
}

#[tokio::test]
async fn apply_command_on_unknown_world_fails_and_writes_nothing() {
    let r = repo().await;
    let author = r
        .create_user("author", None, ServerRole::User, 0)
        .await
        .unwrap();
    let cmd = UnsequencedCommand {
        world_id: Uuid::from_u128(999),
        author,
        ts: 1,
        ops: vec![Operation::Create {
            doc: world_doc(1, Uuid::from_u128(999), serde_json::json!({})),
        }],
    };
    assert!(r.apply_command(cmd).await.is_err());
    assert!(r.get_document(Uuid::from_u128(1)).await.unwrap().is_none());
}

#[tokio::test]
async fn seq_is_durable_across_reconnect() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m2.db");
    let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());

    let world_id;
    let author;
    {
        let r = SqliteRepository::connect(&url).await.unwrap();
        let w = r.create_world("W", 0).await.unwrap();
        world_id = w.id;
        author = r
            .create_user("author", None, ServerRole::User, 0)
            .await
            .unwrap();
        r.apply_command(UnsequencedCommand {
            world_id,
            author,
            ts: 1,
            ops: vec![Operation::Create {
                doc: world_doc(1, world_id, serde_json::json!({})),
            }],
        })
        .await
        .unwrap();
    }
    // Reconnect: seq must continue from 2, not restart at 1.
    let r = SqliteRepository::connect(&url).await.unwrap();
    let c = r
        .apply_command(UnsequencedCommand {
            world_id,
            author,
            ts: 2,
            ops: vec![Operation::Create {
                doc: world_doc(2, world_id, serde_json::json!({})),
            }],
        })
        .await
        .unwrap();
    assert_eq!(c.command.seq, 2);
}

#[tokio::test]
async fn create_with_foreign_world_scope_is_rejected() {
    let r = repo().await;
    let w = r.create_world("W", 0).await.unwrap();
    let author = r
        .create_user("author", None, ServerRole::User, 0)
        .await
        .unwrap();
    // Document scoped to a different world than the command sequences under.
    let cmd = UnsequencedCommand {
        world_id: w.id,
        author,
        ts: 1,
        ops: vec![Operation::Create {
            doc: world_doc(1, Uuid::from_u128(777), serde_json::json!({})),
        }],
    };
    assert!(r.apply_command(cmd).await.is_err());
    assert!(r.get_document(Uuid::from_u128(1)).await.unwrap().is_none());
}

#[tokio::test]
async fn delete_with_foreign_world_scope_is_rejected() {
    let r = repo().await;
    let w = r.create_world("W", 0).await.unwrap();
    let author = r
        .create_user("author", None, ServerRole::User, 0)
        .await
        .unwrap();
    let cmd = UnsequencedCommand {
        world_id: w.id,
        author,
        ts: 1,
        ops: vec![Operation::Delete {
            doc: world_doc(1, Uuid::from_u128(777), serde_json::json!({})),
        }],
    };
    assert!(r.apply_command(cmd).await.is_err());
    // The whole command rolled back: the seq was not consumed.
    assert_eq!(r.get_world(w.id).await.unwrap().unwrap().seq, 0);
}

#[tokio::test]
async fn update_cannot_change_document_id() {
    let r = repo().await;
    let w = r.create_world("W", 0).await.unwrap();
    let author = r
        .create_user("author", None, ServerRole::User, 0)
        .await
        .unwrap();
    r.apply_command(UnsequencedCommand {
        world_id: w.id,
        author,
        ts: 1,
        ops: vec![Operation::Create {
            doc: world_doc(1, w.id, serde_json::json!({})),
        }],
    })
    .await
    .unwrap();

    // An update whose pointer rewrites the envelope id is rejected before
    // any write, so no forked duplicate row appears.
    let bad = UnsequencedCommand {
        world_id: w.id,
        author,
        ts: 2,
        ops: vec![Operation::Update {
            doc_id: Uuid::from_u128(1),
            changes: vec![FieldChange {
                remove: false,
                path: "/id".into(),
                old: serde_json::json!(Uuid::from_u128(1)),
                new: serde_json::json!(Uuid::from_u128(2)),
            }],
        }],
    };
    assert!(r.apply_command(bad).await.is_err());
    assert!(r.get_document(Uuid::from_u128(1)).await.unwrap().is_some());
    assert!(r.get_document(Uuid::from_u128(2)).await.unwrap().is_none());
}

#[tokio::test]
async fn update_stamps_updated_at_from_command_ts() {
    let r = repo().await;
    let w = r.create_world("W", 0).await.unwrap();
    let author = r
        .create_user("author", None, ServerRole::User, 0)
        .await
        .unwrap();
    // world_doc sets updated_at = 0.
    r.apply_command(UnsequencedCommand {
        world_id: w.id,
        author,
        ts: 1,
        ops: vec![Operation::Create {
            doc: world_doc(1, w.id, serde_json::json!({ "hp": 1 })),
        }],
    })
    .await
    .unwrap();

    r.apply_command(UnsequencedCommand {
        world_id: w.id,
        author,
        ts: 42,
        ops: vec![Operation::Update {
            doc_id: Uuid::from_u128(1),
            changes: vec![FieldChange {
                remove: false,
                path: "/system/hp".into(),
                old: serde_json::json!(1),
                new: serde_json::json!(2),
            }],
        }],
    })
    .await
    .unwrap();

    assert_eq!(
        r.get_document(Uuid::from_u128(1))
            .await
            .unwrap()
            .unwrap()
            .updated_at,
        42
    );
}

#[tokio::test]
async fn query_documents_filters_by_world_and_type() {
    let r = repo().await;
    let w = r.create_world("W", 0).await.unwrap();
    let author = r
        .create_user("author", None, ServerRole::User, 0)
        .await
        .unwrap();
    for id in [1u128, 2] {
        r.apply_command(UnsequencedCommand {
            world_id: w.id,
            author,
            ts: 1,
            ops: vec![Operation::Create {
                doc: world_doc(id, w.id, serde_json::json!({})),
            }],
        })
        .await
        .unwrap();
    }
    let actors = r.query_documents(w.id, "actor").await.unwrap();
    assert_eq!(actors.len(), 2);
    assert!(r.query_documents(w.id, "item").await.unwrap().is_empty());
}

#[tokio::test]
async fn query_all_documents_spans_multiple_doc_types() {
    let r = repo().await;
    let w = r.create_world("W", 0).await.unwrap();
    let author = r
        .create_user("author", None, ServerRole::User, 0)
        .await
        .unwrap();
    for (id, doc_type) in [(1u128, "actor"), (2, "scene"), (3, "wall")] {
        let mut doc = world_doc(id, w.id, serde_json::json!({}));
        doc.doc_type = doc_type.into();
        doc.engine = crate::data::document::tests::default_test_engine(doc_type);
        r.apply_command(UnsequencedCommand {
            world_id: w.id,
            author,
            ts: 1,
            ops: vec![Operation::Create { doc }],
        })
        .await
        .unwrap();
    }

    let all = r.query_all_documents(w.id).await.unwrap();
    let types: std::collections::BTreeSet<_> = all.iter().map(|d| d.doc_type.clone()).collect();
    assert_eq!(all.len(), 3, "query_all_documents is not type-scoped");
    assert_eq!(
        types,
        ["actor", "scene", "wall"]
            .into_iter()
            .map(String::from)
            .collect(),
        "every doc_type created appears in one call"
    );
}

#[tokio::test]
async fn documents_by_source_finds_instances_for_push() {
    let r = repo().await;
    let w = r.create_world("W", 0).await.unwrap();
    let author = r
        .create_user("author", None, ServerRole::User, 0)
        .await
        .unwrap();
    let src = Uuid::from_u128(77);
    let mut doc = world_doc(1, w.id, serde_json::json!({}));
    doc.source = Some(Source {
        id: src,
        pack: Some("dnd5e".into()),
        version: 1,
    });
    r.apply_command(UnsequencedCommand {
        world_id: w.id,
        author,
        ts: 1,
        ops: vec![Operation::Create { doc }],
    })
    .await
    .unwrap();

    let found = r.documents_by_source(Some("dnd5e"), src).await.unwrap();
    assert_eq!(found.len(), 1);
    assert!(r
        .documents_by_source(Some("dnd5e"), Uuid::from_u128(0))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn events_since_returns_the_suffix() {
    let r = repo().await;
    let w = r.create_world("W", 0).await.unwrap();
    let author = r
        .create_user("author", None, ServerRole::User, 0)
        .await
        .unwrap();
    for id in [1u128, 2, 3] {
        r.apply_command(UnsequencedCommand {
            world_id: w.id,
            author,
            ts: 1,
            ops: vec![Operation::Create {
                doc: world_doc(id, w.id, serde_json::json!({})),
            }],
        })
        .await
        .unwrap();
    }
    let tail = r.events_since(w.id, 1).await.unwrap();
    assert_eq!(tail.len(), 2);
    assert_eq!(tail[0].command.seq, 2);
    assert_eq!(tail[1].command.seq, 3);
}

#[tokio::test]
async fn multi_op_command_snapshot_reflects_the_final_post_loop_state_for_every_op() {
    // The write-loop counterpart of permission.rs's
    // multi_op_leak_within_one_command_is_closed_by_the_post_loop_accumulator: proves
    // apply_intent's OWN snapshot construction (not a hand-built one) gives the FIRST op's
    // OpSnapshot the override the SECOND op in the SAME command adds.
    use crate::data::command::{FieldChange, Operation};
    use crate::data::document::{DocRole, PermissionSet, Scope, Visibility};
    use crate::data::membership::PermissionContext;

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
    let mut perms = PermissionSet::default();
    perms.users.insert(gm, DocRole::Owner);
    let mut d = tests_doc(perms, serde_json::json!({ "secret": "X" }));
    d.scope = Scope::World { world_id: w.id };
    let doc_id = d.id;
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create { doc: d }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let stored = r
        .apply_intent(
            &ctx,
            w.id,
            vec![
                Operation::Update {
                    doc_id,
                    changes: vec![FieldChange {
                        remove: false,
                        path: "/system/secret".into(),
                        old: serde_json::json!("X"),
                        new: serde_json::json!("Y"),
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
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    let op0 = stored.snapshot.per_op[0].as_ref().unwrap();
    assert!(
        op0.overrides_at_commit
            .iter()
            .any(|(p, v)| p == "/system/secret" && *v == Visibility::GmOnly),
        "the FIRST op's snapshot must already carry the override the SECOND op adds: {:?}",
        op0.overrides_at_commit
    );
}

#[tokio::test]
async fn create_op_snapshot_in_a_same_command_create_then_update_reflects_the_post_update_state() {
    // The Create-arm counterpart of multi_op_command_snapshot_reflects_the_final_post_loop_
    // state_for_every_op: proves the Create op's OWN persisted OpSnapshot carries the SAME
    // doc_id's later-in-command Update result (a reassigned `/owner`), not the value at the
    // moment the Create ran within the loop. Uses apply_command (the trusted replay
    // substrate) because apply_intent's Phase-1 OCC pre-image check rejects an Update
    // targeting a not-yet-committed same-batch Create (see
    // apply_intent_same_batch_create_then_engine_update_is_rejected) — a real op sequence
    // reaching this code path arrives via that substrate, not the client-intent gate.
    use crate::data::command::{FieldChange, Operation, UnsequencedCommand};
    use crate::data::document::{DocRole, PermissionSet, Scope};

    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let other = r
        .create_user("other", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let mut perms = PermissionSet::default();
    perms.users.insert(gm, DocRole::Owner);
    let mut d = tests_doc(perms, serde_json::json!({}));
    d.scope = Scope::World { world_id: w.id };
    let doc_id = d.id;

    let stored = r
        .apply_command(UnsequencedCommand {
            world_id: w.id,
            author: gm,
            ts: 1,
            ops: vec![
                Operation::Create { doc: d },
                Operation::Update {
                    doc_id,
                    changes: vec![FieldChange {
                        remove: false,
                        path: "/owner".into(),
                        old: serde_json::Value::Null,
                        new: serde_json::json!(other),
                    }],
                },
            ],
        })
        .await
        .unwrap();

    let create_snapshot = stored.snapshot.per_op[0].as_ref().unwrap();
    assert_eq!(
        create_snapshot.owner_at_commit,
        Some(other),
        "the Create op's own snapshot must reflect the POST-Update owner, not the owner \
         at the moment the Create ran: {:?}",
        create_snapshot.owner_at_commit
    );
}

#[tokio::test]
async fn reused_id_gets_a_fresh_created_seq_and_the_stale_ops_own_snapshot_witnesses_the_old_one() {
    use crate::data::command::Operation;
    use crate::data::document::{DocRole, PermissionSet, Scope};
    use crate::data::membership::PermissionContext;

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
    let mut perms = PermissionSet::default();
    perms.users.insert(gm, DocRole::Owner);
    let reused_id = Uuid::new_v4();
    // "item" is not engine-defined (a client-only doc_type) — `engine` must be `None`,
    // unlike `tests_engine_doc`'s always-`Some` shape.
    let mut d1 = tests_doc(perms.clone(), serde_json::json!({}));
    d1.doc_type = "item".into();
    d1.engine = None;
    d1.id = reused_id;
    d1.scope = Scope::World { world_id: w.id };
    let stored_create1 = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Create { doc: d1 }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    let old_created_seq = stored_create1.command.seq;

    let old_doc = r.get_document(reused_id).await.unwrap().unwrap();
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Delete { doc: old_doc }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut d2 = tests_doc(perms, serde_json::json!({}));
    d2.doc_type = "item".into();
    d2.engine = None;
    d2.id = reused_id;
    d2.scope = Scope::World { world_id: w.id };
    let stored_create2 = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Create { doc: d2 }],
            3,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    let new_created_seq = stored_create2.command.seq;
    assert_ne!(
        old_created_seq, new_created_seq,
        "a reused id must get a FRESH created_seq"
    );

    let (_, current_created_seq) = r
        .get_document_with_created_seq(reused_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current_created_seq, new_created_seq);
}

#[tokio::test]
async fn events_since_back_compat_parses_a_bare_command_row_carrying_no_snapshot() {
    use crate::data::command::Command;

    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();

    // Simulate a bare-Command row: bump the world seq and insert a bare Command directly,
    // bypassing apply_command/apply_intent's StoredCommand-shaped persistence.
    let cmd = Command {
        seq: 1,
        world_id: w.id,
        author: gm,
        ts: 0,
        ops: vec![],
    };
    sqlx::query(
        "INSERT INTO world_events (world_id, seq, author_id, ts, command_json) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(cmd.world_id.to_string())
    .bind(cmd.seq)
    .bind(cmd.author.to_string())
    .bind(cmd.ts)
    .bind(serde_json::to_string(&cmd).unwrap())
    .execute(&r.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE worlds SET seq = 1 WHERE id = ?")
        .bind(w.id.to_string())
        .execute(&r.pool)
        .await
        .unwrap();

    let replayed = r.events_since(w.id, 0).await.unwrap();
    assert_eq!(replayed.len(), 1);
    assert_eq!(replayed[0].command, cmd);
    assert!(replayed[0].snapshot.per_op.is_empty());
    assert!(replayed[0].snapshot.world_gm_at_commit.is_empty());
}

#[tokio::test]
async fn apply_intent_create_then_conflicting_update() {
    use crate::data::membership::PermissionContext;
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
    let doc = world_doc(1, w.id, serde_json::json!({ "hp": 10 }));
    let c1 = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Create { doc: doc.clone() }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    assert_eq!(c1.command.seq, 1);
    // Matching pre-image update succeeds.
    let ok = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Update {
                doc_id: doc.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/system/hp".into(),
                    old: serde_json::json!(10),
                    new: serde_json::json!(5),
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    assert_eq!(ok.command.seq, 2);
    // Stale pre-image (current is 5, not 10) → Conflict, no mutation.
    let conflict = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Update {
                doc_id: doc.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/system/hp".into(),
                    old: serde_json::json!(10),
                    new: serde_json::json!(1),
                }],
            }],
            3,
            WriteOrigin::Client,
        )
        .await;
    assert!(matches!(conflict, Err(DataError::Conflict(_))));
    assert_eq!(
        r.get_document(doc.id).await.unwrap().unwrap().system["hp"],
        serde_json::json!(5)
    );
}

#[tokio::test]
async fn apply_intent_remove_makes_key_absent_and_occ_guards_the_removal() {
    use crate::data::membership::PermissionContext;
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
    let doc = world_doc(1, w.id, serde_json::json!({ "foo": "bar", "baz": 1 }));
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create { doc: doc.clone() }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // A stale-pre-image removal (`old` != current) Conflicts and mutates nothing.
    let stale = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Update {
                doc_id: doc.id,
                changes: vec![FieldChange {
                    remove: true,
                    path: "/system/foo".into(),
                    old: serde_json::json!("wrong-value"),
                    new: serde_json::Value::Null,
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await;
    assert!(matches!(stale, Err(DataError::Conflict(_))));
    assert_eq!(
        r.get_document(doc.id).await.unwrap().unwrap().system["foo"],
        serde_json::json!("bar"),
        "conflicted removal leaves the key untouched"
    );

    // A matching-pre-image removal makes the key GENUINELY ABSENT (not null).
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Update {
            doc_id: doc.id,
            changes: vec![FieldChange {
                remove: true,
                path: "/system/foo".into(),
                old: serde_json::json!("bar"),
                new: serde_json::Value::Null,
            }],
        }],
        3,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let stored = r.get_document(doc.id).await.unwrap().unwrap();
    let sys = stored.system.as_object().unwrap();
    assert!(
        !sys.contains_key("foo"),
        "removed key must be absent, not present-as-null"
    );
    assert_eq!(sys["baz"], serde_json::json!(1), "sibling key untouched");
}

#[tokio::test]
async fn apply_intent_whole_band_replacement_removal_still_works() {
    // Regression: band-level replacement (a `remove: false` Update of the whole
    // `/system` band whose new value omits a key) is how the merge engine's
    // `planToUpdate` removes keys — it must keep producing genuine absence,
    // unaffected by the new leaf-level `remove_pointer` path.
    use crate::data::membership::PermissionContext;
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
    let doc = world_doc(1, w.id, serde_json::json!({ "foo": "bar", "baz": 1 }));
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create { doc: doc.clone() }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Update {
            doc_id: doc.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/system".into(),
                old: serde_json::json!({ "foo": "bar", "baz": 1 }),
                new: serde_json::json!({ "baz": 1 }),
            }],
        }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let stored = r.get_document(doc.id).await.unwrap().unwrap();
    let sys = stored.system.as_object().unwrap();
    assert!(!sys.contains_key("foo"), "band replacement drops the key");
    assert_eq!(sys["baz"], serde_json::json!(1));
}

/// Regression pin: a single intent batching `[Create(token), Update(token,
/// /engine/x=...)]` must be rejected wholesale, never partially committed. The `Update`
/// validation branch loads the CURRENT stored document (`Self::load_document`) before any
/// row is mutated, so a same-batch `Create` for the same id has not yet inserted its row,
/// and the `Update` finds no document to load. This pins the ordering `Room::publish`'s
/// movement gate depends on: the gate only runs when `SceneEcs::token_move` finds the token
/// already hydrated, and this ordering guarantee is what prevents a same-batch Create+Update
/// from committing ungated and unhydrated. Any future refactor that mutates rows per-op
/// instead of validating the whole batch up front could silently reopen this gap.
#[tokio::test]
async fn apply_intent_same_batch_create_then_engine_update_is_rejected() {
    use crate::data::membership::PermissionContext;
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
    let mut tok = world_doc(1, w.id, serde_json::json!({}));
    tok.doc_type = "token".into();
    tok.engine = crate::data::document::tests::default_test_engine("token");

    let err = r
        .apply_intent(
            &ctx,
            w.id,
            vec![
                Operation::Create { doc: tok.clone() },
                Operation::Update {
                    doc_id: tok.id,
                    changes: vec![FieldChange {
                        remove: false,
                        path: "/engine/x".into(),
                        old: serde_json::json!(0.0),
                        new: serde_json::json!(999.0),
                    }],
                },
            ],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DataError::Conflict(_)),
        "expected Conflict (Update's existence check rejecting the not-yet-inserted Create \
         target), got: {err:?}"
    );
    // Nothing committed: the whole batch (including the Create) was rejected, no partial
    // commit of just the Create half.
    assert!(r.get_document(tok.id).await.unwrap().is_none());
}

#[tokio::test]
async fn apply_intent_server_message_revision_may_write_property_overrides_but_nothing_else_under_permissions(
) {
    use crate::chat::MESSAGE_DOC_TYPE;
    use crate::data::document::{DocRole, PermissionSet};
    use crate::data::membership::PermissionContext;

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

    let doc_id = Uuid::new_v4();
    let doc = Document {
        id: doc_id,
        scope: Scope::World { world_id: w.id },
        doc_type: MESSAGE_DOC_TYPE.to_string(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: Some(gm),
        permissions: PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        embedded: Default::default(),
        parent_id: None,
        engine: Some(serde_json::json!({
            "channel": "all", "user_owner": gm, "kind": "normal",
            "audience": {"kind": "public"}, "content": []
        })),
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    };
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create { doc: doc.clone() }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // `/permissions/property_overrides` is admitted under ServerMessageRevision.
    let ok = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/permissions/property_overrides".into(),
                    old: serde_json::json!({}),
                    new: serde_json::json!({"/engine/content/0/spec": "gm_only"}),
                }],
            }],
            1,
            WriteOrigin::ServerMessageRevision,
        )
        .await;
    assert!(
        ok.is_ok(),
        "property_overrides write should be admitted: {ok:?}"
    );

    // `/permissions/default` is NOT admitted under the same origin.
    let denied = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/permissions/default".into(),
                    old: serde_json::json!("observer"),
                    new: serde_json::json!("owner"),
                }],
            }],
            2,
            WriteOrigin::ServerMessageRevision,
        )
        .await;
    assert!(
        matches!(denied, Err(DataError::Forbidden)),
        "widening /permissions/default must stay forbidden under ServerMessageRevision, got {denied:?}"
    );
}

#[tokio::test]
async fn apply_intent_server_message_revision_engine_write_ignores_a_declared_requirement_on_an_unrelated_doc_type(
) {
    // A world's `CapabilityRequirement` carries no `doc_type` (see
    // `CapabilityRequirement`'s doc), so a requirement declared for an
    // actor's `/engine/vision` still ancestor-overlaps ANY doc's whole-band
    // `/engine` write (`declared_caps_for_path`'s ancestor rule). Without
    // the `is_server_message_revision` exemption, this would deny every
    // `handle_recalc_roll`/`handle_edit_message`/`handle_delete_message`
    // `/engine` write in a world that happens to declare any such
    // requirement, even though the GM's moderation authority was already
    // vetted upstream and the write never touches `/engine/vision` at all.
    use crate::chat::MESSAGE_DOC_TYPE;
    use crate::data::document::{CapabilityRequirement, DocRole, PermissionSet};
    use crate::data::membership::PermissionContext;

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

    r.set_world_cap_requirements(
        w.id,
        &[CapabilityRequirement {
            path_prefix: "/engine/vision".into(),
            caps: ["dnd5e:gm_vision".to_string()].into_iter().collect(),
        }],
    )
    .await
    .unwrap();

    let doc_id = Uuid::new_v4();
    let old_engine = serde_json::json!({
        "channel": "all", "user_owner": gm, "kind": "normal",
        "audience": {"kind": "public"}, "content": []
    });
    let doc = Document {
        id: doc_id,
        scope: Scope::World { world_id: w.id },
        doc_type: MESSAGE_DOC_TYPE.to_string(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: Some(gm),
        permissions: PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        embedded: Default::default(),
        parent_id: None,
        engine: Some(old_engine.clone()),
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    };
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create { doc: doc.clone() }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let new_engine = serde_json::json!({
        "channel": "all", "user_owner": gm, "kind": "normal",
        "audience": {"kind": "public"}, "content": [], "edited_at": 1
    });
    let ok = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/engine".into(),
                    old: old_engine,
                    new: new_engine,
                }],
            }],
            1,
            WriteOrigin::ServerMessageRevision,
        )
        .await;
    assert!(
        ok.is_ok(),
        "a ServerMessageRevision /engine write must ignore a declared \
         requirement scoped to an unrelated doc_type's field: {ok:?}"
    );
}

#[tokio::test]
async fn apply_intent_server_message_revision_write_to_an_unscoped_path_still_enforces_declared_requirements(
) {
    // The `is_scoped_smr_write` exemption must be a THREE-way conjunction
    // (origin + doc_type + exact scoped path), not a two-way one keyed on
    // origin alone. A `ServerMessageRevision` write to `/name` (a path this
    // origin's real callers never touch) must still go through the
    // additive `declared_caps_for_path` check like any other write, and be
    // denied when a matching requirement exists that the scoped access
    // grant does not hold.
    use crate::chat::MESSAGE_DOC_TYPE;
    use crate::data::document::{CapabilityRequirement, DocRole, PermissionSet};
    use crate::data::membership::PermissionContext;

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

    r.set_world_cap_requirements(
        w.id,
        &[CapabilityRequirement {
            path_prefix: "/name".into(),
            caps: ["dnd5e:rename_message".to_string()].into_iter().collect(),
        }],
    )
    .await
    .unwrap();

    let doc_id = Uuid::new_v4();
    let engine = serde_json::json!({
        "channel": "all", "user_owner": gm, "kind": "normal",
        "audience": {"kind": "public"}, "content": []
    });
    let doc = Document {
        id: doc_id,
        scope: Scope::World { world_id: w.id },
        doc_type: MESSAGE_DOC_TYPE.to_string(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: Some(gm),
        permissions: PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        embedded: Default::default(),
        parent_id: None,
        engine: Some(engine),
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    };
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create { doc: doc.clone() }],
        0,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let denied = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/name".into(),
                    old: serde_json::json!(null),
                    new: serde_json::json!("renamed"),
                }],
            }],
            1,
            WriteOrigin::ServerMessageRevision,
        )
        .await;
    assert!(
        matches!(denied, Err(DataError::Forbidden)),
        "a ServerMessageRevision write to a path outside the \
         /engine + /permissions/property_overrides scope must still be \
         gated by a matching declared requirement, got {denied:?}"
    );
}

#[tokio::test]
async fn apply_intent_rejects_unauthorized_and_oversized() {
    use crate::data::document::{DocRole, PermissionSet};
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    // A doc only the GM can write (no per-user role; default None).
    let mut doc = world_doc(2, w.id, serde_json::json!({}));
    doc.permissions = PermissionSet {
        default: DocRole::None,
        ..Default::default()
    };
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create { doc: doc.clone() }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    // A player updating it → Forbidden.
    let player = r.create_user("p", None, ServerRole::User, 0).await.unwrap();
    let p_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let forbidden = r
        .apply_intent(
            &p_ctx,
            w.id,
            vec![Operation::Update {
                doc_id: doc.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/system/x".into(),
                    old: serde_json::json!(null),
                    new: serde_json::json!(1),
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await;
    assert!(matches!(forbidden, Err(DataError::Forbidden)));
    // Oversized create → TooLarge.
    let big = world_doc(
        3,
        w.id,
        serde_json::json!({ "blob": "x".repeat(300 * 1024) }),
    );
    let too_large = r
        .apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Create { doc: big }],
            3,
            WriteOrigin::Client,
        )
        .await;
    assert!(matches!(too_large, Err(DataError::TooLarge(_))));
}

// A doc owned by `player` (floor: read + write_fields), created by the GM.
async fn world_with_player_owned_doc(
    r: &SqliteRepository,
) -> (
    Uuid,
    Uuid,
    crate::data::membership::PermissionContext,
    Document,
) {
    use crate::data::document::{DocRole, PermissionSet};
    use crate::data::membership::PermissionContext;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = r.create_user("p", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let mut doc = world_doc(1, w.id, serde_json::json!({ "hp": 10 }));
    let mut perms = PermissionSet::default();
    perms.users.insert(player, DocRole::Owner);
    doc.permissions = perms;
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create { doc: doc.clone() }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    (w.id, player, gm_ctx, doc)
}

fn update(doc_id: Uuid, path: &str, old: serde_json::Value, new: serde_json::Value) -> Operation {
    Operation::Update {
        doc_id,
        changes: vec![FieldChange {
            remove: false,
            path: path.into(),
            old,
            new,
        }],
    }
}

#[tokio::test]
async fn apply_intent_update_gated_by_path_capability() {
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let (world, player, _gm_ctx, doc) = world_with_player_owned_doc(&r).await;
    let p = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };

    // Owner holds core:write_fields → /system writes succeed.
    r.apply_intent(
        &p,
        world,
        vec![update(
            doc.id,
            "/system/hp",
            serde_json::json!(10),
            serde_json::json!(5),
        )],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // ...but not core:manage_embedded → /embedded is forbidden.
    let emb = r
        .apply_intent(
            &p,
            world,
            vec![update(
                doc.id,
                "/embedded/items",
                serde_json::json!(null),
                serde_json::json!([]),
            )],
            3,
            WriteOrigin::Client,
        )
        .await;
    assert!(matches!(emb, Err(DataError::Forbidden)));

    // ...nor core:edit_permissions → /permissions is forbidden (no escalation).
    let acl = r
        .apply_intent(
            &p,
            world,
            vec![update(
                doc.id,
                "/permissions/default",
                serde_json::json!("none"),
                serde_json::json!("owner"),
            )],
            4,
            WriteOrigin::Client,
        )
        .await;
    assert!(matches!(acl, Err(DataError::Forbidden)));

    // ...and an immutable envelope field maps to no capability → forbidden.
    let env = r
        .apply_intent(
            &p,
            world,
            vec![update(
                doc.id,
                "/owner",
                serde_json::json!(null),
                serde_json::json!(player),
            )],
            5,
            WriteOrigin::Client,
        )
        .await;
    assert!(matches!(env, Err(DataError::Forbidden)));
}

#[tokio::test]
async fn apply_intent_granted_capability_enables_embedded() {
    use crate::data::document::{CapabilityGrants, DocRole, PermissionSet};
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = r.create_user("p", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    // Owner doc that additionally grants Owners core:manage_embedded.
    let mut doc = world_doc(1, w.id, serde_json::json!({}));
    let mut perms = PermissionSet::default();
    perms.users.insert(player, DocRole::Owner);
    let mut grants = CapabilityGrants::default();
    grants
        .by_role
        .entry(DocRole::Owner)
        .or_default()
        .insert(crate::data::permission::cap::MANAGE_EMBEDDED.to_string());
    perms.capabilities = grants;
    doc.permissions = perms;
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create { doc: doc.clone() }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let p = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    // With the grant, the owner may now manage embedded documents.
    r.apply_intent(
        &p,
        w.id,
        vec![update(
            doc.id,
            "/embedded/items",
            serde_json::json!(null),
            serde_json::json!([]),
        )],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    assert_eq!(
        r.get_document(doc.id)
            .await
            .unwrap()
            .unwrap()
            .embedded
            .len(),
        1
    );
}

#[tokio::test]
async fn apply_intent_delete_requires_delete_capability() {
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let (world, player, gm_ctx, doc) = world_with_player_owned_doc(&r).await;
    let p = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    // Owner floor does not include core:delete.
    let denied = r
        .apply_intent(
            &p,
            world,
            vec![Operation::Delete { doc: doc.clone() }],
            2,
            WriteOrigin::Client,
        )
        .await;
    assert!(matches!(denied, Err(DataError::Forbidden)));
    assert!(r.get_document(doc.id).await.unwrap().is_some());
    // The GM holds every capability and may delete.
    r.apply_intent(
        &gm_ctx,
        world,
        vec![Operation::Delete { doc }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    assert!(r.get_document(Uuid::from_u128(1)).await.unwrap().is_none());
}

#[tokio::test]
async fn apply_intent_delete_broadcasts_stored_doc_not_client_body() {
    use crate::data::document::{DocRole, PermissionSet};
    use crate::data::membership::PermissionContext;
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
    // Stored doc is GM-only with a real secret.
    let mut stored = world_doc(1, w.id, serde_json::json!({ "secret": 1 }));
    stored.permissions = PermissionSet {
        default: DocRole::None,
        ..Default::default()
    };
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create { doc: stored }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    // A Delete carrying a forged body (same id, permissive perms, bogus
    // system) must not drive the broadcast — the stored doc wins.
    let mut forged = world_doc(1, w.id, serde_json::json!({ "secret": 999 }));
    forged.permissions = PermissionSet {
        default: DocRole::Observer,
        ..Default::default()
    };
    let cmd = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Delete { doc: forged }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    let Operation::Delete { doc } = &cmd.command.ops[0] else {
        panic!("expected Delete");
    };
    assert_eq!(doc.permissions.default, DocRole::None);
    assert_eq!(doc.system["secret"], serde_json::json!(1));
}

#[tokio::test]
async fn apply_intent_world_default_grants_apply() {
    use crate::data::document::{CapabilityGrants, DocRole, PermissionSet};
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = r.create_user("p", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    // World default: Owners hold core:manage_embedded everywhere in this world.
    let mut all = CapabilityGrants::default();
    all.by_role
        .entry(DocRole::Owner)
        .or_default()
        .insert(crate::data::permission::cap::MANAGE_EMBEDDED.to_string());
    let wd = WorldCapDefaults {
        all,
        ..Default::default()
    };
    r.set_world_cap_defaults(w.id, &wd).await.unwrap();

    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    // An owner-held doc with NO per-document capability grant.
    let mut doc = world_doc(1, w.id, serde_json::json!({}));
    let mut perms = PermissionSet::default();
    perms.users.insert(player, DocRole::Owner);
    doc.permissions = perms;
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create { doc: doc.clone() }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // The world default alone authorizes the owner to manage embedded docs.
    let p = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    r.apply_intent(
        &p,
        w.id,
        vec![update(
            doc.id,
            "/embedded/items",
            serde_json::json!(null),
            serde_json::json!([]),
        )],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    assert_eq!(
        r.get_document(doc.id)
            .await
            .unwrap()
            .unwrap()
            .embedded
            .len(),
        1
    );
}

#[tokio::test]
async fn apply_intent_create_violating_system_schema_is_rejected_and_seq_untouched() {
    use crate::data::document::SchemaDeclaration;
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    // Register: actor /system/mechanics requires object with numeric `version`.
    let decls = vec![SchemaDeclaration {
        module_id: "example-system".into(),
        version: "1".into(),
        schema_format: 1,
        doc_type: "actor".into(),
        subtree_pointer: "/system/mechanics".into(),
        schema: serde_json::from_value(serde_json::json!({
            "type": "object", "required": ["version"],
            "properties": { "version": { "type": "number" } }
        }))
        .unwrap(),
    }];
    r.set_world_schema_declarations(w.id, &decls).await.unwrap();
    let seq_before = r.get_world(w.id).await.unwrap().unwrap().seq;

    // A Create whose /system/mechanics.version is a string violates the schema.
    let doc = world_doc(
        1,
        w.id,
        serde_json::json!({ "mechanics": { "version": "oops" } }),
    );
    let err = r
        .apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Create { doc }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::SchemaViolation { .. }));
    // Rejected intent consumes no seq (transaction dropped).
    let seq_after = r.get_world(w.id).await.unwrap().unwrap().seq;
    assert_eq!(seq_before, seq_after);
}

#[tokio::test]
async fn apply_intent_create_conforming_system_schema_succeeds() {
    use crate::data::document::SchemaDeclaration;
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let decls = vec![SchemaDeclaration {
        module_id: "example-system".into(),
        version: "1".into(),
        schema_format: 1,
        doc_type: "actor".into(),
        subtree_pointer: "/system/mechanics".into(),
        schema: serde_json::from_value(serde_json::json!({
            "type": "object", "required": ["version"],
            "properties": { "version": { "type": "number" } }
        }))
        .unwrap(),
    }];
    r.set_world_schema_declarations(w.id, &decls).await.unwrap();
    let doc = world_doc(
        1,
        w.id,
        serde_json::json!({ "mechanics": { "version": 2 } }),
    );
    assert!(r
        .apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Create { doc }],
            1,
            WriteOrigin::Client,
        )
        .await
        .is_ok());
}

#[tokio::test]
async fn create_rejects_a_second_singleton_doc_of_the_same_type() {
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };

    let first = singleton_test_doc(1, w.id, "world-settings");
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create { doc: first }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let second = singleton_test_doc(2, w.id, "world-settings");
    let err = r
        .apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Create { doc: second }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DataError::Conflict(_)),
        "a second world-settings doc in the same world must be rejected"
    );
}

#[tokio::test]
async fn create_allows_singleton_doc_types_in_different_worlds() {
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm_a = r
        .create_user("gm-a", None, ServerRole::User, 0)
        .await
        .unwrap();
    let gm_b = r
        .create_user("gm-b", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world_a = r.create_world_owned("A", gm_a, 0).await.unwrap();
    let world_b = r.create_world_owned("B", gm_b, 0).await.unwrap();
    let ctx_a = PermissionContext {
        user_id: gm_a,
        world_role: WorldRole::Gm,
    };
    let ctx_b = PermissionContext {
        user_id: gm_b,
        world_role: WorldRole::Gm,
    };

    r.apply_intent(
        &ctx_a,
        world_a.id,
        vec![Operation::Create {
            doc: singleton_test_doc(1, world_a.id, "world-settings"),
        }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let result = r
        .apply_intent(
            &ctx_b,
            world_b.id,
            vec![Operation::Create {
                doc: singleton_test_doc(2, world_b.id, "world-settings"),
            }],
            1,
            WriteOrigin::Client,
        )
        .await;
    assert!(result.is_ok(), "singleton scoping is per-world, not global");
}

#[tokio::test]
async fn create_does_not_gate_non_singleton_doc_types() {
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };

    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create {
            doc: world_doc(1, w.id, serde_json::json!({})),
        }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let second = r
        .apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Create {
                doc: world_doc(2, w.id, serde_json::json!({})),
            }],
            2,
            WriteOrigin::Client,
        )
        .await;
    assert!(
        second.is_ok(),
        "non-singleton doc types (e.g. actor) must remain uncapped"
    );
}

#[tokio::test]
async fn create_gate_is_race_safe_under_concurrent_creates() {
    use crate::data::membership::PermissionContext;
    let r = std::sync::Arc::new(repo().await);
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };

    let r1 = r.clone();
    let ctx1 = gm_ctx;
    let world_id = w.id;
    let fut1 = r1.apply_intent(
        &ctx1,
        world_id,
        vec![Operation::Create {
            doc: singleton_test_doc(1, world_id, "faction-registry"),
        }],
        1,
        WriteOrigin::Client,
    );
    let r2 = r.clone();
    let ctx2 = gm_ctx;
    let fut2 = r2.apply_intent(
        &ctx2,
        world_id,
        vec![Operation::Create {
            doc: singleton_test_doc(2, world_id, "faction-registry"),
        }],
        2,
        WriteOrigin::Client,
    );

    let (res1, res2) = tokio::join!(fut1, fut2);
    let ok_count = [res1.is_ok(), res2.is_ok()].iter().filter(|x| **x).count();
    assert_eq!(
        ok_count, 1,
        "exactly one of two concurrent singleton Creates must succeed, never both, never neither"
    );
}

#[tokio::test]
async fn create_rejects_intra_batch_duplicate_singleton_creates() {
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };

    // A single Intent batching TWO Creates of the same singleton doc_type:
    // neither has been inserted when the other's Phase-1 check runs, so
    // the DB-only check alone would let both through. The second must be
    // rejected by the intra-batch `claimed_singletons` tracking instead.
    let err = r
        .apply_intent(
            &gm_ctx,
            w.id,
            vec![
                Operation::Create {
                    doc: singleton_test_doc(1, w.id, "world-settings"),
                },
                Operation::Create {
                    doc: singleton_test_doc(2, w.id, "world-settings"),
                },
            ],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DataError::Conflict(_)),
        "a second same-batch world-settings Create must be rejected"
    );
    // The whole batch is one transaction: the rejected second op must
    // also roll back the first op's insert, not leave it half-applied.
    assert!(
        r.query_documents(w.id, "world-settings")
            .await
            .unwrap()
            .is_empty(),
        "a rejected batch must not partially commit"
    );
}

#[tokio::test]
async fn create_rejects_n_way_intra_batch_duplicate_singleton_creates() {
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };

    // Five Creates of the same singleton doc_type in ONE batch: the first
    // claims it, and every one of the remaining four must be rejected by
    // `claimed_singletons`, not just the second.
    let err = r
        .apply_intent(
            &gm_ctx,
            w.id,
            vec![
                Operation::Create {
                    doc: singleton_test_doc(1, w.id, "world-settings"),
                },
                Operation::Create {
                    doc: singleton_test_doc(2, w.id, "world-settings"),
                },
                Operation::Create {
                    doc: singleton_test_doc(3, w.id, "world-settings"),
                },
                Operation::Create {
                    doc: singleton_test_doc(4, w.id, "world-settings"),
                },
                Operation::Create {
                    doc: singleton_test_doc(5, w.id, "world-settings"),
                },
            ],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DataError::Conflict(_)),
        "a same-batch world-settings Create beyond the first must be rejected"
    );
    // The whole batch is one transaction: rejection must roll back ALL
    // preceding inserts in the batch, not leave any of them applied.
    assert!(
        r.query_documents(w.id, "world-settings")
            .await
            .unwrap()
            .is_empty(),
        "a rejected N-way batch must not partially commit"
    );
}

#[tokio::test]
async fn create_allows_different_singleton_doc_types_in_the_same_batch() {
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };

    let result = r
        .apply_intent(
            &gm_ctx,
            w.id,
            vec![
                Operation::Create {
                    doc: singleton_test_doc(1, w.id, "world-settings"),
                },
                Operation::Create {
                    doc: singleton_test_doc(2, w.id, "faction-registry"),
                },
            ],
            1,
            WriteOrigin::Client,
        )
        .await;
    assert!(
        result.is_ok(),
        "different singleton doc_types in the same batch must not over-reject"
    );
}

#[tokio::test]
async fn apply_intent_update_violating_system_schema_is_rejected_and_seq_untouched() {
    use crate::data::document::SchemaDeclaration;
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let decls = vec![SchemaDeclaration {
        module_id: "example-system".into(),
        version: "1".into(),
        schema_format: 1,
        doc_type: "actor".into(),
        subtree_pointer: "/system/mechanics".into(),
        schema: serde_json::from_value(serde_json::json!({
            "type": "object", "required": ["version"],
            "properties": { "version": { "type": "number" } }
        }))
        .unwrap(),
    }];
    r.set_world_schema_declarations(w.id, &decls).await.unwrap();

    // Create a conforming actor with /system/mechanics = { version: 1 }.
    let doc = world_doc(
        1,
        w.id,
        serde_json::json!({ "mechanics": { "version": 1 } }),
    );
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create { doc: doc.clone() }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let seq_before = r.get_world(w.id).await.unwrap().unwrap().seq;
    let update = Operation::Update {
        doc_id: doc.id,
        changes: vec![FieldChange {
            remove: false,
            path: "/system/mechanics/version".into(),
            old: serde_json::json!(1),
            new: serde_json::json!("oops"),
        }],
    };
    let err = r
        .apply_intent(&gm_ctx, w.id, vec![update], 2, WriteOrigin::Client)
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::SchemaViolation { .. }));
    let seq_after = r.get_world(w.id).await.unwrap().unwrap().seq;
    assert_eq!(seq_before, seq_after);
}

// --- combat family ingress: singleton registry, one active combat per
// scene, combatant parentage ---

#[tokio::test]
async fn resource_registry_is_a_singleton() {
    use crate::data::membership::PermissionContext;
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
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create {
            doc: singleton_test_doc(1, w.id, "resource-registry"),
        }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let err = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Create {
                doc: singleton_test_doc(2, w.id, "resource-registry"),
            }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::Conflict(_)));
}

/// A `combat` document bound to `scene`, `active` as given.
fn combat_doc(id: u128, world: Uuid, scene: Uuid, active: bool) -> Document {
    let mut d = world_doc(id, world, serde_json::json!({}));
    d.doc_type = "combat".into();
    d.engine = Some(serde_json::json!({
        "scene_id": scene.to_string(), "active": active, "round": 0, "turn": null,
        "turn_control": "owner_may_end", "order": [],
        "movement": { "resource": null, "interpretation": "per_cell", "enforcement": "none" }
    }));
    d
}

/// A `combatant` document parented as given.
fn combatant_doc(id: u128, world: Uuid, parent: Option<Uuid>) -> Document {
    let mut d = world_doc(id, world, serde_json::json!({}));
    d.doc_type = "combatant".into();
    d.parent_id = parent;
    d.engine = crate::data::document::tests::default_test_engine("combatant");
    d
}

#[tokio::test]
async fn a_second_active_combat_on_the_same_scene_is_rejected_but_another_scene_is_fine() {
    use crate::data::membership::PermissionContext;
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
    let scene_a = Uuid::from_u128(0xa);
    let scene_b = Uuid::from_u128(0xb);
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create {
            doc: combat_doc(1, w.id, scene_a, true),
        }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let err = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Create {
                doc: combat_doc(2, w.id, scene_a, true),
            }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, DataError::Conflict(_)),
        "second active combat on scene A must conflict"
    );
    // Inactive on the same scene is allowed; active on another scene is allowed.
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create {
            doc: combat_doc(3, w.id, scene_a, false),
        }],
        3,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create {
            doc: combat_doc(4, w.id, scene_b, true),
        }],
        4,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn activating_a_combat_by_update_is_gated_like_create() {
    use crate::data::membership::PermissionContext;
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
    let scene = Uuid::from_u128(0xa);
    r.apply_intent(
        &ctx,
        w.id,
        vec![
            Operation::Create {
                doc: combat_doc(1, w.id, scene, true),
            },
            Operation::Create {
                doc: combat_doc(2, w.id, scene, false),
            },
        ],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let activate = Operation::Update {
        doc_id: Uuid::from_u128(2),
        changes: vec![FieldChange {
            remove: false,
            path: "/engine/active".into(),
            old: serde_json::json!(false),
            new: serde_json::json!(true),
        }],
    };
    let err = r
        .apply_intent(&ctx, w.id, vec![activate.clone()], 2, WriteOrigin::Client)
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::Conflict(_)));
    // Deactivate the first, then the same Update succeeds.
    let deactivate = Operation::Update {
        doc_id: Uuid::from_u128(1),
        changes: vec![FieldChange {
            remove: false,
            path: "/engine/active".into(),
            old: serde_json::json!(true),
            new: serde_json::json!(false),
        }],
    };
    r.apply_intent(
        &ctx,
        w.id,
        vec![deactivate, activate],
        3,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn two_active_combats_for_one_scene_in_one_batch_are_rejected() {
    use crate::data::membership::PermissionContext;
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
    let scene = Uuid::from_u128(0xa);
    let err = r
        .apply_intent(
            &ctx,
            w.id,
            vec![
                Operation::Create {
                    doc: combat_doc(1, w.id, scene, true),
                },
                Operation::Create {
                    doc: combat_doc(2, w.id, scene, true),
                },
            ],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::Conflict(_)));
    assert!(
        r.query_documents(w.id, "combat").await.unwrap().is_empty(),
        "rejected batch must not partially commit"
    );
}

#[tokio::test]
async fn combatant_parent_must_be_a_combat_in_this_world() {
    use crate::data::membership::PermissionContext;
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
    let scene = Uuid::from_u128(0xa);
    // Parent is an actor, not a combat.
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create {
            doc: world_doc(9, w.id, serde_json::json!({})),
        }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let err = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Create {
                doc: combatant_doc(10, w.id, Some(Uuid::from_u128(9))),
            }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::OpFailed(_)));
    // Parent missing entirely.
    let err = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Create {
                doc: combatant_doc(11, w.id, Some(Uuid::from_u128(99))),
            }],
            3,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::OpFailed(_)));
    // Parent is a combat: ok, including a same-batch parent.
    r.apply_intent(
        &ctx,
        w.id,
        vec![
            Operation::Create {
                doc: combat_doc(1, w.id, scene, false),
            },
            Operation::Create {
                doc: combatant_doc(12, w.id, Some(Uuid::from_u128(1))),
            },
        ],
        4,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    // Deleting the combat cascades to its combatants.
    let combat = r.get_document(Uuid::from_u128(1)).await.unwrap().unwrap();
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Delete { doc: combat }],
        5,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    assert!(r.get_document(Uuid::from_u128(12)).await.unwrap().is_none());
}

#[tokio::test]
async fn update_snapshot_records_pre_image_permissions() {
    use crate::data::membership::PermissionContext;
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
    let mut d = world_doc(1, w.id, serde_json::json!({}));
    d.permissions.default = crate::data::document::DocRole::None;
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create { doc: d }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let stored = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Update {
                doc_id: Uuid::from_u128(1),
                changes: vec![FieldChange {
                    remove: false,
                    path: "/permissions/default".into(),
                    old: serde_json::json!("none"),
                    new: serde_json::json!("observer"),
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    let snap = stored.snapshot.per_op[0].as_ref().unwrap();
    assert_eq!(
        snap.permissions_before_commit.as_ref().unwrap().default,
        crate::data::document::DocRole::None
    );
    assert_eq!(
        snap.permissions_at_commit.as_ref().unwrap().default,
        crate::data::document::DocRole::Observer
    );
}
