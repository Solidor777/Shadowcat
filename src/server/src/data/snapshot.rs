//! Commit-time redaction snapshot: `StoredCommand`, `CommandSnapshot`, `OpSnapshot`. Carries
//! the policy in force AT COMMIT alongside a `Command`, so replay redaction can compute the
//! recipient's hidden set as `hidden_current ∪ hidden_commit` instead of re-deriving
//! `hidden_commit` from today's (wrong) policy. Server-internal only: never serialized to the
//! wire — `Operation`/`Command`/`ClientMsg`/`ServerMsg` and their ts-rs/Zod mirrors are
//! untouched by this module's existence.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::data::command::Command;
use crate::data::document::{PermissionSet, Visibility};

/// Commit-time redaction inputs for one op in a `Command`, sufficient to compute the
/// commit-time half of redaction WITHOUT any live lookup — no `&Repository`, no actor-lookup
/// closure, by construction (a live parameter cannot be reintroduced here without a loud
/// signature change). Built ONCE per command, from the command's own post-image (never from an
/// op's own per-iteration intermediate state — a per-op snapshot mid-command would leak a value
/// a LATER op in the same command hides).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpSnapshot {
    /// Effective owner at commit (`permission::effective_owner`/`SqliteRepository::
    /// load_effective_owner` evaluated against the post-image's actor-link state). `None` if
    /// the document has no effective owner.
    pub owner_at_commit: Option<Uuid>,
    /// `doc_type` at commit. Carried here too (not just read off `Operation::Create`/`Delete`'s
    /// own `doc`) because `Operation::Update` has no `doc_type` of its own, and
    /// `permission::effective_role`'s token-owner-floor check needs it.
    pub doc_type: String,
    /// The document's permission-override tree at commit: `property_overrides` for the document
    /// itself plus every embedded descendant, addressed identically to the live redaction
    /// walk's convention (`{prefix}/embedded/{key}/{idx}`, built from the POST-image's
    /// `embedded` map). For `Update`, pruned to the ancestor/descendant closure of this op's own
    /// `changes` paths (only an overlapping override can possibly redact THIS op's field-level
    /// deltas) UNLESS `retraction_hidden_at_commit` is `Some`, in which case that field carries
    /// the full, unpruned set separately. For `Create`/`Delete` (whose "changed paths" are the
    /// whole document), this is the full, unpruned set.
    pub overrides_at_commit: Vec<(String, Visibility)>,
    /// Present only when this `Update` op's own `changes` narrow visibility
    /// (`permission::touches_permissions`): the FULL (unpruned) commit-time hidden-pointer set
    /// for the document, with each pointer's `Visibility` tier retained (never a bare pointer
    /// list — the tier is needed to filter per-recipient via `Access::can_see` at replay time,
    /// not apply the same retraction to every recipient regardless of their own access). Always
    /// `None` for `Create`/`Delete` (a whole-document reveal/removal needs no incremental
    /// retraction of stale client-side field values).
    pub retraction_hidden_at_commit: Option<Vec<(String, Visibility)>>,
    /// Present only for `Update`/`Delete` (a `Create` establishes a fresh generation and needs
    /// no witness): the target document's `documents.created_seq` as read at commit time.
    /// Compared against the CURRENT document's `created_seq` at redaction time; a mismatch means
    /// the id was deleted and recreated since commit, and the op is dropped rather than
    /// redacted-and-delivered against the wrong generation.
    pub created_seq_at_commit: Option<i64>,
    /// The target document's OWN `PermissionSet` at commit — `default`/`users`/`gm_role`/
    /// `capabilities` only; `property_overrides` is always empty here (that data is separately
    /// captured, pruned, in `overrides_at_commit`/`retraction_hidden_at_commit`). `Some` only
    /// for `Update`, whose `Operation` carries no `permissions` of its own to reuse directly
    /// (unlike `Create`/`Delete`, which reuse their own carried `doc.permissions` verbatim).
    /// Feeds the whole-document commit-time `cap::READ` gate via `permission::resolve_access`,
    /// reused unmodified rather than re-derived.
    pub permissions_at_commit: Option<PermissionSet>,
    /// `Update` only: the document's permissions BEFORE this op applied, with
    /// `property_overrides` cleared like `permissions_at_commit`. Feeds
    /// `filter_command`'s READ-transition rule (a recipient who gains READ in
    /// this op receives a synthesized `Create`, one who loses it a `Delete`).
    /// `None` on `Create`/`Delete` and on rows persisted before the field
    /// existed, where the rule stays inert.
    #[serde(default)]
    pub permissions_before_commit: Option<PermissionSet>,
}

/// Commit-time redaction inputs for a whole `Command`, index-aligned with `Command.ops`. Built
/// ONCE per command, after every op in the command has applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandSnapshot {
    /// `None` at an index means "no snapshot recorded for this op" — the back-compat case for a
    /// legacy `world_events` row carrying no `snapshot` key. `filter_command` DROPS an op whose
    /// snapshot is `None` on replay, rather than falling back to a live-lookup redaction.
    pub per_op: Vec<Option<OpSnapshot>>,
    /// Whether each of the world's members held GM standing in this world AT THIS COMMAND'S
    /// COMMIT — computed once per command (world role has nothing to do with which documents an
    /// op touches, unlike `overrides_at_commit`). `filter_command` looks up the redacting
    /// recipient's own entry, defaulting to `false` (fail-closed, non-GM) for a user absent from
    /// this map (not yet a world member at commit time).
    pub world_gm_at_commit: HashMap<Uuid, bool>,
}

/// A `Command` paired with its commit-time redaction snapshot. Server-internal transport shape:
/// never serialized to the wire. Persisted into `world_events.command_json` and carried through
/// the room broadcast/ring/resync path in place of a bare `Command`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredCommand {
    /// The wire-shaped command.
    pub command: Command,
    /// The commit-time redaction snapshot, index-aligned with `command.ops`.
    pub snapshot: CommandSnapshot,
}

impl StoredCommand {
    /// Deserialize a `world_events.command_json` row, tolerating a legacy bare-`Command` row
    /// (neither a `command` nor a `snapshot` key at the top level — the two shapes are
    /// structurally disjoint, so `Command`'s own fields never satisfy `StoredCommand`'s). Such a
    /// row is wrapped with an all-`None` `CommandSnapshot` and an empty `world_gm_at_commit` map:
    /// `filter_command` then drops every op in it on replay rather than falling back to a
    /// live-lookup redaction — an accepted cost against undated history, never a silent gap.
    pub fn from_stored_json(raw: &str) -> Result<Self, serde_json::Error> {
        if let Ok(stored) = serde_json::from_str::<StoredCommand>(raw) {
            return Ok(stored);
        }
        let command: Command = serde_json::from_str(raw)?;
        let per_op = vec![None; command.ops.len()];
        Ok(StoredCommand {
            command,
            snapshot: CommandSnapshot {
                per_op,
                world_gm_at_commit: HashMap::new(),
            },
        })
    }
}

#[cfg(test)]
mod tests;
