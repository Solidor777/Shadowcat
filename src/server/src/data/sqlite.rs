use async_trait::async_trait;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::auth::role::ServerRole;
use crate::data::command::{set_pointer, Command, Operation, UnsequencedCommand};
use crate::data::document::{
    CapabilityGrants, CapabilityRequirement, ContractDeclaration, Document, Scope, World, WorldRole,
};
use crate::data::permission::{
    cap, declared_caps_for_document, declared_caps_for_path, required_cap_for_path,
    resolve_access_world,
};
use crate::data::repository::Repository;
use crate::data::validation;
use crate::data::DataError;

/// Auth-facing projection of a user row.
#[derive(Debug, Clone)]
pub struct UserRecord {
    pub id: Uuid,
    pub username: String,
    pub password_hash: Option<String>,
    pub server_role: ServerRole,
}

/// SQLite-backed storage. Holds a connection pool; migrations are embedded
/// from `migrations/` and run at connect time.
pub struct SqliteRepository {
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

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

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

    /// Change an existing member's role; `NotFound` if they are not a member.
    pub async fn set_role(
        &self,
        world: Uuid,
        user: Uuid,
        role: WorldRole,
    ) -> Result<(), DataError> {
        let res =
            sqlx::query("UPDATE world_members SET role = ? WHERE world_id = ? AND user_id = ?")
                .bind(serde_json::to_value(role)?.as_str().unwrap().to_string())
                .bind(world.to_string())
                .bind(user.to_string())
                .execute(&self.pool)
                .await?;
        if res.rows_affected() == 0 {
            return Err(DataError::NotFound);
        }
        Ok(())
    }

    pub async fn remove_member(&self, world: Uuid, user: Uuid) -> Result<(), DataError> {
        sqlx::query("DELETE FROM world_members WHERE world_id = ? AND user_id = ?")
            .bind(world.to_string())
            .bind(user.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_members(&self, world: Uuid) -> Result<Vec<(Uuid, WorldRole)>, DataError> {
        let rows = sqlx::query("SELECT user_id, role FROM world_members WHERE world_id = ?")
            .bind(world.to_string())
            .fetch_all(&self.pool)
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

    /// Replace the user's opaque UI-state JSON. `NotFound` if the user is absent.
    /// The string is stored verbatim; shape/size are validated at the HTTP boundary.
    pub async fn set_ui_state(&self, user: Uuid, json: &str) -> Result<(), DataError> {
        let res = sqlx::query("UPDATE users SET ui_state = ? WHERE id = ?")
            .bind(json)
            .bind(user.to_string())
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Err(DataError::NotFound);
        }
        Ok(())
    }

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

    pub async fn admin_exists(&self) -> Result<bool, DataError> {
        let row = sqlx::query("SELECT 1 FROM users WHERE server_role = 'admin' LIMIT 1")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    /// Insert an admin only if no admin exists yet, in a single guarded
    /// statement. Returns the new id, or `None` when an admin already exists.
    /// The single-writer pool serializes the insert, closing the first-run
    /// check-then-create race (two concurrent setups cannot both succeed).
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
             WHERE NOT EXISTS (SELECT 1 FROM users WHERE server_role = 'admin')",
        )
        .bind(id.to_string())
        .bind(username)
        .bind(password_hash)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok((res.rows_affected() == 1).then_some(id))
    }

    pub async fn get_setting(&self, key: &str) -> Result<Option<String>, DataError> {
        let row = sqlx::query("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get("value")))
    }

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

    /// Set a world's default capability grants (additive over the per-document
    /// floor). Stored as JSON in the settings table.
    pub async fn set_world_cap_defaults(
        &self,
        world: Uuid,
        grants: &CapabilityGrants,
    ) -> Result<(), DataError> {
        let json = serde_json::to_string(grants)?;
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

    /// Depth-first descendant ids of `root` within one transaction (children
    /// before parents), via the `parent_id` index. Excludes `root`. Used to
    /// expand a parent delete into per-descendant reversible Delete ops.
    async fn descendants_first(
        tx: &mut sqlx::SqliteConnection,
        root: Uuid,
    ) -> Result<Vec<Uuid>, DataError> {
        let mut out = Vec::new();
        let child_rows = sqlx::query("SELECT id FROM documents WHERE parent_id = ? ORDER BY id")
            .bind(root.to_string())
            .fetch_all(&mut *tx)
            .await?;
        for r in child_rows {
            let child = Uuid::parse_str(r.get::<String, _>("id").as_str())
                .map_err(|e| DataError::OpFailed(e.to_string()))?;
            // Recurse first so deeper descendants precede their parent.
            let mut sub = Box::pin(Self::descendants_first(&mut *tx, child)).await?;
            out.append(&mut sub);
            out.push(child);
        }
        Ok(out)
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
        // Keep the FTS index in lockstep with the row, inside the same tx.
        sqlx::query("DELETE FROM documents_fts WHERE doc_id = ?")
            .bind(doc.id.to_string())
            .execute(&mut *conn)
            .await?;
        // Two visibility-split columns: `content` indexes only non-GM-readable
        // text (GM-only properties stripped), `content_all` indexes everything.
        sqlx::query(
            "INSERT INTO documents_fts (content, content_all, doc_id, world_id) VALUES (?, ?, ?, ?)",
        )
        .bind(crate::data::search::index_content_public(doc))
        .bind(crate::data::search::index_content(doc))
        .bind(doc.id.to_string())
        .bind(world_id)
        .execute(&mut *conn)
        .await?;
        Ok(())
    }

    /// Remove a document's FTS row. Call alongside every document delete so the
    /// index never references a removed document.
    async fn delete_document_fts(
        conn: &mut sqlx::SqliteConnection,
        id: Uuid,
    ) -> Result<(), DataError> {
        sqlx::query("DELETE FROM documents_fts WHERE doc_id = ?")
            .bind(id.to_string())
            .execute(conn)
            .await?;
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

#[async_trait]
impl Repository for SqliteRepository {
    async fn apply_command(&self, cmd: UnsequencedCommand) -> Result<Command, DataError> {
        let mut tx = self.pool.begin().await?;

        // Allocate the next per-world seq from the single durable source.
        let seq: i64 = sqlx::query("UPDATE worlds SET seq = seq + 1 WHERE id = ? RETURNING seq")
            .bind(cmd.world_id.to_string())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(DataError::NotFound)?
            .get("seq");

        let sequenced = Command {
            seq,
            world_id: cmd.world_id,
            author: cmd.author,
            ts: cmd.ts,
            ops: cmd.ops,
        };

        // Apply each operation.
        for op in &sequenced.ops {
            match op {
                Operation::Create { doc } => {
                    check_command_scope(doc, sequenced.world_id)?;
                    Self::upsert_document(&mut tx, doc, seq).await?;
                }
                Operation::Delete { doc } => {
                    check_command_scope(doc, sequenced.world_id)?;
                    sqlx::query("DELETE FROM documents WHERE id = ?")
                        .bind(doc.id.to_string())
                        .execute(&mut *tx)
                        .await?;
                    Self::delete_document_fts(&mut tx, doc.id).await?;
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
                        set_pointer(&mut value, &ch.path, ch.new.clone())?;
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
                    // updated_at tracks last mutation; the command ts is authoritative.
                    doc.updated_at = sequenced.ts;
                    Self::upsert_document(&mut tx, &doc, seq).await?;
                }
            }
        }

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
        ops: Vec<Operation>,
        ts: i64,
    ) -> Result<Command, DataError> {
        // Load world default grants before opening the transaction: the
        // single-writer pool holds one connection, so a settings query mid-tx
        // would deadlock.
        let world_defaults = self.world_cap_defaults(world_id).await?;
        let world_reqs = self.world_cap_requirements(world_id).await?;
        let mut tx = self.pool.begin().await?;

        // Phase 1 — authorize, structurally validate, and check pre-images.
        // No row is mutated; any failure here drops the transaction, so the
        // per-world seq is never consumed by a rejected intent.
        for op in &ops {
            match op {
                Operation::Create { doc } => {
                    check_command_scope(doc, world_id)?;
                    validation::validate_system_size(doc)?;
                    let access =
                        resolve_access_world(ctx.user_id, ctx.world_role, doc, &world_defaults);
                    if !access.has(cap::WRITE_FIELDS) {
                        return Err(DataError::Forbidden);
                    }
                    // Create writes the whole body at once, so any declared
                    // requirement whose protected path is populated must be
                    // authorized — otherwise Create is a wholesale bypass of the
                    // declarative gate that Update enforces field-by-field.
                    let doc_json = serde_json::to_value(doc)?;
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
                    if !resolve_access_world(ctx.user_id, ctx.world_role, &cur, &world_defaults)
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
                    let access =
                        resolve_access_world(ctx.user_id, ctx.world_role, &cur, &world_defaults);
                    // Field-level OCC: every change's pre-image must equal the
                    // current value at its pointer (absent reads as Null).
                    let whole = serde_json::to_value(&cur)?;
                    for ch in changes {
                        validation::validate_field_path(&ch.path)?;
                        // Each field path requires its capability; an immutable
                        // envelope field (id, scope, owner, source, ...) maps to
                        // no capability and is rejected for everyone. /system ->
                        // write_fields, /embedded -> manage_embedded,
                        // /permissions -> edit_permissions.
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
                        if actual != ch.old {
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
                        if !resolve_access_world(ctx.user_id, ctx.world_role, &cur, &world_defaults)
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

        let sequenced = Command {
            seq,
            world_id,
            author: ctx.user_id,
            ts,
            ops: authoritative_ops,
        };

        for op in &sequenced.ops {
            match op {
                Operation::Create { doc } => Self::upsert_document(&mut tx, doc, seq).await?,
                Operation::Delete { doc } => {
                    sqlx::query("DELETE FROM documents WHERE id = ?")
                        .bind(doc.id.to_string())
                        .execute(&mut *tx)
                        .await?;
                    Self::delete_document_fts(&mut tx, doc.id).await?;
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
                        set_pointer(&mut value, &ch.path, ch.new.clone())?;
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
                    doc.updated_at = ts;
                    Self::upsert_document(&mut tx, &doc, seq).await?;
                }
            }
        }

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

    async fn query_children(&self, parent: Uuid) -> Result<Vec<Document>, DataError> {
        let rows = sqlx::query("SELECT json FROM documents WHERE parent_id = ? ORDER BY id")
            .bind(parent.to_string())
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

    async fn world_cap_defaults(&self, world: Uuid) -> Result<CapabilityGrants, DataError> {
        match self.get_setting(&world_caps_key(world)).await? {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(CapabilityGrants::default()),
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

        // Visibility-split index: a non-GM matches and snippets only the
        // GM-only-stripped `content` column, so neither the MATCH (oracle), the
        // bm25 score, nor the snippet can reveal GM-only text. A GM/admin
        // searches the full `content_all` column. (Server admin resolves to the
        // Gm world role in `permission_context`.)
        let is_gm = ctx.world_role == WorldRole::Gm;
        let column = if is_gm { "content_all" } else { "content" };
        // Column-filter the MATCH to the chosen column. This whole expression is
        // a bound parameter (?1) — `column` is a server-chosen literal, never user
        // input. The snippet column index must be a literal in the SQL, so two
        // static query strings are selected rather than building SQL at runtime.
        let scoped_match = format!("{{{column}}} : ({match_expr})");
        let sql = if is_gm {
            "SELECT doc_id, bm25(documents_fts) AS score, \
             snippet(documents_fts, 1, '<mark>', '</mark>', '…', 16) AS snippet \
             FROM documents_fts \
             WHERE documents_fts MATCH ?1 AND world_id = ?2 \
             ORDER BY score LIMIT ?3 OFFSET ?4"
        } else {
            "SELECT doc_id, bm25(documents_fts) AS score, \
             snippet(documents_fts, 0, '<mark>', '</mark>', '…', 16) AS snippet \
             FROM documents_fts \
             WHERE documents_fts MATCH ?1 AND world_id = ?2 \
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
                .bind(&scoped_match)
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
                let access =
                    resolve_access_world(ctx.user_id, ctx.world_role, &doc, &world_defaults);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::command::FieldChange;
    use crate::data::document::Source;

    async fn repo() -> SqliteRepository {
        SqliteRepository::connect("sqlite::memory:").await.unwrap()
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
                contract: "shadowcat.surface:sidebar".into(),
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

    #[tokio::test]
    async fn ui_state_round_trips_and_defaults_to_none() {
        let repo = repo().await;
        let user = repo
            .create_user("u", Some("hash"), ServerRole::User, 0)
            .await
            .unwrap();

        // Unset → None.
        assert_eq!(repo.get_ui_state(user).await.unwrap(), None);

        // Set then read back verbatim.
        repo.set_ui_state(user, r#"{"global":{"locale":"en"}}"#)
            .await
            .unwrap();
        assert_eq!(
            repo.get_ui_state(user).await.unwrap().as_deref(),
            Some(r#"{"global":{"locale":"en"}}"#)
        );

        // Replace (not merge).
        repo.set_ui_state(user, r#"{"global":{"locale":"fr"}}"#)
            .await
            .unwrap();
        assert_eq!(
            repo.get_ui_state(user).await.unwrap().as_deref(),
            Some(r#"{"global":{"locale":"fr"}}"#)
        );

        // Unknown user → NotFound.
        let ghost = Uuid::from_u128(1);
        assert!(matches!(
            repo.set_ui_state(ghost, "{}").await,
            Err(DataError::NotFound)
        ));
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
            source: None,
            owner: None,
            permissions: perms,
            embedded: Default::default(),
            parent_id: None,
            system,
            created_at: 0,
            updated_at: 0,
        }
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
        r.apply_intent(&gm_ctx, w.id, vec![Operation::Create { doc: d.clone() }], 1)
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
                    path: "/system/hp".into(),
                    old: serde_json::json!(10),
                    new: serde_json::json!(8),
                }],
            }],
            2,
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
                        path: "/system/vision/range".into(),
                        old: serde_json::json!(30),
                        new: serde_json::json!(60),
                    }],
                }],
                3,
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
                        path: "/system".into(),
                        old: serde_json::json!({ "vision": { "range": 30 }, "hp": 8 }),
                        new: serde_json::json!({ "vision": { "range": 99 }, "hp": 8 }),
                    }],
                }],
                3,
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
                    path: "/system/vision/range".into(),
                    old: serde_json::json!(30),
                    new: serde_json::json!(60),
                }],
            }],
            4,
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
            )
            .await;
        assert!(matches!(err, Err(DataError::Forbidden)));

        // CAN create a doc that does not populate the protected path.
        let mut plain = tests_doc(perms, serde_json::json!({ "hp": 10 }));
        plain.scope = Scope::World { world_id: w.id };
        plain.owner = Some(player);
        r.apply_intent(&player_ctx, w.id, vec![Operation::Create { doc: plain }], 2)
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
        r.apply_intent(&ctx, w.id, vec![Operation::Create { doc: d.clone() }], 1)
            .await
            .unwrap();
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM documents_fts WHERE documents_fts MATCH 'Goblin' AND world_id = ?",
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
                    path: "/system/name".into(),
                    old: serde_json::json!("Goblin"),
                    new: serde_json::json!("Orc"),
                }],
            }],
            2,
        )
        .await
        .unwrap();
        let goblin: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM documents_fts WHERE documents_fts MATCH 'Goblin'",
        )
        .fetch_one(r.pool())
        .await
        .unwrap();
        let orc: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM documents_fts WHERE documents_fts MATCH 'Orc'",
        )
        .fetch_one(r.pool())
        .await
        .unwrap();
        assert_eq!((goblin, orc), (0, 1));

        // Delete → removed.
        r.apply_intent(&ctx, w.id, vec![Operation::Delete { doc: d.clone() }], 3)
            .await
            .unwrap();
        let after: i64 = sqlx::query_scalar("SELECT count(*) FROM documents_fts WHERE doc_id = ?")
            .bind(d.id.to_string())
            .fetch_one(r.pool())
            .await
            .unwrap();
        assert_eq!(after, 0);
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
            r.apply_intent(&gm_ctx, w.id, vec![Operation::Create { doc: d }], i + 1)
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
            source: None,
            owner: None,
            permissions: Default::default(),
            embedded: Default::default(),
            parent_id: None,
            system,
            created_at: 0,
            updated_at: 0,
        }
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
            .apply_intent(&ctx, w.id, vec![Operation::Create { doc: doc.clone() }], 1)
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
                        path: "/system/hp".into(),
                        old: serde_json::json!(10),
                        new: serde_json::json!(5),
                    }],
                }],
                2,
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
                        path: "/system/hp".into(),
                        old: serde_json::json!(10),
                        new: serde_json::json!(1),
                    }],
                }],
                3,
            )
            .await;
        assert!(matches!(conflict, Err(DataError::Conflict(_))));
        assert_eq!(
            r.get_document(doc.id).await.unwrap().unwrap().system["hp"],
            serde_json::json!(5)
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
                        path: "/system/x".into(),
                        old: serde_json::json!(null),
                        new: serde_json::json!(1),
                    }],
                }],
                2,
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
            .apply_intent(&gm_ctx, w.id, vec![Operation::Create { doc: big }], 3)
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
            .apply_intent(&p, world, vec![Operation::Delete { doc: doc.clone() }], 2)
            .await;
        assert!(matches!(denied, Err(DataError::Forbidden)));
        assert!(r.get_document(doc.id).await.unwrap().is_some());
        // The GM holds every capability and may delete.
        r.apply_intent(&gm_ctx, world, vec![Operation::Delete { doc }], 2)
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
        r.apply_intent(&ctx, w.id, vec![Operation::Create { doc: stored }], 1)
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
            .apply_intent(&ctx, w.id, vec![Operation::Delete { doc: forged }], 2)
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
        let mut wd = CapabilityGrants::default();
        wd.by_role
            .entry(DocRole::Owner)
            .or_default()
            .insert(crate::data::permission::cap::MANAGE_EMBEDDED.to_string());
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
}
