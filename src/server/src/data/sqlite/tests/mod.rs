use super::*;
use crate::data::command::FieldChange;
use crate::data::document::Source;

async fn repo() -> SqliteRepository {
    SqliteRepository::connect("sqlite::memory:").await.unwrap()
}

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
    let mut token_doc =
        crate::data::document::tests::world_scoped_doc(world.id, token, "token");
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
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM world_invites WHERE created_by IS NULL"
        )
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

/// A world-scoped actor document with the given permissions and system body.
/// Callers overwrite `scope` with the real world id.
fn tests_doc(
    perms: crate::data::document::PermissionSet,
    system: serde_json::Value,
) -> Document {
    Document {
        id: Uuid::new_v4(),
        scope: Scope::World {
            world_id: Uuid::from_u128(9),
        },
        doc_type: "actor".into(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: None,
        permissions: perms,
        embedded: Default::default(),
        parent_id: None,
        // "actor" is engine-defined; a minimal valid body so `Create`
        // clears the ingress gate. Unrelated to `system` (opaque,
        // caller-supplied) — this helper predates the engine band.
        engine: crate::data::document::tests::default_test_engine("actor"),
        system,
        created_at: 0,
        updated_at: 0,
    }
}

/// A world-scoped document of `doc_type` carrying an `engine` body
/// (no `system` content — `engine`-typed docs in this battery don't
/// need one). Callers overwrite `scope` with the real world id.
fn tests_engine_doc(
    perms: crate::data::document::PermissionSet,
    doc_type: &str,
    engine: serde_json::Value,
) -> Document {
    let mut d = tests_doc(perms, serde_json::json!({}));
    d.doc_type = doc_type.into();
    d.engine = Some(engine);
    d
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
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE server_role = 'admin'")
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

fn world_doc(id: u128, world: Uuid, system: serde_json::Value) -> Document {
    Document {
        id: Uuid::from_u128(id),
        scope: Scope::World { world_id: world },
        doc_type: "actor".into(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: None,
        permissions: Default::default(),
        embedded: Default::default(),
        parent_id: None,
        // "actor" is engine-defined; a minimal valid body so `Create`
        // clears the ingress gate. Callers that override `doc_type`
        // afterward must also recompute `engine` for the new type.
        engine: crate::data::document::tests::default_test_engine("actor"),
        system,
        created_at: 0,
        updated_at: 0,
    }
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
    let doc_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM documents WHERE world_id = ?")
            .bind(w.id.to_string())
            .fetch_one(target.pool())
            .await
            .unwrap();
    assert_eq!(doc_count, 0);
}

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
async fn create_op_snapshot_in_a_same_command_create_then_update_reflects_the_post_update_state(
) {
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
async fn reused_id_gets_a_fresh_created_seq_and_the_stale_ops_own_snapshot_witnesses_the_old_one(
) {
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

fn update(
    doc_id: Uuid,
    path: &str,
    old: serde_json::Value,
    new: serde_json::Value,
) -> Operation {
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

/// A world-scoped document of `doc_type` with a valid `engine` body for
/// singleton create-gate tests. Mirrors `world_doc`/`tests_engine_doc`
/// but lets the caller pick `doc_type` (needed for the singleton types,
/// which `world_doc` hardcodes to "actor").
fn singleton_test_doc(id: u128, world: Uuid, doc_type: &str) -> Document {
    let mut d = world_doc(id, world, serde_json::json!({}));
    d.doc_type = doc_type.into();
    d.engine = crate::data::document::tests::default_test_engine(doc_type);
    d
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

// --- World invites ---

/// A world with a GM and two redeemers, plus one live invite.
async fn invite_fixture(role: WorldRole) -> (SqliteRepository, Uuid, Uuid, Uuid, Uuid) {
    use crate::auth::role::ServerRole;
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let a = r.create_user("a", None, ServerRole::User, 0).await.unwrap();
    let b = r.create_user("b", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let id = Uuid::new_v4();
    assert!(r
        .create_invite(
            NewInvite {
                id,
                world: w.id,
                secret_hash: "phc",
                role,
                created_by: gm,
                now: 10,
                expires_at: 1_000_000,
            },
            64
        )
        .await
        .unwrap());
    (r, w.id, id, a, b)
}

#[tokio::test]
async fn consume_invite_seats_exactly_one_redeemer() {
    let (r, world, invite, a, b) = invite_fixture(WorldRole::Player).await;

    let first = r.consume_invite(invite, a, 20).await.unwrap().unwrap();
    assert_eq!(
        first,
        SeatedByInvite {
            world,
            world_name: "W".into(),
            role: WorldRole::Player,
        }
    );
    // The guarded UPDATE is the whole gate: a second redemption of the same
    // row cannot observe it as available, so b is never seated.
    assert_eq!(r.consume_invite(invite, b, 21).await.unwrap(), None);
    assert_eq!(
        r.member_role(world, a).await.unwrap(),
        Some(WorldRole::Player)
    );
    assert_eq!(r.member_role(world, b).await.unwrap(), None);
}

#[tokio::test]
async fn consume_invite_refuses_expired_and_revoked_rows() {
    let (r, world, invite, a, _) = invite_fixture(WorldRole::Player).await;
    // `now` past the row's expiry.
    assert_eq!(r.consume_invite(invite, a, 2_000_000).await.unwrap(), None);
    assert_eq!(r.member_role(world, a).await.unwrap(), None);
    // The row was still live at a valid `now` — expiry is the only reason
    // it failed above. Revoked, it fails for a second, distinct reason.
    assert!(r.revoke_invite(world, invite, 30).await.unwrap());
    assert_eq!(r.consume_invite(invite, a, 40).await.unwrap(), None);
    assert_eq!(r.member_role(world, a).await.unwrap(), None);
    // Revoking a revoked row is not a second success.
    assert!(!r.revoke_invite(world, invite, 50).await.unwrap());
}

#[tokio::test]
async fn revoke_invite_is_scoped_to_its_world() {
    use crate::auth::role::ServerRole;
    let (r, world, invite, a, _) = invite_fixture(WorldRole::Player).await;
    let other_gm = r
        .create_user("other", None, ServerRole::User, 0)
        .await
        .unwrap();
    let other = r.create_world_owned("Other", other_gm, 0).await.unwrap();

    // Another world's id does not unlock this invite.
    assert!(!r.revoke_invite(other.id, invite, 30).await.unwrap());
    let seated = r.consume_invite(invite, a, 40).await.unwrap().unwrap();
    assert_eq!((seated.world, seated.role), (world, WorldRole::Player));
}

#[tokio::test]
async fn consume_invite_never_changes_a_role_already_held() {
    let (r, world, invite, _, _) = invite_fixture(WorldRole::Spectator).await;
    let gm = r.list_members(world).await.unwrap()[0].0;
    assert_eq!(r.member_role(world, gm).await.unwrap(), Some(WorldRole::Gm));

    let seated = r.consume_invite(invite, gm, 20).await.unwrap().unwrap();
    assert_eq!(
        (seated.world, seated.role),
        (world, WorldRole::Gm),
        "the returned role is the membership actually held"
    );
    assert_eq!(r.member_role(world, gm).await.unwrap(), Some(WorldRole::Gm));
}

#[tokio::test]
async fn list_invites_never_returns_the_stored_hash() {
    let (r, world, invite, _, _) = invite_fixture(WorldRole::Player).await;
    let listed = r.list_invites(world).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, invite);
    assert_eq!(listed[0].secret_hash, "");
    // The by-id lookup, redemption's only reader, still sees it.
    assert_eq!(
        r.invite_by_id(invite).await.unwrap().unwrap().secret_hash,
        "phc"
    );
}

#[tokio::test]
async fn create_invite_caps_live_invites_and_a_spent_one_frees_a_slot() {
    let (r, world, first, a, _) = invite_fixture(WorldRole::Player).await;
    // Cap of 1: the world already holds one live invite.
    assert!(!r
        .create_invite(
            NewInvite {
                id: Uuid::new_v4(),
                world,
                secret_hash: "phc",
                role: WorldRole::Player,
                created_by: a,
                now: 10,
                expires_at: 1_000_000,
            },
            1
        )
        .await
        .unwrap());
    r.consume_invite(first, a, 20).await.unwrap().unwrap();
    assert!(r
        .create_invite(
            NewInvite {
                id: Uuid::new_v4(),
                world,
                secret_hash: "phc",
                role: WorldRole::Player,
                created_by: a,
                now: 20,
                expires_at: 1_000_000,
            },
            1
        )
        .await
        .unwrap());
}

// ---- Token ownership: actor-inherited with a per-token override ----
//
// effective_owner(token) = the token's own `owner` override, else the LINKED
// actor's owner, resolved SERVER-SIDE at authz time against live actor state.
// Every reject leg below is paired with an accept leg that differs ONLY in the
// resolution input (which user, which override, which actor owner), so a rule
// inverted or defaulted-open flips the pair rather than passing both.

/// A world-scoped `token` doc, optionally linked to `actor_id`. `permissions`
/// deliberately stays at the `buildTokenDoc` shipping default (`default:
/// Observer`, no per-user entry) — the whole point is that write authority
/// comes from effective ownership, not from a stamped permission entry.
fn owned_token_doc(world: Uuid, actor_id: Option<Uuid>) -> Document {
    use crate::data::document::{DocRole, PermissionSet, Scope};
    let mut engine = serde_json::json!({
        "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0
    });
    if let Some(a) = actor_id {
        engine["actor_id"] = serde_json::json!(a.to_string());
    }
    let mut d = tests_engine_doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        "token",
        engine,
    );
    d.scope = Scope::World { world_id: world };
    d
}

/// A world-scoped `actor` doc owned by `owner`.
fn actor_doc_owned_by(world: Uuid, owner: Option<Uuid>) -> Document {
    use crate::data::document::{DocRole, PermissionSet, Scope};
    let mut d = tests_doc(
        PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        serde_json::json!({}),
    );
    d.scope = Scope::World { world_id: world };
    d.owner = owner;
    d
}

/// Attempt `/engine/x` + `/engine/y` as `user` (a Player) on `token`.
async fn try_move(
    r: &SqliteRepository,
    world: Uuid,
    user: Uuid,
    token: Uuid,
    from: (f64, f64),
    to: (f64, f64),
    ts: i64,
) -> Result<Command, DataError> {
    use crate::data::command::FieldChange;
    use crate::data::membership::PermissionContext;
    r.apply_intent(
        &PermissionContext {
            user_id: user,
            world_role: WorldRole::Player,
        },
        world,
        vec![Operation::Update {
            doc_id: token,
            changes: vec![
                FieldChange {
                    remove: false,
                    path: "/engine/x".into(),
                    old: serde_json::json!(from.0),
                    new: serde_json::json!(to.0),
                },
                FieldChange {
                    remove: false,
                    path: "/engine/y".into(),
                    old: serde_json::json!(from.1),
                    new: serde_json::json!(to.1),
                },
            ],
        }],
        ts,
        WriteOrigin::Client,
    )
    .await
    .map(|stored| stored.command)
}

/// GM, world, and two ordinary player accounts (`owner_id` is a FK, so every
/// owner must be a real user row).
async fn ownership_fixture() -> (SqliteRepository, Uuid, Uuid, Uuid, Uuid) {
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let p1 = r
        .create_user("player-one", None, ServerRole::User, 0)
        .await
        .unwrap();
    let p2 = r
        .create_user("player-two", None, ServerRole::User, 0)
        .await
        .unwrap();
    (r, gm, w.id, p1, p2)
}

async fn gm_create(r: &SqliteRepository, gm: Uuid, world: Uuid, docs: Vec<Document>, ts: i64) {
    use crate::data::membership::PermissionContext;
    r.apply_intent(
        &PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        },
        world,
        docs.into_iter()
            .map(|doc| Operation::Create { doc })
            .collect(),
        ts,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn linked_token_inherits_actor_owner_for_writes() {
    let (r, gm, w, p1, p2) = ownership_fixture().await;
    let actor = actor_doc_owned_by(w, Some(p1));
    let token = owned_token_doc(w, Some(actor.id));
    gm_create(&r, gm, w, vec![actor, token.clone()], 1).await;

    // The actor's owner may move the linked token — with NO per-token `owner`
    // and NO per-token permissions entry: authority is inherited, live.
    try_move(&r, w, p1, token.id, (0.0, 0.0), (5.0, 7.0), 2)
        .await
        .expect("the linked actor's owner may move the token");

    // Non-vacuity: the SAME token, the SAME path, the SAME pre-image — only the
    // user differs. A rule defaulted open (or inverted) would let this pass too.
    let denied = try_move(&r, w, p2, token.id, (5.0, 7.0), (9.0, 9.0), 3).await;
    assert!(
        matches!(denied, Err(DataError::Forbidden)),
        "a player who owns neither the token nor its actor must not move it, got {denied:?}"
    );
}

#[tokio::test]
async fn per_token_owner_override_beats_the_linked_actors_owner() {
    let (r, gm, w, p1, p2) = ownership_fixture().await;
    let actor = actor_doc_owned_by(w, Some(p1));
    let mut token = owned_token_doc(w, Some(actor.id));
    token.owner = Some(p2); // GM override on the individual token
    gm_create(&r, gm, w, vec![actor, token.clone()], 1).await;

    // The override holder writes...
    try_move(&r, w, p2, token.id, (0.0, 0.0), (2.0, 3.0), 2)
        .await
        .expect("the per-token owner override may move the token");

    // ...and the actor's owner, who WOULD inherit but for the override, cannot.
    // Paired with the accept leg above, this pins precedence in both directions:
    // inverting it (actor owner beats override) flips both assertions.
    let denied = try_move(&r, w, p1, token.id, (2.0, 3.0), (4.0, 4.0), 3).await;
    assert!(
        matches!(denied, Err(DataError::Forbidden)),
        "the token's own override must supersede the actor's owner, got {denied:?}"
    );
}

#[tokio::test]
async fn reassigning_the_actors_owner_moves_token_authority_with_no_restamp() {
    use crate::data::command::FieldChange;
    use crate::data::membership::PermissionContext;
    let (r, gm, w, p1, p2) = ownership_fixture().await;
    let actor = actor_doc_owned_by(w, Some(p1));
    let token = owned_token_doc(w, Some(actor.id));
    gm_create(&r, gm, w, vec![actor.clone(), token.clone()], 1).await;

    try_move(&r, w, p1, token.id, (0.0, 0.0), (1.0, 1.0), 2)
        .await
        .expect("the original actor owner may move the token");

    // The GM re-assigns the ACTOR's owner. The token document is never touched.
    r.apply_intent(
        &PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        },
        w,
        vec![Operation::Update {
            doc_id: actor.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/owner".into(),
                old: serde_json::json!(p1.to_string()),
                new: serde_json::json!(p2.to_string()),
            }],
        }],
        3,
        WriteOrigin::Client,
    )
    .await
    .expect("a GM may re-assign an actor's owner");

    // Authority followed the actor, with no write to the token: the token's own
    // `owner` is STILL unset — proving resolution, not a stamped copy.
    let stored = r.get_document(token.id).await.unwrap().unwrap();
    assert_eq!(
        stored.owner, None,
        "the token must carry no stamped owner — ownership is resolved, not copied"
    );

    try_move(&r, w, p2, token.id, (1.0, 1.0), (6.0, 6.0), 4)
        .await
        .expect("the actor's NEW owner may move the token");
    let denied = try_move(&r, w, p1, token.id, (6.0, 6.0), (8.0, 8.0), 5).await;
    assert!(
        matches!(denied, Err(DataError::Forbidden)),
        "the actor's ORIGINAL owner must lose the token, got {denied:?}"
    );
}

#[tokio::test]
async fn ownership_fails_closed_on_every_degenerate_link() {
    let (r, gm, w, p1, _p2) = ownership_fixture().await;

    // (a) No link at all, no override: nobody inherits.
    let unlinked = owned_token_doc(w, None);
    // (b) Dangling link: `actor_id` names a document that does not exist.
    let dangling = owned_token_doc(w, Some(Uuid::new_v4()));
    // (c) Linked to an actor with NO owner.
    let unowned_actor = actor_doc_owned_by(w, None);
    let linked_unowned = owned_token_doc(w, Some(unowned_actor.id));
    // (d) Control: identical shape, but the actor IS owned by p1.
    let owned_actor = actor_doc_owned_by(w, Some(p1));
    let linked_owned = owned_token_doc(w, Some(owned_actor.id));
    gm_create(
        &r,
        gm,
        w,
        vec![
            unlinked.clone(),
            dangling.clone(),
            unowned_actor,
            linked_unowned.clone(),
            owned_actor,
            linked_owned.clone(),
        ],
        1,
    )
    .await;

    for (label, id) in [
        ("no link", unlinked.id),
        ("dangling link", dangling.id),
        ("actor with no owner", linked_unowned.id),
    ] {
        let denied = try_move(&r, w, p1, id, (0.0, 0.0), (3.0, 3.0), 2).await;
        assert!(
            matches!(denied, Err(DataError::Forbidden)),
            "{label} must fail closed (no owner => no write), got {denied:?}"
        );
    }

    // Non-vacuity for the whole loop: the same player, the same move, on a token
    // whose only difference is a RESOLVABLE owned actor — this one succeeds, so
    // the three rejections above are the ownership rule, not a blanket denial.
    try_move(&r, w, p1, linked_owned.id, (0.0, 0.0), (3.0, 3.0), 3)
        .await
        .expect("the control leg (resolvable owned actor) must succeed");
}

#[tokio::test]
async fn an_effective_owner_cannot_reassign_or_widen_ownership() {
    use crate::data::command::FieldChange;
    use crate::data::membership::PermissionContext;
    let (r, gm, w, p1, p2) = ownership_fixture().await;
    let actor = actor_doc_owned_by(w, Some(p1));
    let token = owned_token_doc(w, Some(actor.id));
    gm_create(&r, gm, w, vec![actor, token.clone()], 1).await;

    let as_p1 = PermissionContext {
        user_id: p1,
        world_role: WorldRole::Player,
    };
    // The effective owner holds the `DocRole::Owner` floor (READ + WRITE_FIELDS)
    // and nothing more: `/owner` and `/permissions` need EDIT_PERMISSIONS, which
    // that floor does not include. Without this, an inheriting owner could pin
    // the token to themselves or hand it to anyone.
    for change in [
        FieldChange {
            remove: false,
            path: "/owner".into(),
            old: serde_json::Value::Null,
            new: serde_json::json!(p2.to_string()),
        },
        FieldChange {
            remove: false,
            path: "/permissions/default".into(),
            old: serde_json::json!("observer"),
            new: serde_json::json!("owner"),
        },
    ] {
        let path = change.path.clone();
        let denied = r
            .apply_intent(
                &as_p1,
                w,
                vec![Operation::Update {
                    doc_id: token.id,
                    changes: vec![change],
                }],
                2,
                WriteOrigin::Client,
            )
            .await;
        assert!(
            matches!(denied, Err(DataError::Forbidden)),
            "an effective owner must not write {path}, got {denied:?}"
        );
    }

    // Non-vacuity: the same user, same doc, same call shape — a WRITE_FIELDS path
    // succeeds, so the two rejections are the capability split, not a dead player.
    try_move(&r, w, p1, token.id, (0.0, 0.0), (1.0, 2.0), 3)
        .await
        .expect("the effective owner still holds WRITE_FIELDS");
}

#[tokio::test]
async fn effective_owner_of_joins_the_linked_actor_on_the_pool() {
    let (r, gm, w, p1, _p2) = ownership_fixture().await;
    let actor = actor_doc_owned_by(w, Some(p1));
    let actor_id = actor.id;
    let token = owned_token_doc(w, Some(actor_id));
    let token_id = token.id;
    gm_create(&r, gm, w, vec![actor, token], 1).await;

    let token = r.get_document(token_id).await.unwrap().unwrap();
    assert_eq!(r.effective_owner_of(&token).await.unwrap(), Some(p1));

    // Dangling link fails closed.
    let mut dangling = token.clone();
    dangling.engine = Some(serde_json::json!({
        "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0,
        "actor_id": Uuid::from_u128(999999).to_string()
    }));
    assert_eq!(r.effective_owner_of(&dangling).await.unwrap(), None);

    // A non-token resolves to its literal owner without any join.
    let actor = r.get_document(actor_id).await.unwrap().unwrap();
    assert_eq!(r.effective_owner_of(&actor).await.unwrap(), Some(p1));
}

#[tokio::test]
async fn the_owner_capability_floor_is_scoped_to_tokens() {
    use crate::data::command::FieldChange;
    use crate::data::membership::PermissionContext;
    let (r, gm, w, p1, _p2) = ownership_fixture().await;
    // An `actor` the player owns. `owner` carries a provenance-only
    // meaning on every non-`token` doc_type: it admits the
    // OwnerOrGm redaction tier but grants NO capability, so the player cannot
    // write the actor's body. Widening this is a separate design decision.
    let mut actor = actor_doc_owned_by(w, Some(p1));
    actor.system = serde_json::json!({ "hp": 10 });
    gm_create(&r, gm, w, vec![actor.clone()], 1).await;

    let denied = r
        .apply_intent(
            &PermissionContext {
                user_id: p1,
                world_role: WorldRole::Player,
            },
            w,
            vec![Operation::Update {
                doc_id: actor.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/system/hp".into(),
                    old: serde_json::json!(10),
                    new: serde_json::json!(1),
                }],
            }],
            2,
            WriteOrigin::Client,
        )
        .await;
    assert!(
        matches!(denied, Err(DataError::Forbidden)),
        "the owner floor must not leak to non-token doc_types, got {denied:?}"
    );
}

#[tokio::test]
async fn a_removal_carrying_a_new_value_is_rejected_at_ingress() {
    // `remove: true` deletes the key; `new` is unused. The pairing has no
    // legitimate meaning, and `new` is checked by neither the OCC comparison
    // (which reads `old`) nor `required_cap_for_path` — so any consumer that
    // mirrors a change by unconditionally setting `new` lands an attacker-chosen
    // value where the store lands absence. Denied at ingress.
    use crate::data::command::FieldChange;
    let (r, gm, w, p1, p2) = ownership_fixture().await;
    let actor = actor_doc_owned_by(w, Some(p1));
    let token = owned_token_doc(w, Some(actor.id));
    gm_create(&r, gm, w, vec![actor, token.clone()], 1).await;

    let attempt = |remove: bool, new: serde_json::Value, ts: i64| {
        let r = &r;
        let path = "/engine/actor_id".to_string();
        async move {
            r.apply_intent(
                &crate::data::membership::PermissionContext {
                    user_id: p1,
                    world_role: WorldRole::Player,
                },
                w,
                vec![Operation::Update {
                    doc_id: token.id,
                    changes: vec![FieldChange {
                        remove,
                        path,
                        old: serde_json::json!(null),
                        new,
                    }],
                }],
                ts,
                WriteOrigin::Client,
            )
            .await
        }
    };

    // Rejected: a removal carrying a value.
    let denied = attempt(true, serde_json::json!(p2.to_string()), 2).await;
    assert!(
        matches!(denied, Err(DataError::OpFailed(_))),
        "a removal must not carry a `new` value, got {denied:?}"
    );

    // Non-vacuity: the SAME change with `new: null` clears ingress and is judged
    // on its merits (it fails the OCC pre-image check, not the shape gate) — so
    // the rejection above is the shape rule, not a blanket denial of removals.
    let occ = attempt(true, serde_json::Value::Null, 3).await;
    assert!(
        matches!(occ, Err(DataError::Conflict(_))),
        "a well-shaped removal reaches the OCC check, got {occ:?}"
    );
}

#[tokio::test]
async fn the_actor_join_does_not_cross_world_scope() {
    // `load_document` is keyed on id alone. An `actor_id` naming an actor in
    // ANOTHER world must not resolve an owner: it breaks world isolation, and
    // room hydration loads actors `WHERE world_id = ?`, so the derived vision
    // path structurally cannot see such an actor — resolving one here would be a
    // second ECS/DB ownership fork.
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let p1 = r
        .create_user("player-one", None, ServerRole::User, 0)
        .await
        .unwrap();
    let token_world = r.create_world_owned("token-world", gm, 0).await.unwrap();
    let actor_world = r.create_world_owned("actor-world", gm, 0).await.unwrap();

    // The actor is owned by p1 but lives in a different world from the token it is linked to.
    let foreign_actor = actor_doc_owned_by(actor_world.id, Some(p1));
    gm_create(&r, gm, actor_world.id, vec![foreign_actor.clone()], 1).await;
    let token = owned_token_doc(token_world.id, Some(foreign_actor.id));
    gm_create(&r, gm, token_world.id, vec![token.clone()], 2).await;

    // p1 is a member of the token's world too, so only the scope check can deny this.
    r.add_member(token_world.id, p1, WorldRole::Player)
        .await
        .unwrap();
    let denied = try_move(&r, token_world.id, p1, token.id, (0.0, 0.0), (3.0, 3.0), 3).await;
    assert!(
        matches!(denied, Err(DataError::Forbidden)),
        "a cross-world actor link must not confer ownership, got {denied:?}"
    );

    // Non-vacuity: the identical setup with the actor in the TOKEN's own world
    // succeeds — proving the denial is the scope check, not the membership or
    // the link machinery.
    let local_actor = actor_doc_owned_by(token_world.id, Some(p1));
    let local_token = owned_token_doc(token_world.id, Some(local_actor.id));
    gm_create(
        &r,
        gm,
        token_world.id,
        vec![local_actor, local_token.clone()],
        4,
    )
    .await;
    try_move(
        &r,
        token_world.id,
        p1,
        local_token.id,
        (0.0, 0.0),
        (3.0, 3.0),
        5,
    )
    .await
    .expect("a same-world actor link confers ownership");
}

#[tokio::test]
async fn created_seq_is_set_once_and_survives_updates() {
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
    let mut d = tests_doc(perms, serde_json::json!({ "hp": 1 }));
    d.scope = Scope::World { world_id: w.id };
    let doc_id = d.id;

    let stored = r
        .apply_intent(
            &ctx,
            w.id,
            vec![Operation::Create { doc: d }],
            1,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    let first_seq = stored.command.seq;

    let mut tx = r.pool.begin().await.unwrap();
    let created_after_create = SqliteRepository::document_created_seq(&mut *tx, doc_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(created_after_create, Some(first_seq));

    r.apply_intent(
        &ctx,
        w.id,
        vec![Operation::Update {
            doc_id,
            changes: vec![FieldChange {
                remove: false,
                path: "/system/hp".into(),
                old: serde_json::json!(1),
                new: serde_json::json!(2),
            }],
        }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let mut tx = r.pool.begin().await.unwrap();
    let created_after_update = SqliteRepository::document_created_seq(&mut *tx, doc_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        created_after_update, created_after_create,
        "created_seq must not change across an update to a still-live row"
    );
}

#[tokio::test]
async fn created_seq_is_absent_for_a_missing_document() {
    let r = repo().await;
    let mut tx = r.pool.begin().await.unwrap();
    let missing = SqliteRepository::document_created_seq(&mut *tx, Uuid::new_v4())
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(missing, None);
}

#[tokio::test]
async fn world_member_roles_reflects_every_current_member() {
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

    let mut tx = r.pool.begin().await.unwrap();
    let roles = SqliteRepository::world_member_roles(&mut *tx, w.id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(roles.get(&gm), Some(&WorldRole::Gm));
    assert_eq!(roles.get(&player), Some(&WorldRole::Player));
}

#[tokio::test]
async fn get_document_with_created_seq_matches_a_separate_created_seq_read() {
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
    let mut d = tests_doc(perms, serde_json::json!({ "hp": 1 }));
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

    let (doc, created_seq) = r
        .get_document_with_created_seq(doc_id)
        .await
        .unwrap()
        .expect("document must exist");
    assert_eq!(doc.id, doc_id);
    let mut tx = r.pool.begin().await.unwrap();
    let separate = SqliteRepository::document_created_seq(&mut *tx, doc_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(Some(created_seq), separate);
}
