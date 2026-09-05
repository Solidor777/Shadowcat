//! The requester's view of what each document hides, consulted by the merge
//! at EVERY recursion level by document IDENTITY — the correlated
//! `(template child, instance child)` pair the merge is currently inside —
//! never by array index. Two index spaces exist around an embedded merge:
//! the LIVE document's child positions (which `permission::collect_overrides`
//! addresses) and the merged OUTPUT's child positions (which
//! `embedded::prefix_conflicts` addresses); a dropped preceding sibling or a
//! template-side reorder makes them disagree, so a hidden-pointer set
//! expressed in one space can never be compared against a conflict path
//! expressed in the other. Asking the oracle per document, with pointers
//! relative to that document's own root, removes the index from the
//! question entirely.

use crate::data::document::Document;
use crate::data::permission::{hidden_own_pointers, Access};
use crate::merge::MergeError;

/// Which side of the merge a document sits on. The requester's access can
/// differ per side (the template's owner is not the instance's owner), so the
/// oracle resolves each side against its own `Access`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The template (parent) side: a hidden pointer here is EXCLUDED from the
    /// parent diff, so hidden template data never moves into the instance.
    Template,
    /// The instance (child) side: a hidden pointer here withholds any conflict
    /// overlapping it from the reply, leaving the child-wins default in place.
    Child,
}

/// Per-document visibility oracle. `hidden` answers, for ONE document at ONE
/// level, the pointers (relative to that document's own root — `/name`,
/// `/engine/…`, `/system/…`) of properties the requester may not see; the
/// merge asks it for the root documents and again for every correlated
/// embedded pair, so the answer is always in the coordinate space of the
/// document being merged.
pub trait MergeVisibility {
    /// The requester-hidden pointers of `doc`'s own properties on `side`.
    /// `Err` means the question cannot be answered (an override pointer the
    /// classifier cannot place); the merge then fails closed and discloses
    /// nothing.
    fn hidden(&self, side: Side, doc: &Document) -> Result<Vec<String>, MergeError>;
}

/// Every property visible on both sides: the oracle for a requester who sees
/// every tier, and for the conformance corpus, whose cases carry no
/// visibility dimension.
#[derive(Debug, Clone, Copy, Default)]
pub struct AllVisible;

impl MergeVisibility for AllVisible {
    fn hidden(&self, _side: Side, _doc: &Document) -> Result<Vec<String>, MergeError> {
        Ok(Vec::new())
    }
}

/// The `Access`-backed oracle: `permission::hidden_own_pointers` — the SAME
/// per-level primitive `filter_properties` strips by — under the requester's
/// resolved access on each side's ROOT document. An embedded child is tested
/// on the tier alone against the root's access, exactly as `filter_properties`
/// recurses with the recipient's access and never resolves whole-document
/// READ for a child.
#[derive(Debug, Clone, Copy)]
pub struct RequesterView<'a> {
    /// The requester's access on the template root.
    pub template: &'a Access,
    /// The requester's access on the instance root.
    pub child: &'a Access,
}

impl MergeVisibility for RequesterView<'_> {
    fn hidden(&self, side: Side, doc: &Document) -> Result<Vec<String>, MergeError> {
        let access = match side {
            Side::Template => self.template,
            Side::Child => self.child,
        };
        hidden_own_pointers(doc, access).map_err(|_| MergeError::VisibilityUnknown)
    }
}
