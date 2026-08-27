//! OCC pre-image comparison, membership/role guards, world/user/scene delete
//! cascades, asset and schema-declaration round trips, world-scoped UI state
//! merge, explored-fog persistence, and the engine/property-override ingress
//! gates on Create/Update.

use super::*;

// --- values_semantically_eq: OCC pre-image PosInt/Float variant equality ---

#[test]
fn values_semantically_eq_accepts_whole_number_float_vs_posint() {
    // Stored Float(100.0) vs a client-echoed PosInt(100) pre-image: same
    // numeric value, different serde_json variant -- must be treated equal.
    let stored = serde_json::json!(100.0);
    let echoed = serde_json::Value::Number(serde_json::Number::from(100u64));
    assert!(values_semantically_eq(&stored, &echoed));
    assert!(values_semantically_eq(&echoed, &stored));
}

#[test]
fn values_semantically_eq_rejects_genuinely_stale_pre_image() {
    // PosInt(99) vs Float(100.0): different values -- must still Conflict.
    let stale = serde_json::Value::Number(serde_json::Number::from(99u64));
    let current = serde_json::json!(100.0);
    assert!(!values_semantically_eq(&stale, &current));
}

#[test]
fn values_semantically_eq_recurses_into_nested_array_and_object() {
    // ActorsPanel-style vision pre-image: an array of objects with a Number
    // leaf that differs only in serde_json variant must be equal; the same
    // structure with a genuinely different nested value must not be.
    let a = serde_json::json!([{ "mode": "dark", "range": 30 }]);
    let b = serde_json::json!([{ "mode": "dark", "range": 30.0 }]);
    assert!(values_semantically_eq(&a, &b));

    let c = serde_json::json!([{ "mode": "dark", "range": 31.0 }]);
    assert!(!values_semantically_eq(&a, &c));
}

#[test]
fn values_semantically_eq_falls_back_to_exact_beyond_f64_precision() {
    // 2^53 + 1 cannot be represented exactly as f64 -- comparing it against
    // its lossy f64 neighbor must NOT be equated; fall back to exact/raw.
    let big_int = serde_json::Value::Number(serde_json::Number::from((1u64 << 53) + 1));
    let lossy_float = serde_json::json!(((1u64 << 53) + 1) as f64);
    assert!(!values_semantically_eq(&big_int, &lossy_float));
}

#[test]
fn values_semantically_eq_accepts_negative_whole_number_variant_mismatch() {
    // NegInt(-50) vs Float(-50.0): same negative whole number, different
    // variant -- must be treated equal.
    let neg_int = serde_json::Value::Number(serde_json::Number::from(-50i64));
    let neg_float = serde_json::json!(-50.0);
    assert!(values_semantically_eq(&neg_int, &neg_float));
}

#[test]
fn values_semantically_eq_rejects_large_posint_pair_aliased_by_f64() {
    // 2^62 and 2^62 + 1 are both PosInt (both fit in i128 exactly) but
    // alias to the same f64 value if compared lossily -- the both-integer
    // path must compare them exactly and reject the match.
    let a = serde_json::Value::Number(serde_json::Number::from(1u64 << 62));
    let b = serde_json::Value::Number(serde_json::Number::from((1u64 << 62) + 1));
    // Sanity: confirm these two DO alias under a naive f64 cast, i.e. this
    // is a real repro and not a vacuous case.
    assert_eq!(a.as_f64(), b.as_f64());
    assert!(!values_semantically_eq(&a, &b));
}

#[test]
fn values_semantically_eq_rejects_large_negint_pair_aliased_by_f64() {
    // Negative counterpart: two distinct large NegInt values that alias
    // when cast to f64 must still be rejected as unequal.
    let a = serde_json::Value::Number(serde_json::Number::from(-(1i64 << 62)));
    let b = serde_json::Value::Number(serde_json::Number::from(-((1i64 << 62) + 1)));
    assert_eq!(a.as_f64(), b.as_f64());
    assert!(!values_semantically_eq(&a, &b));
}

#[test]
fn values_semantically_eq_rejects_posint_vs_negint_same_magnitude() {
    // PosInt(100) vs NegInt(-100): same absolute value, opposite sign --
    // sign must be respected, not just magnitude.
    let pos = serde_json::Value::Number(serde_json::Number::from(100u64));
    let neg = serde_json::Value::Number(serde_json::Number::from(-100i64));
    assert!(!values_semantically_eq(&pos, &neg));
}

#[test]
fn values_semantically_eq_accepts_equal_small_posint_pair() {
    // Both-integer, genuinely equal values must still compare equal.
    let a = serde_json::Value::Number(serde_json::Number::from(5u64));
    let b = serde_json::Value::Number(serde_json::Number::from(5u64));
    assert!(values_semantically_eq(&a, &b));
}

#[tokio::test]
async fn list_members_includes_usernames() {
    let r = repo().await;
    let gm = r
        .create_user("alice", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let members = r.list_members(w.id).await.unwrap();
    assert!(members.iter().any(|(_, name, _)| name == "alice"));
}

#[tokio::test]
async fn list_members_orders_by_username() {
    let r = repo().await;
    let gm = r
        .create_user("zeke", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    // Non-alphabetical insertion order: zeke (owner/GM), then mona, then abby.
    let mona = r
        .create_user("mona", None, ServerRole::User, 0)
        .await
        .unwrap();
    let abby = r
        .create_user("abby", None, ServerRole::User, 0)
        .await
        .unwrap();
    r.add_member(w.id, mona, WorldRole::Player).await.unwrap();
    r.add_member(w.id, abby, WorldRole::Player).await.unwrap();

    let members = r.list_members(w.id).await.unwrap();
    let names: Vec<&str> = members.iter().map(|(_, name, _)| name.as_str()).collect();
    assert_eq!(names, vec!["abby", "mona", "zeke"]);
}

#[tokio::test]
async fn list_members_orders_case_insensitively() {
    let r = repo().await;
    let gm = r
        .create_user("Bob", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let alice = r
        .create_user("alice", None, ServerRole::User, 0)
        .await
        .unwrap();
    let charlie = r
        .create_user("Charlie", None, ServerRole::User, 0)
        .await
        .unwrap();
    r.add_member(w.id, alice, WorldRole::Player).await.unwrap();
    r.add_member(w.id, charlie, WorldRole::Player)
        .await
        .unwrap();

    let members = r.list_members(w.id).await.unwrap();
    let names: Vec<&str> = members.iter().map(|(_, name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        vec!["alice", "Bob", "Charlie"],
        "case-insensitive order: alice before Bob before Charlie"
    );
}

#[tokio::test]
async fn cannot_remove_sole_gm() {
    let r = repo().await;
    let gm = r
        .create_user("gm", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let err = r.remove_member(w.id, gm).await.unwrap_err();
    assert!(matches!(err, DataError::Conflict(_)));
}

#[tokio::test]
async fn cannot_demote_sole_gm() {
    let r = repo().await;
    let gm = r
        .create_user("gm", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let err = r.set_role(w.id, gm, WorldRole::Player).await.unwrap_err();
    assert!(matches!(err, DataError::Conflict(_)));
}

#[tokio::test]
async fn can_remove_gm_when_another_exists() {
    let r = repo().await;
    let gm1 = r
        .create_user("gm1", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let gm2 = r
        .create_user("gm2", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm1, 0).await.unwrap();
    r.add_member(w.id, gm2, WorldRole::Gm).await.unwrap();
    assert!(r.remove_member(w.id, gm1).await.is_ok());
}

#[tokio::test]
async fn repository_trait_member_role_matches_inherent_method() {
    use crate::auth::role::ServerRole;
    use crate::data::repository::Repository;

    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = r
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let stranger = r
        .create_user("st", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    r.add_member(w.id, player, WorldRole::Player).await.unwrap();

    let dyn_repo: &dyn Repository = &r;
    assert_eq!(
        dyn_repo.member_role(w.id, player).await.unwrap(),
        Some(WorldRole::Player)
    );
    assert_eq!(dyn_repo.member_role(w.id, stranger).await.unwrap(), None);
}

#[tokio::test]
async fn parent_id_round_trips_and_query_children_filters() {
    let repo = repo().await;
    let owner = repo
        .create_user("u", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let scene = Uuid::from_u128(10);
    let token = Uuid::from_u128(11);
    let scene_doc = crate::data::document::tests::world_scoped_doc(world.id, scene, "scene");
    let mut token_doc = crate::data::document::tests::world_scoped_doc(world.id, token, "token");
    token_doc.parent_id = Some(scene);
    repo.apply_command(UnsequencedCommand {
        world_id: world.id,
        author: owner,
        ts: 0,
        ops: vec![
            Operation::Create { doc: scene_doc },
            Operation::Create { doc: token_doc },
        ],
    })
    .await
    .unwrap();

    let children = repo.query_children(scene).await.unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].id, token);
    assert_eq!(children[0].parent_id, Some(scene));
    // The scene itself has no parent, so it is not its own child.
    assert!(repo.query_children(token).await.unwrap().is_empty());
}

/// Seed `world` with one of every world-keyed row family: a member, a
/// scene doc + child token (⇒ documents, FTS rows, a world_events row),
/// an asset, an invite, an explored_fog row, and all five settings blobs.
/// Returns the scene id.
async fn seed_world_rows(repo: &SqliteRepository, world: Uuid, owner: Uuid) -> Uuid {
    let scene = Uuid::new_v4();
    let token = Uuid::new_v4();
    let mk = |id, parent: Option<Uuid>, ty| {
        let mut d = crate::data::document::tests::world_scoped_doc(world, id, ty);
        d.parent_id = parent;
        d.owner = Some(owner);
        d.name = Some("Searchable alpha text".into());
        Operation::Create { doc: d }
    };
    repo.apply_command(UnsequencedCommand {
        world_id: world,
        author: owner,
        ts: 0,
        ops: vec![mk(scene, None, "scene"), mk(token, Some(scene), "token")],
    })
    .await
    .unwrap();
    repo.insert_asset(&crate::data::asset::Asset {
        id: Uuid::new_v4(),
        world_id: world,
        storage_key: format!("{world}/asset"),
        original_name: "a.png".into(),
        content_type: "image/png".into(),
        byte_size: 4,
        created_by: Some(owner),
        created_at: 0,
        version: 1,
    })
    .await
    .unwrap();
    assert!(repo
        .create_invite(
            NewInvite {
                id: Uuid::new_v4(),
                world,
                secret_hash: "phc",
                role: WorldRole::Player,
                created_by: owner,
                now: 0,
                expires_at: i64::MAX,
            },
            10,
        )
        .await
        .unwrap());
    repo.set_explored(world, scene, owner, &[1, 0, 0, 0, 2, 0, 0, 0])
        .await
        .unwrap();
    repo.set_world_cap_defaults(world, &WorldCapDefaults::default())
        .await
        .unwrap();
    repo.set_world_cap_requirements(world, &[]).await.unwrap();
    repo.set_world_contract_declarations(world, &[])
        .await
        .unwrap();
    repo.set_world_schema_declarations(world, &[])
        .await
        .unwrap();
    repo.set_world_enabled_modules(world, &[]).await.unwrap();
    scene
}

/// COUNT(*) of rows in `table` whose `col` equals `bind`. Test-only
/// dynamic identifiers (values stay parameterized), hence `AssertSqlSafe`.
async fn count_where(repo: &SqliteRepository, table: &str, col: &str, bind: String) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT COUNT(*) FROM {table} WHERE {col} = ?"
    )))
    .bind(bind)
    .fetch_one(repo.pool())
    .await
    .unwrap()
}

#[tokio::test]
async fn delete_world_removes_every_keyed_row() {
    let repo = repo().await;
    let u1 = repo
        .create_user("u1", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let u2 = repo
        .create_user("u2", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let w1 = repo.create_world_owned("w1", u1, 0).await.unwrap().id;
    let w2 = repo.create_world_owned("w2", u2, 0).await.unwrap().id;
    repo.add_member(w1, u2, WorldRole::Player).await.unwrap();
    seed_world_rows(&repo, w1, u1).await;
    seed_world_rows(&repo, w2, u2).await;

    repo.delete_world(w1).await.expect("delete w1");

    // Every world-keyed family: w1 rows gone, w2 rows intact.
    for (table, col, gone, kept) in [
        ("worlds", "id", 0, 1),
        ("world_members", "world_id", 0, 1),
        ("documents", "world_id", 0, 2),
        ("world_events", "world_id", 0, 1),
        ("assets", "world_id", 0, 1),
        ("world_invites", "world_id", 0, 1),
        ("explored_fog", "world_id", 0, 1),
        // THE PIN: the FTS AFTER DELETE triggers fired under the FK
        // cascade on the bundled SQLite — no explicit FTS delete exists
        // in delete_world's transaction.
        ("documents_fts_public", "world_id", 0, 2),
        ("documents_fts_gm", "world_id", 0, 2),
    ] {
        assert_eq!(
            count_where(&repo, table, col, w1.to_string()).await,
            gone,
            "{table} rows for deleted w1"
        );
        assert_eq!(
            count_where(&repo, table, col, w2.to_string()).await,
            kept,
            "{table} rows for surviving w2"
        );
    }
    // The five FK-less settings blobs are purged for w1, kept for w2.
    for (k1, k2) in [
        (world_caps_key(w1), world_caps_key(w2)),
        (world_caps_req_key(w1), world_caps_req_key(w2)),
        (world_contracts_key(w1), world_contracts_key(w2)),
        (world_schemas_key(w1), world_schemas_key(w2)),
        (world_modules_key(w1), world_modules_key(w2)),
    ] {
        assert_eq!(count_where(&repo, "settings", "key", k1).await, 0);
        assert_eq!(count_where(&repo, "settings", "key", k2).await, 1);
    }
    // The deleted world's users survive (only membership rows cascade).
    assert!(repo.user_exists(u1).await.unwrap());
}

/// Persist a REAL session record for `user` through the production store,
/// so assertions against `$.data.user.id` exercise the actual `save()`
/// serialization, not a hand-rolled imitation of it.
async fn seed_session(repo: &SqliteRepository, key: i128, user: Uuid, name: &str) {
    use tower_sessions::session_store::SessionStore;
    let read_pool = repo.open_read_pool().await.unwrap();
    let store = crate::auth::session::SqlxSqliteStore::new(repo.pool().clone(), read_pool);
    store.migrate().await.unwrap();
    let mut data = std::collections::HashMap::new();
    data.insert(
        "user".to_string(),
        serde_json::to_value(crate::auth::session::SessionUser {
            id: user,
            username: name.into(),
            role: ServerRole::User,
        })
        .unwrap(),
    );
    let record = tower_sessions::session::Record {
        id: tower_sessions::session::Id(key),
        data,
        expiry_date: tower_sessions::cookie::time::OffsetDateTime::now_utc()
            + tower_sessions::cookie::time::Duration::days(1),
    };
    store.save(&record).await.unwrap();
}

/// COUNT(*) of live sessions whose embedded identity is `user`.
async fn session_count_for(repo: &SqliteRepository, user: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM tower_sessions \
         WHERE json_extract(data, '$.data.user.id') = ?",
    )
    .bind(user.to_string())
    .fetch_one(repo.pool())
    .await
    .unwrap()
}

#[tokio::test]
async fn delete_user_scrubs_everything() {
    let repo = repo().await;
    let admin = repo
        .create_user("root", Some("h"), ServerRole::Admin, 0)
        .await
        .unwrap();
    let u = repo
        .create_user("u", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("w", admin, 0).await.unwrap().id;
    repo.add_member(w, u, WorldRole::Player).await.unwrap();

    // U owns a document and authors its creating event.
    let scene = Uuid::new_v4();
    let mut d = crate::data::document::tests::world_scoped_doc(w, scene, "scene");
    d.owner = Some(u);
    repo.apply_command(UnsequencedCommand {
        world_id: w,
        author: u,
        ts: 0,
        ops: vec![Operation::Create { doc: d }],
    })
    .await
    .unwrap();
    // U uploaded an asset and minted an invite.
    let asset_id = Uuid::new_v4();
    repo.insert_asset(&crate::data::asset::Asset {
        id: asset_id,
        world_id: w,
        storage_key: format!("{w}/{asset_id}"),
        original_name: "a.png".into(),
        content_type: "image/png".into(),
        byte_size: 4,
        created_by: Some(u),
        created_at: 0,
        version: 1,
    })
    .await
    .unwrap();
    assert!(repo
        .create_invite(
            NewInvite {
                id: Uuid::new_v4(),
                world: w,
                secret_hash: "phc",
                role: WorldRole::Player,
                created_by: u,
                now: 0,
                expires_at: i64::MAX,
            },
            10,
        )
        .await
        .unwrap());
    // Fog memory for U (purged) and for the admin (survives).
    repo.set_explored(w, scene, u, &[1, 0, 0, 0, 2, 0, 0, 0])
        .await
        .unwrap();
    repo.set_explored(w, scene, admin, &[1, 0, 0, 0, 2, 0, 0, 0])
        .await
        .unwrap();
    // Live sessions for both.
    seed_session(&repo, 1, u, "u").await;
    seed_session(&repo, 2, admin, "root").await;

    repo.delete_user(u).await.expect("delete");

    assert!(!repo.user_exists(u).await.unwrap());
    assert_eq!(
        count_where(&repo, "world_members", "user_id", u.to_string()).await,
        0
    );
    // SET NULL families: the rows survive, attribution nulls.
    assert_eq!(repo.get_document(scene).await.unwrap().unwrap().owner, None);
    assert_eq!(
        count_where(&repo, "world_events", "author_id", u.to_string()).await,
        0
    );
    let null_authored: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM world_events WHERE author_id IS NULL")
            .fetch_one(repo.pool())
            .await
            .unwrap();
    assert_eq!(null_authored, 1, "event row survives with author nulled");
    let a = repo.get_asset(asset_id).await.unwrap().expect("row intact");
    assert_eq!(a.created_by, None);
    assert_eq!(
        count_where(&repo, "world_invites", "created_by", u.to_string()).await,
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM world_invites WHERE created_by IS NULL")
            .fetch_one(repo.pool())
            .await
            .unwrap(),
        1,
        "invite row survives with minter nulled"
    );
    // FK-less purges: U's fog and sessions die, the admin's survive.
    assert_eq!(
        count_where(&repo, "explored_fog", "user_id", u.to_string()).await,
        0
    );
    assert_eq!(
        count_where(&repo, "explored_fog", "user_id", admin.to_string()).await,
        1
    );
    assert_eq!(session_count_for(&repo, u).await, 0);
    assert_eq!(session_count_for(&repo, admin).await, 1);
}

#[tokio::test]
async fn delete_user_guards() {
    let repo = repo().await;
    // delete_user's documented boot coupling: the session table exists
    // before any route can reach it; repo-level tests create it themselves.
    crate::auth::session::SqlxSqliteStore::new(
        repo.pool().clone(),
        repo.open_read_pool().await.unwrap(),
    )
    .migrate()
    .await
    .unwrap();
    assert!(matches!(
        repo.delete_user(Uuid::new_v4()).await,
        Err(DataError::NotFound)
    ));
    let a1 = repo
        .create_user("a1", Some("h"), ServerRole::Admin, 0)
        .await
        .unwrap();
    match repo.delete_user(a1).await {
        Err(DataError::Conflict(m)) => {
            assert_eq!(m, "cannot delete the server's only administrator")
        }
        other => panic!("expected Conflict, got {other:?}"),
    }
    let a2 = repo
        .create_user("a2", Some("h"), ServerRole::Admin, 0)
        .await
        .unwrap();
    repo.delete_user(a1)
        .await
        .expect("with two admins, deleting one succeeds");
    assert!(
        matches!(repo.delete_user(a2).await, Err(DataError::Conflict(_))),
        "the survivor is now the last admin"
    );
}

#[tokio::test]
async fn user_delete_nulls_asset_created_by() {
    let repo = repo().await;
    let u = repo
        .create_user("u", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let keeper = repo
        .create_user("keeper", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("w", keeper, 0).await.unwrap().id;
    let asset_id = Uuid::new_v4();
    repo.insert_asset(&crate::data::asset::Asset {
        id: asset_id,
        world_id: w,
        storage_key: format!("{w}/{asset_id}"),
        original_name: "a.png".into(),
        content_type: "image/png".into(),
        byte_size: 4,
        created_by: Some(u),
        created_at: 0,
        version: 1,
    })
    .await
    .unwrap();

    // Raw row delete: this pins the 0011 FK ACTION itself (repo-level
    // delete_user arrives in the next task).
    sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(u.to_string())
        .execute(repo.pool())
        .await
        .expect("user delete must not FK-fail on authored assets");

    let a = repo.get_asset(asset_id).await.unwrap().expect("row intact");
    assert_eq!(a.created_by, None);
    assert_eq!(a.byte_size, 4);
    assert_eq!(a.version, 1);
}

#[tokio::test]
async fn delete_world_not_found() {
    let repo = repo().await;
    assert!(matches!(
        repo.delete_world(Uuid::new_v4()).await,
        Err(DataError::NotFound)
    ));
}

#[tokio::test]
async fn upsert_member_inserts_updates_and_guards() {
    let repo = repo().await;
    let gm = repo
        .create_user("gm", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let p = repo
        .create_user("p", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("w", gm, 0).await.unwrap().id;

    // New member insert.
    repo.upsert_member(w, p, WorldRole::Player).await.unwrap();
    assert_eq!(
        repo.member_role(w, p).await.unwrap(),
        Some(WorldRole::Player)
    );
    // Same call with a different role updates in place (upsert).
    repo.upsert_member(w, p, WorldRole::Spectator)
        .await
        .unwrap();
    assert_eq!(
        repo.member_role(w, p).await.unwrap(),
        Some(WorldRole::Spectator)
    );
    // Demoting the world's ONLY GM → Conflict.
    match repo.upsert_member(w, gm, WorldRole::Player).await {
        Err(DataError::Conflict(m)) => {
            assert_eq!(m, "cannot demote the world's only GM")
        }
        other => panic!("expected Conflict, got {other:?}"),
    }
    // With a second GM promoted, demoting the first succeeds.
    repo.upsert_member(w, p, WorldRole::Gm).await.unwrap();
    repo.upsert_member(w, gm, WorldRole::Player).await.unwrap();
    assert_eq!(
        repo.member_role(w, gm).await.unwrap(),
        Some(WorldRole::Player)
    );
    // Unknown user or unknown world → NotFound, never an FK 500.
    assert!(matches!(
        repo.upsert_member(w, Uuid::new_v4(), WorldRole::Player)
            .await,
        Err(DataError::NotFound)
    ));
    assert!(matches!(
        repo.upsert_member(Uuid::new_v4(), p, WorldRole::Player)
            .await,
        Err(DataError::NotFound)
    ));
}

/// World + owner + a scene doc with one token child + fog rows for the
/// scene (owner and `other`) + a fog row for a second scene (survivor).
/// Returns `(world, scene_id, other_scene_id, other_user)`.
async fn fog_purge_fixture(repo: &SqliteRepository, owner: Uuid) -> (Uuid, Uuid, Uuid, Uuid) {
    let other = repo
        .create_user("watcher", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("w", owner, 0).await.unwrap().id;
    let scene = Uuid::new_v4();
    let token = Uuid::new_v4();
    let other_scene = Uuid::new_v4();
    let mk = |id, parent: Option<Uuid>, ty| {
        let mut d = crate::data::document::tests::world_scoped_doc(w, id, ty);
        d.parent_id = parent;
        d.owner = Some(owner);
        Operation::Create { doc: d }
    };
    repo.apply_command(UnsequencedCommand {
        world_id: w,
        author: owner,
        ts: 0,
        ops: vec![
            mk(scene, None, "scene"),
            mk(token, Some(scene), "token"),
            mk(other_scene, None, "scene"),
        ],
    })
    .await
    .unwrap();
    for user in [owner, other] {
        repo.set_explored(w, scene, user, &[1, 0, 0, 0, 2, 0, 0, 0])
            .await
            .unwrap();
    }
    repo.set_explored(w, other_scene, owner, &[1, 0, 0, 0, 2, 0, 0, 0])
        .await
        .unwrap();
    (w, scene, other_scene, other)
}

#[tokio::test]
async fn scene_delete_purges_fog_via_apply_intent() {
    let repo = repo().await;
    let owner = repo
        .create_user("u", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let (w, scene, other_scene, _other) = fog_purge_fixture(&repo, owner).await;

    let ctx = repo
        .permission_context(w, owner, ServerRole::User)
        .await
        .unwrap();
    let scene_doc = repo.get_document(scene).await.unwrap().unwrap();
    repo.apply_intent(
        &ctx,
        w,
        vec![Operation::Delete { doc: scene_doc }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    assert_eq!(
        count_where(&repo, "explored_fog", "scene_id", scene.to_string()).await,
        0,
        "deleted scene's fog rows purged (all users)"
    );
    assert_eq!(
        count_where(&repo, "explored_fog", "scene_id", other_scene.to_string()).await,
        1,
        "other scene's fog survives"
    );
}

#[tokio::test]
async fn scene_delete_purges_fog_via_apply_command() {
    let repo = repo().await;
    let owner = repo
        .create_user("u", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let (w, scene, other_scene, _other) = fog_purge_fixture(&repo, owner).await;

    let scene_doc = repo.get_document(scene).await.unwrap().unwrap();
    repo.apply_command(UnsequencedCommand {
        world_id: w,
        author: owner,
        ts: 1,
        ops: vec![Operation::Delete { doc: scene_doc }],
    })
    .await
    .unwrap();

    assert_eq!(
        count_where(&repo, "explored_fog", "scene_id", scene.to_string()).await,
        0,
        "apply_command parity: fog purged through the same shared helper"
    );
    assert_eq!(
        count_where(&repo, "explored_fog", "scene_id", other_scene.to_string()).await,
        1
    );
}

#[tokio::test]
async fn deleting_a_scene_expands_to_descendant_delete_ops() {
    let repo = repo().await;
    let owner = repo
        .create_user("u", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let world = repo.create_world_owned("w", owner, 0).await.unwrap();
    let scene = Uuid::from_u128(10);
    let t1 = Uuid::from_u128(11);
    let t2 = Uuid::from_u128(12);
    let mk = |id, parent: Option<Uuid>, ty| {
        let mut d = crate::data::document::tests::world_scoped_doc(world.id, id, ty);
        d.parent_id = parent;
        d.owner = Some(owner);
        Operation::Create { doc: d }
    };
    repo.apply_command(UnsequencedCommand {
        world_id: world.id,
        author: owner,
        ts: 0,
        ops: vec![
            mk(scene, None, "scene"),
            mk(t1, Some(scene), "token"),
            mk(t2, Some(scene), "token"),
        ],
    })
    .await
    .unwrap();

    let ctx = repo
        .permission_context(world.id, owner, ServerRole::User)
        .await
        .unwrap();
    // Delete the scene only; expect the Command to carry 3 Delete ops.
    let scene_doc = repo.get_document(scene).await.unwrap().unwrap();
    let cmd = repo
        .apply_intent(
            &ctx,
            world.id,
            vec![Operation::Delete { doc: scene_doc }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap()
        .command;
    let deleted: Vec<Uuid> = cmd
        .ops
        .iter()
        .filter_map(|o| match o {
            Operation::Delete { doc } => Some(doc.id),
            _ => None,
        })
        .collect();
    assert_eq!(deleted.len(), 3, "scene + 2 children");
    assert!(deleted.contains(&scene) && deleted.contains(&t1) && deleted.contains(&t2));
    // Children deleted before their parent (reversible-order invariant).
    let scene_pos = deleted.iter().position(|&d| d == scene).unwrap();
    assert!(deleted.iter().position(|&d| d == t1).unwrap() < scene_pos);
    // Store is empty for the world's scene entities.
    assert!(repo.query_children(scene).await.unwrap().is_empty());
    assert!(repo.get_document(t1).await.unwrap().is_none());
}

#[tokio::test]
async fn self_referential_parent_create_is_rejected() {
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
    let mut d = tests_doc(perms, serde_json::json!({ "name": "Loop" }));
    d.scope = Scope::World { world_id: w.id };
    d.parent_id = Some(d.id); // its own parent poisons the descendant walk
    let err = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Create { doc: d }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    // OpFailed, not Forbidden: the self-parent check precedes the access check.
    assert!(
        matches!(&err, DataError::OpFailed(m) if m.contains("own parent")),
        "expected self-parent rejection, got {err:?}"
    );
}

#[tokio::test]
async fn cross_world_parent_create_is_rejected() {
    use crate::data::document::{DocRole, PermissionSet, Scope};
    use crate::data::membership::PermissionContext;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let wa = r.create_world_owned("A", gm, 0).await.unwrap();
    let wb = r.create_world_owned("B", gm, 0).await.unwrap();
    // Parent persisted in world B (the self-FK references the global documents
    // table, so a cross-world parent_id satisfies the FK and must be caught by
    // the scope check instead).
    let parent_id = Uuid::from_u128(77);
    let parent = crate::data::document::tests::world_scoped_doc(wb.id, parent_id, "scene");
    r.apply_command(UnsequencedCommand {
        world_id: wb.id,
        author: gm,
        ts: 0,
        ops: vec![Operation::Create { doc: parent }],
    })
    .await
    .unwrap();
    // Child in world A pointing at the world-B parent.
    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let mut perms = PermissionSet::default();
    perms.users.insert(gm, DocRole::Owner);
    let mut child = tests_doc(perms, serde_json::json!({}));
    child.scope = Scope::World { world_id: wa.id };
    child.parent_id = Some(parent_id);
    let err = r
        .apply_intent(
            &ctx,
            wa.id,
            vec![Operation::Create { doc: child }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, DataError::OpFailed(m) if m.contains("scope")),
        "expected cross-world parent rejection, got {err:?}"
    );
}

#[tokio::test]
async fn self_referential_parent_delete_terminates() {
    // The trusted apply_command path does not reject a self-parent (only
    // apply_intent does), so a self-referential row can reach the store via
    // replay or migration. The descendant walk's visited-set must terminate
    // rather than recurse forever; without it this test stack-overflows.
    let r = repo().await;
    let owner = r
        .create_user("u", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let world = r.create_world_owned("w", owner, 0).await.unwrap();
    let id = Uuid::from_u128(42);
    let mut d = crate::data::document::tests::world_scoped_doc(world.id, id, "scene");
    d.parent_id = Some(id); // self-referential
    d.owner = Some(owner);
    r.apply_command(UnsequencedCommand {
        world_id: world.id,
        author: owner,
        ts: 0,
        ops: vec![Operation::Create { doc: d.clone() }],
    })
    .await
    .unwrap();
    let cmd = r
        .apply_command(UnsequencedCommand {
            world_id: world.id,
            author: owner,
            ts: 1,
            ops: vec![Operation::Delete { doc: d }],
        })
        .await
        .unwrap()
        .command;
    // The self-reference yields no extra descendant op — just the row itself.
    let deletes = cmd
        .ops
        .iter()
        .filter(|o| matches!(o, Operation::Delete { .. }))
        .count();
    assert_eq!(deletes, 1);
    assert!(r.get_document(id).await.unwrap().is_none());
}

#[tokio::test]
async fn query_scene_entities_returns_scenes_and_children_only() {
    // Guards loader/predicate drift: query_scene_entities must select exactly
    // the docs is_scene_entity accepts (scenes plus anything with a parent).
    let r = repo().await;
    let owner = r
        .create_user("u", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let world = r.create_world_owned("w", owner, 0).await.unwrap();
    let scene = Uuid::from_u128(10);
    let token = Uuid::from_u128(11);
    let actor = Uuid::from_u128(12);
    let mk = |id, parent: Option<Uuid>, ty| {
        let mut d = crate::data::document::tests::world_scoped_doc(world.id, id, ty);
        d.parent_id = parent;
        d.owner = Some(owner);
        Operation::Create { doc: d }
    };
    r.apply_command(UnsequencedCommand {
        world_id: world.id,
        author: owner,
        ts: 0,
        ops: vec![
            mk(scene, None, "scene"),
            mk(token, Some(scene), "token"),
            mk(actor, None, "actor"), // top-level non-scene → excluded
        ],
    })
    .await
    .unwrap();
    let ids: Vec<Uuid> = r
        .query_scene_entities(world.id)
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.id)
        .collect();
    assert!(ids.contains(&scene) && ids.contains(&token));
    assert!(
        !ids.contains(&actor),
        "top-level non-scene doc must be excluded"
    );
    assert_eq!(ids.len(), 2);
}

#[tokio::test]
async fn asset_insert_get_replace_delete_list_round_trip() {
    use crate::data::asset::Asset;
    let r = repo().await;
    let owner = r
        .create_user("u", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let world = r.create_world_owned("w", owner, 0).await.unwrap();
    let id = Uuid::from_u128(500);
    let a = Asset {
        id,
        world_id: world.id,
        storage_key: format!("{}/{}", world.id, id),
        original_name: "battlemap.png".into(),
        content_type: "image/png".into(),
        byte_size: 1234,
        created_by: Some(owner),
        created_at: 0,
        version: 1,
    };
    r.insert_asset(&a).await.unwrap();
    assert_eq!(r.get_asset(id).await.unwrap().unwrap(), a);

    // Replace bumps version and updates byte metadata.
    let v = r
        .replace_asset_bytes(id, &a.storage_key, "image/jpeg", 4321)
        .await
        .unwrap();
    assert_eq!(v, 2);
    let after = r.get_asset(id).await.unwrap().unwrap();
    assert_eq!(
        (after.version, after.byte_size, after.content_type.as_str()),
        (2, 4321, "image/jpeg")
    );

    // List returns the world's assets.
    assert_eq!(r.list_assets_by_world(world.id).await.unwrap().len(), 1);

    // Delete returns the removed record and empties the store.
    assert_eq!(r.delete_asset(id).await.unwrap().unwrap().id, id);
    assert!(r.get_asset(id).await.unwrap().is_none());
    assert!(r.list_assets_by_world(world.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn contract_declarations_round_trip_and_default_empty() {
    use crate::data::document::{Cardinality, ContractDeclaration, ContractProvide};
    let repo = repo().await;
    let world = repo.create_world("w", 0).await.unwrap();

    // Unset → empty.
    assert!(repo
        .world_contract_declarations(world.id)
        .await
        .unwrap()
        .is_empty());

    let decls = vec![ContractDeclaration {
        module_id: "core-ui".into(),
        version: "0.1.0".into(),
        provides: vec![ContractProvide {
            contract: "example.surface:widget".into(),
            cardinality: Cardinality::Singleton,
        }],
        requires: vec![],
    }];
    repo.set_world_contract_declarations(world.id, &decls)
        .await
        .unwrap();

    let got = repo.world_contract_declarations(world.id).await.unwrap();
    assert_eq!(got, decls);
}

#[tokio::test]
async fn schema_declarations_round_trip_and_default_empty() {
    use crate::data::document::{Schema, SchemaDeclaration, SchemaType};
    let repo = repo().await;
    let world = repo.create_world("W", 0).await.unwrap();

    // Default empty.
    assert!(repo
        .world_schema_declarations(world.id)
        .await
        .unwrap()
        .is_empty());

    let decls = vec![SchemaDeclaration {
        module_id: "example-system".into(),
        version: "1.0.0".into(),
        schema_format: 1,
        doc_type: "actor".into(),
        subtree_pointer: "/system/stats".into(),
        schema: Schema {
            ty: Some(SchemaType::Object),
            ..Default::default()
        },
    }];
    repo.set_world_schema_declarations(world.id, &decls)
        .await
        .unwrap();
    let got = repo.world_schema_declarations(world.id).await.unwrap();
    assert_eq!(got, decls);
}

#[tokio::test]
async fn worlds_for_user_scopes_to_membership_and_admin_sees_all() {
    let repo = repo().await;
    let a = repo
        .create_user("a", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let b = repo
        .create_user("b", Some("h"), ServerRole::User, 0)
        .await
        .unwrap();
    let admin = repo
        .create_user("ad", Some("h"), ServerRole::Admin, 0)
        .await
        .unwrap();

    // a GMs world1; b GMs world2 (each creator seated as GM).
    let w1 = repo.create_world_owned("world1", a, 0).await.unwrap();
    let w2 = repo.create_world_owned("world2", b, 0).await.unwrap();
    // a is added to world2 as a player.
    repo.add_member(w2.id, a, WorldRole::Player).await.unwrap();

    // a sees only their two worlds, with the right roles; never b-only state.
    let mut a_worlds = repo.worlds_for_user(a, ServerRole::User).await.unwrap();
    a_worlds.sort_by(|x, y| x.0.name.cmp(&y.0.name));
    assert_eq!(a_worlds.len(), 2);
    assert_eq!((a_worlds[0].0.id, a_worlds[0].1), (w1.id, WorldRole::Gm));
    assert_eq!(
        (a_worlds[1].0.id, a_worlds[1].1),
        (w2.id, WorldRole::Player)
    );

    // b sees only world2.
    let b_worlds = repo.worlds_for_user(b, ServerRole::User).await.unwrap();
    assert_eq!(b_worlds.len(), 1);
    assert_eq!(b_worlds[0].0.id, w2.id);

    // A server admin sees every world as GM.
    let admin_worlds = repo
        .worlds_for_user(admin, ServerRole::Admin)
        .await
        .unwrap();
    assert_eq!(admin_worlds.len(), 2);
    assert!(admin_worlds.iter().all(|(_, r)| *r == WorldRole::Gm));
}

/// Parse a user's stored UI-state for structural assertions.
async fn ui_state_of(repo: &SqliteRepository, user: Uuid) -> serde_json::Value {
    serde_json::from_str(&repo.get_ui_state(user).await.unwrap().unwrap()).unwrap()
}

#[tokio::test]
async fn ui_state_merges_per_top_level_key_and_per_world() {
    let repo = repo().await;
    let user = repo
        .create_user("u", Some("hash"), ServerRole::User, 0)
        .await
        .unwrap();

    // Unset → None.
    assert_eq!(repo.get_ui_state(user).await.unwrap(), None);

    // Seed one session's slices: global + world w1.
    repo.merge_ui_state(
        user,
        &serde_json::json!({
            "global": { "locale": "en", "lastWorld": "w1" },
            "worlds": { "w1": { "panelLayout": { "version": 1, "dock": true } } },
        }),
        64 * 1024,
    )
    .await
    .unwrap();

    // A second session writing ONLY w2 must not revert global or w1 —
    // the clobber this granularity exists to prevent.
    repo.merge_ui_state(
        user,
        &serde_json::json!({ "worlds": { "w2": { "chatRead": { "general": 5 } } } }),
        64 * 1024,
    )
    .await
    .unwrap();
    let v = ui_state_of(&repo, user).await;
    assert_eq!(v["global"]["locale"], "en");
    assert_eq!(v["global"]["lastWorld"], "w1");
    assert_eq!(v["worlds"]["w1"]["panelLayout"]["dock"], true);
    assert_eq!(v["worlds"]["w2"]["chatRead"]["general"], 5);

    // A `worlds.w1.chatRead`-only patch merges INSIDE w1 — the other
    // owner's `panelLayout` key survives (leaf-key granularity).
    repo.merge_ui_state(
        user,
        &serde_json::json!({ "worlds": { "w1": { "chatRead": { "general": 9 } } } }),
        64 * 1024,
    )
    .await
    .unwrap();
    let v = ui_state_of(&repo, user).await;
    assert_eq!(v["worlds"]["w1"]["panelLayout"]["dock"], true);
    assert_eq!(v["worlds"]["w1"]["chatRead"]["general"], 9);

    // Re-writing w1's `panelLayout` replaces THAT KEY wholesale (stale
    // nested keys inside the blob drop; no deep merge) and leaves
    // `chatRead`, w2, and global untouched.
    repo.merge_ui_state(
        user,
        &serde_json::json!({ "worlds": { "w1": { "panelLayout": { "version": 2 } } } }),
        64 * 1024,
    )
    .await
    .unwrap();
    let v = ui_state_of(&repo, user).await;
    assert_eq!(v["worlds"]["w1"]["panelLayout"]["version"], 2);
    assert_eq!(v["worlds"]["w1"]["panelLayout"].get("dock"), None);
    assert_eq!(v["worlds"]["w1"]["chatRead"]["general"], 9);
    assert_eq!(v["worlds"]["w2"]["chatRead"]["general"], 5);

    // A `global.locale`-only patch merges INSIDE global — `lastWorld`
    // (the other owner's key) survives.
    repo.merge_ui_state(
        user,
        &serde_json::json!({ "global": { "locale": "fr" } }),
        64 * 1024,
    )
    .await
    .unwrap();
    let v = ui_state_of(&repo, user).await;
    assert_eq!(v["global"]["locale"], "fr");
    assert_eq!(v["global"]["lastWorld"], "w1");
    assert_eq!(v["worlds"]["w1"]["panelLayout"]["version"], 2);

    // Unknown user → NotFound.
    let ghost = Uuid::from_u128(1);
    assert!(matches!(
        repo.merge_ui_state(ghost, &serde_json::json!({}), 64 * 1024)
            .await,
        Err(DataError::NotFound)
    ));
}

#[tokio::test]
async fn ui_state_merge_null_removes_key_and_entry() {
    let repo = repo().await;
    let user = repo
        .create_user("u", Some("hash"), ServerRole::User, 0)
        .await
        .unwrap();

    // Seed two worlds and a global slice with two keys.
    repo.merge_ui_state(
        user,
        &serde_json::json!({
            "global": { "locale": "en", "lastWorld": "w1" },
            "worlds": {
                "w1": { "panelLayout": { "v": 1 }, "chatRead": { "general": 3 } },
                "w2": { "panelLayout": { "v": 2 } },
            },
        }),
        64 * 1024,
    )
    .await
    .unwrap();

    // `worlds.w1: null` removes the WHOLE w1 entry; the sibling w2 entry
    // survives untouched.
    repo.merge_ui_state(
        user,
        &serde_json::json!({ "worlds": { "w1": null } }),
        64 * 1024,
    )
    .await
    .unwrap();
    let v = ui_state_of(&repo, user).await;
    assert_eq!(v["worlds"].get("w1"), None);
    assert_eq!(v["worlds"]["w2"]["panelLayout"]["v"], 2);

    // Reseed w1 with two leaf keys, then remove just one of them via a
    // leaf-level `null` — the sibling leaf key survives.
    repo.merge_ui_state(
        user,
        &serde_json::json!({
            "worlds": { "w1": { "panelLayout": { "v": 1 }, "chatRead": { "general": 3 } } },
        }),
        64 * 1024,
    )
    .await
    .unwrap();
    repo.merge_ui_state(
        user,
        &serde_json::json!({ "worlds": { "w1": { "chatRead": null } } }),
        64 * 1024,
    )
    .await
    .unwrap();
    let v = ui_state_of(&repo, user).await;
    assert_eq!(v["worlds"]["w1"]["panelLayout"]["v"], 1);
    assert_eq!(v["worlds"]["w1"].get("chatRead"), None);

    // A leaf-level `null` inside `global` removes only that key; the
    // sibling global key survives.
    repo.merge_ui_state(
        user,
        &serde_json::json!({ "global": { "locale": null } }),
        64 * 1024,
    )
    .await
    .unwrap();
    let v = ui_state_of(&repo, user).await;
    assert_eq!(v["global"].get("locale"), None);
    assert_eq!(v["global"]["lastWorld"], "w1");
}

#[tokio::test]
async fn ui_state_merge_caps_the_merged_result_not_the_patch() {
    let repo = repo().await;
    let user = repo
        .create_user("u", Some("hash"), ServerRole::User, 0)
        .await
        .unwrap();
    let big = "x".repeat(600);
    repo.merge_ui_state(
        user,
        &serde_json::json!({ "worlds": { "w1": { "panelLayout": big } } }),
        1024,
    )
    .await
    .unwrap();

    // The second patch is small, but merged with w1 it exceeds the cap —
    // and the store must be left UNCHANGED (the tx never commits).
    let err = repo
        .merge_ui_state(
            user,
            &serde_json::json!({ "worlds": { "w2": { "panelLayout": "y".repeat(600) } } }),
            1024,
        )
        .await;
    assert!(matches!(err, Err(DataError::TooLarge(_))));
    let v = ui_state_of(&repo, user).await;
    assert_eq!(v["worlds"].get("w2"), None);
}

#[tokio::test]
async fn explored_fog_round_trips_and_is_per_scene_user() {
    let repo = repo().await;
    let world = Uuid::from_u128(9);
    let scene_a = Uuid::from_u128(10);
    let scene_b = Uuid::from_u128(11);
    let alice = Uuid::from_u128(20);
    let bob = Uuid::from_u128(21);

    // Unexplored → None.
    assert_eq!(repo.get_explored(scene_a, alice).await.unwrap(), None);

    // Set then read back the exact blob.
    repo.set_explored(world, scene_a, alice, &[1, 2, 3, 4])
        .await
        .unwrap();
    assert_eq!(
        repo.get_explored(scene_a, alice).await.unwrap(),
        Some(vec![1, 2, 3, 4])
    );

    // Upsert replaces (whole-blob), keyed (scene, user).
    repo.set_explored(world, scene_a, alice, &[9, 9])
        .await
        .unwrap();
    assert_eq!(
        repo.get_explored(scene_a, alice).await.unwrap(),
        Some(vec![9, 9])
    );

    // Isolation: another user and another scene are independent (no cross-player leak).
    assert_eq!(repo.get_explored(scene_a, bob).await.unwrap(), None);
    assert_eq!(repo.get_explored(scene_b, alice).await.unwrap(), None);
    repo.set_explored(world, scene_b, alice, &[7])
        .await
        .unwrap();
    assert_eq!(
        repo.get_explored(scene_a, alice).await.unwrap(),
        Some(vec![9, 9])
    );
    assert_eq!(
        repo.get_explored(scene_b, alice).await.unwrap(),
        Some(vec![7])
    );
}

#[tokio::test]
async fn create_with_invalid_engine_body_is_rejected() {
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
    let mut d = tests_engine_doc(
        perms,
        "wall",
        serde_json::json!({ "seg": { "x1": "not-a-number", "y1": 0.0, "x2": 1.0, "y2": 1.0 } }),
    );
    d.scope = Scope::World { world_id: w.id };
    let err = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Create { doc: d }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::BadEngine(_)));
}

#[tokio::test]
async fn create_of_non_engine_doc_type_with_engine_body_is_rejected() {
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
    let mut d = tests_engine_doc(perms, "item", serde_json::json!({ "anything": 1 }));
    d.scope = Scope::World { world_id: w.id };
    let err = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Create { doc: d }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::BadEngine(_)));
}

#[tokio::test]
async fn update_post_image_with_invalid_engine_is_rejected() {
    use crate::data::command::FieldChange;
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
    let mut d = tests_engine_doc(
        perms,
        "wall",
        serde_json::json!({ "seg": { "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0 } }),
    );
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

    // A field write that leaves the post-image engine undeserializable
    // (wrong type at /engine/seg/x1) must be rejected.
    let err = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/engine/seg/x1".into(),
                    old: serde_json::json!(0.0),
                    new: serde_json::json!("not-a-number"),
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::BadEngine(_)));
}

#[tokio::test]
async fn create_with_trailing_slash_property_override_key_is_rejected() {
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
    let mut d = tests_doc(perms, serde_json::json!({}));
    d.scope = Scope::World { world_id: w.id };
    d.permissions
        .property_overrides
        .insert("/engine/".into(), Visibility::GmOnly);
    let err = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Create { doc: d }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::BadPath(_)));
}

#[tokio::test]
async fn create_with_missing_leading_slash_property_override_key_is_rejected() {
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
    let mut d = tests_doc(perms, serde_json::json!({}));
    d.scope = Scope::World { world_id: w.id };
    d.permissions
        .property_overrides
        .insert("engine".into(), Visibility::GmOnly);
    let err = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Create { doc: d }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::BadPath(_)));
}

#[tokio::test]
async fn create_with_valid_property_override_keys_succeeds() {
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
    let mut d = tests_doc(perms, serde_json::json!({}));
    d.scope = Scope::World { world_id: w.id };
    d.permissions
        .property_overrides
        .insert("/engine".into(), Visibility::GmOnly);
    d.permissions
        .property_overrides
        .insert("/engine/vision".into(), Visibility::GmOnly);
    d.permissions
        .property_overrides
        .insert("/name".into(), Visibility::GmOnly);
    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Create { doc: d }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn update_with_trailing_slash_property_override_key_is_rejected() {
    use crate::data::command::FieldChange;
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
    let mut d = tests_doc(perms, serde_json::json!({}));
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

    let err = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/permissions/property_overrides".into(),
                    old: serde_json::json!({}),
                    new: serde_json::json!({ "/engine/": "gm_only" }),
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::BadPath(_)));
}

#[tokio::test]
async fn update_with_missing_leading_slash_property_override_key_is_rejected() {
    use crate::data::command::FieldChange;
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
    let mut d = tests_doc(perms, serde_json::json!({}));
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

    let err = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/permissions/property_overrides".into(),
                    old: serde_json::json!({}),
                    new: serde_json::json!({ "engine": "gm_only" }),
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::BadPath(_)));
}

#[tokio::test]
async fn update_with_valid_property_override_keys_succeeds() {
    use crate::data::command::FieldChange;
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
    let mut d = tests_doc(perms, serde_json::json!({}));
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

    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Update {
            doc_id,
            changes: vec![FieldChange {
                remove: false,
                path: "/permissions/property_overrides".into(),
                old: serde_json::json!({}),
                new: serde_json::json!({ "/engine": "gm_only", "/name": "gm_only" }),
            }],
        }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn update_writing_a_valid_engine_subpath_succeeds() {
    use crate::data::command::FieldChange;
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
    let mut d = tests_engine_doc(
        perms,
        "wall",
        serde_json::json!({ "seg": { "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0 } }),
    );
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

    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Update {
            doc_id,
            changes: vec![FieldChange {
                remove: false,
                path: "/engine/seg/x1".into(),
                old: serde_json::json!(0.0),
                new: serde_json::json!(5.0),
            }],
        }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let stored = r.get_document(doc_id).await.unwrap().unwrap();
    assert_eq!(stored.engine.unwrap()["seg"]["x1"], serde_json::json!(5.0));
}

#[tokio::test]
async fn create_actor_omitting_faction_persists_explicit_null() {
    // The stored/broadcast engine body is the RE-SERIALIZED validated
    // struct: `ActorEngine.faction` deserializes an absent key to
    // `None`, and normalization restores that as an explicit `null` on
    // the stored side, matching the client's `faction: string | null`
    // contract even though the ingress body omitted the key entirely.
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
    let mut d = tests_engine_doc(
        perms,
        "actor",
        serde_json::json!({
            "displayName": "Goblin",
            "visual": { "kind": "image", "asset": "a.png" },
            "size": { "w": 1.0, "h": 1.0 },
            "shape": "square",
            "conditions": [],
            "prototype": true
            // "faction" intentionally omitted from the wire submission
        }),
    );
    d.scope = Scope::World { world_id: w.id };
    let doc_id = d.id;

    let cmd = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Create { doc: d }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap()
        .command;

    // The returned Command (broadcast payload) already carries the
    // normalized engine body.
    let broadcast_engine = cmd
        .ops
        .iter()
        .find_map(|o| match o {
            Operation::Create { doc } if doc.id == doc_id => doc.engine.clone(),
            _ => None,
        })
        .expect("create op present");
    assert_eq!(broadcast_engine["faction"], serde_json::Value::Null);
    assert!(broadcast_engine.get("faction").is_some());

    // And the persisted row, independently re-fetched, matches.
    let stored = r.get_document(doc_id).await.unwrap().unwrap();
    let stored_engine = stored.engine.unwrap();
    assert_eq!(stored_engine["faction"], serde_json::Value::Null);
    assert!(stored_engine.get("faction").is_some());
}

#[tokio::test]
async fn apply_intent_update_normalizes_engine_broadcast_and_event_log_smuggled_key() {
    // `validate_engine_tree` re-serializes the post-image `doc.engine`,
    // dropping an unknown key smuggled into a tagged-enum sub-object
    // (`TokenVisual` cannot carry `deny_unknown_fields` -- a serde
    // limitation), but that normalization must reach the broadcast
    // `Command` AND the permanent `world_events` log entry, not just the
    // persisted row. Assert both: the returned `Command`'s `FieldChange`
    // is clean, and a fresh `events_since` replay of that same seq
    // (an independent disk round trip, not the in-memory return value)
    // is clean too.
    use crate::data::command::FieldChange;
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
    let mut d = tests_engine_doc(
        perms,
        "actor",
        serde_json::json!({
            "displayName": "Goblin",
            "visual": { "kind": "image", "asset": "a.png" },
            "size": { "w": 1.0, "h": 1.0 },
            "shape": "square",
            "faction": null,
            "conditions": [],
            "prototype": true
        }),
    );
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
    // OCC pre-image must be the STORED (post-normalization) engine, not
    // the raw submitted body -- the two may already diverge (e.g. key
    // ordering / explicit-null carry-forward) even before this test's
    // own smuggled-key mutation.
    let old_engine = r
        .get_document(doc_id)
        .await
        .unwrap()
        .unwrap()
        .engine
        .unwrap();

    // Wholesale /engine replacement smuggling an unknown key into the
    // `visual` tagged-enum sub-object.
    let smuggled_engine = serde_json::json!({
        "displayName": "Goblin",
        "visual": { "kind": "image", "asset": "b.png", "smuggled": "evil" },
        "size": { "w": 1.0, "h": 1.0 },
        "shape": "square",
        "faction": null,
        "conditions": [],
        "prototype": true
    });
    let cmd = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/engine".into(),
                    old: old_engine,
                    new: smuggled_engine,
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap()
        .command;

    // (i) The returned Command's FieldChange.new is already normalized.
    let broadcast_new = cmd
        .ops
        .iter()
        .find_map(|o| match o {
            Operation::Update {
                doc_id: id,
                changes,
            } if *id == doc_id => changes
                .iter()
                .find(|c| c.path == "/engine")
                .map(|c| c.new.clone()),
            _ => None,
        })
        .expect("update op with /engine change present");
    assert!(
        broadcast_new["visual"].get("smuggled").is_none(),
        "broadcast Command must not carry the smuggled key"
    );

    // (ii) events_since replay (an independent disk round trip through
    // `world_events.command_json`, not the in-memory `cmd` above) is
    // ALSO clean.
    let replayed = r.events_since(w.id, 1).await.unwrap();
    let replayed_cmd = replayed
        .iter()
        .find(|c| c.command.seq == cmd.seq)
        .expect("replayed command present");
    let replayed_new = replayed_cmd
        .command
        .ops
        .iter()
        .find_map(|o| match o {
            Operation::Update {
                doc_id: id,
                changes,
            } if *id == doc_id => changes
                .iter()
                .find(|c| c.path == "/engine")
                .map(|c| c.new.clone()),
            _ => None,
        })
        .expect("replayed update op with /engine change present");
    assert!(
        replayed_new["visual"].get("smuggled").is_none(),
        "events_since replay must not carry the smuggled key"
    );
}

#[tokio::test]
async fn apply_intent_update_normalizes_engine_integer_literal_to_stored_float() {
    // A raw JSON integer literal (`5`, no decimal) submitted for an
    // f64-typed engine field must normalize to the SAME serde_json
    // representation the persisted row round-trips to -- not remain a
    // raw JSON integer Number variant, which would mismatch a
    // client-side float comparison once resync/replay carries it.
    use crate::data::command::FieldChange;
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
    let mut d = tests_engine_doc(
        perms,
        "actor",
        serde_json::json!({
            "displayName": "Goblin",
            "visual": { "kind": "image", "asset": "a.png" },
            "size": { "w": 1.0, "h": 1.0 },
            "shape": "square",
            "faction": null,
            "conditions": [],
            "prototype": true
        }),
    );
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

    // Submit a bare integer literal (`5`, not `5.0`) for /engine/size/w.
    let cmd = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/engine/size/w".into(),
                    old: serde_json::json!(1.0),
                    new: serde_json::json!(5),
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await
        .unwrap()
        .command;

    let broadcast_new = cmd
        .ops
        .iter()
        .find_map(|o| match o {
            Operation::Update {
                doc_id: id,
                changes,
            } if *id == doc_id => changes
                .iter()
                .find(|c| c.path == "/engine/size/w")
                .map(|c| c.new.clone()),
            _ => None,
        })
        .expect("update op with /engine/size/w change present");

    let stored = r.get_document(doc_id).await.unwrap().unwrap();
    let stored_w = stored.engine.unwrap()["size"]["w"].clone();

    // Broadcast value must equal the stored, typed-f64-round-tripped
    // representation -- and its wire form must be the float form, not
    // the raw integer literal that was submitted.
    assert_eq!(broadcast_new, stored_w);
    assert_eq!(
        serde_json::to_string(&broadcast_new).unwrap(),
        "5.0",
        "must be the float serialization, not the raw integer literal"
    );
}

#[tokio::test]
async fn apply_command_update_normalizes_engine_broadcast_and_event_log_smuggled_key() {
    // apply_command mirrors apply_intent's /engine normalization gate
    // (data integrity, not authz) even though it is the trusted
    // undo/replay substrate with no capability/schema/size checks --
    // normalize-then-store must hold for every write path that touches
    // the engine band, or the stored row, the log, and a future replay
    // can diverge.
    use crate::data::command::FieldChange;
    use crate::data::document::{DocRole, PermissionSet, Scope};

    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let mut perms = PermissionSet::default();
    perms.users.insert(gm, DocRole::Owner);
    let mut d = tests_engine_doc(
        perms,
        "actor",
        serde_json::json!({
            "displayName": "Goblin",
            "visual": { "kind": "image", "asset": "a.png" },
            "size": { "w": 1.0, "h": 1.0 },
            "shape": "square",
            "faction": null,
            "conditions": [],
            "prototype": true
        }),
    );
    d.scope = Scope::World { world_id: w.id };
    let doc_id = d.id;
    r.apply_command(UnsequencedCommand {
        world_id: w.id,
        author: gm,
        ts: 1,
        ops: vec![Operation::Create { doc: d }],
    })
    .await
    .unwrap();
    let old_engine = r
        .get_document(doc_id)
        .await
        .unwrap()
        .unwrap()
        .engine
        .unwrap();

    // Wholesale /engine replacement smuggling an unknown key into the
    // `visual` tagged-enum sub-object.
    let smuggled_engine = serde_json::json!({
        "displayName": "Goblin",
        "visual": { "kind": "image", "asset": "b.png", "smuggled": "evil" },
        "size": { "w": 1.0, "h": 1.0 },
        "shape": "square",
        "faction": null,
        "conditions": [],
        "prototype": true
    });
    let cmd = r
        .apply_command(UnsequencedCommand {
            world_id: w.id,
            author: gm,
            ts: 2,
            ops: vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/engine".into(),
                    old: old_engine,
                    new: smuggled_engine,
                }],
            }],
        })
        .await
        .unwrap()
        .command;

    // (a) stored row holds the normalized engine value.
    let stored = r.get_document(doc_id).await.unwrap().unwrap();
    assert!(
        stored.engine.unwrap()["visual"].get("smuggled").is_none(),
        "stored row must not carry the smuggled key"
    );

    // (b) returned Command's FieldChange.new is the normalized value.
    let broadcast_new = cmd
        .ops
        .iter()
        .find_map(|o| match o {
            Operation::Update {
                doc_id: id,
                changes,
            } if *id == doc_id => changes
                .iter()
                .find(|c| c.path == "/engine")
                .map(|c| c.new.clone()),
            _ => None,
        })
        .expect("update op with /engine change present");
    assert!(
        broadcast_new["visual"].get("smuggled").is_none(),
        "returned Command must not carry the smuggled key"
    );
}

#[tokio::test]
async fn apply_command_update_normalizes_engine_integer_literal_to_stored_float() {
    use crate::data::command::FieldChange;
    use crate::data::document::{DocRole, PermissionSet, Scope};

    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let mut perms = PermissionSet::default();
    perms.users.insert(gm, DocRole::Owner);
    let mut d = tests_engine_doc(
        perms,
        "actor",
        serde_json::json!({
            "displayName": "Goblin",
            "visual": { "kind": "image", "asset": "a.png" },
            "size": { "w": 1.0, "h": 1.0 },
            "shape": "square",
            "faction": null,
            "conditions": [],
            "prototype": true
        }),
    );
    d.scope = Scope::World { world_id: w.id };
    let doc_id = d.id;
    r.apply_command(UnsequencedCommand {
        world_id: w.id,
        author: gm,
        ts: 1,
        ops: vec![Operation::Create { doc: d }],
    })
    .await
    .unwrap();

    // Submit a bare integer literal (`5`, not `5.0`) for /engine/size/w.
    let cmd = r
        .apply_command(UnsequencedCommand {
            world_id: w.id,
            author: gm,
            ts: 2,
            ops: vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/engine/size/w".into(),
                    old: serde_json::json!(1.0),
                    new: serde_json::json!(5),
                }],
            }],
        })
        .await
        .unwrap()
        .command;

    let broadcast_new = cmd
        .ops
        .iter()
        .find_map(|o| match o {
            Operation::Update {
                doc_id: id,
                changes,
            } if *id == doc_id => changes
                .iter()
                .find(|c| c.path == "/engine/size/w")
                .map(|c| c.new.clone()),
            _ => None,
        })
        .expect("update op with /engine/size/w change present");

    let stored = r.get_document(doc_id).await.unwrap().unwrap();
    let stored_w = stored.engine.unwrap()["size"]["w"].clone();

    assert_eq!(broadcast_new, stored_w);
    assert_eq!(
        serde_json::to_string(&broadcast_new).unwrap(),
        "5.0",
        "must be the float serialization, not the raw integer literal"
    );
}

#[tokio::test]
async fn apply_command_create_with_invalid_engine_body_is_rejected() {
    use crate::data::document::{DocRole, PermissionSet, Scope};

    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let mut perms = PermissionSet::default();
    perms.users.insert(gm, DocRole::Owner);
    let mut d = tests_engine_doc(
        perms,
        "wall",
        serde_json::json!({ "seg": { "x1": "not-a-number", "y1": 0.0, "x2": 1.0, "y2": 1.0 } }),
    );
    d.scope = Scope::World { world_id: w.id };
    let err = r
        .apply_command(UnsequencedCommand {
            world_id: w.id,
            author: gm,
            ts: 1,
            ops: vec![Operation::Create { doc: d }],
        })
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::BadEngine(_)));
}

#[tokio::test]
async fn apply_command_create_with_envelope_naming_override_is_rejected() {
    use crate::data::document::{DocRole, PermissionSet, Scope, Visibility};

    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let mut perms = PermissionSet::default();
    perms.users.insert(gm, DocRole::Owner);
    perms
        .property_overrides
        .insert("/permissions".into(), Visibility::GmOnly);
    let mut d = tests_doc(perms, serde_json::json!({}));
    d.scope = Scope::World { world_id: w.id };
    let err = r
        .apply_command(UnsequencedCommand {
            world_id: w.id,
            author: gm,
            ts: 1,
            ops: vec![Operation::Create { doc: d }],
        })
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::BadPath(_)));
}

#[tokio::test]
async fn apply_command_update_with_envelope_naming_override_is_rejected() {
    use crate::data::command::FieldChange;
    use crate::data::document::{DocRole, PermissionSet, Scope};

    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let mut perms = PermissionSet::default();
    perms.users.insert(gm, DocRole::Owner);
    let mut d = tests_doc(perms, serde_json::json!({}));
    d.scope = Scope::World { world_id: w.id };
    let doc_id = d.id;
    r.apply_command(UnsequencedCommand {
        world_id: w.id,
        author: gm,
        ts: 1,
        ops: vec![Operation::Create { doc: d }],
    })
    .await
    .unwrap();

    let err = r
        .apply_command(UnsequencedCommand {
            world_id: w.id,
            author: gm,
            ts: 2,
            ops: vec![Operation::Update {
                doc_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/permissions/property_overrides".into(),
                    old: serde_json::json!({}),
                    new: serde_json::json!({ "/permissions": "gm_only" }),
                }],
            }],
        })
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::BadPath(_)));
}

#[tokio::test]
async fn declarative_requirement_blocks_writer_without_extra_cap() {
    use crate::auth::role::ServerRole;
    use crate::data::command::{FieldChange, Operation};
    use crate::data::document::{CapabilityRequirement, DocRole, PermissionSet, Scope};
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

    // A doc the player owns (owner floor: read + write_fields).
    let mut perms = PermissionSet::default();
    perms.users.insert(player, DocRole::Owner);
    let mut d = tests_doc(
        perms,
        serde_json::json!({ "vision": { "range": 30 }, "hp": 10 }),
    );
    d.scope = Scope::World { world_id: w.id };
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Create { doc: d.clone() }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Require dnd5e:gm_vision to write /system/vision.
    r.set_world_cap_requirements(
        w.id,
        &[CapabilityRequirement {
            path_prefix: "/system/vision".into(),
            caps: ["dnd5e:gm_vision".to_string()].into_iter().collect(),
        }],
    )
    .await
    .unwrap();

    let player_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };

    // Owner CAN write a non-restricted /system field (base cap only).
    r.apply_intent(
        &player_ctx,
        w.id,
        vec![Operation::Update {
            doc_id: d.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/system/hp".into(),
                old: serde_json::json!(10),
                new: serde_json::json!(8),
            }],
        }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Owner CANNOT write /system/vision (lacks dnd5e:gm_vision).
    let err = r
        .apply_intent(
            &player_ctx,
            w.id,
            vec![Operation::Update {
                doc_id: d.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/system/vision/range".into(),
                    old: serde_json::json!(30),
                    new: serde_json::json!(60),
                }],
            }],
            3,
            WriteOrigin::Client,
        )
        .await;
    assert!(matches!(err, Err(DataError::Forbidden)));

    // Owner CANNOT evade the requirement via a coarse ANCESTOR write to
    // /system (which would replace the protected /system/vision subtree).
    let err = r
        .apply_intent(
            &player_ctx,
            w.id,
            vec![Operation::Update {
                doc_id: d.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/system".into(),
                    old: serde_json::json!({ "vision": { "range": 30 }, "hp": 8 }),
                    new: serde_json::json!({ "vision": { "range": 99 }, "hp": 8 }),
                }],
            }],
            3,
            WriteOrigin::Client,
        )
        .await;
    assert!(matches!(err, Err(DataError::Forbidden)));

    // GM is unaffected (holds everything).
    r.apply_intent(
        &gm_ctx,
        w.id,
        vec![Operation::Update {
            doc_id: d.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/system/vision/range".into(),
                old: serde_json::json!(30),
                new: serde_json::json!(60),
            }],
        }],
        4,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn declarative_requirement_blocks_create_with_protected_subtree() {
    use crate::auth::role::ServerRole;
    use crate::data::command::Operation;
    use crate::data::document::{CapabilityRequirement, DocRole, PermissionSet, Scope};
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

    // Require dnd5e:gm_vision to touch /system/vision.
    r.set_world_cap_requirements(
        w.id,
        &[CapabilityRequirement {
            path_prefix: "/system/vision".into(),
            caps: ["dnd5e:gm_vision".to_string()].into_iter().collect(),
        }],
    )
    .await
    .unwrap();

    // Grant Players create so this test exercises the declarative requirement,
    // not the world-level create floor (which is GM-only by default).
    let mut create_defaults = WorldCapDefaults::default();
    create_defaults
        .role_caps
        .all
        .entry(WorldRole::Player)
        .or_default()
        .insert("core:create".into());
    r.set_world_cap_defaults(w.id, &create_defaults)
        .await
        .unwrap();

    let player_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };

    // A doc the player will own, carrying a populated /system/vision subtree.
    let mut perms = PermissionSet::default();
    perms.users.insert(player, DocRole::Owner);
    let mut protected = tests_doc(
        perms.clone(),
        serde_json::json!({ "vision": { "range": 120 }, "hp": 10 }),
    );
    protected.scope = Scope::World { world_id: w.id };
    protected.owner = Some(player);

    // CANNOT create it (would seed protected vision without the cap).
    let err = r
        .apply_intent(
            &player_ctx,
            w.id,
            vec![Operation::Create {
                doc: protected.clone(),
            }],
            1,
            WriteOrigin::Client,
        )
        .await;
    assert!(matches!(err, Err(DataError::Forbidden)));

    // CAN create a doc that does not populate the protected path.
    let mut plain = tests_doc(perms, serde_json::json!({ "hp": 10 }));
    plain.scope = Scope::World { world_id: w.id };
    plain.owner = Some(player);
    r.apply_intent(
        &player_ctx,
        w.id,
        vec![Operation::Create { doc: plain }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
}
