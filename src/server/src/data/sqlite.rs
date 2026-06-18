use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::data::document::{World, WorldRole};
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
