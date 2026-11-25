//! Module-placed effects: the server side of `ClientMsg::PlaceEffect`.
//!
//! A module manifest declares placeable effects (`modules::PlacementDecl`). A request names one
//! and supplies only the area. `Room::place_effect` resolves the declaration from the world's
//! enabled, engine-compatible modules, then `Room::place_declared` decides who may place it,
//! clamps the area to the declaration's limits, builds the region from manifest data, validates it
//! with `RegionEngine::validate` and commits one Create under `WriteOrigin::ModulePlacement`.
//! `place_declared` takes the resolved declaration as an argument, so a later path that computes a
//! declaration feeds the same authorization, validation and commit.

use std::path::Path;
use std::sync::Arc;

use uuid::Uuid;

use super::Room;
use crate::data::command::{Operation, WriteOrigin};
use crate::data::document::{Document, PermissionSet, Scope, WorldRole};
use crate::data::engine as eng;
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::data::DataError;
use crate::modules::{engine_compat_ok, ModuleScanCache, PlacementDecl, PlacementWho};
use crate::scene::placed_regions::PLACEMENT_EXPIRES_AT_KEY;

/// Grouped inputs to `Room::place_effect` (mirroring `ToggleDoorInputs`).
pub(crate) struct PlaceEffectInputs {
    /// Correlates a rejection's `ServerMsg::PlaceEffectError` with the originating
    /// `ClientMsg::PlaceEffect` (success is the broadcast `Event` of the Create).
    pub request_id: Uuid,
    /// The installed module's folder id that declares the placement.
    pub module: String,
    /// The `PlacementDecl::id` within that module.
    pub placement: String,
    /// The scene the region is created on.
    pub scene_id: Uuid,
    /// The area the sender chose.
    pub shape: eng::RegionShape,
    /// The caster token, required of a non-GM when the declaration's `who` is `caster_owner`.
    pub caster: Option<Uuid>,
    /// Server-authoritative commit timestamp.
    pub ts: i64,
}

/// Why a `ClientMsg::PlaceEffect` was refused. `Display` yields ONE wording for every variant
/// ("placement rejected"), the wire message for a non-GM sender, because a distinguishable wording
/// would disclose whether a module, declaration, scene or token exists. A GM sender may instead
/// receive the variant-specific text (`place_effect_wire_message`), the role-conditioned
/// disclosure `DoorToggleError` uses.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PlaceEffectError {
    /// The module is not enabled in this world, is engine-incompatible, or does not declare the
    /// placement.
    #[error("placement rejected")]
    UnknownPlacement,
    /// The declaration's `who` does not admit the sender.
    #[error("placement rejected")]
    NotPermitted,
    /// The declaration needs a caster the sender effectively owns on the scene, and the request
    /// names none or one the sender does not own there.
    #[error("placement rejected")]
    CasterNotOwned,
    /// The scene is not a live scene document.
    #[error("placement rejected")]
    UnknownScene,
    /// The area's kind is not allowed or its extent exceeds the declaration's `max_extent`.
    #[error("placement rejected")]
    AreaRefused,
    /// The declaration sets `max_range_cells` and the area's centre is farther than that from the
    /// named caster token.
    #[error("placement rejected")]
    OutOfRange,
    /// The region built from the declaration and area failed `RegionEngine::validate`.
    #[error("placement rejected")]
    InvalidRegion(String),
    /// The sender exceeded the shared per-user write budget (`crate::ws::MESSAGE_RATE_PER_MIN`).
    #[error("placement rejected")]
    RateLimited,
    /// An authority input or the commit itself failed.
    #[error("placement rejected")]
    Data(#[from] DataError),
}

/// The error-to-wire conversion `ws::conn`'s `handle_place_effect` applies: the sender's ROLE
/// decides disclosure, never the variant. A non-GM gets the uniform `Display` wording; a GM gets
/// the variant-specific diagnostic.
///
/// # Examples
///
/// ```text
/// place_effect_wire_message(&PlaceEffectError::AreaRefused, WorldRole::Player) == "placement rejected"
/// place_effect_wire_message(&PlaceEffectError::AreaRefused, WorldRole::Gm) == "the area is not allowed by the placement"
/// ```
pub(crate) fn place_effect_wire_message(err: &PlaceEffectError, world_role: WorldRole) -> String {
    if world_role != WorldRole::Gm {
        return err.to_string();
    }
    match err {
        PlaceEffectError::UnknownPlacement => "no such enabled placement".to_string(),
        PlaceEffectError::NotPermitted => "the placement does not admit you".to_string(),
        PlaceEffectError::CasterNotOwned => "no owned caster token on that scene".to_string(),
        PlaceEffectError::UnknownScene => "no such scene".to_string(),
        PlaceEffectError::AreaRefused => "the area is not allowed by the placement".to_string(),
        PlaceEffectError::OutOfRange => "the area is out of range of the caster".to_string(),
        PlaceEffectError::InvalidRegion(reason) => format!("invalid region: {reason}"),
        PlaceEffectError::RateLimited => "rate limited".to_string(),
        PlaceEffectError::Data(_) => "internal error".to_string(),
    }
}

impl Room {
    /// Resolve the declaration `req` names and place it. The module must be enabled in this
    /// world and engine-compatible (the continuous property `welcome_capability_requirements`
    /// re-checks the same way), and must declare the placement; every miss is one
    /// `PlaceEffectError::UnknownPlacement`.
    ///
    /// # Examples
    ///
    /// ```text
    /// room.place_effect(repo, ctx, modules_dir, cache, req).await?;
    /// ```
    pub(crate) async fn place_effect(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        modules_dir: &Path,
        cache: &Arc<ModuleScanCache>,
        req: PlaceEffectInputs,
    ) -> Result<(), PlaceEffectError> {
        let enabled = repo.world_enabled_modules(self.world_id).await?;
        if !enabled.iter().any(|e| e.id == req.module) {
            return Err(PlaceEffectError::UnknownPlacement);
        }
        // Blocking filesystem I/O off the async worker, the `welcome_capability_requirements`
        // convention; a panicked scan degrades to no modules.
        let dir = modules_dir.to_path_buf();
        let scan = cache.clone();
        let installed = tokio::task::spawn_blocking(move || scan.get_or_scan(&dir))
            .await
            .unwrap_or_default();
        let decl = installed
            .into_iter()
            .find(|m| m.id == req.module && engine_compat_ok(m))
            .and_then(|m| m.placements.into_iter().find(|d| d.id == req.placement))
            .ok_or(PlaceEffectError::UnknownPlacement)?;
        self.place_declared(repo, ctx, &decl, req).await
    }

    /// Place `decl` over `req.shape`: authorize the sender against `decl.who` (a GM always
    /// passes; a spectator never does), require the scene to exist and, for a non-GM, to be
    /// readable by the sender, clamp the area with
    /// `PlacementDecl::admits`, build the region from the declaration, validate it and commit one
    /// `region` Create under `WriteOrigin::ModulePlacement`. The effect set comes from `decl`
    /// alone; `req` contributes the area, the scene and the caster.
    ///
    /// `publish_guard` is held across the whole validate-then-commit body (mirrors
    /// `toggle_door`), so the caster-ownership read and the commit serialize against every other
    /// write.
    ///
    /// # Examples
    ///
    /// ```text
    /// room.place_declared(repo, ctx, &decl, req).await?;
    /// ```
    pub(crate) async fn place_declared(
        &self,
        repo: &dyn Repository,
        ctx: &PermissionContext,
        decl: &PlacementDecl,
        req: PlaceEffectInputs,
    ) -> Result<(), PlaceEffectError> {
        let is_gm = ctx.world_role == WorldRole::Gm;
        if !is_gm && (ctx.world_role != WorldRole::Player || decl.who == PlacementWho::Gm) {
            return Err(PlaceEffectError::NotPermitted);
        }

        // A non-GM needs READ on the target scene, the admission every scene-addressed relay
        // applies (`ws::scene_broadcast_permitted`); a refusal reads as an unknown scene.
        if !is_gm
            && !crate::ws::scene_broadcast_permitted(req.scene_id, ctx, self.world_id, repo).await
        {
            return Err(PlaceEffectError::UnknownScene);
        }

        let _guard = self.publish_guard.lock().await;
        {
            let ecs = self.scene.read().await;
            if !ecs.scene_grid_sizes().contains_key(&req.scene_id) {
                return Err(PlaceEffectError::UnknownScene);
            }
            if !is_gm && decl.who == PlacementWho::CasterOwner {
                let owned = req.caster.is_some_and(|token| {
                    ecs.token_scene_and_effective_owner(token)
                        == Some((req.scene_id, Some(ctx.user_id)))
                });
                if !owned {
                    return Err(PlaceEffectError::CasterNotOwned);
                }
            }
        } // scene read guard dropped — commit_ops_locked takes the write guard itself

        if !decl.admits(&req.shape) {
            return Err(PlaceEffectError::AreaRefused);
        }
        if decl.max_range_cells.is_some() {
            self.check_caster_range(decl, ctx, &req).await?;
        }
        let region = decl.region_engine(req.shape);
        region.validate().map_err(PlaceEffectError::InvalidRegion)?;

        let mut placement = serde_json::json!({
            "module": req.module, "id": decl.id, "by": ctx.user_id
        });
        if let Some(at) = decl.expires_at(req.ts) {
            placement[PLACEMENT_EXPIRES_AT_KEY] = serde_json::json!(at);
        }
        let doc = Document {
            id: Uuid::new_v4(),
            scope: Scope::World {
                world_id: self.world_id,
            },
            doc_type: "region".to_string(),
            schema_version: 1,
            name: decl.label.clone(),
            source: None,
            base: None,
            owner: None,
            permissions: PermissionSet {
                default: crate::data::document::DocRole::Observer,
                ..PermissionSet::default()
            },
            embedded: Default::default(),
            parent_id: Some(req.scene_id),
            engine: Some(serde_json::to_value(&region).map_err(DataError::from)?),
            system: serde_json::json!({ "placement": placement }),
            created_at: req.ts,
            updated_at: req.ts,
        };
        self.commit_ops_locked(
            repo,
            ctx,
            vec![Operation::Create { doc }],
            req.ts,
            WriteOrigin::ModulePlacement,
        )
        .await?;
        Ok(())
    }

    /// Enforce `decl.max_range_cells` against the caster the request names. The caster must be on
    /// the request's scene and, for a non-GM, effectively owned by the sender; it supplies the
    /// measuring point. A GM that names no caster is not range-limited (a GM may always place);
    /// every other missing or foreign caster is `PlaceEffectError::CasterNotOwned`, so a range
    /// limit cannot be sidestepped by omitting the caster. The distance is
    /// `PlacementDecl::within_range` at the scene's `GridShape::world_units_per_cell`, the same
    /// cells-to-scene-units conversion the door-reach check applies.
    async fn check_caster_range(
        &self,
        decl: &PlacementDecl,
        ctx: &PermissionContext,
        req: &PlaceEffectInputs,
    ) -> Result<(), PlaceEffectError> {
        let is_gm = ctx.world_role == WorldRole::Gm;
        if is_gm && req.caster.is_none() {
            return Ok(());
        }
        let ecs = self.scene.read().await;
        let caster_pos = req.caster.and_then(|token| {
            let (scene, owner) = ecs.token_scene_and_effective_owner(token)?;
            let admitted = scene == req.scene_id && (is_gm || owner == Some(ctx.user_id));
            admitted.then(|| ecs.token_position(token)).flatten()
        });
        let Some(pos) = caster_pos else {
            return Err(PlaceEffectError::CasterNotOwned);
        };
        let cell = ecs
            .scene_grid_sizes()
            .get(&req.scene_id)
            .copied()
            .ok_or(PlaceEffectError::UnknownScene)?;
        let wu_per_cell = ecs
            .resolve_grid_shape(req.scene_id, cell)
            .world_units_per_cell();
        if decl.within_range(pos, &req.shape, wu_per_cell) {
            Ok(())
        } else {
            Err(PlaceEffectError::OutOfRange)
        }
    }

    /// Holds the world's publish guard until the returned guard drops; lets a test prove a path
    /// does not queue behind a publish.
    #[cfg(test)]
    pub(crate) async fn hold_publish_guard_for_test(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.publish_guard.lock().await
    }

    /// Remove every placed region whose expiry stamp (`PLACEMENT_EXPIRES_AT_KEY`) is at or before
    /// `now_ms`, as one `Delete` batch under `WriteOrigin::ModulePlacement`, authored as the
    /// world's seed author (`world_seed::seed_author`, a GM). Returns how many regions were
    /// removed. A world with no GM member cannot author the delete and removes nothing. Holds
    /// `publish_guard` across the read and the commit, so a region a GM deleted meanwhile cannot
    /// make the batch stale.
    ///
    /// # Examples
    ///
    /// ```text
    /// let removed = room.expire_due_placements(repo, now_ms).await?;
    /// ```
    pub(crate) async fn expire_due_placements(
        &self,
        repo: &dyn Repository,
        now_ms: i64,
    ) -> Result<usize, DataError> {
        // The sweep runs every second per room: check for due regions before queueing on the
        // guard so an idle sweep never contends with a publish.
        if self
            .scene
            .read()
            .await
            .due_placement_regions(now_ms)
            .is_empty()
        {
            return Ok(0);
        }
        let _guard = self.publish_guard.lock().await;
        let due = self.scene.read().await.due_placement_regions(now_ms);
        if due.is_empty() {
            return Ok(0);
        }
        let Some(ctx) = crate::data::world_seed::seed_author(repo, self.world_id).await else {
            return Ok(0);
        };
        let count = due.len();
        let ops = due
            .into_iter()
            .map(|doc| Operation::Delete { doc })
            .collect();
        self.commit_ops_locked(repo, &ctx, ops, now_ms, WriteOrigin::ModulePlacement)
            .await?;
        Ok(count)
    }
}

/// How often the server's sweep looks for placed regions past their expiry; the `period` the
/// server passes to `spawn_placement_expiry_sweep`.
pub(crate) const PLACEMENT_EXPIRY_PERIOD: std::time::Duration = std::time::Duration::from_secs(1);

/// Spawn the background task that expires module-placed regions. Every `period` it asks each live
/// room to `Room::expire_due_placements`; a failed sweep is logged and retried on the next tick.
/// The task ends when `shutdown` reports a requested shutdown (or its sender is dropped). A room
/// hydrated after a restart carries its regions' persisted stamps, so regions that expired while
/// the server was down are removed on the first tick after the room is created; a world with no
/// live room has no region effect to expire.
///
/// # Examples
///
/// ```text
/// spawn_placement_expiry_sweep(ws.rooms.clone(), repo.clone(), ws.shutdown.clone(), PLACEMENT_EXPIRY_PERIOD);
/// ```
pub(crate) fn spawn_placement_expiry_sweep(
    rooms: Arc<super::RoomRegistry>,
    repo: Arc<crate::data::sqlite::SqliteRepository>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    period: std::time::Duration,
) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(period);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = tick.tick() => {}
                _ = shutdown.wait_for(|requested| *requested) => return,
            }
            let now = crate::ws::time::now_millis();
            for room in rooms.live_rooms() {
                if let Err(e) = room.expire_due_placements(repo.as_ref(), now).await {
                    tracing::warn!(world = %room.world_id, error = %e, "placement expiry sweep failed");
                }
            }
        }
    });
}
