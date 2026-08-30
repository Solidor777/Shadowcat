//! FTS index sync and search-result redaction/pagination, world capability
//! requirements and enabled-module sets, admin/user-account setup, world
//! creation/membership/permission-context resolution, and world export/import
//! round trips through a tar bundle.

use super::*;

#[tokio::test]
async fn fts_sync_reflects_create_update_delete() {
    use crate::auth::role::ServerRole;
    use crate::data::command::{FieldChange, Operation};
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
    let mut d = tests_doc(perms, serde_json::json!({ "name": "Goblin" }));
    d.scope = Scope::World { world_id: w.id };

    // Create → indexed.
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create { doc: d.clone() }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM documents_fts_public WHERE documents_fts_public MATCH 'Goblin' AND world_id = ?",
    )
    .bind(w.id.to_string())
    .fetch_one(r.pool())
    .await
    .unwrap();
    assert_eq!(n, 1);

    // Update → re-indexed (old term gone, new term present).
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Update {
            doc_id: d.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/system/name".into(),
                old: serde_json::json!("Goblin"),
                new: serde_json::json!("Orc"),
            }],
        }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let goblin: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM documents_fts_public WHERE documents_fts_public MATCH 'Goblin'",
    )
    .fetch_one(r.pool())
    .await
    .unwrap();
    let orc: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM documents_fts_public WHERE documents_fts_public MATCH 'Orc'",
    )
    .fetch_one(r.pool())
    .await
    .unwrap();
    assert_eq!((goblin, orc), (0, 1));

    // Delete → removed from both visibility-tier tables.
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Delete { doc: d.clone() }],
        3,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let after_public: i64 =
        sqlx::query_scalar("SELECT count(*) FROM documents_fts_public WHERE doc_id = ?")
            .bind(d.id.to_string())
            .fetch_one(r.pool())
            .await
            .unwrap();
    let after_gm: i64 =
        sqlx::query_scalar("SELECT count(*) FROM documents_fts_gm WHERE doc_id = ?")
            .bind(d.id.to_string())
            .fetch_one(r.pool())
            .await
            .unwrap();
    assert_eq!((after_public, after_gm), (0, 0));
}

#[tokio::test]
async fn search_ranks_and_filters_by_read_access() {
    use crate::auth::role::ServerRole;
    use crate::data::command::Operation;
    use crate::data::document::{DocRole, PermissionSet, Scope, Visibility};
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
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let pl_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };

    // A readable doc (default Observer → player can read) and a GM-only doc
    // (default None → player cannot read), both matching "dragon".
    let mut readable = tests_doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({ "name": "Red Dragon" }),
    );
    readable.scope = Scope::World { world_id: w.id };
    let mut secret = tests_doc(
        PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
        serde_json::json!({ "name": "Secret Dragon" }),
    );
    secret.scope = Scope::World { world_id: w.id };
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create {
            doc: readable.clone(),
        }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create {
            doc: secret.clone(),
        }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // GM sees both.
    let gm_page = r.search(&gm_ctx, w.id, "dragon", 10, None).await.unwrap();
    assert_eq!(gm_page.hits.len(), 2);

    // Player sees only the readable one — the GM-only doc is never leaked.
    let pl_page = r.search(&pl_ctx, w.id, "dragon", 10, None).await.unwrap();
    assert_eq!(pl_page.hits.len(), 1);
    assert_eq!(pl_page.hits[0].document.id, readable.id);
    assert!(pl_page.hits[0].snippet.to_lowercase().contains("dragon"));

    // GM-only property is redacted from a readable hit for the player.
    let mut sheet = tests_doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({ "name": "Knight", "secret": "weakness" }),
    );
    sheet.scope = Scope::World { world_id: w.id };
    sheet
        .permissions
        .property_overrides
        .insert("/system/secret".into(), Visibility::GmOnly);
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create { doc: sheet.clone() }],
        3,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let knight = r.search(&pl_ctx, w.id, "knight", 10, None).await.unwrap();
    assert_eq!(knight.hits.len(), 1);
    assert!(
        knight.hits[0].document.system.get("secret").is_none(),
        "GM-only field leaked in search document"
    );
    // The snippet must not quote GM-only text either.
    assert!(
        !knight.hits[0].snippet.to_lowercase().contains("weakness"),
        "GM-only field leaked in search snippet"
    );

    // Oracle closed: a non-GM searching the GM-only term gets no hit (the
    // term is only in the GM-only `content_all` column).
    let probe = r.search(&pl_ctx, w.id, "weakness", 10, None).await.unwrap();
    assert_eq!(probe.hits.len(), 0, "GM-only term matchable by non-GM");

    // A GM can still search their own GM-only field text.
    let gm_probe = r.search(&gm_ctx, w.id, "weakness", 10, None).await.unwrap();
    assert_eq!(gm_probe.hits.len(), 1);
    assert_eq!(gm_probe.hits[0].document.id, sheet.id);
}

#[tokio::test]
async fn search_admits_the_inheriting_owner_of_a_default_none_linked_token() {
    use crate::auth::role::ServerRole;
    use crate::data::command::Operation;
    use crate::data::document::{DocRole, PermissionSet};
    use crate::data::membership::PermissionContext;

    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let owner = r
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let stranger = r
        .create_user("st", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let owner_ctx = PermissionContext {
        user_id: owner,
        world_role: WorldRole::Player,
    };
    let stranger_ctx = PermissionContext {
        user_id: stranger,
        world_role: WorldRole::Player,
    };

    // Actor owned by `owner`.
    let actor = actor_doc_owned_by(w.id, Some(owner));
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create { doc: actor.clone() }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Linked token, no literal owner, `default: None` — the literal-owner
    // egress path would deny both the owner and the stranger; only the
    // effective (linked-actor) owner may read it.
    let mut token = owned_token_doc(w.id, Some(actor.id));
    token.permissions = PermissionSet {
        default: DocRole::None,
        ..Default::default()
    };
    token.system = serde_json::json!({ "label": "Wizard" });
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create { doc: token.clone() }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let owner_page = r
        .search(&owner_ctx, w.id, "wizard", 10, None)
        .await
        .unwrap();
    assert_eq!(
        owner_page.hits.len(),
        1,
        "inheriting owner must see the default-none linked token in search"
    );
    assert_eq!(owner_page.hits[0].document.id, token.id);

    let stranger_page = r
        .search(&stranger_ctx, w.id, "wizard", 10, None)
        .await
        .unwrap();
    assert_eq!(
        stranger_page.hits.len(),
        0,
        "a non-owner must never see a default-none token in search"
    );
}

#[tokio::test]
async fn search_score_unaffected_by_gm_only_match_non_gm() {
    // Regression: bm25() without explicit per-column weights sums score
    // over BOTH `content` and `content_all`, so a non-GM searcher's
    // ranking would shift when the query term ALSO appears in GM-only
    // text they can never see — leaking the existence of a hidden match
    // through score/rank even though row selection and snippets are
    // already correctly redacted. `content_all` carries name/engine
    // content in addition to system content, widening the surface this
    // affects.
    use crate::auth::role::ServerRole;
    use crate::data::command::Operation;
    use crate::data::document::{DocRole, PermissionSet, Scope, Visibility};
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
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let pl_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };

    // Two otherwise-identical readable docs, both matching "wolf" in
    // publicly visible content. Only `hidden_extra` ALSO repeats "wolf"
    // in a GM-only-redacted property — text the player can never see.
    let mut plain = tests_doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({ "name": "Wolf Pack" }),
    );
    plain.scope = Scope::World { world_id: w.id };
    let mut hidden_extra = tests_doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({ "name": "Wolf Pack", "secret": "wolf lair" }),
    );
    hidden_extra.scope = Scope::World { world_id: w.id };
    hidden_extra
        .permissions
        .property_overrides
        .insert("/system/secret".into(), Visibility::GmOnly);

    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create { doc: plain.clone() }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create {
            doc: hidden_extra.clone(),
        }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let page = r.search(&pl_ctx, w.id, "wolf", 10, None).await.unwrap();
    assert_eq!(page.hits.len(), 2);
    let plain_hit = page
        .hits
        .iter()
        .find(|h| h.document.id == plain.id)
        .expect("plain doc present");
    let hidden_hit = page
        .hits
        .iter()
        .find(|h| h.document.id == hidden_extra.id)
        .expect("hidden_extra doc present");
    assert_eq!(
        plain_hit.score, hidden_hit.score,
        "GM-only text repeating the query term shifted a non-GM searcher's score"
    );
}

#[tokio::test]
async fn search_paginates_without_underfill() {
    use crate::auth::role::ServerRole;
    use crate::data::command::Operation;
    use crate::data::document::{DocRole, PermissionSet, Scope};
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
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let pl_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };

    // 6 matching docs; alternating readable/secret. Player can read 3.
    for i in 0..6 {
        let role = if i % 2 == 0 {
            DocRole::Observer
        } else {
            DocRole::None
        };
        let mut d = tests_doc(
            PermissionSet {
                default: role,
                ..Default::default()
            },
            serde_json::json!({ "name": format!("dragon {i}") }),
        );
        d.scope = Scope::World { world_id: w.id };
        r.apply_intent(
            &gm_ctx,
            w.id,
            vec![Operation::Create { doc: d }],
            i + 1,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    }

    // Page size 2: first page returns 2 readable hits despite interleaved secrets.
    let p1 = r.search(&pl_ctx, w.id, "dragon", 2, None).await.unwrap();
    assert_eq!(p1.hits.len(), 2);
    assert!(p1.next_cursor.is_some());
    let p2 = r
        .search(&pl_ctx, w.id, "dragon", 2, p1.next_cursor)
        .await
        .unwrap();
    assert_eq!(p2.hits.len(), 1); // only 3 readable total
    assert!(p2.next_cursor.is_none());
}

#[tokio::test]
async fn world_cap_requirements_round_trip() {
    use crate::auth::role::ServerRole;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    // Default is empty.
    assert!(r.world_cap_requirements(w.id).await.unwrap().is_empty());
    let reqs = vec![CapabilityRequirement {
        path_prefix: "/system/vision".into(),
        caps: ["dnd5e:gm_vision".to_string()].into_iter().collect(),
    }];
    r.set_world_cap_requirements(w.id, &reqs).await.unwrap();
    assert_eq!(r.world_cap_requirements(w.id).await.unwrap(), reqs);
}

#[tokio::test]
async fn world_enabled_modules_round_trip() {
    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let author = r.create_user("a", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", author, 0).await.unwrap();

    assert!(r.world_enabled_modules(w.id).await.unwrap().is_empty());

    let ids = vec!["actors-plus".to_string(), "example-system".to_string()];
    r.set_world_enabled_modules(w.id, &ids).await.unwrap();
    assert_eq!(r.world_enabled_modules(w.id).await.unwrap(), ids);

    // A subsequent set fully replaces, not appends.
    r.set_world_enabled_modules(w.id, &["example-system".to_string()])
        .await
        .unwrap();
    assert_eq!(
        r.world_enabled_modules(w.id).await.unwrap(),
        vec!["example-system".to_string()]
    );
}

#[tokio::test]
async fn user_by_username_and_admin_exists() {
    use crate::auth::role::ServerRole;
    let r = repo().await;
    assert!(!r.admin_exists().await.unwrap());
    let id = r
        .create_user("admin1", Some("phc-hash"), ServerRole::Admin, 100)
        .await
        .unwrap();
    assert!(r.admin_exists().await.unwrap());
    let rec = r.user_by_username("admin1").await.unwrap().unwrap();
    assert_eq!(rec.id, id);
    assert_eq!(rec.server_role, ServerRole::Admin);
    assert_eq!(rec.password_hash.as_deref(), Some("phc-hash"));
    assert!(r.user_by_username("nope").await.unwrap().is_none());
}

#[tokio::test]
async fn settings_get_set_round_trip() {
    let r = repo().await;
    assert!(r.get_setting("k").await.unwrap().is_none());
    r.set_setting("k", "v1").await.unwrap();
    assert_eq!(r.get_setting("k").await.unwrap().as_deref(), Some("v1"));
    r.set_setting("k", "v2").await.unwrap();
    assert_eq!(r.get_setting("k").await.unwrap().as_deref(), Some("v2"));
}

#[tokio::test]
async fn create_admin_if_none_refuses_a_case_insensitive_username_collision() {
    use crate::auth::role::ServerRole;
    let r = repo().await;
    r.create_user("alice", Some("phc"), ServerRole::User, 0)
        .await
        .unwrap();
    // No admin exists, so the admin guard passes — the NOCASE guard is what
    // must reject this. Without it `Alice` (admin) and `alice` (user) would
    // coexist and be indistinguishable in a roster.
    assert!(r
        .create_admin_if_none("Alice", "phc", 0)
        .await
        .unwrap()
        .is_none());
    assert!(!r.admin_exists().await.unwrap());
    // A free name still works.
    assert!(r
        .create_admin_if_none("root", "phc", 0)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn create_admin_if_none_guards_against_a_second_admin() {
    let r = repo().await;
    assert!(r
        .create_admin_if_none("admin", "phc", 0)
        .await
        .unwrap()
        .is_some());
    // A second attempt — even with a different username — creates nothing.
    assert!(r
        .create_admin_if_none("other", "phc", 0)
        .await
        .unwrap()
        .is_none());
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE server_role = 'admin'")
        .fetch_one(r.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn create_then_get_world() {
    let r = repo().await;
    let w = r.create_world("Test", 100).await.unwrap();
    let got = r.get_world(w.id).await.unwrap().unwrap();
    assert_eq!(got, w);
    assert_eq!(got.seq, 0);
}

#[tokio::test]
async fn members_carry_world_role() {
    let r = repo().await;
    let w = r.create_world("Test", 100).await.unwrap();
    let u = r
        .create_user("gm", None, ServerRole::Admin, 100)
        .await
        .unwrap();
    r.add_member(w.id, u, WorldRole::Gm).await.unwrap();
    assert_eq!(r.member_role(w.id, u).await.unwrap(), Some(WorldRole::Gm));
    assert_eq!(
        r.member_role(w.id, Uuid::from_u128(123)).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn world_owned_seats_creator_as_gm() {
    let r = repo().await;
    let creator = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", creator, 0).await.unwrap();
    assert_eq!(
        r.member_role(w.id, creator).await.unwrap(),
        Some(WorldRole::Gm)
    );
    assert_eq!(
        r.member_role(w.id, Uuid::from_u128(123)).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn permission_context_resolves_role_or_forbids() {
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gmx", None, ServerRole::User, 0)
        .await
        .unwrap();
    let admin = r
        .create_user("adx", None, ServerRole::Admin, 0)
        .await
        .unwrap();
    let stranger = r
        .create_user("sx", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();

    let c: PermissionContext = r
        .permission_context(w.id, gm, ServerRole::User)
        .await
        .unwrap();
    assert_eq!(c.world_role, WorldRole::Gm);
    let ac = r
        .permission_context(w.id, admin, ServerRole::Admin)
        .await
        .unwrap();
    assert_eq!(ac.world_role, WorldRole::Gm);
    assert!(matches!(
        r.permission_context(w.id, stranger, ServerRole::User).await,
        Err(DataError::Forbidden)
    ));
}

#[tokio::test]
async fn set_remove_and_list_members() {
    let r = repo().await;
    let gm = r
        .create_user("gm2", None, ServerRole::User, 0)
        .await
        .unwrap();
    let p = r
        .create_user("p2", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    r.add_member(w.id, p, WorldRole::Player).await.unwrap();
    r.set_role(w.id, p, WorldRole::Spectator).await.unwrap();
    assert_eq!(
        r.member_role(w.id, p).await.unwrap(),
        Some(WorldRole::Spectator)
    );
    assert_eq!(r.list_members(w.id).await.unwrap().len(), 2);
    r.remove_member(w.id, p).await.unwrap();
    assert_eq!(r.member_role(w.id, p).await.unwrap(), None);
}

#[tokio::test]
async fn export_world_rows_resolves_owner_username_and_nulls_owner_in_json() {
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let owner = r
        .create_user("owner-user", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let mut doc = world_doc(1, w.id, serde_json::json!({}));
    doc.owner = Some(owner);
    let mut conn = r.pool().acquire().await.unwrap();
    SqliteRepository::upsert_document(&mut conn, &doc, 1)
        .await
        .unwrap();
    drop(conn);

    let data = r.export_world_rows(w.id).await.unwrap();
    assert_eq!(data.documents.len(), 1);
    let exported = &data.documents[0];
    assert_eq!(exported.owner_username.as_deref(), Some("owner-user"));
    assert_eq!(exported.document.owner, None);
    assert_eq!(exported.seq, 1);
    assert_eq!(exported.created_seq, 1);
}

#[tokio::test]
async fn export_world_rows_carries_manifest_watermark_and_row_counts() {
    let r = repo().await;
    let gm = r
        .create_user("gm2", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r
        .create_world_owned("Watermark World", gm, 0)
        .await
        .unwrap();
    let doc = world_doc(2, w.id, serde_json::json!({}));
    let mut conn = r.pool().acquire().await.unwrap();
    SqliteRepository::upsert_document(&mut conn, &doc, 1)
        .await
        .unwrap();
    drop(conn);

    let data = r.export_world_rows(w.id).await.unwrap();
    assert_eq!(data.manifest.world_id, w.id);
    assert_eq!(data.manifest.world_name, "Watermark World");
    assert_eq!(data.manifest.world_seq, w.seq);
    assert_eq!(data.manifest.world_created_at, w.created_at);
    assert_eq!(data.manifest.row_counts.get("documents"), Some(&1));
    // world_members always has at least the creating GM.
    assert_eq!(data.members.len(), 1);
    assert_eq!(data.members[0].username, "gm2");
}

#[tokio::test]
async fn export_world_rows_not_found_for_unknown_world() {
    let r = repo().await;
    let err = r.export_world_rows(Uuid::from_u128(999)).await.unwrap_err();
    assert!(matches!(err, DataError::NotFound));
}

#[tokio::test]
async fn import_world_round_trips_every_table_through_a_real_tar_bundle() {
    let src = repo().await;
    let gm = src
        .create_user("gm3", None, ServerRole::User, 0)
        .await
        .unwrap();
    let owner = src
        .create_user("actor-owner", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = src
        .create_world_owned("Round Trip World", gm, 0)
        .await
        .unwrap();

    let mut doc = world_doc(10, w.id, serde_json::json!({"hp": 5}));
    doc.owner = Some(owner);
    let mut conn = src.pool().acquire().await.unwrap();
    SqliteRepository::upsert_document(&mut conn, &doc, 1)
        .await
        .unwrap();
    drop(conn);

    sqlx::query(
        "INSERT INTO world_events (world_id, seq, author_id, ts, command_json) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(w.id.to_string())
    .bind(2i64)
    .bind(gm.to_string())
    .bind(0i64)
    .bind(r#"{"kind":"Noop","payload":{"embedded_ref":"deadbeef"}}"#)
    .execute(src.pool())
    .await
    .unwrap();

    let export_tmp = tempfile::tempdir().unwrap();
    let asset_id = Uuid::new_v4();
    let asset_dir = export_tmp.path().join(w.id.to_string());
    tokio::fs::create_dir_all(&asset_dir).await.unwrap();
    tokio::fs::write(asset_dir.join(asset_id.to_string()), b"ASSETBYTES")
        .await
        .unwrap();
    src.insert_asset(&crate::data::asset::Asset {
        id: asset_id,
        world_id: w.id,
        storage_key: format!("{}/{asset_id}", w.id),
        original_name: "token.png".to_string(),
        content_type: "image/png".to_string(),
        byte_size: 10,
        created_by: Some(owner),
        created_at: 0,
        version: 1,
        folder_id: None,
        tags: vec![],
        derived_tags: vec![],
        meta: crate::data::asset::AssetMeta::unprocessed("image/png", 1),
    })
    .await
    .unwrap();
    src.set_explored(w.id, doc.id, owner, &[1, 2, 3])
        .await
        .unwrap();
    // A genuine `settings` row (world capability defaults, same storage
    // shape as the schema-declarations registry `world_schemas_key`
    // keys) — exercises the `data.settings` insert loop below.
    src.set_setting(&world_caps_key(w.id), r#"{"marker":"settings-round-trip"}"#)
        .await
        .unwrap();

    let export_data = src.export_world_rows(w.id).await.unwrap();
    let bytes =
        crate::world_bundle::write_bundle(&export_data, export_tmp.path(), Vec::new()).unwrap();
    let tar_path = export_tmp.path().join("bundle.tar");
    tokio::fs::write(&tar_path, &bytes).await.unwrap();

    // Target server: same usernames, different underlying ids.
    let target = repo().await;
    let target_gm = target
        .create_user("gm3", None, ServerRole::User, 0)
        .await
        .unwrap();
    let target_owner = target
        .create_user("actor-owner", None, ServerRole::User, 0)
        .await
        .unwrap();
    assert_ne!(target_gm, gm);
    assert_ne!(target_owner, owner);

    let import_tmp = tempfile::tempdir().unwrap();
    let import_data = crate::world_bundle::read_bundle(&tar_path, import_tmp.path()).unwrap();
    let summary = target.import_world(import_data).await.unwrap();

    assert_eq!(summary.world_id, w.id);
    assert_eq!(summary.skipped_members, 0);
    assert_eq!(summary.skipped_fog, 0);

    // worlds row: id preserved, seq/created_at/updated_at preserved.
    let target_world: (String, i64, i64, i64) =
        sqlx::query_as("SELECT id, seq, created_at, updated_at FROM worlds WHERE id = ?")
            .bind(w.id.to_string())
            .fetch_one(target.pool())
            .await
            .unwrap();
    assert_eq!(target_world.0, w.id.to_string());
    assert_eq!(target_world.1, w.seq);

    // documents: owner re-resolved to the TARGET user's id, both column
    // and JSON body in lockstep.
    let row: (Option<String>, String) =
        sqlx::query_as("SELECT owner_id, json FROM documents WHERE id = ?")
            .bind(doc.id.to_string())
            .fetch_one(target.pool())
            .await
            .unwrap();
    assert_eq!(row.0, Some(target_owner.to_string()));
    let json_doc: serde_json::Value = serde_json::from_str(&row.1).unwrap();
    assert_eq!(
        json_doc.get("owner").and_then(|v| v.as_str()),
        Some(target_owner.to_string().as_str())
    );

    // world_events: command_json byte-identical, author re-resolved.
    let event: (String, Option<String>) =
        sqlx::query_as("SELECT command_json, author_id FROM world_events WHERE world_id = ?")
            .bind(w.id.to_string())
            .fetch_one(target.pool())
            .await
            .unwrap();
    assert_eq!(
        event.0,
        r#"{"kind":"Noop","payload":{"embedded_ref":"deadbeef"}}"#
    );
    assert_eq!(event.1, Some(target_gm.to_string()));

    // assets: storage_key recomputed under the standard scheme, bytes
    // byte-identical after finalize.
    let asset_row = target.get_asset(asset_id).await.unwrap().unwrap();
    assert_eq!(asset_row.storage_key, format!("{}/{asset_id}", w.id));
    let final_path = import_tmp
        .path()
        .join(w.id.to_string())
        .join(asset_id.to_string());
    assert_eq!(tokio::fs::read(&final_path).await.unwrap(), b"ASSETBYTES");

    // explored_fog: user re-resolved.
    let fog_user: String =
        sqlx::query_scalar("SELECT user_id FROM explored_fog WHERE scene_id = ?")
            .bind(doc.id.to_string())
            .fetch_one(target.pool())
            .await
            .unwrap();
    assert_eq!(fog_user, target_owner.to_string());

    // settings: the row lands verbatim on the target under the same key.
    let target_setting = target.get_setting(&world_caps_key(w.id)).await.unwrap();
    assert_eq!(
        target_setting.as_deref(),
        Some(r#"{"marker":"settings-round-trip"}"#)
    );
}

#[tokio::test]
async fn import_world_nulls_owner_when_username_unresolvable() {
    let src = repo().await;
    let gm = src
        .create_user("gm4", None, ServerRole::User, 0)
        .await
        .unwrap();
    let owner = src
        .create_user("owner-not-on-target", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = src.create_world_owned("W4", gm, 0).await.unwrap();
    let mut doc = world_doc(11, w.id, serde_json::json!({}));
    doc.owner = Some(owner);
    let mut conn = src.pool().acquire().await.unwrap();
    SqliteRepository::upsert_document(&mut conn, &doc, 1)
        .await
        .unwrap();
    drop(conn);

    let export_data = src.export_world_rows(w.id).await.unwrap();

    // Target has neither `gm4` nor `owner-not-on-target` — but DOES have
    // a distinct user seated as the sole GM via `worlds` insert directly
    // (import_world does not require any pre-existing user).
    let target = repo().await;
    let import_data = crate::data::world_bundle::WorldImportData {
        manifest: export_data.manifest.clone(),
        documents: export_data.documents.clone(),
        events: export_data.events.clone(),
        members: export_data.members.clone(),
        invites: export_data.invites.clone(),
        assets: export_data.assets.clone(),
        fog: export_data.fog.clone(),
        settings: export_data.settings.clone(),
        staged_assets: Vec::new(),
    };
    let summary = target.import_world(import_data).await.unwrap();
    // `gm4` (the sole world_members row) also doesn't exist on target.
    assert_eq!(summary.skipped_members, 1);

    let row: (Option<String>, String) =
        sqlx::query_as("SELECT owner_id, json FROM documents WHERE id = ?")
            .bind(doc.id.to_string())
            .fetch_one(target.pool())
            .await
            .unwrap();
    assert_eq!(row.0, None);
    let json_doc: serde_json::Value = serde_json::from_str(&row.1).unwrap();
    assert!(json_doc.get("owner").unwrap().is_null());
}

#[tokio::test]
async fn import_world_rejects_world_id_collision_before_writing_any_row() {
    let r = repo().await;
    let gm = r
        .create_user("gm5", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("Collider", gm, 0).await.unwrap();
    let doc = world_doc(12, w.id, serde_json::json!({}));
    let mut conn = r.pool().acquire().await.unwrap();
    SqliteRepository::upsert_document(&mut conn, &doc, 1)
        .await
        .unwrap();
    drop(conn);

    let export_data = r.export_world_rows(w.id).await.unwrap();
    let import_data = crate::data::world_bundle::WorldImportData {
        manifest: export_data.manifest.clone(),
        documents: export_data.documents.clone(),
        events: export_data.events.clone(),
        members: export_data.members.clone(),
        invites: export_data.invites.clone(),
        assets: export_data.assets.clone(),
        fog: export_data.fog.clone(),
        settings: export_data.settings.clone(),
        staged_assets: Vec::new(),
    };

    let err = r.import_world(import_data).await.unwrap_err();
    assert!(matches!(err, DataError::Conflict(_)));

    // Zero partial state: still exactly the one original document, not two.
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM documents WHERE world_id = ?")
        .bind(w.id.to_string())
        .fetch_one(r.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn import_world_rejects_duplicate_singleton_document_before_writing_any_row() {
    use crate::data::membership::PermissionContext;
    let src = repo().await;
    let gm = src
        .create_user("gm-singleton-import", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = src
        .create_world_owned("SingletonImport", gm, 0)
        .await
        .unwrap();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    src.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create {
            doc: singleton_test_doc(30, w.id, "world-settings"),
        }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut export_data = src.export_world_rows(w.id).await.unwrap();
    // A second `world-settings` document under a fresh id, the shape a
    // hand-assembled or corrupted bundle could carry — nothing in
    // `read_bundle` rejects this, so `import_world` itself must.
    let mut dup = export_data.documents[0].clone();
    dup.document.id = Uuid::from_u128(31);
    export_data.documents.push(dup);
    export_data
        .manifest
        .row_counts
        .insert("documents".to_string(), 2);

    let target = repo().await;
    let import_data = crate::data::world_bundle::WorldImportData {
        manifest: export_data.manifest.clone(),
        documents: export_data.documents.clone(),
        events: export_data.events.clone(),
        members: export_data.members.clone(),
        invites: export_data.invites.clone(),
        assets: export_data.assets.clone(),
        fog: export_data.fog.clone(),
        settings: export_data.settings.clone(),
        staged_assets: Vec::new(),
    };

    let err = target.import_world(import_data).await.unwrap_err();
    assert!(matches!(err, DataError::Conflict(_)));

    // Zero partial state: the whole transaction (including the `worlds`
    // row insert that precedes the document loop) rolls back, not just
    // the second document.
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM worlds WHERE id = ?")
        .bind(w.id.to_string())
        .fetch_one(target.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn import_world_drops_fog_row_when_username_unresolvable() {
    let src = repo().await;
    let gm = src
        .create_user("gm6", None, ServerRole::User, 0)
        .await
        .unwrap();
    let rememberer = src
        .create_user("rememberer-not-on-target", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = src.create_world_owned("W6", gm, 0).await.unwrap();
    let scene = world_doc(13, w.id, serde_json::json!({}));
    let mut conn = src.pool().acquire().await.unwrap();
    SqliteRepository::upsert_document(&mut conn, &scene, 1)
        .await
        .unwrap();
    drop(conn);
    src.set_explored(w.id, scene.id, rememberer, &[9, 9, 9])
        .await
        .unwrap();

    let export_data = src.export_world_rows(w.id).await.unwrap();
    assert_eq!(export_data.fog.len(), 1);

    // Target has neither `gm6` nor `rememberer-not-on-target`.
    let target = repo().await;
    let import_data = crate::data::world_bundle::WorldImportData {
        manifest: export_data.manifest.clone(),
        documents: export_data.documents.clone(),
        events: export_data.events.clone(),
        members: export_data.members.clone(),
        invites: export_data.invites.clone(),
        assets: export_data.assets.clone(),
        fog: export_data.fog.clone(),
        settings: export_data.settings.clone(),
        staged_assets: Vec::new(),
    };
    let summary = target.import_world(import_data).await.unwrap();
    assert_eq!(summary.skipped_fog, 1);

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM explored_fog WHERE world_id = ?")
        .bind(w.id.to_string())
        .fetch_one(target.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn import_world_inserts_world_invites_row() {
    let src = repo().await;
    let gm = src
        .create_user("gm7", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = src.create_world_owned("W7", gm, 0).await.unwrap();
    let invite_id = Uuid::new_v4();
    assert!(src
        .create_invite(
            NewInvite {
                id: invite_id,
                world: w.id,
                secret_hash: "PHC$fake-hash-for-test",
                role: WorldRole::Player,
                created_by: gm,
                now: 0,
                expires_at: 1_000,
            },
            10,
        )
        .await
        .unwrap());

    let export_data = src.export_world_rows(w.id).await.unwrap();
    assert_eq!(export_data.invites.len(), 1);
    assert_eq!(
        export_data.invites[0].created_by_username.as_deref(),
        Some("gm7")
    );
    assert_eq!(export_data.invites[0].consumed_by_username, None);

    let target = repo().await;
    let target_gm = target
        .create_user("gm7", None, ServerRole::User, 0)
        .await
        .unwrap();
    let import_data = crate::data::world_bundle::WorldImportData {
        manifest: export_data.manifest.clone(),
        documents: export_data.documents.clone(),
        events: export_data.events.clone(),
        members: export_data.members.clone(),
        invites: export_data.invites.clone(),
        assets: export_data.assets.clone(),
        fog: export_data.fog.clone(),
        settings: export_data.settings.clone(),
        staged_assets: Vec::new(),
    };
    target.import_world(import_data).await.unwrap();

    let row: (String, String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT secret_hash, role, created_by, consumed_by FROM world_invites WHERE id = ?",
    )
    .bind(invite_id.to_string())
    .fetch_one(target.pool())
    .await
    .unwrap();
    assert_eq!(row.0, "PHC$fake-hash-for-test");
    assert_eq!(row.1, "player");
    assert_eq!(row.2, Some(target_gm.to_string()));
    assert_eq!(row.3, None);
}

#[tokio::test]
async fn import_world_rejects_document_with_unclassifiable_property_override() {
    let src = repo().await;
    let gm = src
        .create_user("gm8", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = src.create_world_owned("W8", gm, 0).await.unwrap();
    let mut doc = world_doc(14, w.id, serde_json::json!({}));
    // `/owner` is a structural envelope field, not one of the four
    // content bands `redaction_target` classifies — the same pointer
    // `redaction_target_refuses_every_structural_envelope_field` pins in
    // `permission.rs`.
    doc.permissions.property_overrides.insert(
        "/owner".to_string(),
        crate::data::document::Visibility::GmOnly,
    );

    let export_data = WorldExportData {
        manifest: BundleManifest {
            schema_version: BUNDLE_SCHEMA_VERSION,
            world_id: w.id,
            world_name: "W8".to_string(),
            world_seq: w.seq,
            world_created_at: w.created_at,
            world_updated_at: w.updated_at,
            exported_at_unix_ms: 0,
            row_counts: std::collections::BTreeMap::new(),
        },
        documents: vec![ExportedDocumentRow {
            document: doc,
            owner_username: None,
            seq: 1,
            created_seq: 1,
        }],
        events: vec![],
        members: vec![],
        invites: vec![],
        assets: vec![],
        fog: vec![],
        settings: vec![],
    };

    let target = repo().await;
    let import_data = crate::data::world_bundle::WorldImportData {
        manifest: export_data.manifest.clone(),
        documents: export_data.documents.clone(),
        events: export_data.events.clone(),
        members: export_data.members.clone(),
        invites: export_data.invites.clone(),
        assets: export_data.assets.clone(),
        fog: export_data.fog.clone(),
        settings: export_data.settings.clone(),
        staged_assets: Vec::new(),
    };

    let err = target.import_world(import_data).await.unwrap_err();
    assert!(matches!(err, DataError::BadPath(_)));

    // Transaction did not commit: neither the world nor its document exist.
    let world_exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM worlds WHERE id = ?")
        .bind(w.id.to_string())
        .fetch_optional(target.pool())
        .await
        .unwrap();
    assert_eq!(world_exists, None);
    let doc_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM documents WHERE world_id = ?")
        .bind(w.id.to_string())
        .fetch_one(target.pool())
        .await
        .unwrap();
    assert_eq!(doc_count, 0);
}
