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
///
/// # Examples
///
/// ```
/// use shadowcat::merge::Side;
///
/// assert_ne!(Side::Template, Side::Child);
/// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::data::document::{Document, Scope};
/// use shadowcat::merge::{MergeError, MergeVisibility, Side};
/// use uuid::Uuid;
///
/// struct HideSystem;
/// impl MergeVisibility for HideSystem {
///     fn hidden(&self, _side: Side, _doc: &Document) -> Result<Vec<String>, MergeError> {
///         Ok(vec!["/system".to_string()])
///     }
/// }
///
/// let doc = Document {
///     id: Uuid::new_v4(),
///     scope: Scope::World { world_id: Uuid::new_v4() },
///     doc_type: "actor".into(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: Default::default(),
///     embedded: Default::default(),
///     parent_id: None,
///     engine: None,
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
/// assert_eq!(HideSystem.hidden(Side::Child, &doc).unwrap(), vec!["/system".to_string()]);
/// ```
pub trait MergeVisibility {
    /// The requester-hidden pointers of `doc`'s own properties on `side`.
    /// `Err` means the question cannot be answered (an override pointer the
    /// classifier cannot place); the merge then fails closed and discloses
    /// nothing.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::document::{Document, Scope};
    /// use shadowcat::merge::{AllVisible, MergeVisibility, Side};
    /// use uuid::Uuid;
    ///
    /// let doc = Document {
    ///     id: Uuid::new_v4(),
    ///     scope: Scope::World { world_id: Uuid::new_v4() },
    ///     doc_type: "actor".into(),
    ///     schema_version: 1,
    ///     name: None,
    ///     source: None,
    ///     base: None,
    ///     owner: None,
    ///     permissions: Default::default(),
    ///     embedded: Default::default(),
    ///     parent_id: None,
    ///     engine: None,
    ///     system: serde_json::json!({}),
    ///     created_at: 0,
    ///     updated_at: 0,
    /// };
    /// assert!(AllVisible.hidden(Side::Child, &doc).unwrap().is_empty());
    /// ```
    fn hidden(&self, side: Side, doc: &Document) -> Result<Vec<String>, MergeError>;
}

/// Every property visible on both sides: the oracle for a requester who sees
/// every tier, and for the conformance corpus, whose cases carry no
/// visibility dimension.
///
/// # Examples
///
/// ```
/// use shadowcat::data::document::{Document, Scope};
/// use shadowcat::merge::{AllVisible, MergeVisibility, Side};
/// use uuid::Uuid;
///
/// let doc = Document {
///     id: Uuid::new_v4(),
///     scope: Scope::World { world_id: Uuid::new_v4() },
///     doc_type: "actor".into(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: Default::default(),
///     embedded: Default::default(),
///     parent_id: None,
///     engine: None,
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
/// let oracle = AllVisible;
/// assert!(oracle.hidden(Side::Template, &doc).unwrap().is_empty());
/// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::data::document::{Document, Scope};
/// use shadowcat::data::permission::Access;
/// use shadowcat::merge::{MergeVisibility, RequesterView, Side};
/// use std::collections::BTreeSet;
/// use uuid::Uuid;
///
/// let doc = Document {
///     id: Uuid::new_v4(),
///     scope: Scope::World { world_id: Uuid::new_v4() },
///     doc_type: "actor".into(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: Default::default(),
///     embedded: Default::default(),
///     parent_id: None,
///     engine: None,
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
/// let player = Access {
///     caps: BTreeSet::new(),
///     all: false,
///     see_gm_only: false,
///     is_owner: false,
/// };
/// let vis = RequesterView { template: &player, child: &player };
/// // `/base` is hardcoded `OwnerOrGm`; an ordinary player sees neither tier.
/// assert_eq!(vis.hidden(Side::Template, &doc).unwrap(), vec!["/base".to_string()]);
/// ```
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
