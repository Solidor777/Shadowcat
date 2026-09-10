//! Accounts, seats and invites: the `users` / `world_members` / `world_invites`
//! half of `SqliteRepository`, in a sibling `impl` block so `sqlite.rs` stays
//! under the file-size limit.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use super::*;

impl SqliteRepository {
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::auth::session::SqlxSqliteStore;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// // `tower_sessions` is created by `SqlxSqliteStore::migrate` at boot;
    /// // a repo-level example must run it itself before deleting a user.
    /// SqlxSqliteStore::new(repo.pool().clone(), repo.pool().clone())
    ///     .migrate()
    ///     .await
    ///     .unwrap();
    /// let id = repo.create_user("mock_target", None, ServerRole::User, 0).await?;
    /// repo.delete_user(id).await?;
    /// assert!(!repo.user_exists(id).await?);
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::document::WorldRole;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let gm = repo.create_user("mock_gm", None, ServerRole::User, 0).await?;
    /// let world = repo.create_world_owned("MOCK_WORLD", gm, 0).await?;
    /// let player = repo.create_user("mock_player", None, ServerRole::User, 0).await?;
    /// repo.add_member(world.id, player, WorldRole::Player).await?;
    /// repo.set_role(world.id, player, WorldRole::Spectator).await?;
    /// assert_eq!(repo.member_role(world.id, player).await?, Some(WorldRole::Spectator));
    /// # Ok(())
    /// # }
    /// ```
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
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let gm = repo.create_user("mock_gm", None, ServerRole::User, 0).await?;
    /// let world = repo.create_world_owned("MOCK_WORLD", gm, 0).await?;
    /// // Removing the only GM is refused with DataError::Conflict.
    /// let err = repo.remove_member(world.id, gm).await.unwrap_err();
    /// assert!(matches!(err, shadowcat::data::DataError::Conflict(_)));
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
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::document::WorldRole;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let gm = repo.create_user("mock_gm", None, ServerRole::User, 0).await?;
    /// repo.create_world_owned("MOCK_WORLD", gm, 0).await?;
    /// let worlds = repo.worlds_for_user(gm, ServerRole::User).await?;
    /// assert_eq!(worlds.len(), 1);
    /// assert_eq!(worlds[0].1, WorldRole::Gm);
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::document::WorldRole;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let gm = repo.create_user("mock_gm", None, ServerRole::User, 0).await?;
    /// let world = repo.create_world_owned("MOCK_WORLD", gm, 0).await?;
    /// let ctx = repo.permission_context(world.id, gm, ServerRole::User).await?;
    /// assert_eq!(ctx.world_role, WorldRole::Gm);
    /// # Ok(())
    /// # }
    /// ```
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
    /// ```
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

    /// Look up an account by exact username, or `None`.
    ///
    /// # Examples
    ///
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// assert!(!repo.user_exists(uuid::Uuid::nil()).await?);
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let first = repo.create_user_unique("mock-unique", "hash", ServerRole::User, 0).await?;
    /// assert!(first.is_some());
    /// let collision = repo.create_user_unique("mock-unique", "hash", ServerRole::User, 0).await?;
    /// assert!(collision.is_none());
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// repo.create_user("mock_listed", None, ServerRole::User, 0).await?;
    /// let users = repo.list_users().await?;
    /// assert_eq!(users.len(), 1);
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let first = repo.create_admin_if_none("mock_admin", "hash", 0).await?;
    /// assert!(first.is_some());
    /// let second = repo.create_admin_if_none("mock_admin_2", "hash", 0).await?;
    /// assert!(second.is_none()); // an admin already exists
    /// # Ok(())
    /// # }
    /// ```
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

    /// Seat `user_id` in `world_id` with `role` (upsert; idempotent for an
    /// existing member with the same role).
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::document::WorldRole;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let gm = repo.create_user("mock_gm", None, ServerRole::User, 0).await?;
    /// let world = repo.create_world_owned("MOCK_WORLD", gm, 0).await?;
    /// let player = repo.create_user("mock_player", None, ServerRole::User, 0).await?;
    /// repo.add_member(world.id, player, WorldRole::Player).await?;
    /// assert_eq!(repo.member_role(world.id, player).await?, Some(WorldRole::Player));
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::document::WorldRole;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let gm = repo.create_user("mock_gm", None, ServerRole::User, 0).await?;
    /// let world = repo.create_world_owned("MOCK_WORLD", gm, 0).await?;
    /// let player = repo.create_user("mock_player", None, ServerRole::User, 0).await?;
    /// repo.upsert_member(world.id, player, WorldRole::Player).await?;
    /// assert_eq!(repo.member_role(world.id, player).await?, Some(WorldRole::Player));
    /// # Ok(())
    /// # }
    /// ```
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
    /// ```
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
        Self::load_member_role(&self.pool, world_id, user_id).await
    }

    /// `member_role` over any executor, for a caller already inside a
    /// transaction (the single-writer pool holds one connection, so a pool
    /// query mid-transaction would deadlock).
    pub(super) async fn load_member_role<'e, E>(
        executor: E,
        world_id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<WorldRole>, DataError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let row = sqlx::query("SELECT role FROM world_members WHERE world_id = ? AND user_id = ?")
            .bind(world_id.to_string())
            .bind(user_id.to_string())
            .fetch_optional(executor)
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::document::WorldRole;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let gm = repo.create_user("mock_gm", None, ServerRole::User, 0).await?;
    /// let world = repo.create_world_owned("MOCK_WORLD", gm, 0).await?;
    /// let found = repo.member_id_by_username(world.id, "mock_gm").await?;
    /// assert_eq!(found, Some(gm));
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::document::WorldRole;
    /// use shadowcat::data::sqlite::{NewInvite, SqliteRepository};
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let gm = repo.create_user("mock_gm", None, ServerRole::User, 0).await?;
    /// let world = repo.create_world_owned("MOCK_WORLD", gm, 0).await?;
    /// let invite = NewInvite {
    ///     id: uuid::Uuid::new_v4(),
    ///     world: world.id,
    ///     secret_hash: "mock-hash",
    ///     role: WorldRole::Player,
    ///     created_by: gm,
    ///     now: 0,
    ///     expires_at: 1_000,
    /// };
    /// let stored = repo.create_invite(invite, 10).await?;
    /// assert!(stored);
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::document::WorldRole;
    /// use shadowcat::data::sqlite::{NewInvite, SqliteRepository};
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let gm = repo.create_user("mock_gm", None, ServerRole::User, 0).await?;
    /// let world = repo.create_world_owned("MOCK_WORLD", gm, 0).await?;
    /// let id = uuid::Uuid::new_v4();
    /// let invite = NewInvite {
    ///     id,
    ///     world: world.id,
    ///     secret_hash: "mock-hash",
    ///     role: WorldRole::Player,
    ///     created_by: gm,
    ///     now: 0,
    ///     expires_at: 1_000,
    /// };
    /// repo.create_invite(invite, 10).await?;
    /// assert!(repo.invite_by_id(id).await?.is_some());
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::document::WorldRole;
    /// use shadowcat::data::sqlite::{NewInvite, SqliteRepository};
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let gm = repo.create_user("mock_gm", None, ServerRole::User, 0).await?;
    /// let world = repo.create_world_owned("MOCK_WORLD", gm, 0).await?;
    /// let invite = NewInvite {
    ///     id: uuid::Uuid::new_v4(),
    ///     world: world.id,
    ///     secret_hash: "mock-hash",
    ///     role: WorldRole::Player,
    ///     created_by: gm,
    ///     now: 0,
    ///     expires_at: 1_000,
    /// };
    /// repo.create_invite(invite, 10).await?;
    /// let invites = repo.list_invites(world.id).await?;
    /// assert_eq!(invites.len(), 1);
    /// assert!(invites[0].secret_hash.is_empty()); // never leaked to the listing
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::document::WorldRole;
    /// use shadowcat::data::sqlite::{NewInvite, SqliteRepository};
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let gm = repo.create_user("mock_gm", None, ServerRole::User, 0).await?;
    /// let world = repo.create_world_owned("MOCK_WORLD", gm, 0).await?;
    /// let id = uuid::Uuid::new_v4();
    /// let invite = NewInvite {
    ///     id,
    ///     world: world.id,
    ///     secret_hash: "mock-hash",
    ///     role: WorldRole::Player,
    ///     created_by: gm,
    ///     now: 0,
    ///     expires_at: 1_000,
    /// };
    /// repo.create_invite(invite, 10).await?;
    /// assert!(repo.revoke_invite(world.id, id, 1).await?);
    /// assert!(!repo.revoke_invite(world.id, id, 1).await?); // already revoked
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::document::WorldRole;
    /// use shadowcat::data::sqlite::{NewInvite, SqliteRepository};
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let gm = repo.create_user("mock_gm", None, ServerRole::User, 0).await?;
    /// let world = repo.create_world_owned("MOCK_WORLD", gm, 0).await?;
    /// let id = uuid::Uuid::new_v4();
    /// let invite = NewInvite {
    ///     id,
    ///     world: world.id,
    ///     secret_hash: "mock-hash",
    ///     role: WorldRole::Player,
    ///     created_by: gm,
    ///     now: 0,
    ///     expires_at: 1_000,
    /// };
    /// repo.create_invite(invite, 10).await?;
    /// let redeemer = repo.create_user("mock_redeemer", None, ServerRole::User, 0).await?;
    /// let seated = repo.consume_invite(id, redeemer, 1).await?.unwrap();
    /// assert_eq!(seated.role, WorldRole::Player);
    /// # Ok(())
    /// # }
    /// ```
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
}
