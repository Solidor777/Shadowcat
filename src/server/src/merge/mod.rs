//! The server-owned template merge engine: the 3-way merge (`merge3`,
//! `compute_pull`, `compute_revert`, `plan_to_update`, `apply_resolutions`)
//! behind the `MergePull`/`MergePush`/`MergeRevert` intents. Its behaviour is
//! pinned by one conformance corpus
//! (`src/client/core/src/__fixtures__/merge-conformance.json`, whose
//! generation transcript is `scripts/merge-corpus-generation.log`); the test
//! suite asserts value-identical results for every case, so a behavioural
//! change fails the corpus rather than drifting silently. Every cross-tree
//! value is cloned at its crossing point (owned values, never aliases), which
//! the corpus's aliasing-sensitive cases pin behaviourally.
//!
//! # Properties the corpus does not pin
//!
//! Three properties are outside what the corpus can observe; none can
//! change a merge RESULT, only an order or an unreachable input's handling:
//!
//! - **Key sorting is UTF-8 byte order** (Rust `String` `Ord`, via the
//!   `BTreeSet` traversals in `tree::structural_diff_at`,
//!   `embedded::merge3_embedded`/`embedded::revert_embedded` and
//!   `plan::plan_to_update`). Merge results are order-independent by design;
//!   only diff/conflict/change ORDER within a result depends on it, and a
//!   UTF-16 ordering would differ only on keys mixing U+E000–U+FFFF with
//!   astral-plane characters.
//! - **`tree::get_pointer`/`tree::delete_pointer` reject non-canonical
//!   array-index tokens** (`"01"`, `"+1"`, …). Merge-generated pointers only
//!   ever carry canonical indices (`structural_diff` and `prefix_conflicts`
//!   format `usize` values), so a coercing reading is never needed.
//! - **Base correlation stringifies `Uuid` canonically**
//!   (`embedded::merge3_embedded`'s `base_by_source` keys). A non-canonical
//!   legacy `sourceId` (uppercase hex, braces, …) correlates as
//!   instance-added — the keep direction, never the drop direction.
//!
//! One deliberate delta from the corpus generator's engine is recorded in
//! the fixture's own `amendments` key: the `/base` refresh is emitted only
//! when it differs from the stored value (`plan_to_update`).

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

/// `MergeBase`/`MergeBands`, the snapshot builders and the placement
/// exclusion set.
pub mod bands;
/// Embedded-collection merge/revert and subtree restamping.
pub(crate) mod embedded;
/// `merge3`, the pull/revert computations, and update-plan emission.
pub mod plan;
/// Single-tree diff/merge primitives and JSON-pointer helpers.
pub(crate) mod tree;
/// The per-document visibility oracle the merge consults by identity.
pub mod visibility;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

pub use bands::{
    is_placement_excluded, placement_exclusions, snapshot_base, EmbeddedBaseChild, MergeBands,
    MergeBase, StoredBase,
};
pub use plan::{
    apply_resolutions, compute_pull, compute_revert, merge3, plan_to_update, MergePlan,
};
pub use tree::PointerError;
pub use visibility::{AllVisible, MergeVisibility, RequesterView, Side};

/// A merge computation that refused to run. Wire-level errors (missing
/// documents, authorization, stale resolutions) live in the protocol layer,
/// not here.
///
/// # Examples
///
/// ```
/// use shadowcat::merge::MergeError;
///
/// let err = MergeError::CorruptBase;
/// assert_eq!(
///     err.to_string(),
///     "the stored merge base does not parse as a StoredBase snapshot"
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeError {
    /// The child's stored `base` snapshot is present but does not parse as a
    /// `StoredBase`. A corrupt snapshot cannot carry correlation information
    /// the merge could trust, and falling back to a clean template-wins
    /// merge would silently destroy child-local edits — so the pull fails
    /// closed and nothing is written. Carries no user data: the offending
    /// document is identified by the caller's context.
    CorruptBase,
    /// The `MergeVisibility` oracle could not answer what the requester may
    /// see of a document (an override pointer the redaction classifier cannot
    /// place). The merge discloses nothing and writes nothing.
    VisibilityUnknown,
    /// The merge's own apply of a parent-only diff could not write its
    /// pointer. Unreachable by construction (`set_pointer`'s doc states why)
    /// but propagated rather than asserted: the release profile aborts on
    /// panic, so an invariant breach here must surface as a refused intent,
    /// never as a dead server.
    Pointer(PointerError),
}

impl std::fmt::Display for MergeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MergeError::CorruptBase => {
                f.write_str("the stored merge base does not parse as a StoredBase snapshot")
            }
            MergeError::VisibilityUnknown => {
                f.write_str("the requester's view of a merged document could not be resolved")
            }
            MergeError::Pointer(e) => write!(f, "the merge could not write a pointer: {e}"),
        }
    }
}

impl std::error::Error for MergeError {}

/// How `take_template` resolves a conflict: `"set"` writes the parent value,
/// `"delete"` removes the key.
///
/// # Examples
///
/// ```
/// use shadowcat::merge::ParentKind;
///
/// assert_ne!(ParentKind::Set, ParentKind::Delete);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum ParentKind {
    /// Write the parent/template value at the conflict path.
    Set,
    /// Remove the key at the conflict path.
    Delete,
}

/// A field changed on both the template (parent) and instance (child) sides
/// since the last sync. `base`/`parent`/`child` are ABSENT (not `null`) when
/// that side has no value at `path` — the parent's side deleted it, the
/// child's side deleted it, or neither side's snapshot contained it — so the
/// client's Zod mirror (`WireMergeConflict`) reads a missing side as an
/// absent key, distinct from an explicit `null` value.
///
/// # Examples
///
/// ```
/// use shadowcat::merge::{MergeConflict, ParentKind};
///
/// let conflict = MergeConflict {
///     path: "/system/hp".to_string(),
///     base: Some(serde_json::json!(5)),
///     parent: Some(serde_json::json!(20)),
///     child: Some(serde_json::json!(10)),
///     parent_kind: ParentKind::Set,
/// };
/// assert_eq!(conflict.path, "/system/hp");
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "camelCase")]
pub struct MergeConflict {
    /// The RFC-6901 pointer of the conflicting field.
    pub path: String,
    /// The value at `path` in the last-synced snapshot both sides diverged
    /// from; absent when the snapshot has no value there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "unknown")]
    pub base: Option<Value>,
    /// The template/parent side's current value at `path`; absent iff the
    /// parent deleted it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "unknown")]
    pub parent: Option<Value>,
    /// The instance/child side's current value at `path`; absent iff the
    /// child deleted it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "unknown")]
    pub child: Option<Value>,
    /// How `take_template` resolves this conflict: `Set` writes `parent`,
    /// `Delete` removes the key.
    pub parent_kind: ParentKind,
}
