//! World rows and per-world state: the `worlds` / `settings` / `explored_fog`
//! half of `SqliteRepository`, in a sibling `impl` block so `sqlite.rs` stays
//! under the file-size limit.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use super::*;

impl SqliteRepository {
    /// Insert a new world row with `seq = 0` and return it.
    ///
    /// # Examples
    ///
    /// ```
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
    /// assert_eq!(repo.member_role(world.id, gm).await?, Some(WorldRole::Gm));
    /// # Ok(())
    /// # }
    /// ```
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

    /// The player's serialized explored-cell blob for a scene, or `None` when unexplored.
    /// Per-(scene, user) SECRET memory — never broadcast; dispatched per-recipient over `vision`.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// assert!(repo.get_explored(uuid::Uuid::nil(), uuid::Uuid::nil()).await?.is_none());
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let world = repo.create_world("MOCK_WORLD", 0).await?;
    /// repo.delete_world(world.id).await?;
    /// assert!(repo.get_world(world.id).await?.is_none());
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let world = uuid::Uuid::nil();
    /// let (scene, user) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
    /// repo.set_explored(world, scene, user, &[1, 2, 3]).await?;
    /// assert_eq!(repo.get_explored(scene, user).await?, Some(vec![1, 2, 3]));
    /// # Ok(())
    /// # }
    /// ```
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
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::document::WorldCapDefaults;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let world = repo.create_world("MOCK_WORLD", 0).await?;
    /// repo.set_world_cap_defaults(world.id, &WorldCapDefaults::default()).await?;
    /// assert!(repo.world_cap_defaults(world.id).await?.all.by_role.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_world_cap_defaults(
        &self,
        world: Uuid,
        defaults: &WorldCapDefaults,
    ) -> Result<(), DataError> {
        let json = serde_json::to_string(defaults)?;
        self.set_setting(&world_caps_key(world), &json).await
    }

    /// Replace a world's declarative capability requirements (stored as JSON).
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::document::CapabilityRequirement;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let world = repo.create_world("MOCK_WORLD", 0).await?;
    /// let reqs = vec![CapabilityRequirement {
    ///     path_prefix: "/engine/vision".into(),
    ///     caps: ["mock:gm_vision".to_string()].into_iter().collect(),
    /// }];
    /// repo.set_world_cap_requirements(world.id, &reqs).await?;
    /// assert_eq!(repo.world_cap_requirements(world.id).await?.len(), 1);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_world_cap_requirements(
        &self,
        world: Uuid,
        reqs: &[CapabilityRequirement],
    ) -> Result<(), DataError> {
        let json = serde_json::to_string(reqs)?;
        self.set_setting(&world_caps_req_key(world), &json).await
    }

    /// Replace a world's UI contract declarations (stored as JSON in settings).
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::document::ContractDeclaration;
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let world = repo.create_world("MOCK_WORLD", 0).await?;
    /// let decls = vec![ContractDeclaration {
    ///     module_id: "mock-module".into(),
    ///     version: "1.0.0".into(),
    ///     provides: vec![],
    ///     requires: vec![],
    /// }];
    /// repo.set_world_contract_declarations(world.id, &decls).await?;
    /// assert_eq!(repo.world_contract_declarations(world.id).await?.len(), 1);
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::data::document::{Schema, SchemaDeclaration};
    /// use shadowcat::data::repository::Repository;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let world = repo.create_world("MOCK_WORLD", 0).await?;
    /// let decls = vec![SchemaDeclaration {
    ///     module_id: "mock-module".into(),
    ///     version: "1.0.0".into(),
    ///     schema_format: 1,
    ///     doc_type: "actor".into(),
    ///     subtree_pointer: "/system".into(),
    ///     schema: Schema::default(),
    /// }];
    /// repo.set_world_schema_declarations(world.id, &decls).await?;
    /// assert_eq!(repo.world_schema_declarations(world.id).await?.len(), 1);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_world_schema_declarations(
        &self,
        world: Uuid,
        decls: &[SchemaDeclaration],
    ) -> Result<(), DataError> {
        let json = serde_json::to_string(decls)?;
        self.set_setting(&world_schemas_key(world), &json).await
    }
}
