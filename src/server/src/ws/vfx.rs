//! Shared `PlayVfx` validation + authorization, reused by BOTH the raw `ClientMsg::PlayVfx`
//! handler (`ws::conn`) and the `/fx` chat command (`chat::fx`) — both call sites must agree
//! on validation and authorization, since a chat-issued effect and a raw frame carry
//! identical trust.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use uuid::Uuid;

use crate::data::document::WorldRole;
use crate::data::membership::PermissionContext;
use crate::data::permission::{cap, effective_owner, resolve_access_world};
use crate::data::repository::Repository;

/// Max byte length of `PlayVfx.asset`/`PlayVfx.sound` — generous for a UUID-shaped id, bounds
/// a malicious frame's size.
pub const MAX_ID_BYTES: usize = 128;
/// Inclusive upper bound on `PlayVfx.scale`; `(0, MAX_SCALE]`, `0` and negative refused.
pub const MAX_SCALE: f64 = 8.0;
/// Inclusive upper bound on `PlayVfx.duration_ms`.
pub const MAX_DURATION_MS: u32 = 60_000;

/// A `PlayVfx` request, already deserialized off the wire — the fields both call sites
/// (the raw frame handler and `/fx`) validate identically.
///
/// # Examples
///
/// ```
/// use shadowcat::ws::vfx::VfxRequest;
/// use uuid::Uuid;
///
/// let req = VfxRequest {
///     scene: Uuid::new_v4(), asset: "fx-asset".into(), x: 10.0, y: 20.0,
///     scale: Some(2.0), rotation: None, duration_ms: Some(500), sound: None, elevation: None,
/// };
/// assert_eq!(req.scale, Some(2.0));
/// ```
pub struct VfxRequest {
    /// Scene the effect plays on.
    pub scene: Uuid,
    /// The spritesheet or animated-source asset id.
    pub asset: String,
    /// Scene-coordinate x.
    pub x: f64,
    /// Scene-coordinate y.
    pub y: f64,
    /// Uniform scale multiplier; `None` = native scale.
    pub scale: Option<f64>,
    /// Rotation in degrees; `None` = unrotated.
    pub rotation: Option<f64>,
    /// Playback duration cap in ms; `None` = one loop.
    pub duration_ms: Option<u32>,
    /// Paired sound asset id.
    pub sound: Option<String>,
    /// Elevation the effect plays at.
    pub elevation: Option<f64>,
}

/// Bounds-only validation (no I/O): finite coordinates inside the shared movement-coordinate
/// bound, `scale ∈ (0, MAX_SCALE]` when present, `duration_ms <= MAX_DURATION_MS` when
/// present, `asset`/`sound` non-empty (`asset` only) and `<= MAX_ID_BYTES` bytes when present.
/// Returns `false` on any violation — the caller silently drops, exactly like
/// `scene_ping`/`emote`'s admission gates (no error frame, so a non-reader never learns why).
///
/// # Examples
///
/// ```
/// use shadowcat::ws::vfx::{validate_bounds, VfxRequest};
/// use uuid::Uuid;
///
/// let req = VfxRequest {
///     scene: Uuid::new_v4(), asset: "a".into(), x: 0.0, y: 0.0,
///     scale: Some(2.0), rotation: None, duration_ms: Some(1000), sound: None, elevation: None,
/// };
/// assert!(validate_bounds(&req));
/// ```
pub fn validate_bounds(req: &VfxRequest) -> bool {
    if !req.x.is_finite() || !req.y.is_finite() {
        return false;
    }
    let bound = crate::scene::move_exec::MAX_GATE_WALK_COORD;
    if req.x.abs() > bound || req.y.abs() > bound {
        return false;
    }
    if let Some(s) = req.scale {
        if !(s.is_finite() && s > 0.0 && s <= MAX_SCALE) {
            return false;
        }
    }
    if let Some(d) = req.duration_ms {
        if u64::from(d) > u64::from(MAX_DURATION_MS) {
            return false;
        }
    }
    if req.asset.is_empty() || req.asset.len() > MAX_ID_BYTES {
        return false;
    }
    if let Some(s) = &req.sound {
        if s.len() > MAX_ID_BYTES {
            return false;
        }
    }
    true
}

/// Whether `ctx` may fire a `PlayVfx` on `scene`: the doc must exist, be a `scene`, belong to
/// THIS world, and grant the sender `cap::READ` (the SAME `scene_ping_permitted` shape — a
/// one-shot is a table gesture like a ping, deliberately weaker than a token-authoring gate),
/// AND the sender's world role must be `Gm` or `Player` — a spectator is refused outright,
/// checked BEFORE the document read so a spectator never spends a repository round-trip.
/// Denial is a SILENT drop at the call site: any error frame or behavior split would leak
/// scene existence to a non-reader.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// use shadowcat::data::document::WorldRole;
/// use shadowcat::data::membership::PermissionContext;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::ws::vfx::vfx_permitted;
/// use uuid::Uuid;
///
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// let ctx = PermissionContext { user_id: Uuid::new_v4(), world_role: WorldRole::Spectator };
/// assert!(!vfx_permitted(Uuid::new_v4(), &ctx, Uuid::new_v4(), &repo).await);
/// # }
/// ```
pub async fn vfx_permitted(
    scene: Uuid,
    ctx: &PermissionContext,
    world_id: Uuid,
    repo: &dyn Repository,
) -> bool {
    if ctx.world_role == WorldRole::Spectator {
        return false;
    }
    let Ok(Some(doc)) = repo.get_document(scene).await else {
        return false;
    };
    if doc.doc_type != "scene" {
        return false;
    }
    if crate::data::document::world_of(&doc) != Some(world_id) {
        return false;
    }
    let Ok(defaults) = repo.world_cap_defaults(world_id).await else {
        return false;
    };
    let access = resolve_access_world(
        ctx.user_id,
        ctx.world_role,
        &doc,
        &defaults.grants_for(&doc.doc_type),
        effective_owner(&doc, None),
    );
    access.has(cap::READ)
}

#[cfg(test)]
mod tests;
