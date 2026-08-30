//! The server-owned combat clock: loads a `CombatSnapshot`, runs a pure
//! transition (`transition`), and hands ONE command's ops to `Room`.
//! INVARIANT: every formula a transition acts on is evaluated HERE, through
//! `crate::formula` over the combatant's formula host (`eval`); an
//! evaluation failure skips its one write and surfaces as a GM-only chat
//! notice — the clock never stops on a bad formula.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::HashSet;

use uuid::Uuid;

use crate::data::command::Operation;
use crate::data::document::{WorldCapDefaults, WorldRole};
use crate::data::engine::combat::TurnControl;
use crate::data::membership::PermissionContext;
use crate::data::permission::{
    cap, effective_owner, required_cap_for_path, resolve_access_world, Access,
};
use crate::data::repository::Repository;
use crate::ws::protocol::{ClientMsg, ResourceOp as WireResourceOp, ServerMsg};
use crate::ws::room::Room;

pub mod effects;
pub(crate) mod eval;
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
    /// Rewind refused: the clock state the target boundary describes is not a
    /// valid `CombatEngine` against the combat as it stands now, so applying it
    /// would be refused by the engine-ingress gate and roll the whole command
    /// back. The reachable case is `rewind_restore` off with a target boundary
    /// whose `turn` names a combatant since removed from `/engine/order` — an
    /// exhausted `Event`, which `transition::resolve_event` deletes and drops
    /// from the order, and which only a restore would bring back. Distinct
    /// wording is safe: `CombatRewind` is GM-only (`authorize`), so this text
    /// reaches only a caller who may already read every combatant in the combat.
    #[error("cannot rewind to that boundary")]
    RewindUnreachable,
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
    // The world's default capability grants, read ONCE per intent — an input to
    // `authorize`'s whole-document `cap::READ` resolution, which may consult
    // several combatants (`CombatRoll` names a list) and must resolve each
    // against the same grants rather than re-reading them per combatant.
    // Propagates with `?` rather than defaulting: an unresolvable authority
    // input fails closed (`CombatError::Data`, rendered as the same generic
    // "combat rejected" every other refusal is) instead of being guessed at.
    // Read AFTER the snapshot so an unknown/foreign `combat_id` still costs
    // only the snapshot's own existence-hiding refusal.
    let world_defaults = repo.world_cap_defaults(room.world_id).await?;
    authorize(&snap, ctx, &msg, &world_defaults)?;
    let ops = build_ops(room, repo, ctx, &snap, msg, now).await?;
    room.commit_combat(repo, ctx, ops, now).await?;
    Ok(())
}

/// The access `ctx` holds on combatant `c`'s document, resolved through the
/// SAME `effective_owner` + `resolve_access_world` pair document egress uses
/// (`filter_command`) and the movement-budget gate reads
/// (`SceneEcs::ctx_access`) — never a hand-rolled readability or ownership
/// predicate. A combatant's hidden state IS whole-document unreadability, so
/// `permissions.users` entries and capability grants decide it here exactly as
/// they do at egress.
///
/// A `combatant` document never carries an actor link (`token_actor_link` is
/// `token`-only), so the no-join `effective_owner(doc, None)` resolution is
/// exact — the same reasoning `scene_ping_permitted` states for a scene doc.
///
/// Returns the whole `Access` because `authorize` asks THREE questions of it —
/// whole-document `cap::READ`, effective ownership, and (for the intents that
/// author content on the combatant — `CombatantAct::WritesEngine`) the write
/// capability `required_cap_for_path` maps that write to — and resolving those
/// from different rules is how the answers drift apart.
fn combatant_access(
    c: &Combatant,
    ctx: &PermissionContext,
    world_defaults: &WorldCapDefaults,
) -> Access {
    resolve_access_world(
        ctx.user_id,
        ctx.world_role,
        &c.doc,
        &world_defaults.grants_for(&c.doc.doc_type),
        effective_owner(&c.doc, None),
    )
}

/// The band root every combat transition's combatant write lands under:
/// `transition::roll` writes `/engine/initiative` and `transition::resource`
/// writes `/engine/resources/<key>/current`. `writes_a_content_band` classifies
/// a write path by its BAND, so every path under this root resolves to the same
/// capability and the root is an exact stand-in for either of them.
const COMBATANT_WRITE_BAND: &str = "/engine";

/// What a combat intent does to the combatant document it names, and therefore
/// which capabilities `owns_combatant` demands on it.
enum CombatantAct {
    /// The intent authors no content on the named combatant. `CombatAdvance`
    /// ends the turn its caller holds; the combatant writes
    /// `transition::advance` does produce — `run_boundary`'s `recover`
    /// amounts, effect ticks, an `Event`'s `lifespan` decrement — are
    /// server-COMPUTED consequences of the clock moving, and they land on
    /// whichever combatants the boundary sweep touches, not on one the caller
    /// named or supplied a value for. Authorization is therefore the
    /// turn-ownership gameplay rule (who may end THIS turn) alone: no
    /// per-document write capability of the caller's could gate those writes
    /// coherently, since they reach combatants the caller has no relationship
    /// with at all.
    EndsTurn,
    /// The intent writes the named combatant's `engine` band: `CombatRoll`
    /// through `transition::roll`, `CombatResource` through
    /// `transition::resource`.
    WritesEngine,
}

/// Whether `ctx` may act on combatant `c` as its owner for `act`: it holds
/// whole-document `cap::READ` on that combatant, is its effective owner, and —
/// under `CombatantAct::WritesEngine` — additionally holds the capability
/// writing `COMBATANT_WRITE_BAND` requires, all read off ONE `combatant_access`
/// resolution.
///
/// The `cap::READ` half is the real read authority, not a
/// `permissions.default` test, so a `permissions.users` entry moves it in BOTH
/// directions: a per-user grant on a `default: none` combatant makes its owner
/// able to act (they genuinely receive that document at egress), and a per-user
/// `None` override on a `default: observer` combatant refuses even its `owner`
/// (they never receive it, so admitting their writes would be an authorization
/// hole — these writes commit under `WriteOrigin::CombatTransition`, which
/// waives `apply_intent`'s own ownership check, making `authorize` the sole
/// gate).
///
/// The write half exists because ownership is NOT a proxy for write capability
/// on a combatant: `effective_role`'s ownership floor is scoped to
/// `TOKEN_DOC_TYPE`, so a combatant's owner is floored at nothing and can hold
/// `DocRole::Observer` (`cap::READ` without `cap::WRITE_FIELDS`). The required
/// capability is READ FROM `required_cap_for_path` — the single statement of
/// the path-to-capability rule, and the very check
/// `WriteOrigin::CombatTransition` waives inside `apply_intent` — rather than
/// restated here, so the two cannot answer differently. An unmappable path
/// (`None`) refuses, matching `apply_intent`'s own treatment of one.
///
/// Ownership stays a hard requirement alongside the capability, never an
/// alternative to it: a combat intent is an act BY a combatant's owner, so a
/// non-owner holding `cap::WRITE_FIELDS` may write that document through an
/// ordinary `Intent` but may not drive the clock with it.
fn owns_combatant(
    c: &Combatant,
    ctx: &PermissionContext,
    world_defaults: &WorldCapDefaults,
    act: CombatantAct,
) -> bool {
    let access = combatant_access(c, ctx, world_defaults);
    let may_write = match act {
        CombatantAct::EndsTurn => true,
        CombatantAct::WritesEngine => {
            required_cap_for_path(COMBATANT_WRITE_BAND).is_some_and(|need| access.has(need))
        }
    };
    access.is_owner && access.has(cap::READ) && may_write
}

/// Non-GM authorization scope for a combat intent; a GM may always act.
/// `CombatAdvance` additionally admits the CURRENT turn's owner when
/// `turn_control == TurnControl::OwnerMayEnd`. `CombatRoll`/`CombatResource`
/// admit the owner of every NAMED combatant. Every other variant
/// (`CombatStart`/`CombatPause`/`CombatEnd`/`CombatRewind`/`CombatSort`) is
/// GM-only and consults no combatant at all.
///
/// "Owner" here is `owns_combatant`: effective ownership AND whole-document
/// `cap::READ`, both resolved through the shared `resolve_access_world`
/// authority. `CombatRoll`/`CombatResource` pass `CombatantAct::WritesEngine`
/// and therefore additionally demand the capability writing the combatant's
/// `engine` band requires; `CombatAdvance` passes `CombatantAct::EndsTurn` and
/// demands no write capability, because its combatant writes are server-
/// computed clock consequences rather than caller-authored content (see that
/// variant's own doc). This is the ONLY authorization these writes get — they
/// commit under `WriteOrigin::CombatTransition`, which waives `apply_intent`'s
/// own per-op ownership AND capability checks — so a predicate that diverged
/// from those shared authorities would be an authorization hole in one
/// direction and a refusal of a legitimate owner in the other.
///
/// INVARIANT: every refusal here is `CombatError::Forbidden` or
/// `CombatError::NotFound`, which render IDENTICALLY via `CombatError`'s
/// `Display` — this function never leaks which case fired.
fn authorize(
    snap: &CombatSnapshot,
    ctx: &PermissionContext,
    msg: &ClientMsg,
    world_defaults: &WorldCapDefaults,
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
                Some(c) if owns_combatant(c, ctx, world_defaults, CombatantAct::EndsTurn) => Ok(()),
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
                if !owns_combatant(c, ctx, world_defaults, CombatantAct::WritesEngine) {
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
            if owns_combatant(c, ctx, world_defaults, CombatantAct::WritesEngine) {
                Ok(())
            } else {
                Err(CombatError::Forbidden)
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
        ClientMsg::CombatEnd { .. } => end(snap, room.world_id, ctx.user_id, now),
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
