//! Server-side resolution of `TokenVisual::RandomVariant` pools (`data::sqlite::token_art`): at a
//! token Create, at a token Update that sets one, and across the linked tokens of a shared actor
//! whose own pool is saved.

use super::*;
use crate::data::asset::query::{AssetFilter, AssetKind, AssetSort, FolderFilter};
use crate::data::engine::{TokenArtFilter, TokenEngine, TokenOverrides, TokenVisual};
use futures_util::TryStreamExt;
use sqlx::SqliteConnection;

/// The `doc_type` of a placed token, the only type whose Create carries per-token art slots.
const TOKEN_TYPE: &str = "token";
/// The `doc_type` of a shared actor a linked token reads its visual from.
const ACTOR_TYPE: &str = "actor";
/// The key of an instanced token's embedded actor copy under `Document.embedded`.
const EMBEDDED_ACTOR_KEY: &str = "actor";

/// An empty override record: every whitelisted field absent, so setting `visual` on it changes
/// nothing else about the linked token.
fn empty_overrides() -> TokenOverrides {
    TokenOverrides {
        name: None,
        visual: None,
        size: None,
        shape: None,
        vision: None,
        facing_vision: None,
        light: None,
        movement: None,
        aura: None,
        sound: None,
        vfx: None,
        disposition: None,
        bars: None,
        nameplate_mode: None,
    }
}

impl SqliteRepository {
    /// Resolves every `TokenVisual::RandomVariant` a token document carries into one concrete
    /// visual, in place, before the document is validated, based and stored; returns whether it
    /// changed anything. Runs for a token Create (`draw_linked` true) and for a token Update
    /// (`draw_linked` false), so a pool a GM sets on a placed token is drawn at the save too.
    ///
    /// Per-token slots are resolved, each by its own draw: `TokenEngine.visual`,
    /// `TokenOverrides.visual`, and the `engine.visual` of every embedded actor copy of an
    /// instanced token. With `draw_linked`, a linked token (`actor_id` set) whose
    /// `overrides.visual` is absent and whose shared actor's `visual` is a pool also draws once
    /// into the token's OWN `overrides.visual`; the actor document is only read. A Create keeps
    /// that draw because the token is new; an Update omits it because a token edit that never
    /// touched art must not redraw the art on every save. A concrete `overrides.visual` the write
    /// already carries is an authored choice and suppresses the linked draw, which also makes a
    /// replayed Create a no-op. A pool whose `filter` matches no asset yields its `fallback`. A
    /// body that does not parse as a token engine is left untouched for `validate_engine_tree`
    /// to refuse.
    ///
    /// Runs on the transaction's own connection: the single-writer pool would deadlock on a
    /// second acquire.
    ///
    /// # Examples
    ///
    /// ```text
    /// Self::resolve_token_random_variants(&mut *tx, world_id, doc, true).await?;
    /// ```
    pub(super) async fn resolve_token_random_variants(
        conn: &mut SqliteConnection,
        world_id: Uuid,
        doc: &mut Document,
        draw_linked: bool,
    ) -> Result<bool, DataError> {
        if doc.doc_type != TOKEN_TYPE {
            return Ok(false);
        }
        let mut resolved = false;
        if let Some(engine_json) = doc.engine.clone() {
            if let Ok(mut engine) = serde_json::from_value::<TokenEngine>(engine_json) {
                let mut changed = false;
                if let Some(visual) = engine.visual.as_mut() {
                    changed |= Self::draw_into_slot(conn, world_id, visual).await?;
                }
                if let Some(visual) = engine.overrides.as_mut().and_then(|o| o.visual.as_mut()) {
                    changed |= Self::draw_into_slot(conn, world_id, visual).await?;
                }
                let override_visual_present = engine
                    .overrides
                    .as_ref()
                    .is_some_and(|o| o.visual.is_some());
                if let (true, Some(actor_id), false) =
                    (draw_linked, engine.actor_id, override_visual_present)
                {
                    if let Some(pool) = Self::linked_actor_pool(conn, world_id, actor_id).await? {
                        let mut slot = pool;
                        Self::draw_into_slot(conn, world_id, &mut slot).await?;
                        engine.overrides.get_or_insert_with(empty_overrides).visual = Some(slot);
                        changed = true;
                    }
                }
                if changed {
                    doc.engine = Some(serde_json::to_value(&engine)?);
                    resolved = true;
                }
            }
        }
        for copy in doc
            .embedded
            .get_mut(EMBEDDED_ACTOR_KEY)
            .into_iter()
            .flatten()
        {
            if let Some(serde_json::Value::Object(body)) = copy.engine.as_mut() {
                let parsed = body
                    .get("visual")
                    .and_then(|v| serde_json::from_value::<TokenVisual>(v.clone()).ok());
                if let Some(mut visual) = parsed {
                    if Self::draw_into_slot(conn, world_id, &mut visual).await? {
                        body.insert("visual".into(), serde_json::to_value(&visual)?);
                        resolved = true;
                    }
                }
            }
        }
        Ok(resolved)
    }

    /// The Update ops that give every token linked to `actor` a fresh draw from the actor's pool,
    /// written to each token's own `overrides.visual`; empty unless `actor` is a shared actor
    /// whose `visual` is a pool. Called after an actor Update that wrote the actor's `visual`,
    /// so every save of the pool setting picks again for the tokens already placed, not only for
    /// tokens created later. A draw replaces a concrete `overrides.visual` a token already holds,
    /// because a drawn value and an authored one are indistinguishable in storage.
    ///
    /// Each token is rewritten in the caller's transaction (`seq`, `ts`) and its post-image is
    /// returned beside its op so the caller records it for the commit-time snapshot. The writes
    /// are consequences of the already-authorized actor write, so they carry no per-token
    /// capability check, like the lifts of a table-folder delete.
    ///
    /// # Examples
    ///
    /// ```text
    /// let derived = Self::redraw_linked_token_art(&mut *tx, world_id, &actor, seq, ts).await?;
    /// ```
    pub(super) async fn redraw_linked_token_art(
        conn: &mut SqliteConnection,
        world_id: Uuid,
        actor: &Document,
        seq: i64,
        ts: i64,
    ) -> Result<Vec<(Operation, Document)>, DataError> {
        if actor.doc_type != ACTOR_TYPE {
            return Ok(Vec::new());
        }
        let pool = actor
            .engine
            .as_ref()
            .and_then(|e| e.get("visual"))
            .and_then(|v| serde_json::from_value::<TokenVisual>(v.clone()).ok())
            .filter(|v| matches!(v, TokenVisual::RandomVariant { .. }));
        let Some(pool) = pool else {
            return Ok(Vec::new());
        };
        let rows = sqlx::query(
            "SELECT id FROM documents WHERE world_id = ? AND doc_type = ? \
             AND json_extract(json, '$.engine.actor_id') = ? ORDER BY id",
        )
        .bind(world_id.to_string())
        .bind(TOKEN_TYPE)
        .bind(actor.id.to_string())
        .fetch_all(&mut *conn)
        .await?;
        let mut derived = Vec::with_capacity(rows.len());
        for row in rows {
            let id = Uuid::parse_str(row.get::<String, _>("id").as_str())
                .map_err(|e| DataError::OpFailed(e.to_string()))?;
            let Some(mut token) = Self::load_document(&mut *conn, id).await? else {
                continue;
            };
            let Some(mut engine) = token
                .engine
                .clone()
                .and_then(|e| serde_json::from_value::<TokenEngine>(e).ok())
            else {
                continue;
            };
            let old_overrides = match &engine.overrides {
                Some(o) => serde_json::to_value(o)?,
                None => serde_json::Value::Null,
            };
            let mut slot = pool.clone();
            Self::draw_into_slot(conn, world_id, &mut slot).await?;
            engine.overrides.get_or_insert_with(empty_overrides).visual = Some(slot);
            token.engine = Some(serde_json::to_value(&engine)?);
            validation::validate_engine_tree(&mut token)?;
            token.updated_at = ts;
            Self::upsert_document(&mut *conn, &token, seq).await?;
            let new_overrides = token
                .engine
                .as_ref()
                .and_then(|e| e.get("overrides"))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            derived.push((
                Operation::Update {
                    doc_id: id,
                    changes: vec![FieldChange {
                        remove: false,
                        path: "/engine/overrides".into(),
                        old: old_overrides,
                        new: new_overrides,
                    }],
                },
                token,
            ));
        }
        Ok(derived)
    }

    /// Replaces `slot` with its drawn concrete visual when it is a `RandomVariant`; returns
    /// whether it did. Any other visual is left exactly as authored.
    async fn draw_into_slot(
        conn: &mut SqliteConnection,
        world_id: Uuid,
        slot: &mut TokenVisual,
    ) -> Result<bool, DataError> {
        let TokenVisual::RandomVariant { filter, fallback } = &*slot else {
            return Ok(false);
        };
        if matches!(**fallback, TokenVisual::RandomVariant { .. }) {
            return Err(DataError::BadEngine(
                "token: a random-variant pool's fallback cannot itself be a pool".into(),
            ));
        }
        let drawn = match Self::pick_pool_asset(conn, world_id, filter).await? {
            Some(asset) => TokenVisual::Image {
                asset: asset.to_string(),
                transform: None,
            },
            None => (**fallback).clone(),
        };
        *slot = drawn;
        Ok(true)
    }

    /// The linked actor's own `visual` when it is a pool, read from the committed actor document.
    /// A dangling link, a foreign-world actor and a non-actor document all read as no pool,
    /// matching `load_effective_owner`'s fail-closed link resolution.
    async fn linked_actor_pool(
        conn: &mut SqliteConnection,
        world_id: Uuid,
        actor_id: Uuid,
    ) -> Result<Option<TokenVisual>, DataError> {
        let Some(actor) = Self::load_document(&mut *conn, actor_id).await? else {
            return Ok(None);
        };
        if actor.doc_type != ACTOR_TYPE || world_of(&actor) != Some(world_id) {
            return Ok(None);
        }
        let visual = actor
            .engine
            .as_ref()
            .and_then(|e| e.get("visual"))
            .and_then(|v| serde_json::from_value::<TokenVisual>(v.clone()).ok());
        Ok(visual.filter(|v| matches!(v, TokenVisual::RandomVariant { .. })))
    }

    /// One uniformly random image asset id of `world_id` matching `filter`, or `None` when the
    /// pool is empty. The pick is a one-time server-side choice that nothing re-derives, so it
    /// takes fresh OS entropy (`chat::rolls::entropy_seed`) rather than a reproducible seed.
    async fn pick_pool_asset(
        conn: &mut SqliteConnection,
        world_id: Uuid,
        filter: &TokenArtFilter,
    ) -> Result<Option<Uuid>, DataError> {
        let asset_filter = AssetFilter {
            folder: filter.folder.map(|folder| FolderFilter::In {
                folder,
                recursive: true,
            }),
            tags: filter.tags.clone(),
            kind: Some(AssetKind::Image),
            query: None,
        };
        let Some(mut qb) = Self::asset_query_builder(
            world_id,
            &asset_filter,
            AssetSort::default(),
            None,
            u32::MAX,
        )?
        else {
            return Ok(None);
        };
        // Algorithm R reservoir sampling of size 1 (Vitter 1985): the i-th streamed row replaces
        // the kept one with probability 1/i, so every row is kept with probability 1/n while
        // memory stays one id however large the pool is.
        let mut rows = qb.build().fetch(&mut *conn);
        let mut chosen: Option<String> = None;
        let mut seen: u64 = 0;
        while let Some(row) = rows.try_next().await? {
            seen += 1;
            if crate::chat::rolls::entropy_seed().is_multiple_of(seen) {
                chosen = Some(row.get("id"));
            }
        }
        chosen
            .map(|id| Uuid::parse_str(&id).map_err(|e| DataError::OpFailed(e.to_string())))
            .transpose()
    }
}
