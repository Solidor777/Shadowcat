//! The decision for a player's rotation write on a token, and the reads that feed it.
//!
//! One predicate, `rotation_refusal`, decides every refusal of a token's rotation: the per-token
//! `TokenEngine::lock_rotation` and the scene's `FacingTurnRule`. `Room::publish` calls it for a
//! non-GM `Update`; `Room::execute_move` calls its lock half, `lock_refusal`, to decide whether a
//! completed move may auto-rotate, so the lock cannot mean two things at two sites. A GM is exempt
//! from the manual-write gate by the caller (the gate sits in `Room::publish`'s non-GM branch).

use uuid::Uuid;

use super::{mirror_field_change, MirrorInput, SceneEcs};
use crate::data::command::FieldChange;
use crate::data::engine as eng;
use crate::data::membership::PermissionContext;

/// Why a rotation was refused.
///
/// # Examples
///
/// ```
/// use shadowcat::scene::rotation_gate::{lock_refusal, RotationRefusal};
///
/// assert_eq!(lock_refusal(true), Some(RotationRefusal::Locked));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationRefusal {
    /// The token has `TokenEngine::lock_rotation` set.
    Locked,
    /// The scene's `FacingTurnRule::OwnTurn` is in force and the token does not hold the turn.
    NotYourTurn,
}

/// Where a token stands in its scene's active combat, as far as the turn rule cares.
///
/// # Examples
///
/// ```
/// use shadowcat::scene::rotation_gate::TurnStanding;
///
/// assert_ne!(TurnStanding::HoldsTurn, TurnStanding::AwaitsTurn);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStanding {
    /// No active combat on the token's scene, the token names no combatant in it, or the caller
    /// cannot read that combatant (a refusal would disclose it). The turn rule never applies.
    Unconstrained,
    /// The token's combatant holds `CombatEngine::turn`.
    HoldsTurn,
    /// The token's combatant is in the active combat and does not hold the turn.
    AwaitsTurn,
}

/// The lock half of the rotation decision: a locked token is never rotated by anyone's write or
/// by a move's auto-rotate.
///
/// # Examples
///
/// ```
/// use shadowcat::scene::rotation_gate::lock_refusal;
///
/// assert!(lock_refusal(false).is_none());
/// ```
pub fn lock_refusal(locked: bool) -> Option<RotationRefusal> {
    locked.then_some(RotationRefusal::Locked)
}

/// The full rotation decision for a non-GM write: the lock first, then the scene's turn rule.
/// `None` means the rotation may proceed.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::FacingTurnRule;
/// use shadowcat::scene::rotation_gate::{rotation_refusal, RotationRefusal, TurnStanding};
///
/// let off_turn = rotation_refusal(false, FacingTurnRule::OwnTurn, TurnStanding::AwaitsTurn);
/// assert_eq!(off_turn, Some(RotationRefusal::NotYourTurn));
/// ```
pub fn rotation_refusal(
    locked: bool,
    rule: eng::FacingTurnRule,
    standing: TurnStanding,
) -> Option<RotationRefusal> {
    lock_refusal(locked).or(match (rule, standing) {
        (eng::FacingTurnRule::OwnTurn, TurnStanding::AwaitsTurn) => {
            Some(RotationRefusal::NotYourTurn)
        }
        _ => None,
    })
}

/// A token `Update`'s effect on its rotation, resolved from the committed document.
///
/// # Examples
///
/// ```
/// use shadowcat::scene::rotation_gate::TokenRotationWrite;
///
/// let w = TokenRotationWrite {
///     scene: uuid::Uuid::new_v4(),
///     locked: false,
///     rotation_changed: true,
/// };
/// assert!(w.rotation_changed);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TokenRotationWrite {
    /// The scene the token is parented to.
    pub scene: Uuid,
    /// The token's committed `TokenEngine::lock_rotation`.
    pub locked: bool,
    /// Whether the post-image's `/engine/rotation` differs from the committed one. A post-image
    /// with no numeric rotation counts as a change (it cannot commit as valid, so the answer is
    /// never relied on to admit it).
    pub rotation_changed: bool,
}

impl SceneEcs {
    /// Resolve whether `changes` would alter a token's rotation: the committed rotation is
    /// compared with the post-image of all `changes` applied in array order (the same store-equal
    /// mutation rule `token_move` uses, so a wholesale `/engine` write is judged on what it would
    /// commit). Bitwise inequality, not an epsilon window. `None` when the token is unknown, is
    /// not a token, has no `parent_id` or has an engine body that does not decode.
    ///
    /// # Examples
    ///
    /// ```
    /// let ecs = shadowcat::scene::SceneEcs::new();
    /// assert!(ecs.token_rotation_write(uuid::Uuid::new_v4(), &[]).is_none());
    /// ```
    pub fn token_rotation_write(
        &self,
        token_id: Uuid,
        changes: &[FieldChange],
    ) -> Option<TokenRotationWrite> {
        let &e = self.index.get(&token_id)?;
        let (scene, locked, committed, doc_value) = {
            let tok = self.world.get::<&super::SceneEntity>(e).ok()?;
            if tok.doc.doc_type != "token" {
                return None;
            }
            let scene = tok.doc.parent_id?;
            let t: eng::TokenEngine = self.engine_as_cached(token_id, &tok.doc)?;
            let v = serde_json::to_value(&tok.doc).ok()?;
            (scene, t.lock_rotation, t.rotation, v)
        };
        let mut v = doc_value;
        for ch in changes {
            mirror_field_change(&mut v, ch, MirrorInput::Proposed);
        }
        let rotation_changed = v
            .pointer("/engine/rotation")
            .and_then(|r| r.as_f64())
            .is_none_or(|post| post != committed);
        Some(TokenRotationWrite {
            scene,
            locked,
            rotation_changed,
        })
    }

    /// Where `token` stands in `scene`'s active combat for the turn rule. `Unconstrained` when
    /// there is no active combat, the token names no combatant in it, or `ctx` lacks whole-document
    /// `cap::READ` on that combatant: a turn refusal would otherwise disclose a combatant the
    /// caller cannot read (the same standing `BudgetGate::enforced` applies to the movement gate).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::document::{WorldCapDefaults, WorldRole};
    /// use shadowcat::data::membership::PermissionContext;
    /// use shadowcat::scene::rotation_gate::TurnStanding;
    /// use shadowcat::scene::SceneEcs;
    ///
    /// let ecs = SceneEcs::new();
    /// let ctx = PermissionContext { user_id: uuid::Uuid::new_v4(), world_role: WorldRole::Player };
    /// let defaults = WorldCapDefaults::default();
    /// let standing =
    ///     ecs.token_turn_standing(uuid::Uuid::new_v4(), uuid::Uuid::new_v4(), &ctx, &defaults);
    /// assert_eq!(standing, TurnStanding::Unconstrained);
    /// ```
    pub fn token_turn_standing(
        &self,
        scene: Uuid,
        token: Uuid,
        ctx: &PermissionContext,
        world_defaults: &crate::data::document::WorldCapDefaults,
    ) -> TurnStanding {
        let Some((combat_id, ce)) = self.active_combat_for_scene(scene) else {
            return TurnStanding::Unconstrained;
        };
        let Some((combatant_id, _, access)) =
            self.combatant_for_token(combat_id, token, ctx, world_defaults)
        else {
            return TurnStanding::Unconstrained;
        };
        if !access.has(crate::data::permission::cap::READ) {
            return TurnStanding::Unconstrained;
        }
        if ce.turn == Some(combatant_id) {
            TurnStanding::HoldsTurn
        } else {
            TurnStanding::AwaitsTurn
        }
    }
}

#[cfg(test)]
mod tests;
