// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use async_trait::async_trait;
use sqlx::sqlite::SqlitePoolOptions;
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
    CONDITION_REGISTRY_DOC_TYPE, FACTION_REGISTRY_DOC_TYPE, WORLD_SETTINGS_DOC_TYPE,
};
use crate::data::permission::{
    cap, declared_caps_for_document, declared_caps_for_path, required_cap_for_path,
    resolve_access_world, Access,
};
use crate::data::repository::Repository;
use crate::data::validation;
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
    crate::chat::CHAT_SETTINGS_DOC_TYPE,
    crate::chat::DICE_SETTINGS_DOC_TYPE,
];

/// One-level merge of a single key into `map`: when both the existing
/// `map[key]` and the incoming `value` are JSON objects, merges `value`'s
/// entries into the existing object (each of THOSE entries replaces
/// wholesale — this never recurses past one level, so an opaque leaf blob
/// like `panelLayout` is never deep-merged); otherwise `value` replaces
/// `map[key]` wholesale. The shared leaf-key merge step behind
/// `SqliteRepository::merge_ui_state`'s per-top-level-key and per-`worlds.<id>`
/// merge rule.
fn merge_one_level(
    map: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: &serde_json::Value,
) {
    let existing_is_object = map.get(key).is_some_and(serde_json::Value::is_object);
    if existing_is_object && value.is_object() {
        // Safe: `existing_is_object` just confirmed `map[key]` is present and an object.
        let existing_obj = map
            .get_mut(key)
            .and_then(serde_json::Value::as_object_mut)
            .expect("existing_is_object confirmed map[key] is a present object");
        for (k, v) in value.as_object().expect("value.is_object() checked above") {
            existing_obj.insert(k.clone(), v.clone());
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

/// SQLite-backed storage. Holds a connection pool; migrations are embedded
/// from `migrations/` and run at connect time.
pub struct SqliteRepository {
    /// Single-connection pool: the one writer serializing every transaction.
    pool: SqlitePool,
}

impl SqliteRepository {
    /// Connect to `url` (e.g. "sqlite::memory:" or "sqlite:///path/to.db")
    /// and run migrations. Foreign keys are enabled per connection.
    pub async fn connect(url: &str) -> Result<Self, DataError> {
        let pool = SqlitePoolOptions::new()
            // Single writer connection serializes apply_command transactions,
            // avoiding SQLITE_BUSY contention on the per-world seq allocation.
            .max_connections(1)
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    sqlx::query("PRAGMA foreign_keys = ON;")
                        .execute(conn)
                        .await?;
                    Ok(())
                })
            })
            .connect(url)
            .await?;
        sqlx::migrate!()
            .run(&pool)
            .await
            .map_err(sqlx::Error::from)?;
        Ok(Self { pool })
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

    /// Insert a new asset record. `version` starts at 1.
    pub async fn insert_asset(&self, a: &crate::data::asset::Asset) -> Result<(), DataError> {
        sqlx::query(
            "INSERT INTO assets \
             (id, world_id, storage_key, original_name, content_type, byte_size, created_by, created_at, version) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(a.id.to_string())
        .bind(a.world_id.to_string())
        .bind(&a.storage_key)
        .bind(&a.original_name)
        .bind(&a.content_type)
        .bind(a.byte_size)
        .bind(a.created_by.map(|u| u.to_string()))
        .bind(a.created_at)
        .bind(a.version)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Map an `assets` row to the `Asset` struct (uuid columns parse from TEXT).
    ///
    /// # Examples
    ///
    /// ```text
    /// let asset = Self::asset_from_row(&row)?;
    /// ```
    fn asset_from_row(
        row: &sqlx::sqlite::SqliteRow,
    ) -> Result<crate::data::asset::Asset, DataError> {
        let parse = |s: String| Uuid::parse_str(&s).map_err(|e| DataError::OpFailed(e.to_string()));
        Ok(crate::data::asset::Asset {
            id: parse(row.get::<String, _>("id"))?,
            world_id: parse(row.get::<String, _>("world_id"))?,
            storage_key: row.get("storage_key"),
            original_name: row.get("original_name"),
            content_type: row.get("content_type"),
            byte_size: row.get("byte_size"),
            created_by: row
                .get::<Option<String>, _>("created_by")
                .map(parse)
                .transpose()?,
            created_at: row.get("created_at"),
            version: row.get("version"),
        })
    }

    /// Fetch one asset row by id, or `None` if absent.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// assert!(repo.get_asset(uuid::Uuid::nil()).await?.is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_asset(
        &self,
        id: Uuid,
    ) -> Result<Option<crate::data::asset::Asset>, DataError> {
        let row = sqlx::query("SELECT * FROM assets WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| Self::asset_from_row(&r)).transpose()
    }

    /// Swap the bytes behind a stable id; bump and return the new version.
    pub async fn replace_asset_bytes(
        &self,
        id: Uuid,
        storage_key: &str,
        content_type: &str,
        byte_size: i64,
    ) -> Result<i64, DataError> {
        let v: i64 = sqlx::query(
            "UPDATE assets SET storage_key = ?, content_type = ?, byte_size = ?, version = version + 1 \
             WHERE id = ? RETURNING version",
        )
        .bind(storage_key)
        .bind(content_type)
        .bind(byte_size)
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(DataError::NotFound)?
        .get("version");
        Ok(v)
    }

    /// Remove the record, returning it (so the caller can delete the file).
    /// Single atomic `DELETE ... RETURNING` so two concurrent deletes can't both
    /// observe the row and double-fire side effects (file remove + broadcast) —
    /// only the call that actually removes the row gets `Some`.
    pub async fn delete_asset(
        &self,
        id: Uuid,
    ) -> Result<Option<crate::data::asset::Asset>, DataError> {
        let row = sqlx::query("DELETE FROM assets WHERE id = ? RETURNING *")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| Self::asset_from_row(&r)).transpose()
    }

    /// All asset rows for `world`, newest first.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let none = repo.list_assets_by_world(uuid::Uuid::nil()).await?;
    /// assert!(none.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_assets_by_world(
        &self,
        world: Uuid,
    ) -> Result<Vec<crate::data::asset::Asset>, DataError> {
        let rows = sqlx::query("SELECT * FROM assets WHERE world_id = ? ORDER BY created_at, id")
            .bind(world.to_string())
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(Self::asset_from_row).collect()
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
    /// otherwise replace `stored[K]` wholesale. Absent keys are untouched.
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

    /// Upsert a document row from its envelope, stamping `seq`, and rewrite its
    /// FTS index row in the same transaction (crash-consistent). Takes a
    /// `&mut SqliteConnection` because it runs multiple statements; callers pass
    /// `&mut *tx`.
    async fn upsert_document(
        conn: &mut sqlx::SqliteConnection,
        doc: &Document,
        seq: i64,
    ) -> Result<(), DataError> {
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
        let json = serde_json::to_string(doc)?;
        sqlx::query(
            "INSERT INTO documents (id, scope_kind, world_id, pack, doc_type, schema_version, \
             source_id, source_pack, source_version, owner_id, parent_id, seq, json, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
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
        .bind(json)
        .bind(doc.created_at)
        .bind(doc.updated_at)
        .execute(&mut *conn)
        .await?;
        // Two SEPARATE single-column tables, not two columns of one table:
        // bm25()'s row-length normalization is computed from the WHOLE ROW
        // (all columns), so a shared table would let a non-GM query's score
        // be shifted by the mere LENGTH of GM-only text on the same row even
        // when column weights zero out that column's term-frequency
        // contribution (see the FTS section of migrations/0001_init.sql).
        // Separate tables make each tier's row length genuinely isolated.
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
    /// evolution, hand-edited rows). `apply_command` validates on write as
    /// of this gate, so it can no longer seed such fixtures; this is not a
    /// production code path and must stay `#[cfg(test)]`-only.
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
    async fn apply_command(&self, cmd: UnsequencedCommand) -> Result<Command, DataError> {
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
                    Self::upsert_document(&mut tx, &doc, seq).await?;
                    normalized_ops.push(Operation::Create { doc });
                }
                Operation::Delete { doc } => {
                    check_command_scope(doc, sequenced.world_id)?;
                    Self::delete_document_tx(&mut tx, doc.id).await?;
                    normalized_ops.push(op.clone());
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

        // Append to the log.
        sqlx::query("INSERT INTO world_events (world_id, seq, author_id, ts, command_json) VALUES (?, ?, ?, ?, ?)")
            .bind(sequenced.world_id.to_string())
            .bind(seq)
            .bind(sequenced.author.to_string())
            .bind(sequenced.ts)
            .bind(serde_json::to_string(&sequenced)?)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(sequenced)
    }

    async fn apply_intent(
        &self,
        ctx: &crate::data::membership::PermissionContext,
        world_id: Uuid,
        mut ops: Vec<Operation>,
        ts: i64,
        origin: WriteOrigin,
    ) -> Result<Command, DataError> {
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
        for op in &mut ops {
            match op {
                Operation::Create { doc } => {
                    check_command_scope(doc, world_id)?;
                    validation::validate_system_size(doc)?;
                    validation::validate_property_overrides(doc)?;
                    validation::validate_engine_tree(doc)?;
                    validation::validate_system_schema_tree(doc, &world_schemas)?;
                    // A self-referential parent_id satisfies the self-FK and
                    // commits, then poisons the doc's deletion (the descendant
                    // walk would loop). Reject it; and when the parent already
                    // exists it must be in this world (an unborn same-command
                    // parent is left to the FK at apply time, so batched
                    // scene+children creates still pass).
                    if let Some(pid) = doc.parent_id {
                        if pid == doc.id {
                            return Err(DataError::OpFailed(
                                "document cannot be its own parent".into(),
                            ));
                        }
                        if let Some(parent) = Self::load_document(&mut *tx, pid).await? {
                            check_command_scope(&parent, world_id)?;
                        }
                    }
                    let create_owner = Self::load_effective_owner(&mut *tx, doc).await?;
                    let access = resolve_access_world(
                        ctx.user_id,
                        ctx.world_role,
                        doc,
                        &world_defaults.grants_for(&doc.doc_type),
                        create_owner,
                    );
                    if !access.has(cap::WRITE_FIELDS) {
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
                    if ctx.world_role != WorldRole::Gm
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
                    if !resolve_access_world(
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
                    // server edit/delete handlers, never derivable from any
                    // wire frame — re-opens this path for their sanitized
                    // authoritative revision; the ordinary WRITE_FIELDS/OCC
                    // checks below still apply on top of it.
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
                    // (`handle_edit_message`/`handle_delete_message`) already
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
                    // CAVEAT: this concrete cap set does NOT auto-satisfy an ADDITIVE
                    // `declared_caps_for_path` world/module requirement on a
                    // message `/engine` (sub-)path (checked further below).
                    // No first-party module declares one today, so this is
                    // inert; if one is ever added for `doc_type: "message"`,
                    // it would block a GM's already-vetted moderation write —
                    // review this branch again before adding such a
                    // requirement.
                    let access = if cur.doc_type == crate::chat::MESSAGE_DOC_TYPE
                        && origin == WriteOrigin::ServerMessageRevision
                    {
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
                    for ch in changes {
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
                        if !access.has(need) {
                            tracing::debug!(
                                user = %ctx.user_id, path = %ch.path, capability = need,
                                "intent denied: missing capability"
                            );
                            return Err(DataError::Forbidden);
                        }
                        // Declarative requirements are additive: a module/world
                        // may demand extra capabilities for a sub-path on top of
                        // the structural base above.
                        for extra in declared_caps_for_path(&ch.path, &world_reqs) {
                            if !access.has(extra) {
                                tracing::debug!(
                                    user = %ctx.user_id, path = %ch.path, capability = extra,
                                    "intent denied: missing declared capability"
                                );
                                return Err(DataError::Forbidden);
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
                        if !resolve_access_world(
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
        let mut normalized_ops = Vec::with_capacity(sequenced.ops.len());
        for op in &sequenced.ops {
            match op {
                Operation::Create { doc } => {
                    Self::upsert_document(&mut tx, doc, seq).await?;
                    normalized_ops.push(op.clone());
                }
                Operation::Delete { doc } => {
                    Self::delete_document_tx(&mut tx, doc.id).await?;
                    normalized_ops.push(op.clone());
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
                    // Tier-2 structural schema gate on the MERGED post-image
                    // (existing row + applied `FieldChange`s), matching
                    // `validate_engine_tree` above: never the pre-image.
                    validation::validate_system_schema_tree(&doc, &world_schemas)?;
                    doc.updated_at = ts;
                    Self::upsert_document(&mut tx, &doc, seq).await?;

                    // `validate_engine_tree` above normalizes `doc.engine` (a
                    // JSON-number literal coerced to its typed f64
                    // representation; an unknown key smuggled into a
                    // tagged-enum sub-object dropped by the
                    // deserialize-then-reserialize round trip) but that
                    // normalization only reached the persisted row until now.
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

        sqlx::query("INSERT INTO world_events (world_id, seq, author_id, ts, command_json) VALUES (?, ?, ?, ?, ?)")
            .bind(sequenced.world_id.to_string())
            .bind(seq)
            .bind(sequenced.author.to_string())
            .bind(ts)
            .bind(serde_json::to_string(&sequenced)?)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(sequenced)
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

    async fn events_since(&self, world_id: Uuid, seq: i64) -> Result<Vec<Command>, DataError> {
        let rows = sqlx::query(
            "SELECT command_json FROM world_events WHERE world_id = ? AND seq > ? ORDER BY seq",
        )
        .bind(world_id.to_string())
        .bind(seq)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                Ok(serde_json::from_str(
                    r.get::<String, _>("command_json").as_str(),
                )?)
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
        // TWO SEPARATE single-column tables (migrations/0001_init.sql),
        // not two columns of one table: SQLite FTS5's bm25() computes each
        // row's document-length normalization term from the token count of
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
                hits.push(SearchHit {
                    document: crate::data::permission::filter_properties(&doc, &access),
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

#[cfg(test)]
mod tests {
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
        let store = crate::auth::session::SqlxSqliteStore::new(repo.pool().clone());
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
        crate::auth::session::SqlxSqliteStore::new(repo.pool().clone())
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
            .unwrap();
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
            .unwrap();
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
            module_id: "nightfox".into(),
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
            .unwrap();

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
            .unwrap();

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
            .find(|c| c.seq == cmd.seq)
            .expect("replayed command present");
        let replayed_new = replayed_cmd
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
            .unwrap();

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
            .unwrap();

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
            .unwrap();

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

        let ids = vec!["actors-plus".to_string(), "nightfox".to_string()];
        r.set_world_enabled_modules(w.id, &ids).await.unwrap();
        assert_eq!(r.world_enabled_modules(w.id).await.unwrap(), ids);

        // A subsequent set fully replaces, not appends.
        r.set_world_enabled_modules(w.id, &["nightfox".to_string()])
            .await
            .unwrap();
        assert_eq!(
            r.world_enabled_modules(w.id).await.unwrap(),
            vec!["nightfox".to_string()]
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
        // message (satisfied without the fix), so this Update would otherwise
        // let them forge `kind`/`content` post-hoc. Must be rejected outright:
        // c-1 has no legitimate message-edit path.
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

        // Client origin: still blanket-rejected (c-1 invariant intact).
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
        assert_eq!(c1.seq, 1);
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
        assert_eq!(c2.seq, 2);
        assert_eq!(
            r.get_document(Uuid::from_u128(1))
                .await
                .unwrap()
                .unwrap()
                .system["hp"],
            serde_json::json!(3)
        );

        // Invert the update — hp returns to 10
        r.apply_command(c2.invert()).await.unwrap();
        assert_eq!(
            r.get_document(Uuid::from_u128(1))
                .await
                .unwrap()
                .unwrap()
                .system["hp"],
            serde_json::json!(10)
        );

        // Invert the create — document gone
        r.apply_command(c1.invert()).await.unwrap();
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
        assert_eq!(c.seq, 2);
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
        assert_eq!(tail[0].seq, 2);
        assert_eq!(tail[1].seq, 3);
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
        assert_eq!(c1.seq, 1);
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
        assert_eq!(ok.seq, 2);
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
        let Operation::Delete { doc } = &cmd.ops[0] else {
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
            module_id: "nightfox".into(),
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
            module_id: "nightfox".into(),
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
            module_id: "nightfox".into(),
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
        // An `actor` the player owns. `owner` keeps its pre-existing
        // provenance-only meaning on every non-`token` doc_type: it admits the
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
}
