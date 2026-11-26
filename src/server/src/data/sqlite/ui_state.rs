//! This module holds the per-user UI state of `SqliteRepository`: `get_ui_state`, which reads the
//! `users` row's `ui_state` blob, and `merge_ui_state`, which merges a patch into it one level
//! deep, in a sibling `impl` block.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use super::*;

/// The top-level `ui_state` keys whose value is a map of per-id slices (`worlds.<worldId>`,
/// `modules.<moduleId>`). `merge_ui_state` merges each id's slice one level deep; the generic
/// top-level rule would replace every named id's whole slice and drop its sibling keys.
/// `http::routes::put_ui_state` reads the same list for its object-shape check.
pub(crate) const PER_ID_NAMESPACES: [&str; 2] = ["worlds", "modules"];

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

impl SqliteRepository {
    /// The user's stored opaque UI-state JSON string, or `None` when unset.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let user = repo.create_user("mock_user", None, ServerRole::User, 0).await?;
    /// assert!(repo.get_ui_state(user).await?.is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_ui_state(&self, user: Uuid) -> Result<Option<String>, DataError> {
        let row = sqlx::query("SELECT ui_state FROM users WHERE id = ?")
            .bind(user.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.and_then(|r| r.get::<Option<String>, _>("ui_state")))
    }

    /// Merge a partial UI-state patch into the user's stored blob, one level
    /// at the individual leaf key (`global.<field>` / `worlds.<id>.<key>` /
    /// `modules.<id>.<key>`) — **the single server-side statement of this rule.** For each
    /// top-level patch key `K`: if `K` is in `PER_ID_NAMESPACES` (`worlds` or `modules`; an
    /// object; route-validated), then for each `(id, slice)` in it — when BOTH the stored
    /// `K.<id>` and
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), shadowcat::data::DataError> {
    /// use shadowcat::auth::role::ServerRole;
    /// use shadowcat::data::sqlite::SqliteRepository;
    /// let repo = SqliteRepository::connect("sqlite::memory:").await?;
    /// let user = repo.create_user("mock_user", None, ServerRole::User, 0).await?;
    /// let patch = serde_json::json!({ "global": { "theme": "dark" } });
    /// repo.merge_ui_state(user, &patch, 4096).await?;
    /// assert_eq!(
    ///     repo.get_ui_state(user).await?,
    ///     Some(r#"{"global":{"theme":"dark"}}"#.to_string())
    /// );
    /// # Ok(())
    /// # }
    /// ```
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
            if PER_ID_NAMESPACES.contains(&key.as_str()) {
                let ns_patch = value.as_object().ok_or_else(|| {
                    DataError::OpFailed(format!("ui_state patch `{key}` must be an object"))
                })?;
                let ns = stored_obj
                    .entry(key.as_str())
                    .or_insert_with(|| serde_json::json!({}));
                let ns_obj = ns.as_object_mut().ok_or_else(|| {
                    DataError::OpFailed(format!("stored ui_state `{key}` is not an object"))
                })?;
                for (id, slice) in ns_patch {
                    merge_one_level(ns_obj, id, slice);
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
}
