use async_trait::async_trait;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::data::command::{set_pointer, Command, Operation, UnsequencedCommand};
use crate::data::document::{Document, Scope, World, WorldRole};
use crate::data::repository::Repository;
use crate::data::DataError;

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

    pub async fn get_world(&self, id: Uuid) -> Result<Option<World>, DataError> {
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

    pub async fn create_user(
        &self,
        username: &str,
        server_role: &str,
        now: i64,
    ) -> Result<Uuid, DataError> {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, username, server_role, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(username)
        .bind(server_role)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(id)
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

    /// Upsert a document row from its envelope, stamping `seq`.
    async fn upsert_document<'e, E>(executor: E, doc: &Document, seq: i64) -> Result<(), DataError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
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
             source_id, source_pack, source_version, owner_id, seq, json, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET scope_kind=excluded.scope_kind, world_id=excluded.world_id, \
             pack=excluded.pack, doc_type=excluded.doc_type, schema_version=excluded.schema_version, \
             source_id=excluded.source_id, source_pack=excluded.source_pack, \
             source_version=excluded.source_version, owner_id=excluded.owner_id, seq=excluded.seq, \
             json=excluded.json, updated_at=excluded.updated_at",
        )
        .bind(doc.id.to_string())
        .bind(scope_kind)
        .bind(world_id)
        .bind(pack)
        .bind(&doc.doc_type)
        .bind(doc.schema_version as i64)
        .bind(source_id)
        .bind(source_pack)
        .bind(source_version)
        .bind(doc.owner.map(|o| o.to_string()))
        .bind(seq)
        .bind(json)
        .bind(doc.created_at)
        .bind(doc.updated_at)
        .execute(executor)
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
                    Self::upsert_document(&mut *tx, doc, seq).await?;
                }
                Operation::Delete { doc } => {
                    check_command_scope(doc, sequenced.world_id)?;
                    sqlx::query("DELETE FROM documents WHERE id = ?")
                        .bind(doc.id.to_string())
                        .execute(&mut *tx)
                        .await?;
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
                    Self::upsert_document(&mut *tx, &doc, seq).await?;
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
        let u = r.create_user("gm", "admin", 100).await.unwrap();
        r.add_member(w.id, u, WorldRole::Gm).await.unwrap();
        assert_eq!(r.member_role(w.id, u).await.unwrap(), Some(WorldRole::Gm));
        assert_eq!(
            r.member_role(w.id, Uuid::from_u128(123)).await.unwrap(),
            None
        );
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
            system,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[tokio::test]
    async fn create_update_delete_round_trip_via_invert() {
        let r = repo().await;
        let w = r.create_world("W", 0).await.unwrap();
        let author = r.create_user("author", "user", 0).await.unwrap();

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
        let author = r.create_user("author", "user", 0).await.unwrap();
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
            author = r.create_user("author", "user", 0).await.unwrap();
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
        let author = r.create_user("author", "user", 0).await.unwrap();
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
        let author = r.create_user("author", "user", 0).await.unwrap();
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
        let author = r.create_user("author", "user", 0).await.unwrap();
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
        let author = r.create_user("author", "user", 0).await.unwrap();
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
        let author = r.create_user("author", "user", 0).await.unwrap();
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
        let author = r.create_user("author", "user", 0).await.unwrap();
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
        let author = r.create_user("author", "user", 0).await.unwrap();
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
}
