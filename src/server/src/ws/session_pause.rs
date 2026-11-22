//! The whole-session pause rule: while the world-settings document's `session.paused` is set, a
//! non-GM member can neither move tokens nor use combat controls. Everything else (chat, dice,
//! pings, doors, scene edits, sheets, notes) stays allowed, and a GM is never refused.
//!
//! Every refusal path reads this module's one rule, so the paths cannot disagree: the
//! `ClientMsg::Intent` arm asks `intent_refusal` (token position writes and combat-family
//! documents, classified by `class_of`), and the typed requests (`MoveRequest` and the eight
//! combat intents) ask `msg_refusal` (classified by `blocks_msg`). A new engine document type
//! must be added to `DOC_TYPE_CLASSES`; a test fails until it is.
//!
//! Server-authored writes (`WriteOrigin::CombatTransition`, region triggers) are never routed
//! through these checks: they only follow a GM action.

use std::sync::Arc;

use uuid::Uuid;

use crate::data::command::Operation;
use crate::data::document::{Document, WorldRole};
use crate::data::engine::{engine_of, SessionState, WorldSettingsEngine};
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::ws::protocol::{ClientMsg, RejectReason, ServerMsg};
use crate::ws::room::Room;

/// How the pause treats writes to one document type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PauseClass {
    /// Never frozen.
    Free,
    /// Frozen only for a write that changes the token's position.
    PositionWrites,
    /// Every write is frozen: the document is combat-clock state, written only through combat
    /// controls.
    Combat,
}

/// The class of every engine document type. Enumerated, not defaulted, so a new engine type is a
/// compile-visible decision: `every_engine_doc_type_is_classified` fails until it is listed.
pub(crate) const DOC_TYPE_CLASSES: &[(&str, PauseClass)] = &[
    ("token", PauseClass::PositionWrites),
    ("scene", PauseClass::Free),
    ("wall", PauseClass::Free),
    ("region", PauseClass::Free),
    ("light", PauseClass::Free),
    ("drawing", PauseClass::Free),
    ("template", PauseClass::Free),
    ("actor", PauseClass::Free),
    ("message", PauseClass::Free),
    ("world-settings", PauseClass::Free),
    ("vision-modes", PauseClass::Free),
    ("light-gradation", PauseClass::Free),
    ("chat-settings", PauseClass::Free),
    ("dice-settings", PauseClass::Free),
    ("channel-registry", PauseClass::Free),
    ("faction-registry", PauseClass::Free),
    ("condition-registry", PauseClass::Free),
    ("combat", PauseClass::Combat),
    ("combatant", PauseClass::Combat),
    ("resource-registry", PauseClass::Free),
    ("effect", PauseClass::Combat),
    ("system-defaults", PauseClass::Free),
    ("combat-history", PauseClass::Combat),
    ("combatant-group", PauseClass::Combat),
    ("asset_folder", PauseClass::Free),
    ("table_folder", PauseClass::Free),
    ("table", PauseClass::Free),
    ("note", PauseClass::Free),
    ("playlist", PauseClass::Free),
    ("audio-state", PauseClass::Free),
    ("module-settings", PauseClass::Free),
];

/// The class of `doc_type`. A type outside `DOC_TYPE_CLASSES` (a client-only type with an opaque
/// `system` band) is `Free`: it holds neither a token position nor combat state.
pub(crate) fn class_of(doc_type: &str) -> PauseClass {
    DOC_TYPE_CLASSES
        .iter()
        .find(|(t, _)| *t == doc_type)
        .map_or(PauseClass::Free, |(_, c)| *c)
}

/// Whether a paused session refuses one `Intent` op on a `doc_type` document. `position_write`
/// is `true` when the op changes a token's position; it matters only for `PositionWrites`.
pub(crate) fn blocks_intent_op(doc_type: &str, position_write: bool) -> bool {
    match class_of(doc_type) {
        PauseClass::Free => false,
        PauseClass::PositionWrites => position_write,
        PauseClass::Combat => true,
    }
}

/// Whether a paused session refuses a typed client request: a `MoveRequest` or one of the eight
/// combat intents.
pub(crate) fn blocks_msg(msg: &ClientMsg) -> bool {
    matches!(
        msg,
        ClientMsg::MoveRequest { .. }
            | ClientMsg::CombatStart { .. }
            | ClientMsg::CombatPause { .. }
            | ClientMsg::CombatEnd { .. }
            | ClientMsg::CombatAdvance { .. }
            | ClientMsg::CombatRewind { .. }
            | ClientMsg::CombatSort { .. }
            | ClientMsg::CombatRoll { .. }
            | ClientMsg::CombatResource { .. }
    )
}

/// The session state a world-settings document carries; an absent document or field is the
/// default (not paused).
pub(crate) fn session_of(doc: Option<&Document>) -> SessionState {
    doc.map(engine_of::<WorldSettingsEngine>)
        .and_then(|w| w.session)
        .unwrap_or_default()
}

/// The correlated refusal frame for a blocked typed request: `MoveError` for a move, `CombatError`
/// for a combat intent, both carrying `RejectReason::Paused`. `None` when `msg` is not blocked.
fn paused_reply(msg: &ClientMsg) -> Option<ServerMsg> {
    let message = "the game is paused".to_string();
    let reason = Some(RejectReason::Paused);
    let request_id = match msg {
        ClientMsg::MoveRequest { request_id, .. }
        | ClientMsg::CombatStart { request_id, .. }
        | ClientMsg::CombatPause { request_id, .. }
        | ClientMsg::CombatEnd { request_id, .. }
        | ClientMsg::CombatAdvance { request_id, .. }
        | ClientMsg::CombatRewind { request_id, .. }
        | ClientMsg::CombatSort { request_id, .. }
        | ClientMsg::CombatRoll { request_id, .. }
        | ClientMsg::CombatResource { request_id, .. } => *request_id,
        _ => return None,
    };
    Some(if matches!(msg, ClientMsg::MoveRequest { .. }) {
        ServerMsg::MoveError {
            request_id,
            message,
            reason,
        }
    } else {
        ServerMsg::CombatError {
            request_id,
            message,
            reason,
        }
    })
}

impl Room {
    /// The world's current session state, read from the live scene ECS's world-settings document.
    pub(crate) async fn session_state(&self) -> SessionState {
        session_of(self.scene().read().await.world_settings_doc())
    }

    /// Whether the pause refuses `ctx`: the session is paused and `ctx` is not a GM.
    pub(crate) async fn session_refuses(&self, ctx: &PermissionContext) -> bool {
        ctx.world_role != WorldRole::Gm && self.session_state().await.paused
    }

    /// The session state before `ops` commit, when `ops` write the world-settings document; the
    /// caller compares it with `session_state` after the commit to detect a pause or resume.
    pub(crate) async fn session_before_write(&self, ops: &[Operation]) -> Option<SessionState> {
        let scene = self.scene().read().await;
        let ws = scene.world_settings_doc()?;
        let writes = ops
            .iter()
            .any(|op| matches!(op, Operation::Update { doc_id, .. } if *doc_id == ws.id));
        writes.then(|| session_of(Some(ws)))
    }
}

/// The `Reject` frame for an `Intent` a paused session refuses `ctx`, or `None` when the intent
/// may proceed. An op is refused when `blocks_intent_op` says so for its document type: a token
/// `Update` is a position write when its post-image position differs from the committed one
/// (`SceneEcs::token_move`, the same comparison `Room::publish`'s position gate makes).
pub(crate) async fn intent_refusal(
    room: &Room,
    repo: &dyn Repository,
    ctx: &PermissionContext,
    intent_id: Uuid,
    ops: &[Operation],
) -> Option<Arc<ServerMsg>> {
    if !room.session_refuses(ctx).await {
        return None;
    }
    for op in ops {
        let blocked = match op {
            Operation::Create { doc } | Operation::Delete { doc } => {
                blocks_intent_op(&doc.doc_type, false)
            }
            Operation::Update { doc_id, changes } => {
                let Ok(Some(doc)) = repo.get_document(*doc_id).await else {
                    continue;
                };
                let moved = doc.doc_type == "token"
                    && room
                        .scene()
                        .read()
                        .await
                        .token_move(*doc_id, changes)
                        .is_some_and(|(_, from, to)| from != to);
                blocks_intent_op(&doc.doc_type, moved)
            }
            Operation::Move { doc_id, .. } => {
                let Ok(Some(doc)) = repo.get_document(*doc_id).await else {
                    continue;
                };
                blocks_intent_op(&doc.doc_type, false)
            }
        };
        if blocked {
            return Some(Arc::new(ServerMsg::Reject {
                intent_id,
                reason: RejectReason::Paused,
                detail: None,
            }));
        }
    }
    None
}

/// The refusal frame for a typed request a paused session refuses `ctx`, or `None` when the
/// request may proceed.
pub(crate) async fn msg_refusal(
    room: &Room,
    ctx: &PermissionContext,
    msg: &ClientMsg,
) -> Option<Arc<ServerMsg>> {
    if !blocks_msg(msg) || !room.session_refuses(ctx).await {
        return None;
    }
    paused_reply(msg).map(Arc::new)
}

#[cfg(test)]
mod tests;
