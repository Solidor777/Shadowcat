//! Create-time resolution of `TokenVisual::RandomVariant` pools (`data::sqlite::token_art`).

use super::*;
use crate::data::asset::query::{AssetFilter, AssetKind, AssetSort, FolderFilter};
use crate::data::engine::{TokenArtFilter, TokenEngine, TokenOverrides, TokenVisual};
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
        light: None,
        movement: None,
        aura: None,
        sound: None,
        vfx: None,
        disposition: None,
    }
}

impl SqliteRepository {
    /// Resolves every `TokenVisual::RandomVariant` a token Create carries into one concrete
    /// visual, in place, before the document is validated, based and stored.
    ///
    /// Three per-token slots are resolved, each by its own draw: `TokenEngine.visual`,
    /// `TokenOverrides.visual`, and an instanced token's embedded-actor `engine.visual`. A linked
    /// token (`actor_id` set) whose `overrides.visual` is absent and whose shared actor's
    /// `visual` is a pool draws once into the token's OWN `overrides.visual`; the actor document
    /// is only read. A concrete `overrides.visual` the Create already carries is an authored
    /// choice and suppresses that draw, which also makes a replayed Create a no-op. A pool whose
    /// `filter` matches no asset yields its `fallback`. A body that does not parse as a token
    /// engine is left untouched for `validate_engine_tree` to refuse.
    ///
    /// Runs on the transaction's own connection: the single-writer pool would deadlock on a
    /// second acquire.
    ///
    /// # Examples
    ///
    /// ```text
    /// Self::resolve_token_random_variants(&mut *tx, world_id, doc).await?;
    /// ```
    pub(super) async fn resolve_token_random_variants(
        conn: &mut SqliteConnection,
        world_id: Uuid,
        doc: &mut Document,
    ) -> Result<(), DataError> {
        if doc.doc_type != TOKEN_TYPE {
            return Ok(());
        }
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
                if let (Some(actor_id), false) = (engine.actor_id, override_visual_present) {
                    if let Some(pool) = Self::linked_actor_pool(conn, world_id, actor_id).await? {
                        let mut slot = pool;
                        Self::draw_into_slot(conn, world_id, &mut slot).await?;
                        engine.overrides.get_or_insert_with(empty_overrides).visual = Some(slot);
                        changed = true;
                    }
                }
                if changed {
                    doc.engine = Some(serde_json::to_value(&engine)?);
                }
            }
        }
        if let Some(copy) = doc
            .embedded
            .get_mut(EMBEDDED_ACTOR_KEY)
            .and_then(|copies| copies.first_mut())
        {
            if let Some(serde_json::Value::Object(body)) = copy.engine.as_mut() {
                let parsed = body
                    .get("visual")
                    .and_then(|v| serde_json::from_value::<TokenVisual>(v.clone()).ok());
                if let Some(mut visual) = parsed {
                    if Self::draw_into_slot(conn, world_id, &mut visual).await? {
                        body.insert("visual".into(), serde_json::to_value(&visual)?);
                    }
                }
            }
        }
        Ok(())
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
        let rows = qb.build().fetch_all(&mut *conn).await?;
        if rows.is_empty() {
            return Ok(None);
        }
        let index = (crate::chat::rolls::entropy_seed() % rows.len() as u64) as usize;
        let id: String = rows[index].get("id");
        Uuid::parse_str(&id)
            .map(Some)
            .map_err(|e| DataError::OpFailed(e.to_string()))
    }
}
