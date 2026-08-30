//! The server-owned combat clock: loads a `CombatSnapshot`, runs a pure
//! transition (`transition`), and hands ONE command's ops to `Room`.
//! INVARIANT: nothing here evaluates a `Formula::Text`; every transition reads
//! only resolved numbers/flags and skips what is unresolved.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::HashSet;

use uuid::Uuid;

use crate::data::command::Operation;
use crate::data::document::WorldRole;
use crate::data::engine::combat::TurnControl;
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::ws::protocol::{ClientMsg, ResourceOp as WireResourceOp, ServerMsg};
use crate::ws::room::Room;

pub mod effects;
pub mod history;
pub mod ops;
pub mod snapshot;
pub mod transition;

#[cfg(test)]
mod tests;

pub use effects::collect_effects;
pub use snapshot::{load_snapshot, CombatSnapshot, Combatant};
/// Test-only seam onto `transition::advance_with_step_count` — see its own
/// doc comment.
#[cfg(test)]
pub(crate) use transition::advance_with_step_count;
pub use transition::{
    advance, end, pause, rebuild_order, resource, rewind, roll, sort, start, ResourceOp, RollPost,
};

/// Why a combat intent was refused. `Display` yields ONE wording for every
/// variant that could disclose a hidden combatant (`NotFound`, `Forbidden`,
/// `NotRunning`, `Data`): "combat rejected".
#[derive(Debug, thiserror::Error)]
pub enum CombatError {
    /// Combat, combatant, resource or host not found (or not readable).
    #[error("combat rejected")]
    NotFound,
    /// The actor may not perform this intent.
    #[error("combat rejected")]
    Forbidden,
    /// The combat is not active.
    #[error("combat rejected")]
    NotRunning,
    /// Nothing to advance/rewind (empty order, first record).
    #[error("nothing to do")]
    Empty,
    /// Rewind refused: at the first record or past the retained history.
    #[error("cannot rewind further")]
    Unrewindable,
    /// A roll failed its caps/parse.
    #[error("{0}")]
    Roll(#[from] crate::chat::rolls::RollError),
    /// Repository failure.
    #[error("combat rejected")]
    Data(#[from] crate::data::DataError),
    /// `CombatRoll` named the same `combatant_id` more than once. Distinct
    /// wording is safe: every named id was already authorized as the
    /// caller's own (or the GM's) by `authorize`, so this discloses nothing
    /// the caller doesn't already know.
    #[error("duplicate combatant in rolls")]
    DuplicateRoll,
    /// The caller's per-minute combat-intent flood budget is exhausted.
    /// Distinct wording (never leaks combat/hidden state, same class as
    /// `SendMessageError::RateLimited`).
    #[error("You are performing combat actions too quickly. Please wait a moment.")]
    RateLimited,
}

/// Dispatches one combat intent frame: checks the caller's flood budget,
/// loads the snapshot, authorizes, resolves the transition's ops, and
/// commits them as ONE server-authored command via `Room::commit_combat`.
/// `None` on success — the broadcast `Event` is the notification, mirroring
/// `SendMessage`'s asymmetric reply protocol; `Some(ServerMsg::CombatError)`
/// on refusal, `message` set to `CombatError`'s own `Display` text so every
/// refusal path (including an unrecognized/foreign `combat_id`) renders
/// identically to the sender. `rate` is checked BEFORE the snapshot's
/// multi-query doc read — cheap check first, mirroring `ScenePing`'s guard
/// order in `conn.rs` — and spends `MESSAGE_RATE_PER_MIN` against the SAME
/// `WsState::message_rate` counter the chat handlers use: a combat intent
/// costs one snapshot doc read plus a commit, the same order of cost, so the
/// budget is read from its single declaration rather than restated here.
pub async fn handle_combat_intent(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    msg: ClientMsg,
    now: i64,
    rate: &crate::ws::PingRateLimiter,
) -> Option<ServerMsg> {
    let (request_id, combat_id) = match &msg {
        ClientMsg::CombatStart {
            request_id,
            combat_id,
        }
        | ClientMsg::CombatPause {
            request_id,
            combat_id,
        }
        | ClientMsg::CombatEnd {
            request_id,
            combat_id,
        }
        | ClientMsg::CombatAdvance {
            request_id,
            combat_id,
        }
        | ClientMsg::CombatRewind {
            request_id,
            combat_id,
        }
        | ClientMsg::CombatSort {
            request_id,
            combat_id,
        }
        | ClientMsg::CombatRoll {
            request_id,
            combat_id,
            ..
        }
        | ClientMsg::CombatResource {
            request_id,
            combat_id,
            ..
        } => (*request_id, *combat_id),
        // Unreachable from the dispatch match's own combined arm, which
        // routes only the eight combat variants here.
        _ => return None,
    };

    if !rate.check(ctx.user_id, now, crate::ws::MESSAGE_RATE_PER_MIN) {
        return Some(to_server_msg(request_id, CombatError::RateLimited));
    }

    match run_intent(room, repo, ctx, msg, combat_id, now).await {
        Ok(()) => None,
        Err(e) => Some(to_server_msg(request_id, e)),
    }
}

/// The load → authorize → resolve → commit pipeline for one combat intent,
/// once `combat_id` has been extracted from `msg`.
async fn run_intent(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    msg: ClientMsg,
    combat_id: Uuid,
    now: i64,
) -> Result<(), CombatError> {
    let snap = load_snapshot(repo, room.world_id, combat_id).await?;
    authorize(&snap, ctx, &msg)?;
    let ops = build_ops(room, repo, ctx, &snap, msg, now).await?;
    room.commit_combat(repo, ctx, ops, now).await?;
    Ok(())
}

/// Non-GM authorization scope for a combat intent; a GM may always act.
/// `CombatAdvance` additionally admits the CURRENT turn's owner when
/// `turn_control == TurnControl::OwnerMayEnd` and that combatant isn't
/// hidden. `CombatRoll`/`CombatResource` admit the owner of every NAMED
/// combatant, provided none is hidden. Every other variant
/// (`CombatStart`/`CombatPause`/`CombatEnd`/`CombatRewind`/`CombatSort`) is
/// GM-only. INVARIANT: every refusal here is `CombatError::Forbidden` or
/// `CombatError::NotFound`, which render IDENTICALLY via `CombatError`'s
/// `Display` — this function never leaks which case fired.
fn authorize(
    snap: &CombatSnapshot,
    ctx: &PermissionContext,
    msg: &ClientMsg,
) -> Result<(), CombatError> {
    if ctx.world_role == WorldRole::Gm {
        return Ok(());
    }
    match msg {
        ClientMsg::CombatAdvance { .. } => {
            if snap.engine.turn_control != TurnControl::OwnerMayEnd {
                return Err(CombatError::Forbidden);
            }
            let current = snap
                .engine
                .turn
                .and_then(|id| snap.combatants.iter().find(|c| c.doc.id == id));
            match current {
                Some(c) if c.doc.owner == Some(ctx.user_id) && !transition::is_hidden(c) => Ok(()),
                _ => Err(CombatError::Forbidden),
            }
        }
        ClientMsg::CombatRoll { rolls, .. } => {
            // An empty `rolls` list has no entry to check ownership
            // against, so the loop below would vacuously succeed for ANY
            // non-GM world member regardless of any relationship to this
            // combat — reject it outright rather than let authorization
            // for "no rolls" fall out of an empty loop.
            if rolls.is_empty() {
                return Err(CombatError::Forbidden);
            }
            for entry in rolls {
                let c = snap
                    .combatants
                    .iter()
                    .find(|c| c.doc.id == entry.combatant_id)
                    .ok_or(CombatError::NotFound)?;
                if c.doc.owner != Some(ctx.user_id) || transition::is_hidden(c) {
                    return Err(CombatError::Forbidden);
                }
            }
            Ok(())
        }
        ClientMsg::CombatResource { combatant_id, .. } => {
            let c = snap
                .combatants
                .iter()
                .find(|c| c.doc.id == *combatant_id)
                .ok_or(CombatError::NotFound)?;
            if c.doc.owner != Some(ctx.user_id) || transition::is_hidden(c) {
                Err(CombatError::Forbidden)
            } else {
                Ok(())
            }
        }
        // CombatStart/CombatPause/CombatEnd/CombatRewind/CombatSort: GM-only.
        _ => Err(CombatError::Forbidden),
    }
}

/// Resolves `msg` into the ops for ONE command, dispatching to the matching
/// pure `transition` function (or, for `CombatRoll`, executing the named
/// rolls first via the channel's resolved dice context). Every other
/// refusal (`NotRunning`, `Unrewindable`, `Empty`, roll caps, ...) surfaces
/// from the transition function's/roll executor's own `Result`.
async fn build_ops(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    snap: &CombatSnapshot,
    msg: ClientMsg,
    now: i64,
) -> Result<Vec<Operation>, CombatError> {
    match msg {
        ClientMsg::CombatStart { .. } => start(snap, now, room.world_id, ctx.user_id),
        ClientMsg::CombatPause { .. } => pause(snap),
        ClientMsg::CombatEnd { .. } => end(snap),
        ClientMsg::CombatAdvance { .. } => advance(snap, room.world_id, ctx.user_id, now),
        ClientMsg::CombatRewind { .. } => rewind(snap, now),
        ClientMsg::CombatSort { .. } => sort(snap),
        ClientMsg::CombatRoll { channel, rolls, .. } => {
            // INVARIANT: `transition::roll` requires at most one entry per
            // combatant id (its own doc comment names the wire-dispatch
            // layer — this function — as responsible for that); a
            // duplicate would build two `FieldChange`s against the same
            // combatant from one stale pre-image.
            let mut seen = HashSet::new();
            for entry in &rolls {
                if !seen.insert(entry.combatant_id) {
                    return Err(CombatError::DuplicateRoll);
                }
            }
            let dice_ctx = crate::chat::resolve_dice_context(repo, room.world_id, &channel).await;
            let mut posts = Vec::with_capacity(rolls.len());
            for entry in &rolls {
                let (formula, outcome, spec, raw) =
                    crate::chat::rolls::execute_roll(&entry.notation, dice_ctx)?;
                posts.push((
                    entry.combatant_id,
                    RollPost {
                        formula,
                        outcome,
                        spec,
                        raw,
                    },
                ));
            }
            roll(snap, &posts, room.world_id, ctx.user_id, &channel, now)
        }
        ClientMsg::CombatResource {
            combatant_id,
            resource: key,
            op,
            ..
        } => {
            let op = match op {
                WireResourceOp::Delta { amount } => ResourceOp::Delta { amount },
                WireResourceOp::Set { value } => ResourceOp::Set { value },
            };
            resource(snap, combatant_id, &key, op)
        }
        // Unreachable: `run_intent` is called only for the eight combat
        // variants extracted by `handle_combat_intent`.
        _ => Err(CombatError::Forbidden),
    }
}

/// Renders a refusal as the wire `ServerMsg::CombatError`, correlated by
/// `request_id`. `message` is `CombatError`'s own `Display` text.
fn to_server_msg(request_id: Uuid, e: CombatError) -> ServerMsg {
    ServerMsg::CombatError {
        request_id,
        message: e.to_string(),
    }
}
