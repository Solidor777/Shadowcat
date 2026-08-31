// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use async_trait::async_trait;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::auth::role::ServerRole;
use crate::data::command::{
    apply_field_change, Command, FieldChange, Operation, UnsequencedCommand, WriteOrigin,
};
use crate::data::document::{
    CapabilityRequirement, ContractDeclaration, Document, SchemaDeclaration, Scope, World,
    WorldCapDefaults, WorldRole,
};
use crate::data::engine::{
    CombatEngine, COMBATANT_DOC_TYPE, COMBAT_DOC_TYPE, COMBAT_HISTORY_DOC_TYPE,
    CONDITION_REGISTRY_DOC_TYPE, FACTION_REGISTRY_DOC_TYPE, RESOURCE_REGISTRY_DOC_TYPE,
    SYSTEM_DEFAULTS_DOC_TYPE, WORLD_SETTINGS_DOC_TYPE,
};
use crate::data::permission::{
    cap, declared_caps_for_document, declared_caps_for_path, required_cap_for_path,
    resolve_access_world, Access,
};
use crate::data::repository::Repository;
use crate::data::snapshot::{CommandSnapshot, StoredCommand};
use crate::data::validation;
use crate::data::world_bundle::{
    BundleManifest, ExportedAssetRow, ExportedDocumentRow, ExportedEventRow, ExportedFogRow,
    ExportedInviteRow, ExportedMemberRow, ExportedSettingRow, ImportSummary, WorldExportData,
    WorldImportData, BUNDLE_SCHEMA_VERSION,
};
use crate::data::DataError;

/// Doc_types capped at one document per world. Checked (transactionally,
/// alongside the existing-id conflict check) at the `apply_intent` Create
/// chokepoint — a stray second singleton doc would otherwise resolve
/// nondeterministically-but-safely via lowest-UUID ordering (see
/// `chat::settings::resolve_content_policy`'s doc comment); this closes that
/// gap at construction time rather than leaving it to read-side tolerance.
const SINGLETON_DOC_TYPES: &[&str] = &[
    WORLD_SETTINGS_DOC_TYPE,
    FACTION_REGISTRY_DOC_TYPE,
    CONDITION_REGISTRY_DOC_TYPE,
    RESOURCE_REGISTRY_DOC_TYPE,
    SYSTEM_DEFAULTS_DOC_TYPE,
    crate::chat::CHAT_SETTINGS_DOC_TYPE,
    crate::chat::DICE_SETTINGS_DOC_TYPE,
];

/// One-level merge of a single key into `map`: when both the existing
/// `map[key]` and the incoming `value` are JSON objects, merges `value`'s
/// entries into the existing object (each of THOSE entries replaces
/// wholesale — this never recurses past one level, so an opaque leaf blob
/// like `panelLayout` is never deep-merged); otherwise `value` replaces
/// `map[key]` wholesale. `null` REMOVES rather than replaces: a `null`
/// `value` removes `key` from `map` entirely (a conceptual counterpart to
/// `FieldChange.remove` elsewhere in the data layer — `ui_state` patches are
/// plain JSON, not typed `FieldChange`s, so there is no shared wire shape),
/// and inside the object-merge branch a `null` entry of `value` removes that
/// leaf key from the existing object instead of storing a literal `null`.
/// The shared leaf-key merge step behind `SqliteRepository::merge_ui_state`'s
/// per-top-level-key and per-`worlds.<id>` merge rule.
fn merge_one_level(
    map: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: &serde_json::Value,
) {
    if value.is_null() {
        map.remove(key);
        return;
    }
    let existing_is_object = map.get(key).is_some_and(serde_json::Value::is_object);
    if existing_is_object && value.is_object() {
        // Safe: `existing_is_object` just confirmed `map[key]` is present and an object.
        let existing_obj = map
            .get_mut(key)
            .and_then(serde_json::Value::as_object_mut)
            .expect("existing_is_object confirmed map[key] is a present object");
        for (k, v) in value.as_object().expect("value.is_object() checked above") {
            if v.is_null() {
                existing_obj.remove(k);
            } else {
                existing_obj.insert(k.clone(), v.clone());
            }
        }
    } else {
        map.insert(key.to_string(), value.clone());
    }
}

/// Auth-facing projection of a user row.
#[derive(Debug, Clone)]
pub struct UserRecord {
    /// Account id.
    pub id: Uuid,
    /// Unique login name.
    pub username: String,
    /// Argon2 PHC string; `None` = login disabled (e.g. seeded fixture accounts).
    pub password_hash: Option<String>,
    /// Server tier (admin/user), orthogonal to any per-world role.
    pub server_role: ServerRole,
}

/// A world invite as stored. `secret_hash` is an Argon2 PHC string over the
/// code's verifier half; the code itself is never stored. The lifecycle
/// columns are read-only context for the GM's listing — they are NOT the
/// redemption gate, which lives entirely in `consume_invite`'s single guarded
/// UPDATE (see [[two-query-guard-needs-tx]]).
#[derive(Debug, Clone)]
pub struct InviteRecord {
    /// Selector half of the invite code (also the row id).
    pub id: Uuid,
    /// World the invite seats into.
    pub world_id: Uuid,
    /// Argon2 PHC string over the code's verifier half; the code is never stored.
    pub secret_hash: String,
    /// Role granted on redemption (for a NEW member; standing is never changed).
    pub role: WorldRole,
    /// Mint time, Unix epoch milliseconds.
    pub created_at: i64,
    /// Expiry, Unix epoch milliseconds.
    pub expires_at: i64,
    /// Set when a GM revokes the invite (listing context only).
    pub revoked_at: Option<i64>,
    /// Set when redeemed (listing context only).
    pub consumed_at: Option<i64>,
}

/// The outcome of a successful redemption: the world the caller is now a member
/// of and the role they actually hold there (which is their PRE-EXISTING role
/// when they were already a member — redemption grants access, never changes
/// standing). Every field is read inside `consume_invite`'s transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatedByInvite {
    /// The world the caller is now a member of.
    pub world: Uuid,
    /// Its display name (for the redemption response).
    pub world_name: String,
    /// The role they hold there (pre-existing role if already a member).
    pub role: WorldRole,
}

/// The fields of an invite row at mint time.
pub struct NewInvite<'a> {
    /// Selector half of the minted code — the row id and the code must agree.
    pub id: Uuid,
    /// World the invite is for.
    pub world: Uuid,
    /// Argon2 PHC string over the code's verifier half.
    pub secret_hash: &'a str,
    /// Role a new member is seated with.
    pub role: WorldRole,
    /// Minting GM's user id.
    pub created_by: Uuid,
    /// Mint time, Unix epoch milliseconds.
    pub now: i64,
    /// Expiry, Unix epoch milliseconds.
    pub expires_at: i64,
}

/// The `documents` row's scope/source column tuple `document_row_columns`
/// derives from a `Document` envelope: `(scope_kind, world_id, pack,
/// source_id, source_pack, source_version)`.
type DocumentRowColumns = (
    &'static str,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
);

/// SQLite-backed storage. Holds a connection pool; migrations are embedded
/// from `migrations/` and run at connect time.
pub struct SqliteRepository {
    /// Single-connection pool: the one writer serializing every transaction.
    pool: SqlitePool,
    /// The connect options `pool` was opened from — cloned to open a second
    /// pool against the identical database (see `open_read_pool`); never
    /// re-derive this by re-parsing a URL string (see
    /// `crate::db::parse_connect_options`'s doc for why).
    connect_options: sqlx::sqlite::SqliteConnectOptions,
}

impl SqliteRepository {
    /// Connect to `url` (e.g. "sqlite::memory:" or "sqlite:///path/to.db")
    /// and run migrations. `url` is parsed once via
    /// [`crate::db::parse_connect_options`] and the resulting options open
    /// the pool through [`crate::db::connect_pool_with_options`] — never
    /// restated here.
    pub async fn connect(url: &str) -> Result<Self, DataError> {
        let connect_options = crate::db::parse_connect_options(url)?;
        let pool = crate::db::connect_pool_with_options(connect_options.clone()).await?;
        sqlx::migrate!()
            .run(&pool)
            .await
            .map_err(sqlx::Error::from)?;
        Ok(Self {
            pool,
            connect_options,
        })
    }

    /// The underlying pool, for callers that run their own queries (tests,
    /// one-shot admin paths).
    ///
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let row: (i64,) = sqlx::query_as("SELECT 1").fetch_one(repo.pool()).await?;
    /// assert_eq!(row.0, 1);
    /// # Ok(())
    /// # }
    /// ```
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Opens a second, read-only pool against the same database `pool`
    /// writes through — see [`crate::db::open_read_only_pool`]. For a
    /// `sqlite::memory:`-backed repository this shares the SAME generated
    /// in-memory database [`Self::connect`] opened, never a fresh, empty one.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let read_pool = repo.open_read_pool().await?;
    /// let row: (i64,) = sqlx::query_as("SELECT 1").fetch_one(&read_pool).await?;
    /// assert_eq!(row.0, 1);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn open_read_pool(&self) -> Result<SqlitePool, sqlx::Error> {
        crate::db::open_read_only_pool(self.connect_options.clone()).await
    }

    /// See `Repository::get_link_preview_cache`.
    pub async fn get_link_preview_cache(
        &self,
        url: &str,
    ) -> Result<Option<crate::data::repository::LinkPreviewCacheRow>, DataError> {
        let row = sqlx::query(
            "SELECT title, description, image_asset_id, fetched_at FROM link_preview_cache WHERE url = ?",
        )
        .bind(url)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let fetched_at_raw: String = row.get("fetched_at");
        let fetched_at_ms = fetched_at_raw
            .parse::<i64>()
            .map_err(|e| DataError::OpFailed(e.to_string()))?;
        let image_asset_id = row
            .get::<Option<String>, _>("image_asset_id")
            .map(|s| Uuid::parse_str(&s).map_err(|e| DataError::OpFailed(e.to_string())))
            .transpose()?;
        Ok(Some(crate::data::repository::LinkPreviewCacheRow {
            title: row.get("title"),
            description: row.get("description"),
            image_asset_id,
            fetched_at_ms,
        }))
    }

    /// See `Repository::upsert_link_preview_cache`.
    pub async fn upsert_link_preview_cache(
        &self,
        url: &str,
        title: Option<&str>,
        description: Option<&str>,
        fetched_at_ms: i64,
    ) -> Result<(), DataError> {
        sqlx::query(
            "INSERT INTO link_preview_cache (url, title, description, fetched_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(url) DO UPDATE SET \
               title = excluded.title, description = excluded.description, fetched_at = excluded.fetched_at",
        )
        .bind(url)
        .bind(title)
        .bind(description)
        .bind(fetched_at_ms.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// See `Repository::set_link_preview_cache_image`.
    pub async fn set_link_preview_cache_image(
        &self,
        url: &str,
        image_asset_id: Uuid,
    ) -> Result<(), DataError> {
        sqlx::query("UPDATE link_preview_cache SET image_asset_id = ? WHERE url = ?")
            .bind(image_asset_id.to_string())
            .bind(url)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Insert a new world row with `seq = 0` and return it.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let world = repo.create_world("MOCK_WORLD_A", 0).await?;
    /// assert_eq!(world.seq, 0);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_world(&self, name: &str, now: i64) -> Result<World, DataError> {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO worlds (id, name, seq, created_at, updated_at) VALUES (?, ?, 0, ?, ?)",
        )
        .bind(id.to_string())
        .bind(name)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(World {
            id,
            name: name.to_string(),
            seq: 0,
            created_at: now,
            updated_at: now,
        })
    }

    /// Create a world and seat its creator as the first GM, atomically.
    /// Reuses the `world_members` table from 0001 (column `role`, serde-encoded
    /// WorldRole), matching the existing `add_member`/`member_role` methods.
    pub async fn create_world_owned(
        &self,
        name: &str,
        creator: Uuid,
        now: i64,
    ) -> Result<World, DataError> {
        let mut tx = self.pool.begin().await?;
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO worlds (id, name, seq, created_at, updated_at) VALUES (?, ?, 0, ?, ?)",
        )
        .bind(id.to_string())
        .bind(name)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO world_members (world_id, user_id, role) VALUES (?, ?, ?)")
            .bind(id.to_string())
            .bind(creator.to_string())
            .bind(
                serde_json::to_value(WorldRole::Gm)?
                    .as_str()
                    .unwrap()
                    .to_string(),
            )
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(World {
            id,
            name: name.to_string(),
            seq: 0,
            created_at: now,
            updated_at: now,
        })
    }

    /// Whether `user` is the world's sole GM, evaluated on the supplied tx
    /// connection so the read and the caller's mutation are atomic — without the
    /// shared tx, the count check and the mutation are separate connection
    /// acquisitions and two concurrent removals could each pass the check (TOCTOU)
    /// and orphan the world. A server admin remains GM everywhere, so the world is
    /// never permanently orphaned; the guard only blocks accidental self-lockout.
    async fn is_last_gm(
        tx: &mut sqlx::SqliteConnection,
        world: Uuid,
        user: Uuid,
    ) -> Result<bool, DataError> {
        let gm = serde_json::to_value(WorldRole::Gm)?
            .as_str()
            .unwrap()
            .to_string();
        let target: Option<String> =
            sqlx::query_scalar("SELECT role FROM world_members WHERE world_id = ? AND user_id = ?")
                .bind(world.to_string())
                .bind(user.to_string())
                .fetch_optional(&mut *tx)
                .await?;
        if target.as_deref() != Some(gm.as_str()) {
            return Ok(false);
        }
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM world_members WHERE world_id = ? AND role = ?",
        )
        .bind(world.to_string())
        .bind(&gm)
        .fetch_one(&mut *tx)
        .await?;
        Ok(n <= 1)
    }

    /// Whether `user` is the server's sole administrator, evaluated on the
    /// supplied tx connection for the same TOCTOU reason as `is_last_gm`: the
    /// count check and the delete must be one atomic unit on the single-writer
    /// pool, or two concurrent deletes could each pass the check.
    async fn is_last_admin(tx: &mut sqlx::SqliteConnection, user: Uuid) -> Result<bool, DataError> {
        let target: Option<String> =
            sqlx::query_scalar("SELECT server_role FROM users WHERE id = ?")
                .bind(user.to_string())
                .fetch_optional(&mut *tx)
                .await?;
        if target.as_deref() != Some(crate::auth::role::ServerRole::Admin.as_str()) {
            return Ok(false);
        }
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE server_role = ?")
            .bind(crate::auth::role::ServerRole::Admin.as_str())
            .fetch_one(&mut *tx)
            .await?;
        Ok(n <= 1)
    }

    /// Delete a user account and everything keyed to it, in one transaction:
    /// memberships CASCADE; documents.owner_id / world_events.author_id /
    /// world_invites.{created_by,consumed_by} SET NULL; assets.created_by SET
    /// NULL (0011); each owned document's JSON-body `owner` is nulled in
    /// lockstep with its column; explored_fog rows (no FK; unindexed scan —
    /// rare admin op) and live sessions are purged explicitly. Sessions MUST die in this same
    /// transaction: `AuthUser` trusts the session record without re-reading
    /// `users`, so a surviving row keeps a deleted account authenticated until
    /// cookie expiry. Refuses to delete the last administrator.
    /// Implicit coupling: `tower_sessions` is created by `SqlxSqliteStore::
    /// migrate`, called from `session_layer` at boot, before any route can reach this;
    /// repo-level tests must run that migrate themselves.
    pub async fn delete_user(&self, target: Uuid) -> Result<(), DataError> {
        let mut tx = self.pool.begin().await?;
        if Self::is_last_admin(&mut tx, target).await? {
            return Err(DataError::Conflict(
                "cannot delete the server's only administrator".into(),
            ));
        }
        let res = sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(target.to_string())
            .execute(&mut *tx)
            .await?;
        if res.rows_affected() == 0 {
            return Err(DataError::NotFound);
        }
        // A document's `owner` ALSO lives inside its JSON body — the
        // `owner_id` column the FK just SET NULL'd is a denormalized copy.
        // Null the JSON field in the same tx so the two representations cannot
        // disagree (never-fork). A ghost owner would be fail-closed anyway (a
        // deleted id matches no session, and ids are never reused), so this is
        // structural agreement, not a behavioral gate. Embedded children keep
        // any stale owner reference: they have no owner_id column (no
        // split-brain to close) and the same fail-closed reasoning applies,
        // uniform with historical event-log blobs.
        sqlx::query(
            "UPDATE documents SET json = json_set(json, '$.owner', null) \
             WHERE json_extract(json, '$.owner') = ?",
        )
        .bind(target.to_string())
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM explored_fog WHERE user_id = ?")
            .bind(target.to_string())
            .execute(&mut *tx)
            .await?;
        // Session identity lives at $.data.user.id inside the JSON blob (the
        // store has no user_id column); JSON1 ships in the bundled SQLite.
        sqlx::query("DELETE FROM tower_sessions WHERE json_extract(data, '$.data.user.id') = ?")
            .bind(target.to_string())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Change an existing member's role; `NotFound` if they are not a member.
    pub async fn set_role(
        &self,
        world: Uuid,
        user: Uuid,
        role: WorldRole,
    ) -> Result<(), DataError> {
        let mut tx = self.pool.begin().await?;
        if role != WorldRole::Gm && Self::is_last_gm(&mut tx, world, user).await? {
            return Err(DataError::Conflict(
                "cannot demote the world's only GM".into(),
            ));
        }
        let res =
            sqlx::query("UPDATE world_members SET role = ? WHERE world_id = ? AND user_id = ?")
                .bind(serde_json::to_value(role)?.as_str().unwrap().to_string())
                .bind(world.to_string())
                .bind(user.to_string())
                .execute(&mut *tx)
                .await?;
        if res.rows_affected() == 0 {
            return Err(DataError::NotFound);
        }
        tx.commit().await?;
        Ok(())
    }

    /// Remove `user` from `world`. Refuses (Conflict) to remove the last GM —
    /// a world must always have at least one.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// # let (world_id, sole_gm) = (uuid::Uuid::nil(), uuid::Uuid::nil());
    /// // Removing the only GM is refused with DataError::Conflict.
    /// let err = repo.remove_member(world_id, sole_gm).await.unwrap_err();
    /// # let _ = err;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn remove_member(&self, world: Uuid, user: Uuid) -> Result<(), DataError> {
        let mut tx = self.pool.begin().await?;
        if Self::is_last_gm(&mut tx, world, user).await? {
            return Err(DataError::Conflict(
                "cannot remove the world's only GM".into(),
            ));
        }
        sqlx::query("DELETE FROM world_members WHERE world_id = ? AND user_id = ?")
            .bind(world.to_string())
            .bind(user.to_string())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The world's members as `(user_id, username, role)`, username order.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let members = repo.list_members(uuid::Uuid::nil()).await?;
    /// assert!(members.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_members(
        &self,
        world: Uuid,
    ) -> Result<Vec<(Uuid, String, WorldRole)>, DataError> {
        let rows = sqlx::query(
            "SELECT m.user_id, u.username, m.role \
             FROM world_members m JOIN users u ON u.id = m.user_id \
             WHERE m.world_id = ? \
             ORDER BY u.username COLLATE NOCASE",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                let uid = Uuid::parse_str(r.get::<String, _>("user_id").as_str())
                    .map_err(|e| DataError::OpFailed(e.to_string()))?;
                let username: String = r.get("username");
                let role: WorldRole =
                    serde_json::from_value(serde_json::Value::String(r.get::<String, _>("role")))?;
                Ok((uid, username, role))
            })
            .collect()
    }

    /// Worlds the user may access, with their effective role. A server admin is
    /// GM on every world (mirrors `permission_context`); otherwise the user's
    /// joined `world_members.role`. Ordered by world name.
    pub async fn worlds_for_user(
        &self,
        user: Uuid,
        server_role: ServerRole,
    ) -> Result<Vec<(World, WorldRole)>, DataError> {
        let rows = if server_role == ServerRole::Admin {
            sqlx::query(
                "SELECT id, name, seq, created_at, updated_at, NULL AS role \
                 FROM worlds ORDER BY name",
            )
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT w.id, w.name, w.seq, w.created_at, w.updated_at, m.role AS role \
                 FROM worlds w \
                 JOIN world_members m ON m.world_id = w.id \
                 WHERE m.user_id = ? ORDER BY w.name",
            )
            .bind(user.to_string())
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter()
            .map(|r| {
                let world = World {
                    id: Uuid::parse_str(r.get::<String, _>("id").as_str())
                        .map_err(|e| DataError::OpFailed(e.to_string()))?,
                    name: r.get("name"),
                    seq: r.get("seq"),
                    created_at: r.get("created_at"),
                    updated_at: r.get("updated_at"),
                };
                // Admin rows carry NULL role → GM; member rows decode their role.
                let role = match r.get::<Option<String>, _>("role") {
                    Some(s) => serde_json::from_value(serde_json::Value::String(s))?,
                    None => WorldRole::Gm,
                };
                Ok((world, role))
            })
            .collect()
    }

    /// Resolve a user's authority within a world: server admins are GM
    /// everywhere; a member resolves to their `role`; a non-member non-admin is
    /// `Forbidden` (cannot establish a context, so cannot join or write).
    pub async fn permission_context(
        &self,
        world: Uuid,
        user: Uuid,
        server_role: ServerRole,
    ) -> Result<crate::data::membership::PermissionContext, DataError> {
        use crate::data::membership::PermissionContext;
        if server_role == ServerRole::Admin {
            return Ok(PermissionContext {
                user_id: user,
                world_role: WorldRole::Gm,
            });
        }
        match self.member_role(world, user).await? {
            Some(role) => Ok(PermissionContext {
                user_id: user,
                world_role: role,
            }),
            None => Err(DataError::Forbidden),
        }
    }

    /// Insert a new account. `password_hash` is a ready Argon2 PHC string
    /// (hashing happens in the auth layer); `None` disables login.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// use shadowcat::auth::role::ServerRole;
    /// let id = repo.create_user("testuser-01", None, ServerRole::User, 0).await?;
    /// assert!(!id.is_nil());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_user(
        &self,
        username: &str,
        password_hash: Option<&str>,
        role: ServerRole,
        now: i64,
    ) -> Result<Uuid, DataError> {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, server_role, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(username)
        .bind(password_hash)
        .bind(role.as_str())
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// The user's stored opaque UI-state JSON string, or `None` when unset.
    pub async fn get_ui_state(&self, user: Uuid) -> Result<Option<String>, DataError> {
        let row = sqlx::query("SELECT ui_state FROM users WHERE id = ?")
            .bind(user.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.and_then(|r| r.get::<Option<String>, _>("ui_state")))
    }

    /// Merge a partial UI-state patch into the user's stored blob, one level
    /// at the individual leaf key (`global.<field>` / `worlds.<id>.<key>`) —
    /// **the single server-side statement of this rule.** For each top-level
    /// patch key `K`: if `K == "worlds"` (an object; route-validated), then
    /// for each `(id, slice)` in it — when BOTH the stored `worlds.<id>` and
    /// `slice` are objects, merge one level (each slice key, e.g.
    /// `panelLayout`/`chatRead`, replaces wholesale — a leaf blob is opaque
    /// and NEVER deep-merged); otherwise insert `slice` wholesale. For any
    /// other `K` (e.g. `global`) — when BOTH `stored[K]` and `patch[K]` are
    /// objects, merge one level (each second-level key replaces wholesale);
    /// otherwise replace `stored[K]` wholesale. Absent keys are untouched. A
    /// `null` in the patch REMOVES rather than replaces: `null` at
    /// `worlds.<id>` removes that whole entry, `null` at a leaf key inside a
    /// `worlds.<id>` slice (or inside `global`) removes just that key, and
    /// `null` at any other top-level `K` removes it entirely — see
    /// `merge_one_level`. This is the recovery path for an over-cap blob.
    /// This leaf-key granularity is the concurrency control — concurrent
    /// sessions of the same user (two tabs, two mutating owners of the same
    /// slice: e.g. the panels module writing `panelLayout` and the chat
    /// module writing `chatRead` inside the same `worlds.<id>`) contend only
    /// on the individual keys both actually write, so a session's write can
    /// never revert a key it did not touch. Read+merge+write run in ONE
    /// transaction (a check-then-act across two pool queries is TOCTOU-racy
    /// even on the single-writer pool). `max_bytes` caps the MERGED
    /// serialization — only this function sees it, so the cap cannot live at
    /// the HTTP boundary. `NotFound` if the user is absent. INVARIANT:
    /// `patch` is an object and `patch.worlds`, when present, is an object
    /// (the HTTP boundary rejects other shapes; violations here surface as
    /// `OpFailed`).
    pub async fn merge_ui_state(
        &self,
        user: Uuid,
        patch: &serde_json::Value,
        max_bytes: usize,
    ) -> Result<(), DataError> {
        let patch_obj = patch
            .as_object()
            .ok_or_else(|| DataError::OpFailed("ui_state patch must be an object".into()))?;
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query("SELECT ui_state FROM users WHERE id = ?")
            .bind(user.to_string())
            .fetch_optional(&mut *tx)
            .await?;
        let Some(row) = row else {
            return Err(DataError::NotFound);
        };
        let mut stored: serde_json::Value = match row.get::<Option<String>, _>("ui_state") {
            Some(s) => serde_json::from_str(&s)?,
            None => serde_json::json!({}),
        };
        let stored_obj = stored
            .as_object_mut()
            .ok_or_else(|| DataError::OpFailed("stored ui_state is not an object".into()))?;
        for (key, value) in patch_obj {
            if key == "worlds" {
                let worlds_patch = value.as_object().ok_or_else(|| {
                    DataError::OpFailed("ui_state patch `worlds` must be an object".into())
                })?;
                let worlds = stored_obj
                    .entry("worlds")
                    .or_insert_with(|| serde_json::json!({}));
                let worlds_obj = worlds.as_object_mut().ok_or_else(|| {
                    DataError::OpFailed("stored ui_state `worlds` is not an object".into())
                })?;
                for (id, slice) in worlds_patch {
                    merge_one_level(worlds_obj, id, slice);
                }
            } else {
                merge_one_level(stored_obj, key, value);
            }
        }
        let merged = serde_json::to_string(&stored)?;
        if merged.len() > max_bytes {
            return Err(DataError::TooLarge(merged.len()));
        }
        sqlx::query("UPDATE users SET ui_state = ? WHERE id = ?")
            .bind(&merged)
            .bind(user.to_string())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The player's serialized explored-cell blob for a scene, or `None` when unexplored.
    /// Per-(scene, user) SECRET memory — never broadcast; dispatched per-recipient over `vision`.
    pub async fn get_explored(
        &self,
        scene: Uuid,
        user: Uuid,
    ) -> Result<Option<Vec<u8>>, DataError> {
        let row = sqlx::query("SELECT cells FROM explored_fog WHERE scene_id = ? AND user_id = ?")
            .bind(scene.to_string())
            .bind(user.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<Vec<u8>, _>("cells")))
    }

    /// Delete a world and every row keyed to it, in one transaction. FK cascades
    /// cover world_members/documents/world_events/assets/world_invites, and the
    /// FTS AFTER DELETE triggers fire under cascade (pinned by test).
    /// `explored_fog` and the per-world `settings` blobs have no FK and are
    /// purged explicitly. Files on disk are the caller's concern — delete
    /// ordering is rows-first, files-second (`http::assets` delete convention).
    pub async fn delete_world(&self, world: Uuid) -> Result<(), DataError> {
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query("DELETE FROM worlds WHERE id = ?")
            .bind(world.to_string())
            .execute(&mut *tx)
            .await?;
        if res.rows_affected() == 0 {
            return Err(DataError::NotFound);
        }
        sqlx::query("DELETE FROM explored_fog WHERE world_id = ?")
            .bind(world.to_string())
            .execute(&mut *tx)
            .await?;
        for key in world_settings_keys(world) {
            sqlx::query("DELETE FROM settings WHERE key = ?")
                .bind(key)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Every world-scoped row `delete_world` would delete, read instead — the
    /// per-world export data source. `users(id)` references are resolved to
    /// portable usernames inline (one `LEFT JOIN`/`JOIN` per table, no N+1
    /// lookups) exactly as documented on each `data::world_bundle::Exported*Row`
    /// type. `NotFound` if `world` does not exist.
    pub async fn export_world_rows(&self, world: Uuid) -> Result<WorldExportData, DataError> {
        let world_row =
            sqlx::query("SELECT name, seq, created_at, updated_at FROM worlds WHERE id = ?")
                .bind(world.to_string())
                .fetch_optional(&self.pool)
                .await?
                .ok_or(DataError::NotFound)?;

        let doc_rows = sqlx::query(
            "SELECT documents.json AS json, documents.seq AS seq, \
             documents.created_seq AS created_seq, users.username AS owner_username \
             FROM documents LEFT JOIN users ON users.id = documents.owner_id \
             WHERE documents.world_id = ? ORDER BY documents.id",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut documents = Vec::with_capacity(doc_rows.len());
        for r in doc_rows {
            let mut document: Document = serde_json::from_str(&r.get::<String, _>("json"))?;
            document.owner = None;
            documents.push(ExportedDocumentRow {
                document,
                owner_username: r.get::<Option<String>, _>("owner_username"),
                seq: r.get("seq"),
                created_seq: r.get("created_seq"),
            });
        }

        let event_rows = sqlx::query(
            "SELECT world_events.seq AS seq, world_events.ts AS ts, \
             world_events.command_json AS command_json, users.username AS author_username \
             FROM world_events LEFT JOIN users ON users.id = world_events.author_id \
             WHERE world_events.world_id = ? ORDER BY world_events.seq",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        let events: Vec<ExportedEventRow> = event_rows
            .into_iter()
            .map(|r| ExportedEventRow {
                seq: r.get("seq"),
                author_username: r.get::<Option<String>, _>("author_username"),
                ts: r.get("ts"),
                command_json: r.get("command_json"),
            })
            .collect();

        let member_rows = sqlx::query(
            "SELECT users.username AS username, world_members.role AS role \
             FROM world_members JOIN users ON users.id = world_members.user_id \
             WHERE world_members.world_id = ? ORDER BY users.username",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut members = Vec::with_capacity(member_rows.len());
        for r in member_rows {
            let role: WorldRole =
                serde_json::from_value(serde_json::Value::String(r.get::<String, _>("role")))?;
            members.push(ExportedMemberRow {
                username: r.get("username"),
                role,
            });
        }

        let invite_rows = sqlx::query(
            "SELECT world_invites.id AS id, world_invites.secret_hash AS secret_hash, \
             world_invites.role AS role, world_invites.created_at AS created_at, \
             world_invites.expires_at AS expires_at, world_invites.revoked_at AS revoked_at, \
             world_invites.consumed_at AS consumed_at, \
             creator.username AS created_by_username, consumer.username AS consumed_by_username \
             FROM world_invites \
             LEFT JOIN users creator ON creator.id = world_invites.created_by \
             LEFT JOIN users consumer ON consumer.id = world_invites.consumed_by \
             WHERE world_invites.world_id = ? ORDER BY world_invites.id",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut invites = Vec::with_capacity(invite_rows.len());
        for r in invite_rows {
            let role: WorldRole =
                serde_json::from_value(serde_json::Value::String(r.get::<String, _>("role")))?;
            invites.push(ExportedInviteRow {
                id: Uuid::parse_str(r.get::<String, _>("id").as_str())
                    .map_err(|e| DataError::OpFailed(e.to_string()))?,
                secret_hash: r.get("secret_hash"),
                role,
                created_by_username: r.get::<Option<String>, _>("created_by_username"),
                created_at: r.get("created_at"),
                expires_at: r.get("expires_at"),
                revoked_at: r.get::<Option<i64>, _>("revoked_at"),
                consumed_at: r.get::<Option<i64>, _>("consumed_at"),
                consumed_by_username: r.get::<Option<String>, _>("consumed_by_username"),
            });
        }

        let asset_rows = sqlx::query(
            "SELECT assets.*, users.username AS created_by_username \
             FROM assets LEFT JOIN users ON users.id = assets.created_by \
             WHERE assets.world_id = ? ORDER BY assets.id",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut full: Vec<crate::data::asset::Asset> = asset_rows
            .iter()
            .map(Self::asset_from_row)
            .collect::<Result<_, _>>()?;
        self.fill_tags(&mut full).await?;
        let mut assets = Vec::with_capacity(asset_rows.len());
        for (r, a) in asset_rows.iter().zip(full) {
            assets.push(ExportedAssetRow {
                id: a.id,
                original_name: a.original_name,
                content_type: a.content_type,
                byte_size: a.byte_size,
                created_by_username: r.get::<Option<String>, _>("created_by_username"),
                created_at: a.created_at,
                version: a.version,
                folder_id: a.folder_id,
                tags: a.tags,
                derived_tags: a.derived_tags,
                meta: a.meta,
            });
        }

        let fog_rows = sqlx::query(
            "SELECT explored_fog.scene_id AS scene_id, explored_fog.cells AS cells, \
             users.username AS username \
             FROM explored_fog JOIN users ON users.id = explored_fog.user_id \
             WHERE explored_fog.world_id = ? ORDER BY explored_fog.scene_id, users.username",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut fog = Vec::with_capacity(fog_rows.len());
        for r in fog_rows {
            fog.push(ExportedFogRow {
                scene_id: Uuid::parse_str(r.get::<String, _>("scene_id").as_str())
                    .map_err(|e| DataError::OpFailed(e.to_string()))?,
                username: r.get("username"),
                cells: r.get("cells"),
            });
        }

        let mut settings = Vec::new();
        for key in world_settings_keys(world) {
            let value: Option<String> =
                sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
                    .bind(&key)
                    .fetch_optional(&self.pool)
                    .await?;
            if let Some(value) = value {
                settings.push(ExportedSettingRow { key, value });
            }
        }

        let mut row_counts = std::collections::BTreeMap::new();
        row_counts.insert("documents".to_string(), documents.len());
        row_counts.insert("world_events".to_string(), events.len());
        row_counts.insert("world_members".to_string(), members.len());
        row_counts.insert("world_invites".to_string(), invites.len());
        row_counts.insert("assets".to_string(), assets.len());
        row_counts.insert("explored_fog".to_string(), fog.len());
        row_counts.insert("settings".to_string(), settings.len());

        let manifest = BundleManifest {
            schema_version: BUNDLE_SCHEMA_VERSION,
            world_id: world,
            world_name: world_row.get("name"),
            world_seq: world_row.get("seq"),
            world_created_at: world_row.get("created_at"),
            world_updated_at: world_row.get("updated_at"),
            exported_at_unix_ms: crate::ws::time::now_millis(),
            row_counts,
        };

        Ok(WorldExportData {
            manifest,
            documents,
            events,
            members,
            invites,
            assets,
            fog,
            settings,
        })
    }

    /// Resolve a portable username to a target-local user id inside `tx`, or
    /// `None` when `username` is `None` (no source owner) OR the username
    /// does not exist on this server — the degradation
    /// `documents.owner_id`/`world_events.author_id`/
    /// `world_invites.{created_by,consumed_by}` are already `ON DELETE SET
    /// NULL`-designed around.
    async fn resolve_username_tx(
        tx: &mut sqlx::SqliteConnection,
        username: Option<&str>,
    ) -> Result<Option<Uuid>, DataError> {
        let Some(username) = username else {
            return Ok(None);
        };
        let id: Option<String> = sqlx::query_scalar("SELECT id FROM users WHERE username = ?")
            .bind(username)
            .fetch_optional(&mut *tx)
            .await?;
        id.map(|s| Uuid::parse_str(&s).map_err(|e| DataError::OpFailed(e.to_string())))
            .transpose()
    }

    /// Insert one imported document row with EXPLICIT `seq`/`created_seq`,
    /// independently preserved from the source server — unlike the live
    /// write path's `upsert_document`, where a fresh Create always sets
    /// `seq == created_seq`. Shares `document_row_columns`/
    /// `reindex_document_fts` with `upsert_document` (search state is
    /// rebuilt from `doc`'s content, never carried across servers —
    /// `documents_fts_public`/`documents_fts_gm` are never exported/imported
    /// directly, see `data::world_bundle`'s module doc). A plain `INSERT`
    /// (not `upsert_document`'s `ON CONFLICT(id) DO UPDATE`): a document id
    /// colliding with an existing row anywhere on the target server (a
    /// separate axis from the already-gated world-id collision) is a
    /// genuine data-integrity fault, and letting the `UNIQUE` constraint
    /// violation surface as an ordinary `DataError::Sqlx` — aborting and
    /// rolling back the whole import transaction — is exactly the "any
    /// row-insert failure mid-transaction rolls back the whole import"
    /// behavior `import_world` already provides, not a case needing special
    /// handling. Callers must run `doc` through the same ingress validation
    /// every live write path runs (`import_world`'s own per-document loop
    /// does) before calling this — this function itself performs none.
    async fn insert_imported_document(
        conn: &mut sqlx::SqliteConnection,
        doc: &Document,
        seq: i64,
        created_seq: i64,
    ) -> Result<(), DataError> {
        let (scope_kind, world_id, pack, source_id, source_pack, source_version) =
            Self::document_row_columns(doc);
        let json = serde_json::to_string(doc)?;
        sqlx::query(
            "INSERT INTO documents (id, scope_kind, world_id, pack, doc_type, schema_version, \
             source_id, source_pack, source_version, owner_id, parent_id, seq, created_seq, json, \
             created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(doc.id.to_string())
        .bind(scope_kind)
        .bind(world_id.clone())
        .bind(pack)
        .bind(&doc.doc_type)
        .bind(doc.schema_version as i64)
        .bind(source_id)
        .bind(source_pack)
        .bind(source_version)
        .bind(doc.owner.map(|o| o.to_string()))
        .bind(doc.parent_id.map(|p| p.to_string()))
        .bind(seq)
        .bind(created_seq)
        .bind(json)
        .bind(doc.created_at)
        .bind(doc.updated_at)
        .execute(&mut *conn)
        .await?;
        Self::reindex_document_fts(conn, doc, world_id).await
    }

    /// Import one `WorldImportData` bundle in a single transaction: reject a
    /// world-id collision with `worlds.id` before any row is written, insert
    /// `worlds` then every table in FK-safe order (`documents`/
    /// `world_events`/`world_members`/`world_invites`/`assets`, then the
    /// FK-less `explored_fog`/`settings`), resolving each row's portable
    /// username(s) against THIS server's `users` table, then finalize every
    /// staged asset file (rename into place beside itself — see
    /// `data::world_bundle::WorldImportData.staged_assets`) before
    /// committing — a failure at any point (including a rename) drops the
    /// transaction unrolled-back, so no partial world is ever visible.
    /// `world_members`/`explored_fog` rows whose username does not resolve
    /// are DROPPED (their `user_id` column is `NOT NULL`, so there is no
    /// `SET NULL` degradation to fall back to, unlike the four nullable
    /// owner/author/created_by/consumed_by columns) — counted in the
    /// returned `ImportSummary` rather than silently absorbed. Every
    /// document is run through the same ingress-validation chokepoint the
    /// live `Create`/`Update` write paths use
    /// (`validation::validate_system_size`/`validate_property_overrides`/
    /// `validate_engine_tree`/`validate_system_schema_tree`) before it
    /// reaches storage — an
    /// imported bundle is untrusted input to THIS server even when it was
    /// exported by a trusted admin from another one. Holds the pool's single
    /// writer connection for the entire call, including the asset-rename
    /// loop — every other server write (chat, moves, document edits) blocks
    /// for the whole import, the same trade-off `POST /api/admin/backup`
    /// already accepts for its snapshot.
    pub async fn import_world(&self, data: WorldImportData) -> Result<ImportSummary, DataError> {
        let mut tx = self.pool.begin().await?;
        let world = data.manifest.world_id;

        let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM worlds WHERE id = ?")
            .bind(world.to_string())
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_some() {
            return Err(DataError::Conflict(format!(
                "world {world} already exists on this server"
            )));
        }

        // Every `assets` row must have a matching staged file, or the
        // finalize loop below would leave a DB row with no backing bytes on
        // a truncated/malformed bundle. Checked up front, before any row is
        // written, so this failure mode is atomic like every other
        // `import_world` rejection.
        let staged_ids: std::collections::HashSet<Uuid> =
            data.staged_assets.iter().map(|(id, _)| *id).collect();
        for row in &data.assets {
            if !staged_ids.contains(&row.id) {
                return Err(DataError::OpFailed(format!(
                    "asset {} has no corresponding staged file in the bundle",
                    row.id
                )));
            }
        }

        sqlx::query(
            "INSERT INTO worlds (id, name, seq, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(world.to_string())
        .bind(&data.manifest.world_name)
        .bind(data.manifest.world_seq)
        .bind(data.manifest.world_created_at)
        .bind(data.manifest.world_updated_at)
        .execute(&mut *tx)
        .await?;

        // The tier-2 structural schema registry `validate_system_schema_tree`
        // needs, read from the BUNDLE's own imported `settings` rows rather
        // than `self.world_schema_declarations(world)` — that method queries
        // `self.pool` via a fresh connection, which would deadlock against
        // the transaction already held here under this server's
        // `max_connections(1)` single-writer pool. Mirrors
        // `world_schema_declarations`'s own `None => Vec::new()` default.
        let world_schemas: Vec<SchemaDeclaration> = data
            .settings
            .iter()
            .find(|s| s.key == world_schemas_key(world))
            .map(|s| serde_json::from_str(&s.value))
            .transpose()?
            .unwrap_or_default();

        // Mirrors `apply_intent`'s intra-batch `claimed_singletons` tracking
        // (see `SINGLETON_DOC_TYPES`'s own doc) — a bundle is untrusted
        // input assembled outside any live `apply_intent` call, so nothing
        // else in this loop would otherwise catch two documents of the same
        // singleton doc_type both landing in one import.
        let mut claimed_singletons: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for row in &data.documents {
            let owner = Self::resolve_username_tx(&mut tx, row.owner_username.as_deref()).await?;
            let mut document = row.document.clone();
            document.owner = owner;
            if SINGLETON_DOC_TYPES.contains(&document.doc_type.as_str())
                && !claimed_singletons.insert(document.doc_type.clone())
            {
                return Err(DataError::Conflict(format!(
                    "bundle contains more than one '{}' document, which is capped at one per world",
                    document.doc_type
                )));
            }
            // Same ingress-validation chokepoint every live `Create`/`Update`
            // runs before a document reaches storage (see e.g. the
            // `Operation::Update` handler in `apply_intent`) — an imported
            // bundle is untrusted input, not a trusted internal write.
            // `validate_engine_tree` mutates `document.engine` in place
            // (re-normalizes it); the persisted row must hold that
            // normalized form, same as every other write path.
            validation::validate_system_size(&document)?;
            validation::validate_property_overrides(&document)?;
            validation::validate_engine_tree(&mut document)?;
            validation::validate_system_schema_tree(&document, &world_schemas)?;
            Self::insert_imported_document(&mut tx, &document, row.seq, row.created_seq).await?;
        }

        for row in &data.events {
            let author = Self::resolve_username_tx(&mut tx, row.author_username.as_deref()).await?;
            sqlx::query(
                "INSERT INTO world_events (world_id, seq, author_id, ts, command_json) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(world.to_string())
            .bind(row.seq)
            .bind(author.map(|u| u.to_string()))
            .bind(row.ts)
            .bind(&row.command_json)
            .execute(&mut *tx)
            .await?;
        }

        let mut skipped_members = 0usize;
        for row in &data.members {
            match Self::resolve_username_tx(&mut tx, Some(row.username.as_str())).await? {
                Some(user_id) => {
                    sqlx::query(
                        "INSERT INTO world_members (world_id, user_id, role) VALUES (?, ?, ?)",
                    )
                    .bind(world.to_string())
                    .bind(user_id.to_string())
                    .bind(
                        serde_json::to_value(row.role)?
                            .as_str()
                            .expect("WorldRole serializes as a string")
                            .to_string(),
                    )
                    .execute(&mut *tx)
                    .await?;
                }
                None => skipped_members += 1,
            }
        }

        for row in &data.invites {
            let created_by =
                Self::resolve_username_tx(&mut tx, row.created_by_username.as_deref()).await?;
            let consumed_by =
                Self::resolve_username_tx(&mut tx, row.consumed_by_username.as_deref()).await?;
            sqlx::query(
                "INSERT INTO world_invites \
                 (id, world_id, secret_hash, role, created_by, created_at, expires_at, \
                  revoked_at, consumed_at, consumed_by) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(row.id.to_string())
            .bind(world.to_string())
            .bind(&row.secret_hash)
            .bind(
                serde_json::to_value(row.role)?
                    .as_str()
                    .expect("WorldRole serializes as a string")
                    .to_string(),
            )
            .bind(created_by.map(|u| u.to_string()))
            .bind(row.created_at)
            .bind(row.expires_at)
            .bind(row.revoked_at)
            .bind(row.consumed_at)
            .bind(consumed_by.map(|u| u.to_string()))
            .execute(&mut *tx)
            .await?;
        }

        for row in &data.assets {
            let created_by =
                Self::resolve_username_tx(&mut tx, row.created_by_username.as_deref()).await?;
            let storage_key = format!("{world}/{}", row.id);
            // `original_retained` is only true if the bundle actually carried
            // the `.orig` sibling for this asset.
            let has_orig = data
                .staged_siblings
                .iter()
                .any(|s| s.asset_id == row.id && s.suffix == ".orig");
            let meta = crate::data::asset::AssetMeta {
                original_retained: row.meta.original_retained && has_orig,
                ..row.meta.clone()
            };
            sqlx::query(
                "INSERT INTO assets \
                 (id, world_id, storage_key, original_name, content_type, byte_size, created_by, \
                  created_at, version, folder_id, width, height, has_alpha, animated, \
                  original_content_type, original_byte_size, original_retained, conversion_note) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(row.id.to_string())
            .bind(world.to_string())
            .bind(storage_key)
            .bind(&row.original_name)
            .bind(&row.content_type)
            .bind(row.byte_size)
            .bind(created_by.map(|u| u.to_string()))
            .bind(row.created_at)
            .bind(row.version)
            .bind(row.folder_id.map(|f| f.to_string()))
            .bind(meta.width.map(i64::from))
            .bind(meta.height.map(i64::from))
            .bind(i64::from(meta.has_alpha))
            .bind(i64::from(meta.animated))
            .bind(&meta.original_content_type)
            .bind(meta.original_byte_size)
            .bind(i64::from(meta.original_retained))
            .bind(&meta.conversion_note)
            .execute(&mut *tx)
            .await?;
            // A bundle is untrusted input: its explicit tags pass the same rule
            // every live writer applies (`tags::normalize_tags`); derived tags
            // are pipeline output and are re-derived on the next refresh.
            let explicit = crate::data::asset::tags::normalize_tags(row.tags.clone())
                .map_err(|m| DataError::OpFailed(format!("asset {} tags: {m}", row.id)))?;
            for (tag, derived) in explicit
                .iter()
                .map(|t| (t, 0_i64))
                .chain(row.derived_tags.iter().map(|t| (t, 1_i64)))
            {
                sqlx::query(
                    "INSERT OR IGNORE INTO asset_tags (asset_id, tag, derived) VALUES (?, ?, ?)",
                )
                .bind(row.id.to_string())
                .bind(tag)
                .bind(derived)
                .execute(&mut *tx)
                .await?;
            }
        }

        let mut skipped_fog = 0usize;
        for row in &data.fog {
            match Self::resolve_username_tx(&mut tx, Some(row.username.as_str())).await? {
                Some(user_id) => {
                    sqlx::query(
                        "INSERT INTO explored_fog (world_id, scene_id, user_id, cells) \
                         VALUES (?, ?, ?, ?)",
                    )
                    .bind(world.to_string())
                    .bind(row.scene_id.to_string())
                    .bind(user_id.to_string())
                    .bind(row.cells.as_slice())
                    .execute(&mut *tx)
                    .await?;
                }
                None => skipped_fog += 1,
            }
        }

        for row in &data.settings {
            sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?)")
                .bind(&row.key)
                .bind(&row.value)
                .execute(&mut *tx)
                .await?;
        }

        // Finalize staged asset files: rename each staged temp file (already
        // living in the target world's asset directory, per
        // `world_bundle::read_bundle`) to its final `<id>` name in that same
        // directory, only after every row above has been accepted by the
        // transaction. A failure here still rolls the WHOLE transaction back
        // (the early `?` return drops `tx` unrolled-back), and best-effort
        // removes every staged/finalized file so a rolled-back import leaves
        // no orphan bytes behind.
        // Canonicals and siblings finalize through one list: each staged
        // file renames to `<id><suffix>` ("" for the canonical) in place.
        let moves: Vec<(String, &std::path::PathBuf)> = data
            .staged_assets
            .iter()
            .map(|(id, staged)| (id.to_string(), staged))
            .chain(
                data.staged_siblings
                    .iter()
                    .map(|s| (format!("{}{}", s.asset_id, s.suffix), &s.staged)),
            )
            .collect();
        let mut finalized: Vec<std::path::PathBuf> = Vec::with_capacity(moves.len());
        for (name, staged) in &moves {
            let dest = staged
                .parent()
                .expect("staged asset path always has a parent directory")
                .join(name);
            if let Err(e) = tokio::fs::rename(staged, &dest).await {
                for done in &finalized {
                    let _ = tokio::fs::remove_file(done).await;
                }
                for (_, remaining) in &moves {
                    let _ = tokio::fs::remove_file(remaining).await;
                }
                return Err(DataError::OpFailed(format!(
                    "failed to finalize imported asset file {name}: {e}"
                )));
            }
            finalized.push(dest);
        }

        // Accepted low-likelihood risk: if every asset above renamed
        // successfully but this commit itself then fails, the renamed files
        // remain on disk with no `assets` row (and no `worlds` row at all)
        // referencing them — an orphan, not a visible/reachable partial
        // world. A SQLite commit failure this late (all statements already
        // succeeded) is rare; no compensating cleanup is implemented for it.
        tx.commit().await?;

        Ok(ImportSummary {
            world_id: world,
            skipped_members,
            skipped_fog,
        })
    }

    /// Upsert the player's explored-cell blob for a scene. Keyed `(scene_id, user_id)`; `world_id`
    /// is denormalized for the world-scoped purge (rows are purged by `delete_world` (world-scoped),
    /// `delete_user` (user-scoped), and `delete_document_tx` (scene-scoped)). Write is whole-blob
    /// last-writer-wins: two of the
    /// user's sockets accumulating concurrently can transiently drop a cell one added but the other
    /// didn't observe. Self-healing: explored is a re-derivable dimmed-memory layer (a dropped cell
    /// re-marks the next time vision covers it) and the live `visible` mask is always exact, so a
    /// transient loss never reveals more than it should — only delays a memory cell.
    pub async fn set_explored(
        &self,
        world: Uuid,
        scene: Uuid,
        user: Uuid,
        cells: &[u8],
    ) -> Result<(), DataError> {
        sqlx::query(
            "INSERT INTO explored_fog (world_id, scene_id, user_id, cells) VALUES (?, ?, ?, ?) \
             ON CONFLICT(scene_id, user_id) DO UPDATE SET cells = excluded.cells, \
             world_id = excluded.world_id",
        )
        .bind(world.to_string())
        .bind(scene.to_string())
        .bind(user.to_string())
        .bind(cells)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Look up an account by exact username, or `None`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// assert!(repo.user_by_username("no-such-user").await?.is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn user_by_username(&self, username: &str) -> Result<Option<UserRecord>, DataError> {
        let row = sqlx::query(
            "SELECT id, username, password_hash, server_role FROM users WHERE username = ?",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        Ok(match row {
            Some(r) => {
                let role_str: String = r.get("server_role");
                let server_role = match role_str.as_str() {
                    "admin" => ServerRole::Admin,
                    _ => ServerRole::User,
                };
                Some(UserRecord {
                    id: Uuid::parse_str(r.get::<String, _>("id").as_str())
                        .map_err(|e| DataError::OpFailed(e.to_string()))?,
                    username: r.get("username"),
                    password_hash: r.get("password_hash"),
                    server_role,
                })
            }
            None => None,
        })
    }

    /// Whether a user row with this id exists. Used to reject a membership
    /// write against an unknown user id with a client-actionable 404 instead of
    /// letting the `world_members.user_id` foreign key surface as a 500.
    pub async fn user_exists(&self, id: Uuid) -> Result<bool, DataError> {
        let row = sqlx::query("SELECT 1 FROM users WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    /// Insert a user only if no existing username matches case-insensitively,
    /// in a single guarded statement. Returns the new id, or `None` on a
    /// collision. Single-statement like `create_admin_if_none`, so the
    /// check-then-create race cannot split across two queries.
    ///
    /// INVARIANT: usernames reaching this method are ASCII-restricted at the
    /// HTTP boundary (`validate_username`), so SQLite's ASCII-only `NOCASE`
    /// collation is a complete case-fold — `Alice` and `alice` cannot coexist
    /// and impersonate one another in a member roster.
    pub async fn create_user_unique(
        &self,
        username: &str,
        password_hash: &str,
        role: ServerRole,
        now: i64,
    ) -> Result<Option<Uuid>, DataError> {
        let id = Uuid::new_v4();
        let res = sqlx::query(
            "INSERT INTO users (id, username, password_hash, server_role, created_at) \
             SELECT ?, ?, ?, ?, ? \
             WHERE NOT EXISTS (SELECT 1 FROM users WHERE username = ? COLLATE NOCASE)",
        )
        .bind(id.to_string())
        .bind(username)
        .bind(password_hash)
        .bind(role.as_str())
        .bind(now)
        .bind(username)
        .execute(&self.pool)
        .await?;
        Ok((res.rows_affected() == 1).then_some(id))
    }

    /// Every account, for the admin user-management surface. Deliberately
    /// projects only the three non-secret columns — the password hash is never
    /// selected, so it cannot reach a response body by accident.
    pub async fn list_users(&self) -> Result<Vec<(Uuid, String, ServerRole)>, DataError> {
        let rows = sqlx::query(
            "SELECT id, username, server_role FROM users ORDER BY username COLLATE NOCASE",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                let id = Uuid::parse_str(r.get::<String, _>("id").as_str())
                    .map_err(|e| DataError::OpFailed(e.to_string()))?;
                let role = match r.get::<String, _>("server_role").as_str() {
                    "admin" => ServerRole::Admin,
                    _ => ServerRole::User,
                };
                Ok((id, r.get("username"), role))
            })
            .collect()
    }

    /// Whether any server-admin account exists (gates the first-run setup window).
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// assert!(!repo.admin_exists().await?); // a fresh database has no admin
    /// # Ok(())
    /// # }
    /// ```
    pub async fn admin_exists(&self) -> Result<bool, DataError> {
        let row = sqlx::query("SELECT 1 FROM users WHERE server_role = 'admin' LIMIT 1")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    /// Insert an admin only if no admin exists yet AND the username is free
    /// case-insensitively, in a single guarded statement. Returns the new id,
    /// or `None` when either guard rejects. The single-writer pool serializes
    /// the insert, closing the first-run check-then-create race (two concurrent
    /// setups cannot both succeed).
    ///
    /// The `NOCASE` half mirrors `create_user_unique`: without it an admin
    /// named `Alice` could coexist with a user named `alice` and be
    /// indistinguishable from them in a roster — the impersonation the ASCII
    /// username policy exists to prevent. Reachable since `DELETE
    /// /api/users/{id}` exists: deletion is last-admin-guarded, so "users
    /// exist but no admin" still cannot arise — the NOCASE guard below stays
    /// as the structural backstop.
    pub async fn create_admin_if_none(
        &self,
        username: &str,
        password_hash: &str,
        now: i64,
    ) -> Result<Option<Uuid>, DataError> {
        let id = Uuid::new_v4();
        let res = sqlx::query(
            "INSERT INTO users (id, username, password_hash, server_role, created_at) \
             SELECT ?, ?, ?, 'admin', ? \
             WHERE NOT EXISTS (SELECT 1 FROM users WHERE server_role = 'admin') \
             AND NOT EXISTS (SELECT 1 FROM users WHERE username = ? COLLATE NOCASE)",
        )
        .bind(id.to_string())
        .bind(username)
        .bind(password_hash)
        .bind(now)
        .bind(username)
        .execute(&self.pool)
        .await?;
        Ok((res.rows_affected() == 1).then_some(id))
    }

    /// Read one key from the server-global `settings` table (e.g. the persisted
    /// session key), or `None`.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// repo.set_setting("mock_key", "mock_value").await?;
    /// assert_eq!(repo.get_setting("mock_key").await?.as_deref(), Some("mock_value"));
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_setting(&self, key: &str) -> Result<Option<String>, DataError> {
        let row = sqlx::query("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get("value")))
    }

    /// Upsert one key in the server-global `settings` table.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// repo.set_setting("mock_key", "v2").await?; // second write overwrites
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_setting(&self, key: &str, value: &str) -> Result<(), DataError> {
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES (?, ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Set a world's capability configuration (per-document defaults + world-level
    /// role_caps). Stored as JSON in the settings table.
    pub async fn set_world_cap_defaults(
        &self,
        world: Uuid,
        defaults: &WorldCapDefaults,
    ) -> Result<(), DataError> {
        let json = serde_json::to_string(defaults)?;
        self.set_setting(&world_caps_key(world), &json).await
    }

    /// Replace a world's declarative capability requirements (stored as JSON).
    pub async fn set_world_cap_requirements(
        &self,
        world: Uuid,
        reqs: &[CapabilityRequirement],
    ) -> Result<(), DataError> {
        let json = serde_json::to_string(reqs)?;
        self.set_setting(&world_caps_req_key(world), &json).await
    }

    /// Replace a world's UI contract declarations (stored as JSON in settings).
    pub async fn set_world_contract_declarations(
        &self,
        world: Uuid,
        decls: &[ContractDeclaration],
    ) -> Result<(), DataError> {
        let json = serde_json::to_string(decls)?;
        self.set_setting(&world_contracts_key(world), &json).await
    }

    /// Replace a world's structural schema declarations (stored as JSON in
    /// settings, beside cap requirements / contract declarations).
    pub async fn set_world_schema_declarations(
        &self,
        world: Uuid,
        decls: &[SchemaDeclaration],
    ) -> Result<(), DataError> {
        let json = serde_json::to_string(decls)?;
        self.set_setting(&world_schemas_key(world), &json).await
    }

    /// Replace a world's enabled installed-module set (stored as JSON in
    /// settings, beside `world_cap_requirements`/`world_contract_declarations`
    /// — enable/disable never mutates either of those; `welcome_capability_requirements` unions
    /// the enabled modules' declared requirements with the stored GM-authored record fresh on
    /// every `Welcome`, leaving the stored record the GM's own edit alone).
    pub async fn set_world_enabled_modules(
        &self,
        world: Uuid,
        ids: &[String],
    ) -> Result<(), DataError> {
        let json = serde_json::to_string(ids)?;
        self.set_setting(&world_modules_key(world), &json).await
    }

    /// Seat `user_id` in `world_id` with `role` (upsert; idempotent for an
    /// existing member with the same role).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// use shadowcat::data::document::WorldRole;
    /// # let (world_id, user_id) = (uuid::Uuid::nil(), uuid::Uuid::nil());
    /// repo.add_member(world_id, user_id, WorldRole::Player).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn add_member(
        &self,
        world_id: Uuid,
        user_id: Uuid,
        role: WorldRole,
    ) -> Result<(), DataError> {
        sqlx::query("INSERT INTO world_members (world_id, user_id, role) VALUES (?, ?, ?)")
            .bind(world_id.to_string())
            .bind(user_id.to_string())
            .bind(serde_json::to_value(role)?.as_str().unwrap().to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Add a member or change an existing member's role — resolve, guard, and
    /// write in ONE transaction (a standalone user_exists → member_role →
    /// set_role/add_member sequence is a TOCTOU: a user deleted between the
    /// check and the insert resurfaces the FK 500 the 404 contract exists to
    /// prevent). The guarded INSERT..SELECT proves user AND world existence
    /// atomically with the upsert: rows_affected == 0 ⇔ target user or world
    /// missing → NotFound. The sole-GM demotion guard runs on the same tx.
    pub async fn upsert_member(
        &self,
        world: Uuid,
        user: Uuid,
        role: WorldRole,
    ) -> Result<(), DataError> {
        let mut tx = self.pool.begin().await?;
        if role != WorldRole::Gm && Self::is_last_gm(&mut tx, world, user).await? {
            return Err(DataError::Conflict(
                "cannot demote the world's only GM".into(),
            ));
        }
        let role_s = serde_json::to_value(role)?.as_str().unwrap().to_string();
        let res = sqlx::query(
            "INSERT INTO world_members (world_id, user_id, role) \
             SELECT ?, ?, ? \
             WHERE EXISTS (SELECT 1 FROM users WHERE id = ?) \
               AND EXISTS (SELECT 1 FROM worlds WHERE id = ?) \
             ON CONFLICT(world_id, user_id) DO UPDATE SET role = excluded.role",
        )
        .bind(world.to_string())
        .bind(user.to_string())
        .bind(role_s)
        .bind(user.to_string())
        .bind(world.to_string())
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            return Err(DataError::NotFound);
        }
        tx.commit().await?;
        Ok(())
    }

    /// The user's role in the world, or `None` when not a member.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let role = repo.member_role(uuid::Uuid::nil(), uuid::Uuid::nil()).await?;
    /// assert!(role.is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn member_role(
        &self,
        world_id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<WorldRole>, DataError> {
        let row = sqlx::query("SELECT role FROM world_members WHERE world_id = ? AND user_id = ?")
            .bind(world_id.to_string())
            .bind(user_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => {
                let role: String = r.get("role");
                Ok(Some(serde_json::from_value(serde_json::Value::String(
                    role,
                ))?))
            }
            None => Ok(None),
        }
    }

    /// The UUID of a member of `world` whose username matches exactly, or
    /// `None`. Mirrors `list_members`' join, scoped to one username.
    pub async fn member_id_by_username(
        &self,
        world: Uuid,
        username: &str,
    ) -> Result<Option<Uuid>, DataError> {
        let row = sqlx::query(
            "SELECT m.user_id FROM world_members m JOIN users u ON u.id = m.user_id \
             WHERE m.world_id = ? AND u.username = ?",
        )
        .bind(world.to_string())
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| {
            Uuid::parse_str(r.get::<String, _>("user_id").as_str())
                .map_err(|e| DataError::OpFailed(e.to_string()))
        })
        .transpose()
    }

    // --- World invites ---

    /// Insert an invite for `world`, bounded by `max_active` live invites
    /// (unconsumed, unrevoked, unexpired). Returns whether it was stored;
    /// `false` means the world is at the cap. Count and insert share one
    /// transaction: on two connections the pair would be a TOCTOU that lets the
    /// cap be exceeded.
    ///
    /// `NewInvite::id` is the selector half of the caller's minted code — the
    /// row id and the code MUST agree, so it is supplied rather than generated
    /// here.
    pub async fn create_invite(
        &self,
        invite: NewInvite<'_>,
        max_active: i64,
    ) -> Result<bool, DataError> {
        let NewInvite {
            id,
            world,
            secret_hash,
            role,
            created_by,
            now,
            expires_at,
        } = invite;
        let mut tx = self.pool.begin().await?;
        let active: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM world_invites WHERE world_id = ? \
             AND consumed_at IS NULL AND revoked_at IS NULL AND expires_at > ?",
        )
        .bind(world.to_string())
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        if active >= max_active {
            return Ok(false);
        }
        sqlx::query(
            "INSERT INTO world_invites \
             (id, world_id, secret_hash, role, created_by, created_at, expires_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(world.to_string())
        .bind(secret_hash)
        .bind(serde_json::to_value(role)?.as_str().unwrap().to_string())
        .bind(created_by.to_string())
        .bind(now)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// An invite row by id, in ANY lifecycle state. Redemption reads this only
    /// to obtain the stored hash — expiry/revocation/single-use are decided by
    /// `consume_invite`, so that every unusable code reaches the caller through
    /// one indistinguishable path.
    pub async fn invite_by_id(&self, id: Uuid) -> Result<Option<InviteRecord>, DataError> {
        let row = sqlx::query(
            "SELECT id, world_id, secret_hash, role, created_at, expires_at, \
             revoked_at, consumed_at FROM world_invites WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(Self::invite_row).transpose()
    }

    /// A world's invites, newest first. Never selects `secret_hash`: the GM
    /// listing must not be able to leak credential material.
    pub async fn list_invites(&self, world: Uuid) -> Result<Vec<InviteRecord>, DataError> {
        let rows = sqlx::query(
            "SELECT id, world_id, '' AS secret_hash, role, created_at, expires_at, \
             revoked_at, consumed_at FROM world_invites WHERE world_id = ? \
             ORDER BY created_at DESC, id",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Self::invite_row).collect()
    }

    /// Revoke an invite, scoped to `world`. Returns whether a row changed —
    /// `false` covers both "no such invite" and "belongs to another world", so
    /// a GM cannot use this route to probe another world's invite ids.
    pub async fn revoke_invite(&self, world: Uuid, id: Uuid, now: i64) -> Result<bool, DataError> {
        let res = sqlx::query(
            "UPDATE world_invites SET revoked_at = ? \
             WHERE id = ? AND world_id = ? AND revoked_at IS NULL AND consumed_at IS NULL",
        )
        .bind(now)
        .bind(id.to_string())
        .bind(world.to_string())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Redeem an invite for `user`: mark it consumed and seat them. Returns the
    /// world and the user's resulting membership role, or `None` when the
    /// invite is unknown, expired, revoked, or already consumed.
    ///
    /// The consume is ONE guarded `UPDATE ... RETURNING`: the lifecycle
    /// predicates and the write are the same statement, so two concurrent
    /// redemptions of one code cannot both observe it as available and
    /// double-seat (a check-then-act pair could — [[two-query-guard-needs-tx]]).
    /// The seating shares the transaction, so a burned invite always
    /// corresponds to a seated member.
    ///
    /// An existing membership is left ALONE (`INSERT OR IGNORE`): redeeming an
    /// invite may only grant access, never change a role the caller already
    /// holds, so a `spectator` invite cannot be used to demote a world's GM.
    pub async fn consume_invite(
        &self,
        id: Uuid,
        user: Uuid,
        now: i64,
    ) -> Result<Option<SeatedByInvite>, DataError> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "UPDATE world_invites SET consumed_at = ?, consumed_by = ? \
             WHERE id = ? AND consumed_at IS NULL AND revoked_at IS NULL AND expires_at > ? \
             RETURNING world_id, role",
        )
        .bind(now)
        .bind(user.to_string())
        .bind(id.to_string())
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let world = Uuid::parse_str(row.get::<String, _>("world_id").as_str())
            .map_err(|e| DataError::OpFailed(e.to_string()))?;
        let invited_role: WorldRole =
            serde_json::from_value(serde_json::Value::String(row.get::<String, _>("role")))?;
        sqlx::query(
            "INSERT OR IGNORE INTO world_members (world_id, user_id, role) VALUES (?, ?, ?)",
        )
        .bind(world.to_string())
        .bind(user.to_string())
        .bind(
            serde_json::to_value(invited_role)?
                .as_str()
                .unwrap()
                .to_string(),
        )
        .execute(&mut *tx)
        .await?;
        let seated: String =
            sqlx::query_scalar("SELECT role FROM world_members WHERE world_id = ? AND user_id = ?")
                .bind(world.to_string())
                .bind(user.to_string())
                .fetch_one(&mut *tx)
                .await?;
        // Read the world's name here, inside the transaction: a lookup after
        // the commit could miss and make a redemption that already burned the
        // invite report as a failure.
        let world_name: String = sqlx::query_scalar("SELECT name FROM worlds WHERE id = ?")
            .bind(world.to_string())
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(SeatedByInvite {
            world,
            world_name,
            role: serde_json::from_value(serde_json::Value::String(seated))?,
        }))
    }

    /// Map an `invites` row to `InviteRecord`.
    ///
    /// # Examples
    ///
    /// ```text
    /// let invite = Self::invite_row(row)?;
    /// ```
    fn invite_row(r: sqlx::sqlite::SqliteRow) -> Result<InviteRecord, DataError> {
        Ok(InviteRecord {
            id: Uuid::parse_str(r.get::<String, _>("id").as_str())
                .map_err(|e| DataError::OpFailed(e.to_string()))?,
            world_id: Uuid::parse_str(r.get::<String, _>("world_id").as_str())
                .map_err(|e| DataError::OpFailed(e.to_string()))?,
            secret_hash: r.get("secret_hash"),
            role: serde_json::from_value(serde_json::Value::String(r.get::<String, _>("role")))?,
            created_at: r.get("created_at"),
            expires_at: r.get("expires_at"),
            revoked_at: r.get("revoked_at"),
            consumed_at: r.get("consumed_at"),
        })
    }

    /// Load a document envelope by id on an arbitrary executor (so it can run
    /// inside a transaction). Mirrors `get_document`'s row→Document mapping.
    async fn load_document<'e, E>(executor: E, id: Uuid) -> Result<Option<Document>, DataError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let row = sqlx::query("SELECT json FROM documents WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(executor)
            .await?;
        match row {
            Some(r) => Ok(Some(serde_json::from_str(
                r.get::<String, _>("json").as_str(),
            )?)),
            None => Ok(None),
        }
    }

    /// Resolve `doc`'s effective owner (`permission::effective_owner`) on an
    /// arbitrary executor, joining the LINKED actor for a token so the rule is
    /// evaluated against LIVE actor state on every write — nothing is stamped,
    /// so re-assigning an actor's owner immediately re-owns its linked tokens.
    /// Runs on the caller's transaction (never `&self.pool`, which would
    /// deadlock mid-transaction on the single-writer pool).
    ///
    /// Costs ONE extra row read, and only for a token carrying an `actor_id`
    /// link. This function performs the JOIN and nothing else: precedence
    /// between the override and the inherited owner is decided EXCLUSIVELY by
    /// `effective_owner`. Deliberately does NOT skip the read when `doc.owner`
    /// is set — re-deriving "the override wins" here would duplicate the rule in
    /// a second place that can silently drift from it.
    async fn load_effective_owner<'e, E>(
        executor: E,
        doc: &Document,
    ) -> Result<Option<Uuid>, DataError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let Some(actor_id) = crate::data::permission::token_actor_link(doc) else {
            return Ok(crate::data::permission::effective_owner(doc, None));
        };
        // A dangling link loads `None` and `effective_owner` fails closed to no owner.
        //
        // `load_document` is keyed on id alone (no `world_id` filter), so a cross-world
        // `actor_id` would otherwise resolve. The cross-world scope check lives inside
        // `permission::effective_owner` itself — see that function's doc comment for the
        // rationale (keeps the reachable set equal to `SceneEcs.actors` by construction).
        let actor = Self::load_document(executor, actor_id).await?;
        Ok(crate::data::permission::effective_owner(
            doc,
            actor.as_ref(),
        ))
    }

    /// `documents.created_seq` for `id`, or `None` if the row doesn't exist. Set once at a
    /// row's genuine first INSERT (`upsert_document`'s `ON CONFLICT` clause omits it, so
    /// SQLite's `excluded.*` semantics leave it untouched across an update) and never touched
    /// again by subsequent updates to a still-live row — the generation marker
    /// `OpSnapshot::created_seq_at_commit` compares against to detect an id reused after a hard
    /// delete. Runs on the caller's transaction (never `&self.pool`, which would deadlock
    /// mid-transaction on the single-writer pool).
    async fn document_created_seq<'e, E>(executor: E, id: Uuid) -> Result<Option<i64>, DataError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let row = sqlx::query("SELECT created_seq FROM documents WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(executor)
            .await?;
        Ok(row.map(|r| r.get::<i64, _>("created_seq")))
    }

    /// Every CURRENT member's world role, on an arbitrary executor (so it can run inside the
    /// `apply_command`/`apply_intent` transaction). Feeds `CommandSnapshot::world_gm_at_commit`
    /// — captured once per command, at the point the command is committing, which IS "at commit
    /// time" for this purpose: the whole point of capturing it now is to freeze what would
    /// otherwise be re-derived live on every future replay.
    async fn world_member_roles<'e, E>(
        executor: E,
        world_id: Uuid,
    ) -> Result<std::collections::HashMap<Uuid, WorldRole>, DataError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let rows = sqlx::query("SELECT user_id, role FROM world_members WHERE world_id = ?")
            .bind(world_id.to_string())
            .fetch_all(executor)
            .await?;
        rows.into_iter()
            .map(|r| {
                let uid = Uuid::parse_str(r.get::<String, _>("user_id").as_str())
                    .map_err(|e| DataError::OpFailed(e.to_string()))?;
                let role: WorldRole =
                    serde_json::from_value(serde_json::Value::String(r.get::<String, _>("role")))?;
                Ok((uid, role))
            })
            .collect()
    }

    /// Build one op's commit-time redaction snapshot from the command's FINAL post-image state
    /// (`post_images`, accumulated across the WHOLE mutation loop) and, for a `Delete`, its
    /// created_seq captured BEFORE the row was removed (`deleted_created_seqs` — the row is gone
    /// by the time this runs, so it cannot be read here). Runs on the caller's open transaction,
    /// after every op in the command has applied and every write has landed. Shared by
    /// `apply_command` and `apply_intent` — the ONE place either loop computes a snapshot, so
    /// they cannot diverge.
    async fn build_op_snapshot(
        tx: &mut sqlx::SqliteConnection,
        op: &Operation,
        post_images: &std::collections::HashMap<Uuid, Document>,
        deleted_created_seqs: &std::collections::HashMap<Uuid, i64>,
        pre_permissions: &std::collections::HashMap<Uuid, crate::data::document::PermissionSet>,
        pre_owners: &std::collections::HashMap<Uuid, Option<Uuid>>,
    ) -> Result<crate::data::snapshot::OpSnapshot, DataError> {
        use crate::data::snapshot::OpSnapshot;
        match op {
            Operation::Create { doc } => {
                // Reads the command's FINAL post-image, never `doc` (this op's own
                // per-iteration intermediate state): a same-command op that later mutates
                // this same id (e.g. an Update reassigning `/owner` or adding an override)
                // must be reflected in the Create's own persisted snapshot, or a stale value
                // is stored forever in `world_events.command_json`. Guaranteed `Some` — both
                // `apply_command` and `apply_intent` unconditionally insert into
                // `post_images` immediately after a Create's own document write.
                let doc = post_images.get(&doc.id).ok_or_else(|| {
                    DataError::OpFailed(format!(
                        "post-image missing for created document {}",
                        doc.id
                    ))
                })?;
                let owner_at_commit = Self::load_effective_owner(&mut *tx, doc).await?;
                let mut overrides_at_commit = Vec::new();
                crate::data::permission::collect_overrides(doc, "", &mut overrides_at_commit)
                    .map_err(|e| DataError::OpFailed(e.to_string()))?;
                Ok(OpSnapshot {
                    owner_at_commit,
                    doc_type: doc.doc_type.clone(),
                    overrides_at_commit,
                    retraction_hidden_at_commit: None,
                    created_seq_at_commit: None,
                    permissions_at_commit: None,
                    permissions_before_commit: None,
                    owner_before_commit: None,
                })
            }
            // Reads the op's OWN carried `doc`, not `post_images` — unlike Create/Update,
            // there is no coherent "final state" for an id deleted within this same
            // command: `post_images` holds no entry for it (the mutation loop never
            // inserts one on Delete), and a later op resurrecting the same id via a fresh
            // Create is a distinct document, not a continuation of this one.
            Operation::Delete { doc } => {
                let owner_at_commit = Self::load_effective_owner(&mut *tx, doc).await?;
                let mut overrides_at_commit = Vec::new();
                crate::data::permission::collect_overrides(doc, "", &mut overrides_at_commit)
                    .map_err(|e| DataError::OpFailed(e.to_string()))?;
                Ok(OpSnapshot {
                    owner_at_commit,
                    doc_type: doc.doc_type.clone(),
                    overrides_at_commit,
                    retraction_hidden_at_commit: None,
                    created_seq_at_commit: deleted_created_seqs.get(&doc.id).copied(),
                    permissions_at_commit: None,
                    permissions_before_commit: None,
                    owner_before_commit: None,
                })
            }
            Operation::Update { doc_id, changes } => {
                let doc = post_images.get(doc_id).ok_or_else(|| {
                    DataError::OpFailed(format!("post-image missing for updated document {doc_id}"))
                })?;
                let owner_at_commit = Self::load_effective_owner(&mut *tx, doc).await?;
                let mut overrides_full = Vec::new();
                crate::data::permission::collect_overrides(doc, "", &mut overrides_full)
                    .map_err(|e| DataError::OpFailed(e.to_string()))?;
                let touches_perms = changes
                    .iter()
                    .any(|c| crate::data::permission::touches_permissions(&c.path));
                let retraction_hidden_at_commit = if touches_perms {
                    Some(overrides_full.clone())
                } else {
                    None
                };
                // Pruned to the ancestor/descendant closure of this op's own changed paths —
                // only an overlapping override can possibly redact THIS op's field-level deltas.
                let overrides_at_commit: Vec<(String, crate::data::document::Visibility)> =
                    overrides_full
                        .into_iter()
                        .filter(|(p, _)| {
                            changes
                                .iter()
                                .any(|c| crate::data::permission::paths_overlap(p, &c.path))
                        })
                        .collect();
                let created_seq_at_commit = Self::document_created_seq(&mut *tx, *doc_id).await?;
                Ok(OpSnapshot {
                    owner_at_commit,
                    doc_type: doc.doc_type.clone(),
                    overrides_at_commit,
                    retraction_hidden_at_commit,
                    created_seq_at_commit,
                    permissions_at_commit: Some(crate::data::document::PermissionSet {
                        property_overrides: Default::default(),
                        ..doc.permissions.clone()
                    }),
                    permissions_before_commit: pre_permissions.get(doc_id).map(|p| {
                        crate::data::document::PermissionSet {
                            property_overrides: Default::default(),
                            ..p.clone()
                        }
                    }),
                    owner_before_commit: pre_owners.get(doc_id).copied().flatten(),
                })
            }
            // Mirrors the Update arm's post-image sourcing without its
            // change-delta pieces: a Move carries no `FieldChange`s, so the
            // path-overlap pruning yields an empty override set and no
            // retraction capture (a Move never touches `permissions`).
            Operation::Move { doc_id, .. } => {
                let doc = post_images.get(doc_id).ok_or_else(|| {
                    DataError::OpFailed(format!("post-image missing for moved document {doc_id}"))
                })?;
                let owner_at_commit = Self::load_effective_owner(&mut *tx, doc).await?;
                let created_seq_at_commit = Self::document_created_seq(&mut *tx, *doc_id).await?;
                Ok(OpSnapshot {
                    owner_at_commit,
                    doc_type: doc.doc_type.clone(),
                    overrides_at_commit: Vec::new(),
                    retraction_hidden_at_commit: None,
                    created_seq_at_commit,
                    permissions_at_commit: Some(crate::data::document::PermissionSet {
                        property_overrides: Default::default(),
                        ..doc.permissions.clone()
                    }),
                    permissions_before_commit: pre_permissions.get(doc_id).map(|p| {
                        crate::data::document::PermissionSet {
                            property_overrides: Default::default(),
                            ..p.clone()
                        }
                    }),
                    owner_before_commit: pre_owners.get(doc_id).copied().flatten(),
                })
            }
        }
    }

    /// Parent-placement checks the Create AND Move arms share — the one
    /// statement of "may a document of this type sit under this parent",
    /// covering the checks that need the database or the batch bookkeeping:
    /// a stored parent must belong to this command's world
    /// (`check_command_scope`), a `combatant`/`combat-history` parent must be
    /// a combat (batch-aware), and an `asset_folder` parent must be a
    /// same-scope folder (`check_asset_folder_parent`, batch-aware). A parent
    /// this same batch Creates is not in the database yet — it resolves
    /// through the batch maps, and its own Create was scope-checked; a parent
    /// that exists nowhere yet is left to the self-FK at apply time, so
    /// batched parent+child creates still pass. `validate_containment` (pure
    /// placement shape) runs separately at every caller.
    async fn check_parent_placement(
        tx: &mut sqlx::SqliteConnection,
        world_id: Uuid,
        doc: &Document,
        batch_folders: &std::collections::HashMap<Uuid, Document>,
        batch_combats: &std::collections::HashSet<Uuid>,
    ) -> Result<(), DataError> {
        if doc.doc_type == COMBATANT_DOC_TYPE || doc.doc_type == COMBAT_HISTORY_DOC_TYPE {
            // `validate_containment` already guarantees `parent_id` is
            // `Some` for a combatant/combat-history document.
            let pid = doc.parent_id.expect(
                "validate_containment requires a combatant/combat-history doc to carry a parent_id",
            );
            let stored_parent = if batch_combats.contains(&pid) {
                None
            } else {
                Self::load_document(&mut *tx, pid).await?
            };
            if let Some(parent) = &stored_parent {
                check_command_scope(parent, world_id)?;
            }
            let parent_is_combat = batch_combats.contains(&pid)
                || stored_parent.is_some_and(|p| p.doc_type == COMBAT_DOC_TYPE);
            if !parent_is_combat {
                return Err(DataError::OpFailed(format!(
                    "{} parent must be a combat document",
                    doc.doc_type
                )));
            }
        } else if let Some(pid) = doc.parent_id {
            // Every other doc_type: no parent-TYPE rule, but a stored parent
            // still belongs to this command's world.
            if !batch_folders.contains_key(&pid) && !batch_combats.contains(&pid) {
                if let Some(parent) = Self::load_document(&mut *tx, pid).await? {
                    check_command_scope(&parent, world_id)?;
                }
            }
        }
        Self::check_asset_folder_parent(&mut *tx, doc, batch_folders).await?;
        Ok(())
    }

    /// Rejects a Move that would parent `moved` beneath itself: walks the
    /// ancestor chain upward from `new_parent`, resolving each hop against
    /// this batch's not-yet-applied Moves first (`batch_moves` — the
    /// prospective parent wins over the stored one, since the walk must see
    /// the tree the batch will leave, and Phase 2 applies nothing until
    /// every op has validated), then this batch's not-yet-inserted Creates
    /// (`batch_folders`), then the stored tree — refusing if `moved` appears
    /// anywhere in the chain (self-parent included).
    /// Bounded: a chain deeper than `MAX_MOVE_ANCESTRY` (or a stored cycle,
    /// which cannot arise but would otherwise loop) is refused, not walked.
    async fn check_move_acyclic(
        tx: &mut sqlx::SqliteConnection,
        moved: Uuid,
        new_parent: Option<Uuid>,
        batch_folders: &std::collections::HashMap<Uuid, Document>,
        batch_moves: &std::collections::HashMap<Uuid, Option<Uuid>>,
    ) -> Result<(), DataError> {
        /// Depth bound for the ancestor walk; no legitimate tree approaches it.
        const MAX_MOVE_ANCESTRY: u32 = 1_000;
        let mut cursor = new_parent;
        let mut hops = 0u32;
        while let Some(pid) = cursor {
            if pid == moved {
                return Err(DataError::OpFailed(
                    "a document cannot be moved beneath itself".into(),
                ));
            }
            hops += 1;
            if hops > MAX_MOVE_ANCESTRY {
                return Err(DataError::OpFailed(
                    "parent chain too deep to verify".into(),
                ));
            }
            cursor = if let Some(prospective) = batch_moves.get(&pid) {
                *prospective
            } else {
                match batch_folders.get(&pid) {
                    Some(batch_doc) => batch_doc.parent_id,
                    None => Self::load_document(&mut *tx, pid)
                        .await?
                        .and_then(|d| d.parent_id),
                }
            };
        }
        Ok(())
    }

    /// Whether a document of `doc_type` already exists in `world_id`, on an
    /// arbitrary executor (so it can run inside the `apply_intent`
    /// transaction — see `SINGLETON_DOC_TYPES`). Mirrors `load_document`'s
    /// tx-generic pattern rather than `query_documents`, which always binds
    /// to `&self.pool` and would deadlock if called mid-transaction against
    /// this single-writer (`max_connections(1)`) pool.
    async fn singleton_doc_exists<'e, E>(
        executor: E,
        world_id: Uuid,
        doc_type: &str,
    ) -> Result<bool, DataError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let row =
            sqlx::query("SELECT 1 FROM documents WHERE world_id = ? AND doc_type = ? LIMIT 1")
                .bind(world_id.to_string())
                .bind(doc_type)
                .fetch_optional(executor)
                .await?;
        Ok(row.is_some())
    }

    /// The id of the `combat` document currently `active: true` on `scene_id`
    /// in `world_id`, or `None` if the scene has no active combat. Runs on
    /// the caller's transaction for the same single-writer reason as
    /// `singleton_doc_exists`. At most one row can ever match, since this is
    /// the same one-active-combat-per-scene invariant `apply_intent`'s
    /// `scene_owner` map enforces.
    async fn active_combat_owner<'e, E>(
        executor: E,
        world_id: Uuid,
        scene_id: Uuid,
    ) -> Result<Option<Uuid>, DataError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let row = sqlx::query(
            "SELECT id FROM documents WHERE world_id = ? AND doc_type = ? \
             AND json_extract(json, '$.engine.active') = 1 \
             AND json_extract(json, '$.engine.scene_id') = ? LIMIT 1",
        )
        .bind(world_id.to_string())
        .bind(COMBAT_DOC_TYPE)
        .bind(scene_id.to_string())
        .fetch_optional(executor)
        .await?;
        row.map(|r| {
            Uuid::parse_str(r.get::<String, _>("id").as_str())
                .map_err(|e| DataError::OpFailed(e.to_string()))
        })
        .transpose()
    }

    /// Lazily seeds `scene_owner[scene]` with the DB's current active-combat
    /// owner the first time this batch's simulation touches `scene`'s
    /// active-combat state (a no-op on every later touch, tracked by
    /// `seeded_scenes`). When `deactivations_this_batch` already names
    /// `scene` -- a genuine same-batch active-true-to-false transition found
    /// by `apply_intent`'s pre-scan -- `scene` is deliberately left
    /// unseeded (absent from `scene_owner`) rather than seeded from the
    /// stale DB row: the batch is itself about to free this scene, so an
    /// EARLIER same-batch claim on it must be validated against the state
    /// the batch will actually leave, not a DB read a LATER op in the same
    /// batch is about to invalidate.
    async fn ensure_scene_owner_seeded<'e, E>(
        executor: E,
        world_id: Uuid,
        scene: Uuid,
        scene_owner: &mut std::collections::HashMap<Uuid, Uuid>,
        seeded_scenes: &mut std::collections::HashSet<Uuid>,
        deactivations_this_batch: &std::collections::HashSet<Uuid>,
    ) -> Result<(), DataError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        if !seeded_scenes.insert(scene) {
            return Ok(());
        }
        if deactivations_this_batch.contains(&scene) {
            return Ok(());
        }
        if let Some(owner) = Self::active_combat_owner(executor, world_id, scene).await? {
            scene_owner.insert(scene, owner);
        }
        Ok(())
    }

    /// Depth-first descendant ids of `root` within one transaction (children
    /// before parents), via the `parent_id` index. Excludes `root`. Used to
    /// expand a parent delete into per-descendant reversible Delete ops.
    ///
    /// The `seen` set (seeded with `root`) makes the walk terminate on any
    /// self-reference or cycle: a single `INSERT` whose `parent_id` equals its
    /// own `id` satisfies the self-FK and commits, so without this guard a
    /// `WHERE parent_id = root` row referencing `root` would recurse forever.
    async fn descendants_first(
        tx: &mut sqlx::SqliteConnection,
        root: Uuid,
    ) -> Result<Vec<Uuid>, DataError> {
        let mut seen = std::collections::HashSet::from([root]);
        let mut out = Vec::new();
        Self::collect_descendants(tx, root, &mut seen, &mut out).await?;
        Ok(out)
    }

    /// Walk `parent_id` links breadth-first from `node`, collecting every
    /// descendant id into `seen` (visited-set bounds a cyclic self-FK walk).
    ///
    /// # Examples
    ///
    /// ```text
    /// Self::collect_descendants(&mut tx, scene_id, &mut seen).await?; // seen: subtree ids
    /// ```
    async fn collect_descendants(
        tx: &mut sqlx::SqliteConnection,
        node: Uuid,
        seen: &mut std::collections::HashSet<Uuid>,
        out: &mut Vec<Uuid>,
    ) -> Result<(), DataError> {
        let child_rows = sqlx::query("SELECT id FROM documents WHERE parent_id = ? ORDER BY id")
            .bind(node.to_string())
            .fetch_all(&mut *tx)
            .await?;
        for r in child_rows {
            let child = Uuid::parse_str(r.get::<String, _>("id").as_str())
                .map_err(|e| DataError::OpFailed(e.to_string()))?;
            // Skip already-visited nodes (self-reference / cycle guard).
            if !seen.insert(child) {
                continue;
            }
            // Recurse first so deeper descendants precede their parent.
            Box::pin(Self::collect_descendants(&mut *tx, child, seen, out)).await?;
            out.push(child);
        }
        Ok(())
    }

    /// Derives the `documents` row's scope/source columns from `doc`'s
    /// envelope — the exact derivation `upsert_document` and
    /// `insert_imported_document` both need before their (differing) INSERT
    /// statements, factored out once so the two document-write paths cannot
    /// silently diverge on it. Returns `(scope_kind, world_id, pack,
    /// source_id, source_pack, source_version)`.
    fn document_row_columns(doc: &Document) -> DocumentRowColumns {
        let (scope_kind, world_id, pack) = match &doc.scope {
            Scope::Compendium { pack } => ("compendium", None, Some(pack.clone())),
            Scope::World { world_id } => ("world", Some(world_id.to_string()), None),
        };
        let (source_id, source_pack, source_version) = match &doc.source {
            Some(s) => (
                Some(s.id.to_string()),
                s.pack.clone(),
                Some(s.version as i64),
            ),
            None => (None, None, None),
        };
        (
            scope_kind,
            world_id,
            pack,
            source_id,
            source_pack,
            source_version,
        )
    }

    /// Rewrite `doc`'s FTS index rows (both visibility-tier tables) in the
    /// caller's transaction — the delete-then-reinsert block
    /// `upsert_document` and `insert_imported_document` both need after
    /// their (differing) `documents` INSERT, factored out once for the same
    /// never-fork reason as `document_row_columns`. `world_id` is passed in
    /// (rather than re-derived) because both callers already computed it via
    /// `document_row_columns`.
    ///
    /// Two SEPARATE single-column tables, not two columns of one table:
    /// bm25()'s row-length normalization is computed from the WHOLE ROW (all
    /// columns), so a shared table would let a non-GM query's score be
    /// shifted by the mere LENGTH of GM-only text on the same row even when
    /// column weights zero out that column's term-frequency contribution.
    /// Separate tables make each tier's row length genuinely isolated.
    async fn reindex_document_fts(
        conn: &mut sqlx::SqliteConnection,
        doc: &Document,
        world_id: Option<String>,
    ) -> Result<(), DataError> {
        sqlx::query("DELETE FROM documents_fts_public WHERE doc_id = ?")
            .bind(doc.id.to_string())
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM documents_fts_gm WHERE doc_id = ?")
            .bind(doc.id.to_string())
            .execute(&mut *conn)
            .await?;
        sqlx::query(
            "INSERT INTO documents_fts_public (content, doc_id, world_id) VALUES (?, ?, ?)",
        )
        .bind(crate::data::search::index_content_public(doc))
        .bind(doc.id.to_string())
        .bind(world_id.clone())
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "INSERT INTO documents_fts_gm (content_all, doc_id, world_id) VALUES (?, ?, ?)",
        )
        .bind(crate::data::search::index_content(doc))
        .bind(doc.id.to_string())
        .bind(world_id)
        .execute(&mut *conn)
        .await?;
        Ok(())
    }

    /// Upsert a document row from its envelope, stamping `seq`, and rewrite its
    /// FTS index row in the same transaction (crash-consistent). Takes a
    /// `&mut SqliteConnection` because it runs multiple statements; callers pass
    /// `&mut *tx`.
    async fn upsert_document(
        conn: &mut sqlx::SqliteConnection,
        doc: &Document,
        seq: i64,
    ) -> Result<(), DataError> {
        let (scope_kind, world_id, pack, source_id, source_pack, source_version) =
            Self::document_row_columns(doc);
        let json = serde_json::to_string(doc)?;
        sqlx::query(
            "INSERT INTO documents (id, scope_kind, world_id, pack, doc_type, schema_version, \
             source_id, source_pack, source_version, owner_id, parent_id, seq, created_seq, json, \
             created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET scope_kind=excluded.scope_kind, world_id=excluded.world_id, \
             pack=excluded.pack, doc_type=excluded.doc_type, schema_version=excluded.schema_version, \
             source_id=excluded.source_id, source_pack=excluded.source_pack, \
             source_version=excluded.source_version, owner_id=excluded.owner_id, \
             parent_id=excluded.parent_id, seq=excluded.seq, \
             json=excluded.json, updated_at=excluded.updated_at",
        )
        .bind(doc.id.to_string())
        .bind(scope_kind)
        .bind(world_id.clone())
        .bind(pack)
        .bind(&doc.doc_type)
        .bind(doc.schema_version as i64)
        .bind(source_id)
        .bind(source_pack)
        .bind(source_version)
        .bind(doc.owner.map(|o| o.to_string()))
        .bind(doc.parent_id.map(|p| p.to_string()))
        .bind(seq)
        .bind(seq)
        .bind(json)
        .bind(doc.created_at)
        .bind(doc.updated_at)
        .execute(&mut *conn)
        .await?;
        Self::reindex_document_fts(conn, doc, world_id).await
    }

    /// Remove a document's FTS rows (both visibility-tier tables). Call
    /// alongside every document delete so the index never references a
    /// removed document.
    async fn delete_document_fts(
        conn: &mut sqlx::SqliteConnection,
        id: Uuid,
    ) -> Result<(), DataError> {
        sqlx::query("DELETE FROM documents_fts_public WHERE doc_id = ?")
            .bind(id.to_string())
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM documents_fts_gm WHERE doc_id = ?")
            .bind(id.to_string())
            .execute(conn)
            .await?;
        Ok(())
    }

    /// Apply a document Delete inside `tx`: the row, its FTS entries, and its
    /// explored-fog rows. SINGLE SOURCE for delete side-effects — BOTH
    /// authoritative delete paths (`apply_intent`, `apply_command`) call this,
    /// so they cannot drift (never-fork). The fog purge is unconditional by id:
    /// only scene documents ever appear as `explored_fog.scene_id`, so it is a
    /// no-op for every other doc_type and carries no doc_type predicate that
    /// could drift from the fog writer's keying.
    async fn delete_document_tx(
        tx: &mut sqlx::SqliteConnection,
        id: Uuid,
    ) -> Result<(), DataError> {
        // Asset-folder hook: every asset filed under a deleted folder moves
        // to the folder's parent BEFORE the row goes, so the `assets.folder_id`
        // FK's `SET NULL` never fires (that would flatten to root instead of
        // the parent). Parent deletes expand children-first, so a sub-folder's
        // assets hop one level per op and end in the surviving ancestor.
        Self::reparent_assets_of_deleted_folder(&mut *tx, id).await?;
        sqlx::query("DELETE FROM documents WHERE id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        Self::delete_document_fts(&mut *tx, id).await?;
        sqlx::query("DELETE FROM explored_fog WHERE scene_id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        Ok(())
    }

    /// Test-only raw insert that bypasses every ingress gate, including
    /// `apply_command`/`apply_intent`'s `/engine` normalization — seeds a
    /// `Document` exactly as given, malformed `engine` body included, to
    /// exercise a reader's fail-closed handling of already-persisted data
    /// that predates or violates the current typed schema (schema
    /// evolution, hand-edited rows). `apply_command` validates on write, so
    /// it cannot seed such fixtures; this is not a production code path and
    /// must stay `#[cfg(test)]`-only.
    #[cfg(test)]
    pub(crate) async fn seed_document_unvalidated(&self, doc: &Document) -> Result<(), DataError> {
        let mut tx = self.pool.begin().await?;
        Self::upsert_document(&mut tx, doc, 0).await?;
        tx.commit().await?;
        Ok(())
    }
}

/// A world-sequenced command may only carry documents scoped to its own world.
/// A foreign scope would file the row outside this world's seq stream, making it
/// unreachable by `events_since` for either world and breaking replay scoping.
fn check_command_scope(doc: &Document, world_id: Uuid) -> Result<(), DataError> {
    match &doc.scope {
        Scope::World { world_id: w } if *w == world_id => Ok(()),
        _ => Err(DataError::OpFailed(
            "document scope does not match the command's world".into(),
        )),
    }
}

/// `doc`'s `CombatEngine`, or `None` if `doc` is not a `combat` document.
/// A stored `combat` document's engine body is always valid by construction
/// (validated at every write), so a parse failure here is treated the same
/// as a non-combat `doc_type` -- the caller's own decision this feeds is
/// advisory-only (see `apply_intent`'s one-active-per-scene tracking), never
/// the authoritative engine validation `validate_engine_tree` performs.
fn combat_engine_of(doc: &Document) -> Option<CombatEngine> {
    if doc.doc_type != COMBAT_DOC_TYPE {
        return None;
    }
    doc.engine
        .clone()
        .and_then(|e| serde_json::from_value(e).ok())
}

/// The `CombatEngine` `cur` would carry after `changes` apply, computed
/// entirely in memory -- no storage touched. Used ONLY to drive the
/// one-active-combat-per-scene decision ahead of the authoritative merge
/// (`validate_engine_tree` on the real post-image); a change this cannot
/// merge or re-parse into a `Document` yields `None` rather than an error,
/// since the real validation elsewhere in `apply_intent` surfaces any such
/// failure through its own path.
fn merged_combat_engine(cur: &Document, changes: &[FieldChange]) -> Option<CombatEngine> {
    if cur.doc_type != COMBAT_DOC_TYPE {
        return None;
    }
    let mut value = serde_json::to_value(cur).ok()?;
    changes
        .iter()
        .try_for_each(|ch| apply_field_change(&mut value, ch))
        .ok()?;
    let doc: Document = serde_json::from_value(value).ok()?;
    doc.engine.and_then(|e| serde_json::from_value(e).ok())
}

/// Largest integer magnitude an `f64` represents exactly (2^53); beyond this,
/// adjacent integers alias to the same `f64`, so a Number/variant comparison
/// falling back to `as f64` would silently equate genuinely different values.
const MAX_EXACT_F64_INT: i128 = 1i128 << 53;

/// Structural equality used ONLY at `SqliteRepository::apply_intent`'s Phase-1 OCC pre-image
/// comparison (`actual != ch.old`). `serde_json::Value::Number`
/// splits whole numbers into `PosInt`/`NegInt` and non-whole numbers into `Float`;
/// an engine field stored as a whole-number `f64` (e.g. `100.0`) serializes to
/// `Float(100.0)`, but a JS client cannot preserve "this was a float" through
/// `JSON.parse`/re-serialize for a whole-number value, so an echoed pre-image
/// comes back as `PosInt(100)`. Raw `==` treats these as unequal, causing a
/// spurious `Conflict` on an otherwise up-to-date write (e.g. an ordinary token
/// drag after a server-executed `execute_move`, or the `ActorsPanel` vision-range
/// editor's nested `range` field). This function recurses into `Object`/`Array`
/// structure and treats mismatched-variant Number leaves as equal when they
/// represent the same value. Two Numbers that BOTH parse as integers (either
/// PosInt/NegInt variant) are compared EXACTLY as `i128`, with no magnitude
/// limit -- this case never touches `f64`, so distinct large integers (past
/// 2^53) never alias into a false match. The `|n| <= 2^53` exactness guard
/// applies ONLY to the genuinely mixed case, one side an integer and the other
/// a `Float`, where an `f64` comparison is unavoidable because the Float side
/// has no exact integer form; outside that range, or for any non-Number
/// mismatch, it falls back to serde's derived `PartialEq`.
fn values_semantically_eq(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    use serde_json::Value;
    match (a, b) {
        (Value::Object(ma), Value::Object(mb)) => {
            ma.len() == mb.len()
                && ma
                    .iter()
                    .all(|(k, va)| mb.get(k).is_some_and(|vb| values_semantically_eq(va, vb)))
        }
        (Value::Array(xa), Value::Array(xb)) => {
            xa.len() == xb.len()
                && xa
                    .iter()
                    .zip(xb.iter())
                    .all(|(va, vb)| values_semantically_eq(va, vb))
        }
        (Value::Number(na), Value::Number(nb)) => {
            if na == nb {
                return true;
            }
            // Variants differ (one PosInt/NegInt, the other Float, or the pair
            // straddles PosInt/NegInt with mismatched sign representation).
            // Compare numerically only when any integer operand is exactly
            // representable as f64; otherwise trust the exact comparison above.
            let ia = na
                .as_i64()
                .map(|v| v as i128)
                .or_else(|| na.as_u64().map(|v| v as i128));
            let ib = nb
                .as_i64()
                .map(|v| v as i128)
                .or_else(|| nb.as_u64().map(|v| v as i128));
            match (ia, ib) {
                // Both sides parse as integers (PosInt/NegInt pair): i128 holds
                // every i64/u64 value without loss, so compare exactly. Never
                // fall through to f64 here -- two distinct integers past 2^53
                // (e.g. 2^62 vs 2^62 + 1) alias to the same f64 and would
                // falsely compare equal, which is an OCC bypass (a stale
                // pre-image would match a genuinely different stored value).
                (Some(va), Some(vb)) => va == vb,
                // Genuinely mixed case: one side is an integer, the other a
                // Float. f64 comparison is unavoidable here since the Float
                // side has no exact integer representation; only exact when
                // the integer side is within f64's exact range.
                (Some(v), None) | (None, Some(v))
                    if v.unsigned_abs() > MAX_EXACT_F64_INT as u128 =>
                {
                    false
                }
                _ => match (na.as_f64(), nb.as_f64()) {
                    (Some(fa), Some(fb)) => fa == fb,
                    _ => false,
                },
            }
        }
        _ => a == b,
    }
}

#[async_trait]
impl Repository for SqliteRepository {
    async fn apply_command(&self, cmd: UnsequencedCommand) -> Result<StoredCommand, DataError> {
        let mut tx = self.pool.begin().await?;

        // Allocate the next per-world seq from the single durable source.
        // Unlike `apply_intent` (seq allocated AFTER Phase-1 validation, so a
        // rejected intent never consumes one), this bump happens BEFORE any
        // op is validated -- safe only because the whole transaction rolls
        // back on any early `?` return below, so a rejected write never
        // commits the bumped seq either. A future error-handling refactor
        // that starts returning `Ok` on a partially-applied/rejected op
        // (instead of aborting via `?`) would silently start consuming seqs
        // on rejected writes; keep this ordering paired with whole-tx
        // rollback semantics.
        let seq: i64 = sqlx::query("UPDATE worlds SET seq = seq + 1 WHERE id = ? RETURNING seq")
            .bind(cmd.world_id.to_string())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(DataError::NotFound)?
            .get("seq");

        // Expand each Delete into its descendants (children-first) so a parent
        // delete removes children through explicit, logged ops rather than the
        // silent SQL FK cascade (#2/#8). apply_command is the trusted substrate
        // (undo/replay), so unlike apply_intent the descendants are not
        // capability-checked.
        let mut ops = Vec::with_capacity(cmd.ops.len());
        for op in cmd.ops {
            match op {
                Operation::Delete { doc } => {
                    for desc in Self::descendants_first(&mut tx, doc.id).await? {
                        let cur = Self::load_document(&mut *tx, desc).await?.ok_or_else(|| {
                            DataError::Conflict(format!("descendant {desc} missing"))
                        })?;
                        ops.push(Operation::Delete { doc: cur });
                    }
                    ops.push(Operation::Delete { doc });
                }
                other => ops.push(other),
            }
        }

        let mut sequenced = Command {
            seq,
            world_id: cmd.world_id,
            author: cmd.author,
            ts: cmd.ts,
            ops,
        };

        // Apply each operation. `normalized_ops` mirrors apply_intent's
        // rebuild: identical to `sequenced.ops` except an Update's
        // `FieldChange.new` under `/engine`(/*) is renormalized to the
        // validated post-image, so the returned `Command`, the
        // `world_events` log entry, and any future `events_since` replay
        // all carry the identical normalized value the row was stored
        // with. apply_command is the trusted substrate (undo/replay) and
        // skips capability/schema/size checks by design (zero production
        // callers), but the engine band's normalize-then-store invariant
        // is data integrity, not authz -- it applies regardless of trust
        // level.
        let mut post_images: std::collections::HashMap<Uuid, Document> =
            std::collections::HashMap::new();
        let mut deleted_created_seqs: std::collections::HashMap<Uuid, i64> =
            std::collections::HashMap::new();
        // Batch-start permissions for each Update target, captured the FIRST time its
        // pre-image is loaded — before any op applies — so a second same-batch Update to
        // the same doc still snapshots the true batch-start permissions, not an
        // intermediate value from an earlier op in this same command.
        let mut pre_permissions: std::collections::HashMap<
            Uuid,
            crate::data::document::PermissionSet,
        > = std::collections::HashMap::new();
        // Batch-start EFFECTIVE OWNER for each Update target, captured in lockstep with
        // `pre_permissions` at the same pre-image load point (first Update of a batch id
        // wins). `Option<Uuid>` inside the map value: an entry's ABSENCE means "not yet
        // captured", its `None` value means "captured, no owner".
        let mut pre_owners: std::collections::HashMap<Uuid, Option<Uuid>> =
            std::collections::HashMap::new();
        let mut normalized_ops = Vec::with_capacity(sequenced.ops.len());
        for op in &sequenced.ops {
            match op {
                Operation::Create { doc } => {
                    check_command_scope(doc, sequenced.world_id)?;
                    let mut doc = doc.clone();
                    // Same band-classification gate as apply_intent: no stored override
                    // can name a pointer redaction cannot classify (data integrity, not
                    // authz — see the /engine gate below for the same rationale).
                    crate::data::validation::validate_property_overrides(&doc)?;
                    crate::data::validation::validate_engine_tree(&mut doc)?;
                    crate::data::validation::validate_containment(&doc)?;
                    Self::upsert_document(&mut tx, &doc, seq).await?;
                    post_images.insert(doc.id, doc.clone());
                    normalized_ops.push(Operation::Create { doc });
                }
                Operation::Delete { doc } => {
                    check_command_scope(doc, sequenced.world_id)?;
                    if let Some(cs) = Self::document_created_seq(&mut *tx, doc.id).await? {
                        deleted_created_seqs.insert(doc.id, cs);
                    }
                    Self::delete_document_tx(&mut tx, doc.id).await?;
                    normalized_ops.push(op.clone());
                }
                Operation::Move {
                    doc_id, parent_id, ..
                } => {
                    let cur = Self::load_document(&mut *tx, *doc_id)
                        .await?
                        .ok_or_else(|| DataError::Conflict(format!("document {doc_id} missing")))?;
                    check_command_scope(&cur, sequenced.world_id)?;
                    // Batch-start snapshot capture, in lockstep with the
                    // Update arm's below.
                    if !pre_permissions.contains_key(doc_id) {
                        pre_permissions.insert(*doc_id, cur.permissions.clone());
                        let pre_owner = Self::load_effective_owner(&mut *tx, &cur).await?;
                        pre_owners.insert(*doc_id, pre_owner);
                    }
                    if cur.parent_id == *parent_id {
                        // No-op: carried in the log for invertibility;
                        // nothing written, nothing bumped, no hooks run.
                        post_images.insert(*doc_id, cur);
                        normalized_ops.push(op.clone());
                    } else {
                        // Trusted substrate: no capability/OCC gate, but the
                        // structural placement rules are data integrity and
                        // trust does not exempt them (same rationale as
                        // `validate_property_overrides` running here). Earlier
                        // ops in this command are already applied, so the
                        // batch bookkeeping maps are empty by construction.
                        let mut doc = cur;
                        doc.parent_id = *parent_id;
                        validation::validate_containment(&doc)?;
                        Self::check_parent_placement(
                            &mut tx,
                            sequenced.world_id,
                            &doc,
                            &Default::default(),
                            &Default::default(),
                        )
                        .await?;
                        Self::check_move_acyclic(
                            &mut tx,
                            *doc_id,
                            *parent_id,
                            &Default::default(),
                            &Default::default(),
                        )
                        .await?;
                        doc.updated_at = sequenced.ts;
                        Self::upsert_document(&mut tx, &doc, seq).await?;
                        if doc.doc_type == crate::data::engine::ASSET_FOLDER_DOC_TYPE {
                            Self::refresh_derived_tags_for_folder_subtree(&mut tx, doc.id).await?;
                        }
                        post_images.insert(*doc_id, doc);
                        normalized_ops.push(op.clone());
                    }
                }
                Operation::Update { doc_id, changes } => {
                    let row = sqlx::query("SELECT json FROM documents WHERE id = ?")
                        .bind(doc_id.to_string())
                        .fetch_optional(&mut *tx)
                        .await?
                        .ok_or(DataError::NotFound)?;
                    let mut value: serde_json::Value =
                        serde_json::from_str(row.get::<String, _>("json").as_str())?;
                    // Captured BEFORE this op applies, and only for the FIRST Update of
                    // this id in the batch — see `pre_permissions`'s own comment. Owner
                    // capture rides the same guard so the two maps stay in lockstep.
                    if !pre_permissions.contains_key(doc_id) {
                        let pre_doc: Document = serde_json::from_value(value.clone())?;
                        pre_permissions.insert(*doc_id, pre_doc.permissions.clone());
                        let pre_owner = Self::load_effective_owner(&mut *tx, &pre_doc).await?;
                        pre_owners.insert(*doc_id, pre_owner);
                    }
                    for ch in changes {
                        // THE `apply_field_change` mutation rule. Never
                        // re-derive the remove/set branch here: the derived scene ECS
                        // mirrors these same changes and must land the same value.
                        apply_field_change(&mut value, ch)?;
                    }
                    let mut doc: Document = serde_json::from_value(value)?;
                    // Identity and world scope are immutable through an update:
                    // changing id forks a duplicate row (load key != upsert key);
                    // changing world files the row outside this world's seq stream.
                    if doc.id != *doc_id {
                        return Err(DataError::OpFailed(
                            "update must not change the document id".into(),
                        ));
                    }
                    check_command_scope(&doc, sequenced.world_id)?;
                    // Same band-classification gate as apply_intent: no
                    // stored override can name a pointer redaction cannot
                    // classify (data integrity, not authz -- see below).
                    crate::data::validation::validate_property_overrides(&doc)?;
                    // Same /engine gate as apply_intent (the trusted
                    // substrate skips capability/schema/size checks by
                    // design, but the engine band's normalize-then-store
                    // invariant is data integrity, not authz -- the row,
                    // the log, and any future replay must carry the
                    // identical normalized value).
                    crate::data::validation::validate_engine_tree(&mut doc)?;
                    // updated_at tracks last mutation; the command ts is authoritative.
                    doc.updated_at = sequenced.ts;
                    Self::upsert_document(&mut tx, &doc, seq).await?;
                    post_images.insert(*doc_id, doc.clone());
                    // A folder's name is a derived tag on every asset beneath
                    // it; any Update to an `asset_folder` (rename being the
                    // one that matters) recomputes that subtree in this tx.
                    if doc.doc_type == crate::data::engine::ASSET_FOLDER_DOC_TYPE {
                        Self::refresh_derived_tags_for_folder_subtree(&mut tx, doc.id).await?;
                    }

                    // Re-derive each `/engine`(/*) `FieldChange.new` from
                    // the SAME validated post-image so the returned
                    // Command and the world_events log entry carry the
                    // identical normalized value the row was stored with
                    // -- never the raw submitted JSON.
                    let normalized_doc_json = serde_json::to_value(&doc)?;
                    let normalized_changes: Vec<FieldChange> = changes
                        .iter()
                        .map(|ch| {
                            if ch.path == "/engine" || ch.path.starts_with("/engine/") {
                                if let Some(v) = normalized_doc_json.pointer(&ch.path) {
                                    return FieldChange {
                                        remove: false,
                                        path: ch.path.clone(),
                                        old: ch.old.clone(),
                                        new: v.clone(),
                                    };
                                }
                            }
                            ch.clone()
                        })
                        .collect();
                    normalized_ops.push(Operation::Update {
                        doc_id: *doc_id,
                        changes: normalized_changes,
                    });
                }
            }
        }
        sequenced.ops = normalized_ops;

        let world_gm_at_commit: std::collections::HashMap<Uuid, bool> =
            Self::world_member_roles(&mut *tx, sequenced.world_id)
                .await?
                .into_iter()
                .map(|(uid, role)| (uid, role == WorldRole::Gm))
                .collect();
        let mut per_op = Vec::with_capacity(sequenced.ops.len());
        for op in &sequenced.ops {
            per_op.push(Some(
                Self::build_op_snapshot(
                    &mut tx,
                    op,
                    &post_images,
                    &deleted_created_seqs,
                    &pre_permissions,
                    &pre_owners,
                )
                .await?,
            ));
        }
        let stored = StoredCommand {
            command: sequenced,
            snapshot: CommandSnapshot {
                per_op,
                world_gm_at_commit,
            },
        };

        // Append to the log.
        sqlx::query("INSERT INTO world_events (world_id, seq, author_id, ts, command_json) VALUES (?, ?, ?, ?, ?)")
            .bind(stored.command.world_id.to_string())
            .bind(seq)
            .bind(stored.command.author.to_string())
            .bind(stored.command.ts)
            .bind(serde_json::to_string(&stored)?)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(stored)
    }

    async fn apply_intent(
        &self,
        ctx: &crate::data::membership::PermissionContext,
        world_id: Uuid,
        mut ops: Vec<Operation>,
        ts: i64,
        origin: WriteOrigin,
    ) -> Result<StoredCommand, DataError> {
        // Load world default grants before opening the transaction: the
        // single-writer pool holds one connection, so a settings query mid-tx
        // would deadlock.
        let world_defaults = self.world_cap_defaults(world_id).await?;
        // Write-enforcement input is ONLY the GM-authored `world_cap_requirements`
        // record. Module-declared `requirements` (published to clients via the
        // Welcome union, see `ws::conn::welcome_capability_requirements`) are
        // advisory client-side UX only and are intentionally NOT consulted here —
        // server authority over write policy stays with the GM/operator, never
        // community module code.
        let world_reqs = self.world_cap_requirements(world_id).await?;
        // Loaded before the transaction (like `world_cap_requirements` above):
        // the single-writer pool would deadlock on a mid-tx settings query.
        // This is the GM-controlled tier-2 structural schema registry; the
        // writer never supplies its own judging schema.
        let world_schemas = self.world_schema_declarations(world_id).await?;
        let mut tx = self.pool.begin().await?;

        // Phase 1 — authorize, structurally validate, and check pre-images.
        // No row is mutated; any failure here drops the transaction, so the
        // per-world seq is never consumed by a rejected intent. `Create`'s
        // `doc` is mutated in place (`&mut ops`) so `validate_engine_tree`
        // can normalize the engine band here and have that normalization
        // survive into Phase 2 storage AND the returned `Command` (broadcast).
        //
        // `claimed_singletons` tracks singleton doc_types already passed by an
        // EARLIER Create in this SAME batch. Phase 2 (the actual inserts)
        // only runs after every op in the batch clears Phase 1, so a batch
        // containing two Creates of the same singleton doc_type would have
        // both ops' `singleton_doc_exists` reads see nothing (neither has
        // been inserted yet) and both pass the DB check alone — this set
        // closes that intra-batch gap the DB check cannot see.
        let mut claimed_singletons: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        // `batch_combats` tracks the ids of `combat` documents Created earlier
        // in this same batch, so a same-batch `combat` + `combatant` pair
        // (a scene+combat+combatants import/setup in one Intent) can satisfy
        // the combatant-parentage check without a DB round trip that would
        // see nothing yet inserted.
        let mut batch_combats: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
        // `batch_folders` plays the same role for `asset_folder` documents: a
        // folder Created earlier in this batch is a valid parent for a later
        // one, and `check_asset_folder_parent` walks through it for cycles.
        let mut batch_folders: std::collections::HashMap<Uuid, Document> =
            std::collections::HashMap::new();
        // `batch_moves` records the PROSPECTIVE parent of each Move already
        // validated in this batch. Phase 2 applies nothing until every op
        // clears Phase 1, so a cycle walk that read only the stored tree
        // would validate each Move against a tree no op has rewritten yet —
        // two Moves that swap a pair of subtrees into a cycle would each
        // pass alone. `check_move_acyclic` consults this map first, so it
        // sees the tree the batch will actually leave.
        let mut batch_moves: std::collections::HashMap<Uuid, Option<Uuid>> =
            std::collections::HashMap::new();
        // `scene_owner` maps a scene id to the id of the `combat` document
        // that holds its active slot, AS OF THIS POINT in a single simulated
        // walk of `ops` in their actual batch order. The one-active-combat-
        // per-scene decision -- for both the Create arm and the Update
        // arm -- is made exactly ONCE, entirely inside Phase 1 (Phase 2
        // performs no independent recomputation of it), consulting and
        // mutating this ONE map: a claim (a Create or Update that would make
        // some combat `active: true` on scene `S`) succeeds when
        // `scene_owner.get(S)` is absent or already equals that combat's own
        // id, and inserts `S -> that combat's id`; a release (an Update that
        // would make its own combat `active: false`) removes `S` only when
        // `scene_owner.get(S)` equals that SAME combat's id, and otherwise
        // does nothing -- an unrelated combat's deactivation can never touch
        // a DIFFERENT combat's claim on the same scene. Seeded lazily by
        // `Self::ensure_scene_owner_seeded`, and consulted only through
        // this map, so the whole batch shares one running fact rather than
        // re-deriving it at each op.
        let mut scene_owner: std::collections::HashMap<Uuid, Uuid> =
            std::collections::HashMap::new();
        // Scenes `Self::ensure_scene_owner_seeded` has already resolved
        // (seeded from the DB, or deliberately left unseeded) for this
        // batch -- a scene is seeded at most once regardless of how many
        // ops in the batch touch it.
        let mut seeded_scenes: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
        // The set of scenes a genuine same-batch active-true-to-false
        // transition will free, computed by a pre-scan over the WHOLE batch
        // before the Phase-1 per-op loop runs (and before `scene_owner` is
        // seeded at all) -- not derived incrementally in op order. A single
        // forward walk of `ops` cannot see a LATER op's deactivation when
        // validating an EARLIER op that claims the same scene (e.g.
        // `[activate combat A, deactivate combat B]` on one scene: A's claim
        // is checked before B's deactivation has run), so which scenes this
        // batch will free must be known before the walk begins.
        // `Self::ensure_scene_owner_seeded` consults this set to decide
        // whether to seed a scene's owner from the DB at all: a scene this
        // batch will free is left unseeded (no owner) from the start, so an
        // earlier same-batch claim on it is validated against the state the
        // batch will actually leave, not a DB row a later op is about to
        // invalidate.
        //
        // Only an op whose PRE-image was already `active: true` qualifies: an
        // op touching a combat that was ALREADY inactive must never mark its
        // scene as freed this batch, or an unrelated activation elsewhere in
        // the batch would wrongly skip its DB conflict check. Two op shapes
        // free a scene:
        // - `Delete` of an `active: true` combat -- the combat, and its claim,
        //   cease to exist outright.
        // - `Update` whose post-merge `active` is `false` (a genuine
        //   true->false transition, via `merged_combat_engine`), OR whose
        //   post-merge `active` STAYS `true` but `scene_id` changed -- moving
        //   away from a scene while remaining active vacates that scene just
        //   as genuinely as deactivating does. These two `Update` cases are
        //   mutually exclusive (`merged_engine.active` cannot be both `true`
        //   and `false` for the same op) and file under different scenes (the
        //   PRE-merge scene in both cases -- the scene the combat is LEAVING,
        //   never the one it is moving to), so neither double-frees nor
        //   conflicts with the other.
        // An op this cannot merge or parse is left alone here -- Phase 1's
        // real validation below surfaces the failure through the ordinary
        // path.
        let mut deactivations_this_batch: std::collections::HashSet<Uuid> =
            std::collections::HashSet::new();
        for op in &ops {
            match op {
                Operation::Update { doc_id, changes } => {
                    if let Some(cur) = Self::load_document(&mut *tx, *doc_id).await? {
                        // Scoped the same way every other load site in this
                        // function is: a foreign-world document must never
                        // influence this batch's validation, even indirectly
                        // through the pre-scan's bookkeeping.
                        check_command_scope(&cur, world_id)?;
                        if let Some(pre_engine) = combat_engine_of(&cur) {
                            if pre_engine.active {
                                if let Some(merged_engine) = merged_combat_engine(&cur, changes) {
                                    if !merged_engine.active
                                        || merged_engine.scene_id != pre_engine.scene_id
                                    {
                                        deactivations_this_batch.insert(pre_engine.scene_id);
                                    }
                                }
                            }
                        }
                    }
                }
                Operation::Delete { doc } => {
                    if let Some(cur) = Self::load_document(&mut *tx, doc.id).await? {
                        check_command_scope(&cur, world_id)?;
                        if let Some(pre_engine) = combat_engine_of(&cur) {
                            if pre_engine.active {
                                deactivations_this_batch.insert(pre_engine.scene_id);
                            }
                        }
                    }
                }
                Operation::Create { .. } => {}
                // A combat document refuses any parent at `validate_containment`,
                // so a Move can never alter a combat engine's active/scene state.
                Operation::Move { .. } => {}
            }
        }
        // Batch-start permissions for each Update target, captured the FIRST time its
        // pre-image is loaded in Phase 1 — before any op applies — so a second same-batch
        // Update to the same doc still snapshots the true batch-start permissions, not an
        // intermediate value written by an earlier op in this same command.
        let mut pre_permissions: std::collections::HashMap<
            Uuid,
            crate::data::document::PermissionSet,
        > = std::collections::HashMap::new();
        // Batch-start EFFECTIVE OWNER for each Update target, captured in lockstep with
        // `pre_permissions` at the same pre-image load point (first Update of a batch id
        // wins). `Option<Uuid>` inside the map value: an entry's ABSENCE means "not yet
        // captured", its `None` value means "captured, no owner".
        let mut pre_owners: std::collections::HashMap<Uuid, Option<Uuid>> =
            std::collections::HashMap::new();
        for op in &mut ops {
            match op {
                Operation::Move {
                    doc_id,
                    parent_id,
                    old_parent_id,
                } => {
                    let cur = Self::load_document(&mut *tx, *doc_id)
                        .await?
                        .ok_or_else(|| DataError::Conflict(format!("document {doc_id} missing")))?;
                    check_command_scope(&cur, world_id)?;
                    // GM-only, and only where the GM's unconditional
                    // short-circuit holds: a `gm_role`-capped GM floor-resolves
                    // through `resolve_access_world` like any other actor and
                    // is refused. `CombatTransition` skips this capability
                    // gate — see the Create arm's matching comment above.
                    if origin != WriteOrigin::CombatTransition {
                        let owner = Self::load_effective_owner(&mut *tx, &cur).await?;
                        let access = resolve_access_world(
                            ctx.user_id,
                            ctx.world_role,
                            &cur,
                            &world_defaults.grants_for(&cur.doc_type),
                            owner,
                        );
                        if ctx.world_role != WorldRole::Gm || !access.all {
                            return Err(DataError::Forbidden);
                        }
                    }
                    // Stored `message` docs refuse every ordinary mutation
                    // path — the same stored-doc_type classification the
                    // Update arm applies below.
                    if cur.doc_type == crate::chat::MESSAGE_DOC_TYPE
                        && origin != WriteOrigin::ServerMessageRevision
                    {
                        return Err(DataError::OpFailed(
                            "message documents cannot be moved".into(),
                        ));
                    }
                    if *old_parent_id != cur.parent_id {
                        return Err(DataError::Conflict(format!(
                            "parent pre-image mismatch for {doc_id}"
                        )));
                    }
                    // Batch-start snapshot capture, in lockstep with the
                    // Update arm's (see `pre_permissions`'s own comment).
                    if !pre_permissions.contains_key(doc_id) {
                        pre_permissions.insert(*doc_id, cur.permissions.clone());
                        let pre_owner = Self::load_effective_owner(&mut *tx, &cur).await?;
                        pre_owners.insert(*doc_id, pre_owner);
                    }
                    if *parent_id != cur.parent_id {
                        // Create-validity on the post-image: a Move is legal
                        // exactly where a Create with this parent would be.
                        let mut post = cur.clone();
                        post.parent_id = *parent_id;
                        validation::validate_containment(&post)?;
                        Self::check_parent_placement(
                            &mut tx,
                            world_id,
                            &post,
                            &batch_folders,
                            &batch_combats,
                        )
                        .await?;
                        Self::check_move_acyclic(
                            &mut tx,
                            *doc_id,
                            *parent_id,
                            &batch_folders,
                            &batch_moves,
                        )
                        .await?;
                        batch_moves.insert(*doc_id, *parent_id);
                    }
                }
                Operation::Create { doc } => {
                    check_command_scope(doc, world_id)?;
                    validation::validate_system_size(doc)?;
                    validation::validate_property_overrides(doc)?;
                    validation::validate_engine_tree(doc)?;
                    validation::validate_containment(doc)?;
                    Self::check_parent_placement(
                        &mut tx,
                        world_id,
                        doc,
                        &batch_folders,
                        &batch_combats,
                    )
                    .await?;
                    if doc.doc_type == crate::data::engine::ASSET_FOLDER_DOC_TYPE {
                        batch_folders.insert(doc.id, doc.clone());
                    }
                    if doc.doc_type == COMBAT_DOC_TYPE {
                        batch_combats.insert(doc.id);
                        // `validate_engine_tree` above already validated and
                        // normalized this body against `CombatEngine`, so the
                        // re-deserialize here cannot fail; `.expect` on the
                        // parse itself (rather than discarding the error via
                        // `.ok()`) surfaces the real `serde_json::Error` text
                        // if that invariant is ever broken by a future schema
                        // change.
                        let combat_engine: CombatEngine = serde_json::from_value(
                            doc.engine
                                .clone()
                                .expect("a combat doc_type always carries an engine body"),
                        )
                        .expect("validate_engine_tree already validated the combat engine body");
                        if combat_engine.active {
                            Self::ensure_scene_owner_seeded(
                                &mut *tx,
                                world_id,
                                combat_engine.scene_id,
                                &mut scene_owner,
                                &mut seeded_scenes,
                                &deactivations_this_batch,
                            )
                            .await?;
                            match scene_owner.get(&combat_engine.scene_id) {
                                Some(&owner) if owner != doc.id => {
                                    return Err(DataError::Conflict(
                                        "an active combat already exists on this scene".into(),
                                    ));
                                }
                                _ => {
                                    scene_owner.insert(combat_engine.scene_id, doc.id);
                                }
                            }
                        }
                    }
                    validation::validate_system_schema_tree(doc, &world_schemas)?;
                    // A self-referential parent_id satisfies the self-FK and
                    // commits, then poisons the doc's deletion (the descendant
                    // walk would loop). Reject it. A stored parent's world
                    // scope is `check_parent_placement`'s check above; an
                    // unborn same-command parent is left to the FK at apply
                    // time, so batched scene+children creates still pass.
                    if doc.parent_id == Some(doc.id) {
                        return Err(DataError::OpFailed(
                            "document cannot be its own parent".into(),
                        ));
                    }
                    let create_owner = Self::load_effective_owner(&mut *tx, doc).await?;
                    let access = resolve_access_world(
                        ctx.user_id,
                        ctx.world_role,
                        doc,
                        &world_defaults.grants_for(&doc.doc_type),
                        create_owner,
                    );
                    // `CombatTransition` is a server-authored write: the combat
                    // clock's own handler has already decided this batch is
                    // legitimate, so the ordinary per-op capability floor and
                    // the world-level create gate below are skipped for this
                    // origin ONLY — every other check in this arm (scope,
                    // size, engine, containment, singleton, one-active-per-
                    // scene, schema) still runs unconditionally.
                    if origin != WriteOrigin::CombatTransition && !access.has(cap::WRITE_FIELDS) {
                        return Err(DataError::Forbidden);
                    }
                    // World-level create authorization: GM/admin hold every
                    // capability; any other actor's WorldRole must hold core:create
                    // for this doc type. Create has no document, so this rides
                    // WorldRole (role_caps), not the per-document DocRole.
                    //
                    // Baseline chat-posting right: a Player may author a `message`,
                    // exempt from the otherwise-GM-only core:create gate. The
                    // WRITE_FIELDS floor above still applies, and the extra
                    // `doc.owner == Some(ctx.user_id)` clause below ties the message
                    // to its poster. REQUIRED PRECONDITION for soundness: the
                    // WS/HTTP client-intent ingress MUST reject any client-authored
                    // `message` op, so that a `message` Create reaches `apply_intent`
                    // only from the server-side message-send handler (which builds a
                    // sanitized doc). Without that ingress rejection this exemption
                    // lets a Player create a self-owned `message` with an arbitrary
                    // body (forged `actor_owner`/`kind`) via a raw `Intent`; do not
                    // rely on this exemption until that rejection is in place, and do
                    // not weaken it thereafter.
                    let is_baseline_message = doc.doc_type == crate::chat::MESSAGE_DOC_TYPE
                        && ctx.world_role == WorldRole::Player
                        && doc.owner == Some(ctx.user_id);
                    if origin != WriteOrigin::CombatTransition
                        && ctx.world_role != WorldRole::Gm
                        && !is_baseline_message
                        && !world_defaults.role_has(ctx.world_role, &doc.doc_type, cap::CREATE)
                    {
                        tracing::debug!(
                            user = %ctx.user_id, doc_type = %doc.doc_type,
                            "create denied: missing core:create"
                        );
                        return Err(DataError::Forbidden);
                    }
                    // Create writes the whole body at once, so any declared
                    // requirement whose protected path is populated must be
                    // authorized — otherwise Create is a wholesale bypass of the
                    // declarative gate that Update enforces field-by-field.
                    let doc_json = serde_json::to_value(&*doc)?;
                    for extra in declared_caps_for_document(&doc_json, &world_reqs) {
                        if !access.has(extra) {
                            tracing::debug!(
                                user = %ctx.user_id, doc = %doc.id, capability = extra,
                                "create denied: missing declared capability"
                            );
                            return Err(DataError::Forbidden);
                        }
                    }
                    // Create is non-clobbering: an existing id is a conflict,
                    // not a silent overwrite (unlike upsert in apply_command).
                    if Self::load_document(&mut *tx, doc.id).await?.is_some() {
                        return Err(DataError::Conflict(format!(
                            "document {} already exists",
                            doc.id
                        )));
                    }
                    // Singleton doc_type create-gate: check-then-insert runs
                    // inside THIS transaction (same `tx` the existing-id check
                    // and the eventual insert use), so a concurrent Create
                    // racing this check cannot both pass it — the single-
                    // writer pool (`max_connections(1)`) serializes competing
                    // `apply_intent` transactions at connection-acquisition,
                    // and this query never touches `&self.pool` (which would
                    // deadlock mid-transaction, not race). `claimed_singletons`
                    // additionally covers a second same-batch Create of the
                    // same singleton doc_type, which the DB read alone cannot
                    // see (see the comment above the Phase-1 loop).
                    if SINGLETON_DOC_TYPES.contains(&doc.doc_type.as_str()) {
                        if claimed_singletons.contains(doc.doc_type.as_str())
                            || Self::singleton_doc_exists(&mut *tx, world_id, &doc.doc_type).await?
                        {
                            return Err(DataError::Conflict(format!(
                                "a '{}' document already exists in this world",
                                doc.doc_type
                            )));
                        }
                        claimed_singletons.insert(doc.doc_type.clone());
                    }
                }
                Operation::Delete { doc } => {
                    let cur = Self::load_document(&mut *tx, doc.id)
                        .await?
                        .ok_or_else(|| {
                            DataError::Conflict(format!("document {} missing", doc.id))
                        })?;
                    // Authorize against the stored doc, scoped to this world, so
                    // a GM of one world cannot delete another world's document.
                    check_command_scope(&cur, world_id)?;
                    let del_owner = Self::load_effective_owner(&mut *tx, &cur).await?;
                    // `CombatTransition` skips this capability gate — see the
                    // Create arm's matching comment above.
                    if origin != WriteOrigin::CombatTransition
                        && !resolve_access_world(
                            ctx.user_id,
                            ctx.world_role,
                            &cur,
                            &world_defaults.grants_for(&cur.doc_type),
                            del_owner,
                        )
                        .has(cap::DELETE)
                    {
                        return Err(DataError::Forbidden);
                    }
                }
                Operation::Update { doc_id, changes } => {
                    let cur = Self::load_document(&mut *tx, *doc_id)
                        .await?
                        .ok_or_else(|| DataError::Conflict(format!("document {doc_id} missing")))?;
                    // Captured BEFORE this op applies, and only for the FIRST Update of
                    // this id in the batch — see `pre_permissions`'s own comment. Owner
                    // capture rides the same guard so the two maps stay in lockstep.
                    if !pre_permissions.contains_key(doc_id) {
                        pre_permissions.insert(*doc_id, cur.permissions.clone());
                        let pre_owner = Self::load_effective_owner(&mut *tx, &cur).await?;
                        pre_owners.insert(*doc_id, pre_owner);
                    }
                    check_command_scope(&cur, world_id)?;
                    // Message docs are server-authored and immutable to clients
                    // in this checkpoint: `Update` carries no `doc_type` for
                    // `ops_target_message` to classify, so it is instead
                    // rejected here against the authoritative STORED doc_type
                    // (never a client-supplied one). Without this, an owning
                    // Player's `DocRole::Owner` grants WRITE_FIELDS on their
                    // own message, letting a raw Update forge `kind`/
                    // `user_owner`/`channel` or rewrite `content` unsanitized.
                    // `WriteOrigin::ServerMessageRevision` — set ONLY by the
                    // server edit/delete handlers or the post-publish
                    // enrichment republish, never derivable from any
                    // wire frame — re-opens this path for their sanitized
                    // authoritative revision; the ordinary WRITE_FIELDS/OCC
                    // checks below still apply on top of it. `CombatTransition`
                    // is NOT exempted here: the condition below rejects any
                    // origin other than `ServerMessageRevision`, so a combat
                    // clock batch may `Create` a `message` doc (roll results,
                    // event messages) but can never reach this arm to `Update`
                    // one — the same blanket rejection `Client` gets.
                    if cur.doc_type == crate::chat::MESSAGE_DOC_TYPE
                        && origin != WriteOrigin::ServerMessageRevision
                    {
                        return Err(DataError::Forbidden);
                    }
                    // A message doc's `gm_role`/`users` fields exist to gate
                    // ordinary READ visibility for OTHER recipients (e.g. an
                    // `Audience::Whisper` a GM isn't individually listed on
                    // resolves them to `DocRole::None`; `Audience::GmOnly`
                    // resolves them to `DocRole::Observer`, READ-only) — not
                    // the server's own moderation capability. The handler
                    // that produced this `ServerMessageRevision` write
                    // (`handle_edit_message`/`handle_delete_message`, or the
                    // post-publish enrichment republish) already
                    // independently vetted owner-or-GM authority before ever
                    // reaching here, so re-deriving capability from the
                    // document's own permission fields for THIS specific
                    // origin+doc_type pair would incorrectly re-restrict a
                    // GM's moderation edit/delete of a restricted-audience
                    // message. PRESUPPOSITION: this branch trusts that the
                    // calling handler has ALREADY performed an owner-or-GM
                    // check before setting `WriteOrigin::ServerMessageRevision`
                    // — the storage layer does not re-derive that decision,
                    // it only authorizes the write's SHAPE. Any future
                    // `ServerMessageRevision` construction site must be
                    // reviewed against this invariant.
                    //
                    // Grant only READ + WRITE_FIELDS (never `all: true`) —
                    // both existing handlers construct a single `/engine`
                    // FieldChange and never touch `/permissions` or
                    // `/embedded`, so the exemption is
                    // scoped to exactly what it is used for. This still
                    // authorizes the GM-not-addressed moderation edit/delete
                    // of `/engine` while denying `/permissions`/`/embedded`
                    // writes by construction, closing the gap even for a
                    // hypothetical future `ServerMessageRevision` caller with
                    // a broader op.
                    // A `CapabilityRequirement` carries no `doc_type` — it is a
                    // world-wide policy keyed on `path_prefix` alone, and
                    // `declared_caps_for_path`'s ancestor-overlap rule treats a
                    // whole-band `/engine` write as covering EVERY requirement
                    // declared anywhere under `/engine`, regardless of which
                    // doc_type that requirement was authored for (e.g. an
                    // actor's `/engine/vision`). A `ServerMessageRevision`
                    // write to EXACTLY `/engine` or
                    // `/permissions/property_overrides` is therefore exempted
                    // from that ADDITIVE check below (`is_scoped_smr_write`) —
                    // the calling handler has already vetted owner-or-GM
                    // authority, and this origin's writes are hard-scoped to
                    // those two paths only, so there is no message-specific
                    // declared requirement for them to legitimately satisfy.
                    // The exemption is scoped by PATH, not just by origin: a
                    // `ServerMessageRevision` write to any OTHER path (e.g.
                    // `/name`, `/system`) still passes through
                    // `declared_caps_for_path` like any other write — the
                    // structural cap check above would almost certainly deny
                    // such a write first (this origin's grant is `all: false`
                    // and holds only `READ`/`WRITE_FIELDS`), but the additive
                    // check must not be silently bypassed for a path this
                    // origin was never meant to touch.
                    let is_server_message_revision = cur.doc_type == crate::chat::MESSAGE_DOC_TYPE
                        && origin == WriteOrigin::ServerMessageRevision;
                    let access = if is_server_message_revision {
                        Access {
                            caps: [cap::READ.to_string(), cap::WRITE_FIELDS.to_string()]
                                .into_iter()
                                .collect(),
                            all: false,
                            see_gm_only: true,
                            is_owner: true,
                        }
                    } else {
                        // Effective owner joined from the LIVE linked actor inside this
                        // transaction — a linked token's owner is never stored on the token.
                        let upd_owner = Self::load_effective_owner(&mut *tx, &cur).await?;
                        resolve_access_world(
                            ctx.user_id,
                            ctx.world_role,
                            &cur,
                            &world_defaults.grants_for(&cur.doc_type),
                            upd_owner,
                        )
                    };
                    // Field-level OCC: every change's pre-image must equal the
                    // current value at its pointer (absent reads as Null).
                    let whole = serde_json::to_value(&cur)?;
                    for ch in &*changes {
                        validation::validate_field_change(ch)?;
                        // Each field path requires its capability
                        // (`permission::required_cap_for_path`): an
                        // immutable envelope field (id, scope, source, ...) maps
                        // to no capability and is rejected for everyone.
                        // /system, /engine, /name, /base -> write_fields;
                        // /embedded -> manage_embedded; /permissions AND /owner
                        // -> edit_permissions. /owner is NOT immutable — it is
                        // an access-control field, writable by a GM (or an
                        // explicit edit_permissions grant) but never by an owner,
                        // since the DocRole::Owner floor excludes that cap.
                        let need = required_cap_for_path(&ch.path).ok_or(DataError::Forbidden)?;
                        // `CombatTransition` skips only the actor-holds-`need`
                        // test below, never `required_cap_for_path`'s mapping
                        // above: an immutable envelope path (`None`) is still
                        // rejected for every origin, this origin included.
                        if origin != WriteOrigin::CombatTransition && !access.has(need) {
                            // A `ServerMessageRevision` write to a message doc may
                            // ALSO write exactly `/permissions/property_overrides`
                            // (never any other `/permissions` subpath) without
                            // holding `cap::EDIT_PERMISSIONS` -- `handle_recalc_roll`
                            // needs this to register a freshly-appended
                            // `RecalcEntry`'s gm_only override pointer. Granting
                            // `EDIT_PERMISSIONS` to this origin instead would ALSO
                            // authorize rewriting `default`/`gm_role`/`users` -- the
                            // message's own audience-enforcement fields -- which
                            // this origin's `all: false` scoping deliberately
                            // excludes (see the `ServerMessageRevision` access-grant
                            // construction above). This exact-path admission widens
                            // nothing for any other doc_type/origin/path.
                            let is_recalc_override_write = is_server_message_revision
                                && ch.path == "/permissions/property_overrides";
                            if !is_recalc_override_write {
                                tracing::debug!(
                                    user = %ctx.user_id, path = %ch.path, capability = need,
                                    "intent denied: missing capability"
                                );
                                return Err(DataError::Forbidden);
                            }
                        }
                        // Declarative requirements are additive: a module/world
                        // may demand extra capabilities for a sub-path on top of
                        // the structural base above. SKIPPED only for a
                        // `ServerMessageRevision` write to exactly `/engine` or
                        // `/permissions/property_overrides` (see the
                        // `is_scoped_smr_write` doc above) — a world's
                        // `CapabilityRequirement` carries no `doc_type`, so an
                        // ancestor write to `/engine` would otherwise inherit a
                        // requirement declared for a wholly unrelated doc_type's
                        // field, blocking a GM's already-vetted moderation write.
                        // Any OTHER path under this origin still goes through
                        // this check, matching `is_recalc_override_write`'s
                        // exact-path shape above.
                        let is_scoped_smr_write = is_server_message_revision
                            && matches!(
                                ch.path.as_str(),
                                "/engine" | "/permissions/property_overrides"
                            );
                        if origin != WriteOrigin::CombatTransition && !is_scoped_smr_write {
                            for extra in declared_caps_for_path(&ch.path, &world_reqs) {
                                if !access.has(extra) {
                                    tracing::debug!(
                                        user = %ctx.user_id, path = %ch.path, capability = extra,
                                        "intent denied: missing declared capability"
                                    );
                                    return Err(DataError::Forbidden);
                                }
                            }
                        }
                        let actual = whole
                            .pointer(&ch.path)
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        // Numeric-aware: a whole-number-valued engine `f64` round-tripped
                        // through a JS client loses its Float-ness (PosInt/Float variant
                        // split), so raw `!=` here would spuriously Conflict an otherwise
                        // up-to-date write. See `values_semantically_eq` doc comment.
                        if !values_semantically_eq(&actual, &ch.old) {
                            return Err(DataError::Conflict(format!(
                                "stale pre-image at {}",
                                ch.path
                            )));
                        }
                    }
                    // One-active-combat-per-scene enforcement for an Update,
                    // run entirely HERE in Phase 1 -- never re-derived by
                    // Phase 2, which performs no independent recomputation of
                    // this invariant (see `scene_owner`'s doc comment above).
                    // Uses the same tolerant merge-simulation the pre-scan
                    // above uses; a merge/parse failure here is left to the
                    // authoritative `validate_engine_tree` pass in Phase 2 to
                    // surface.
                    if let Some(pre_engine) = combat_engine_of(&cur) {
                        if let Some(merged_engine) = merged_combat_engine(&cur, changes) {
                            if merged_engine.active {
                                let scene = merged_engine.scene_id;
                                Self::ensure_scene_owner_seeded(
                                    &mut *tx,
                                    world_id,
                                    scene,
                                    &mut scene_owner,
                                    &mut seeded_scenes,
                                    &deactivations_this_batch,
                                )
                                .await?;
                                match scene_owner.get(&scene) {
                                    Some(&owner) if owner != *doc_id => {
                                        return Err(DataError::Conflict(
                                            "an active combat already exists on this scene".into(),
                                        ));
                                    }
                                    _ => {
                                        scene_owner.insert(scene, *doc_id);
                                    }
                                }
                            } else if pre_engine.active {
                                // A genuine active-true -> false transition:
                                // free the PRE-merge scene (never the
                                // post-merge one) -- an Update that
                                // simultaneously moves a combat to a
                                // different scene AND deactivates it must
                                // free the scene it was actually active on,
                                // never the scene it is moving to, which may
                                // already hold an unrelated genuinely-active
                                // combat this batch never touches.
                                let scene = pre_engine.scene_id;
                                Self::ensure_scene_owner_seeded(
                                    &mut *tx,
                                    world_id,
                                    scene,
                                    &mut scene_owner,
                                    &mut seeded_scenes,
                                    &deactivations_this_batch,
                                )
                                .await?;
                                // Release only when THIS combat is the
                                // current owner in the simulation -- an
                                // unrelated combat's deactivation must never
                                // remove a DIFFERENT combat's claim on this
                                // scene, even if that different combat is the
                                // scene's real, currently-active occupant.
                                if scene_owner.get(&scene) == Some(&*doc_id) {
                                    scene_owner.remove(&scene);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Substitute the authoritative stored document into each Delete op: the
        // client supplies only the id to delete, so the broadcast and the
        // world_events log must carry server state, never the client body
        // (whose forged permissions would otherwise drive per-recipient
        // redaction and persist into the authoritative event log).
        let mut authoritative_ops = Vec::with_capacity(ops.len());
        for op in ops {
            match op {
                Operation::Delete { doc } => {
                    // A scene/parent delete expands to explicit Delete ops for
                    // every descendant (children before parents) so each removal
                    // is an individually reversible op (#8) and broadcasts to
                    // clients (#2) — never a silent FK cascade. Descendants are
                    // discovered here in Phase 2, so each is authorized against
                    // its stored doc with the same DELETE gate Phase 1 applies to
                    // the submitted op.
                    for desc in Self::descendants_first(&mut tx, doc.id).await? {
                        let cur = Self::load_document(&mut *tx, desc).await?.ok_or_else(|| {
                            DataError::Conflict(format!("descendant {desc} missing"))
                        })?;
                        check_command_scope(&cur, world_id)?;
                        let desc_owner = Self::load_effective_owner(&mut *tx, &cur).await?;
                        // `CombatTransition` skips this capability gate — see the
                        // Create arm's matching comment above.
                        if origin != WriteOrigin::CombatTransition
                            && !resolve_access_world(
                                ctx.user_id,
                                ctx.world_role,
                                &cur,
                                &world_defaults.grants_for(&cur.doc_type),
                                desc_owner,
                            )
                            .has(cap::DELETE)
                        {
                            return Err(DataError::Forbidden);
                        }
                        authoritative_ops.push(Operation::Delete { doc: cur });
                    }
                    let cur = Self::load_document(&mut *tx, doc.id)
                        .await?
                        .ok_or_else(|| {
                            DataError::Conflict(format!("document {} missing", doc.id))
                        })?;
                    authoritative_ops.push(Operation::Delete { doc: cur });
                }
                other => authoritative_ops.push(other),
            }
        }

        // Phase 2 — allocate seq, apply, log. Identical machinery to
        // apply_command; authorization above has already cleared every op.
        let seq: i64 = sqlx::query("UPDATE worlds SET seq = seq + 1 WHERE id = ? RETURNING seq")
            .bind(world_id.to_string())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(DataError::NotFound)?
            .get("seq");

        let mut sequenced = Command {
            seq,
            world_id,
            author: ctx.user_id,
            ts,
            ops: authoritative_ops,
        };

        // Rebuilt in place of `sequenced.ops`: identical to the input ops
        // except an Update's `FieldChange.new` under `/engine`(/*) is
        // renormalized to the validated post-image (see below). Since
        // `sequenced` is what gets broadcast AND logged to `world_events`
        // (INSERT further down) AND replayed by `events_since`, this is the
        // single chokepoint that keeps all three in sync with the persisted
        // row.
        let mut post_images: std::collections::HashMap<Uuid, Document> =
            std::collections::HashMap::new();
        let mut deleted_created_seqs: std::collections::HashMap<Uuid, i64> =
            std::collections::HashMap::new();
        let mut normalized_ops = Vec::with_capacity(sequenced.ops.len());
        for op in &sequenced.ops {
            match op {
                Operation::Create { doc } => {
                    Self::upsert_document(&mut tx, doc, seq).await?;
                    post_images.insert(doc.id, doc.clone());
                    normalized_ops.push(op.clone());
                }
                Operation::Delete { doc } => {
                    if let Some(cs) = Self::document_created_seq(&mut *tx, doc.id).await? {
                        deleted_created_seqs.insert(doc.id, cs);
                    }
                    Self::delete_document_tx(&mut tx, doc.id).await?;
                    normalized_ops.push(op.clone());
                }
                Operation::Move {
                    doc_id, parent_id, ..
                } => {
                    let cur = Self::load_document(&mut *tx, *doc_id)
                        .await?
                        .ok_or(DataError::NotFound)?;
                    if cur.parent_id == *parent_id {
                        // No-op: carried in the log for invertibility;
                        // nothing written, nothing bumped, no hooks run.
                        post_images.insert(*doc_id, cur);
                        normalized_ops.push(op.clone());
                    } else {
                        let mut doc = cur;
                        doc.parent_id = *parent_id;
                        doc.updated_at = ts;
                        Self::upsert_document(&mut tx, &doc, seq).await?;
                        // A folder's ancestor names are derived tags on every
                        // asset beneath it; re-parenting recomputes the whole
                        // subtree in this tx, same as the rename hook below.
                        if doc.doc_type == crate::data::engine::ASSET_FOLDER_DOC_TYPE {
                            Self::refresh_derived_tags_for_folder_subtree(&mut tx, doc.id).await?;
                        }
                        post_images.insert(*doc_id, doc);
                        normalized_ops.push(op.clone());
                    }
                }
                Operation::Update { doc_id, changes } => {
                    let row = sqlx::query("SELECT json FROM documents WHERE id = ?")
                        .bind(doc_id.to_string())
                        .fetch_optional(&mut *tx)
                        .await?
                        .ok_or(DataError::NotFound)?;
                    let mut value: serde_json::Value =
                        serde_json::from_str(row.get::<String, _>("json").as_str())?;
                    for ch in changes {
                        // THE `apply_field_change` mutation rule. Never
                        // re-derive the remove/set branch here: the derived scene ECS
                        // mirrors these same changes and must land the same value.
                        apply_field_change(&mut value, ch)?;
                    }
                    let mut doc: Document = serde_json::from_value(value)?;
                    if doc.id != *doc_id {
                        return Err(DataError::OpFailed(
                            "update must not change the document id".into(),
                        ));
                    }
                    check_command_scope(&doc, world_id)?;
                    // Body cap re-checked post-merge: the merged result, not the
                    // pre-image, is what gets stored.
                    validation::validate_system_size(&doc)?;
                    validation::validate_property_overrides(&doc)?;
                    // Engine band re-validated + normalized post-merge (mutates
                    // `doc.engine` in place to the re-serialized validated
                    // struct — see `validate_engine_tree`'s doc comment).
                    validation::validate_engine_tree(&mut doc)?;
                    validation::validate_containment(&doc)?;
                    // One-active-combat-per-scene is validated ONLY in Phase
                    // 1 (see `apply_intent`'s Update arm there, and the
                    // `scene_owner` doc comment) -- this phase trusts that
                    // decision and performs no recomputation of it, the same
                    // way it trusts Phase 1's OCC/capability/containment/
                    // singleton decisions for every other check.
                    // Tier-2 structural schema gate on the MERGED post-image
                    // (existing row + applied `FieldChange`s), matching
                    // `validate_engine_tree` above: never the pre-image.
                    validation::validate_system_schema_tree(&doc, &world_schemas)?;
                    doc.updated_at = ts;
                    Self::upsert_document(&mut tx, &doc, seq).await?;
                    post_images.insert(*doc_id, doc.clone());
                    // A folder's name is a derived tag on every asset beneath
                    // it; any Update to an `asset_folder` (rename being the
                    // one that matters) recomputes that subtree in this tx.
                    if doc.doc_type == crate::data::engine::ASSET_FOLDER_DOC_TYPE {
                        Self::refresh_derived_tags_for_folder_subtree(&mut tx, doc.id).await?;
                    }

                    // `validate_engine_tree` above normalizes `doc.engine` (a
                    // JSON-number literal coerced to its typed f64
                    // representation; an unknown key smuggled into a
                    // tagged-enum sub-object dropped by the
                    // deserialize-then-reserialize round trip), and that
                    // normalization reaches `doc` alone — the caller's own
                    // `FieldChange.new` values are untouched by it.
                    // Re-derive each `/engine`(/*) `FieldChange.new` from the
                    // SAME validated post-image so the broadcast delta and the
                    // `world_events` log entry (and therefore every future
                    // `events_since` replay) carry the identical normalized
                    // value the row was stored with — never the raw
                    // client-submitted JSON. `/system`-prefixed changes are
                    // untouched: only the structurally-typed engine band goes
                    // through `validate_engine_tree`.
                    let normalized_doc_json = serde_json::to_value(&doc)?;
                    let normalized_changes: Vec<FieldChange> = changes
                        .iter()
                        .map(|ch| {
                            if ch.path == "/engine" || ch.path.starts_with("/engine/") {
                                if let Some(v) = normalized_doc_json.pointer(&ch.path) {
                                    return FieldChange {
                                        remove: false,
                                        path: ch.path.clone(),
                                        old: ch.old.clone(),
                                        new: v.clone(),
                                    };
                                }
                            }
                            ch.clone()
                        })
                        .collect();
                    normalized_ops.push(Operation::Update {
                        doc_id: *doc_id,
                        changes: normalized_changes,
                    });
                }
            }
        }
        sequenced.ops = normalized_ops;

        let world_gm_at_commit: std::collections::HashMap<Uuid, bool> =
            Self::world_member_roles(&mut *tx, world_id)
                .await?
                .into_iter()
                .map(|(uid, role)| (uid, role == WorldRole::Gm))
                .collect();
        let mut per_op = Vec::with_capacity(sequenced.ops.len());
        for op in &sequenced.ops {
            per_op.push(Some(
                Self::build_op_snapshot(
                    &mut tx,
                    op,
                    &post_images,
                    &deleted_created_seqs,
                    &pre_permissions,
                    &pre_owners,
                )
                .await?,
            ));
        }
        let stored = StoredCommand {
            command: sequenced,
            snapshot: CommandSnapshot {
                per_op,
                world_gm_at_commit,
            },
        };

        sqlx::query("INSERT INTO world_events (world_id, seq, author_id, ts, command_json) VALUES (?, ?, ?, ?, ?)")
            .bind(stored.command.world_id.to_string())
            .bind(seq)
            .bind(stored.command.author.to_string())
            .bind(stored.command.ts)
            .bind(serde_json::to_string(&stored)?)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(stored)
    }

    async fn get_document(&self, id: Uuid) -> Result<Option<Document>, DataError> {
        let row = sqlx::query("SELECT json FROM documents WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => Ok(Some(serde_json::from_str(
                r.get::<String, _>("json").as_str(),
            )?)),
            None => Ok(None),
        }
    }

    async fn get_document_with_created_seq(
        &self,
        id: Uuid,
    ) -> Result<Option<(Document, i64)>, DataError> {
        let row = sqlx::query("SELECT json, created_seq FROM documents WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => {
                let doc: Document = serde_json::from_str(r.get::<String, _>("json").as_str())?;
                let created_seq: i64 = r.get("created_seq");
                Ok(Some((doc, created_seq)))
            }
            None => Ok(None),
        }
    }

    async fn effective_owner_of(&self, doc: &Document) -> Result<Option<Uuid>, DataError> {
        Self::load_effective_owner(&self.pool, doc).await
    }

    async fn query_documents(
        &self,
        world_id: Uuid,
        doc_type: &str,
    ) -> Result<Vec<Document>, DataError> {
        let rows = sqlx::query(
            "SELECT json FROM documents WHERE world_id = ? AND doc_type = ? ORDER BY id",
        )
        .bind(world_id.to_string())
        .bind(doc_type)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(r.get::<String, _>("json").as_str())?))
            .collect()
    }

    async fn query_documents_by_types(
        &self,
        world_id: Uuid,
        doc_types: &[&str],
    ) -> Result<Vec<Document>, DataError> {
        if doc_types.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = std::iter::repeat_n("?", doc_types.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT json FROM documents WHERE world_id = ? AND doc_type IN ({placeholders}) ORDER BY id"
        );
        // The interpolated segment is only a fixed-count `?, ?, ...` placeholder list built
        // from `doc_types.len()`, never caller-supplied string content — every actual value
        // (world_id, each doc_type) is bound as a parameter below, so this is not injectable.
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(world_id.to_string());
        for doc_type in doc_types {
            query = query.bind(*doc_type);
        }
        let rows = query.fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(r.get::<String, _>("json").as_str())?))
            .collect()
    }

    async fn query_all_documents(&self, world_id: Uuid) -> Result<Vec<Document>, DataError> {
        let rows = sqlx::query("SELECT json FROM documents WHERE world_id = ? ORDER BY id")
            .bind(world_id.to_string())
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(r.get::<String, _>("json").as_str())?))
            .collect()
    }

    async fn query_children(&self, parent: Uuid) -> Result<Vec<Document>, DataError> {
        let rows = sqlx::query("SELECT json FROM documents WHERE parent_id = ? ORDER BY id")
            .bind(parent.to_string())
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(r.get::<String, _>("json").as_str())?))
            .collect()
    }

    async fn query_scene_entities(&self, world: Uuid) -> Result<Vec<Document>, DataError> {
        let rows = sqlx::query(
            "SELECT json FROM documents WHERE world_id = ? \
             AND (doc_type = 'scene' OR parent_id IS NOT NULL) ORDER BY id",
        )
        .bind(world.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(r.get::<String, _>("json").as_str())?))
            .collect()
    }

    async fn documents_by_source(
        &self,
        pack: Option<&str>,
        source_id: Uuid,
    ) -> Result<Vec<Document>, DataError> {
        let rows = match pack {
            Some(p) => {
                sqlx::query(
                    "SELECT json FROM documents WHERE source_pack = ? AND source_id = ? ORDER BY id",
                )
                .bind(p)
                .bind(source_id.to_string())
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query(
                    "SELECT json FROM documents WHERE source_pack IS NULL AND source_id = ? ORDER BY id",
                )
                .bind(source_id.to_string())
                .fetch_all(&self.pool)
                .await?
            }
        };
        rows.into_iter()
            .map(|r| Ok(serde_json::from_str(r.get::<String, _>("json").as_str())?))
            .collect()
    }

    async fn events_since(
        &self,
        world_id: Uuid,
        seq: i64,
    ) -> Result<Vec<StoredCommand>, DataError> {
        let rows = sqlx::query(
            "SELECT command_json FROM world_events WHERE world_id = ? AND seq > ? ORDER BY seq",
        )
        .bind(world_id.to_string())
        .bind(seq)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                StoredCommand::from_stored_json(r.get::<String, _>("command_json").as_str())
                    .map_err(DataError::from)
            })
            .collect()
    }

    async fn get_world(&self, id: Uuid) -> Result<Option<World>, DataError> {
        let row =
            sqlx::query("SELECT id, name, seq, created_at, updated_at FROM worlds WHERE id = ?")
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|r| World {
            id: Uuid::parse_str(r.get::<String, _>("id").as_str()).unwrap(),
            name: r.get("name"),
            seq: r.get("seq"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        }))
    }

    async fn member_role(&self, world: Uuid, user: Uuid) -> Result<Option<WorldRole>, DataError> {
        // Delegates to `SqliteRepository::member_role`, the inherent method of
        // the same name; method resolution on a concrete `SqliteRepository`
        // self prefers the inherent impl, so this is not infinite recursion.
        SqliteRepository::member_role(self, world, user).await
    }

    async fn member_id_by_username(
        &self,
        world: Uuid,
        username: &str,
    ) -> Result<Option<Uuid>, DataError> {
        // Delegates to the inherent method of the same name (see `member_role`
        // above for why this is not infinite recursion).
        SqliteRepository::member_id_by_username(self, world, username).await
    }

    async fn world_cap_defaults(&self, world: Uuid) -> Result<WorldCapDefaults, DataError> {
        match self.get_setting(&world_caps_key(world)).await? {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(WorldCapDefaults::default()),
        }
    }

    async fn world_cap_requirements(
        &self,
        world: Uuid,
    ) -> Result<Vec<CapabilityRequirement>, DataError> {
        match self.get_setting(&world_caps_req_key(world)).await? {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(Vec::new()),
        }
    }

    async fn world_contract_declarations(
        &self,
        world: Uuid,
    ) -> Result<Vec<ContractDeclaration>, DataError> {
        match self.get_setting(&world_contracts_key(world)).await? {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(Vec::new()),
        }
    }

    async fn world_schema_declarations(
        &self,
        world: Uuid,
    ) -> Result<Vec<SchemaDeclaration>, DataError> {
        match self.get_setting(&world_schemas_key(world)).await? {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(Vec::new()),
        }
    }

    async fn world_enabled_modules(&self, world: Uuid) -> Result<Vec<String>, DataError> {
        match self.get_setting(&world_modules_key(world)).await? {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(Vec::new()),
        }
    }

    async fn search(
        &self,
        ctx: &crate::data::membership::PermissionContext,
        world_id: Uuid,
        query: &str,
        limit: u32,
        cursor: Option<i64>,
    ) -> Result<crate::data::search::SearchPage, DataError> {
        use crate::data::search::{build_match, SearchHit, SearchPage};

        // Bound the candidates examined per request: a query matching many docs
        // the actor cannot read would otherwise page to exhaustion, one
        // get_document per candidate, on the single-writer pool. On hitting the
        // budget before `limit`, return a partial page + cursor to resume.
        const MAX_SCAN: i64 = 500;

        let limit = limit.clamp(1, 100) as usize;
        let Some(match_expr) = build_match(query) else {
            return Ok(SearchPage {
                hits: Vec::new(),
                next_cursor: None,
            });
        };
        let world_defaults = self.world_cap_defaults(world_id).await?;

        // Visibility-split index: a non-GM matches, scores, and snippets only
        // against `documents_fts_public` (GM-only properties stripped at
        // index time), so neither the MATCH (oracle), the bm25 score, nor the
        // snippet can reveal GM-only text. A GM/admin searches the separate
        // `documents_fts_gm` table. (Server admin resolves to the Gm world
        // role in `permission_context`.)
        //
        // TWO SEPARATE single-column tables, not two columns of one table:
        // SQLite FTS5's bm25() computes each row's document-length
        // normalization term from the token count of
        // the WHOLE ROW (every declared column combined), not just the
        // matched/weighted column — a documented FTS5 characteristic. In a
        // shared two-column table, per-column bm25() weight arguments zero a
        // column's term-frequency*IDF CONTRIBUTION but cannot remove its
        // tokens from that shared row-length denominator, so a non-GM
        // searcher's score still shifts by the sheer LENGTH of GM-only text
        // on the same row — even text that never matches the query. Separate
        // tables make each tier's row length genuinely isolated: a non-GM
        // query's table contains no GM-only text in any column of any row.
        let is_gm = ctx.world_role == WorldRole::Gm;
        let sql = if is_gm {
            "SELECT doc_id, bm25(documents_fts_gm) AS score, \
             snippet(documents_fts_gm, 0, '<mark>', '</mark>', '…', 16) AS snippet \
             FROM documents_fts_gm \
             WHERE documents_fts_gm MATCH ?1 AND world_id = ?2 \
             ORDER BY score LIMIT ?3 OFFSET ?4"
        } else {
            "SELECT doc_id, bm25(documents_fts_public) AS score, \
             snippet(documents_fts_public, 0, '<mark>', '</mark>', '…', 16) AS snippet \
             FROM documents_fts_public \
             WHERE documents_fts_public MATCH ?1 AND world_id = ?2 \
             ORDER BY score LIMIT ?3 OFFSET ?4"
        };

        // Iterate the BM25-ranked candidates from `cursor`, reading each doc and
        // keeping only those the actor may read, until `limit` readable hits are
        // collected, the candidates are exhausted, or the scan budget is spent.
        // Over-iteration here is what prevents redaction from producing a short
        // page. A negative client cursor is clamped to the start.
        let start: i64 = cursor.unwrap_or(0).max(0);
        let mut offset: i64 = start;
        let mut hits: Vec<SearchHit> = Vec::with_capacity(limit);
        let batch: i64 = (limit as i64).clamp(16, MAX_SCAN);
        let mut next_cursor: Option<i64> = None;

        'outer: loop {
            let rows = sqlx::query(sql)
                .bind(&match_expr)
                .bind(world_id.to_string())
                .bind(batch)
                .bind(offset)
                .fetch_all(&self.pool)
                .await?;

            if rows.is_empty() {
                break; // exhausted; next_cursor stays None
            }

            for row in &rows {
                offset += 1;
                let doc_id: String = row.get("doc_id");
                let doc_id =
                    Uuid::parse_str(&doc_id).map_err(|e| DataError::OpFailed(e.to_string()))?;
                let Some(doc) = self.get_document(doc_id).await? else {
                    continue;
                };
                // One extra pool read per linked-token candidate, bounded by
                // `MAX_SCAN`; the ws hot path never enters here.
                let owner = Self::load_effective_owner(&self.pool, &doc).await?;
                let access = resolve_access_world(
                    ctx.user_id,
                    ctx.world_role,
                    &doc,
                    &world_defaults.grants_for(&doc.doc_type),
                    owner,
                );
                if !access.has(cap::READ) {
                    continue;
                }
                let document = match crate::data::permission::filter_properties(&doc, &access) {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::warn!(doc_id = %doc.id, error = %e, "omitting search hit");
                        continue;
                    }
                };
                hits.push(SearchHit {
                    document,
                    score: row.get("score"),
                    snippet: row.get("snippet"),
                });
                if hits.len() == limit {
                    // More candidates may remain; hand back the rank offset.
                    next_cursor = Some(offset);
                    break 'outer;
                }
            }

            if offset - start >= MAX_SCAN {
                // Scan budget spent before `limit`; resume from here next page.
                next_cursor = Some(offset);
                break;
            }
            if (rows.len() as i64) < batch {
                break; // last batch was partial → no more candidates
            }
        }

        Ok(SearchPage { hits, next_cursor })
    }

    async fn get_explored(&self, scene: Uuid, user: Uuid) -> Result<Option<Vec<u8>>, DataError> {
        // Delegate to the concrete method on SqliteRepository (same query, exposed
        // on the trait so Room::publish can call it through &dyn Repository).
        SqliteRepository::get_explored(self, scene, user).await
    }

    async fn get_link_preview_cache(
        &self,
        url: &str,
    ) -> Result<Option<crate::data::repository::LinkPreviewCacheRow>, DataError> {
        SqliteRepository::get_link_preview_cache(self, url).await
    }

    async fn upsert_link_preview_cache(
        &self,
        url: &str,
        title: Option<&str>,
        description: Option<&str>,
        fetched_at_ms: i64,
    ) -> Result<(), DataError> {
        SqliteRepository::upsert_link_preview_cache(self, url, title, description, fetched_at_ms)
            .await
    }

    async fn set_link_preview_cache_image(
        &self,
        url: &str,
        image_asset_id: Uuid,
    ) -> Result<(), DataError> {
        SqliteRepository::set_link_preview_cache_image(self, url, image_asset_id).await
    }
}

/// Settings key holding a world's default capability grants (JSON).
fn world_caps_key(world: Uuid) -> String {
    format!("world_caps:{world}")
}

/// Settings key holding a world's declarative capability requirements (JSON).
fn world_caps_req_key(world: Uuid) -> String {
    format!("world_caps_req:{world}")
}

/// Settings key holding a world's UI contract declarations (JSON).
fn world_contracts_key(world: Uuid) -> String {
    format!("world_contracts:{world}")
}

/// Settings key holding a world's structural schema declarations (JSON).
fn world_schemas_key(world: Uuid) -> String {
    format!("world_schemas:{world}")
}

/// Settings key holding a world's enabled installed-module ids (JSON).
fn world_modules_key(world: Uuid) -> String {
    format!("world_modules:{world}")
}

/// The per-world `settings` keys. SINGLE SOURCE for "what world-scoped
/// settings blobs exist": `delete_world`'s purge iterates this array, so a
/// new per-world blob added here is purged automatically (never-fork; adding
/// a sixth key fn without extending this array is the drift this prevents).
fn world_settings_keys(world: Uuid) -> [String; 5] {
    [
        world_caps_key(world),
        world_caps_req_key(world),
        world_contracts_key(world),
        world_schemas_key(world),
        world_modules_key(world),
    ]
}

mod assets;

#[cfg(test)]
mod tests;
