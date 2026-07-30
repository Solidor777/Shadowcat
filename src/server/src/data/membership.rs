//! Per-world membership: roles, the per-user PermissionContext, and the
//! queries that resolve a user's role within a world.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use uuid::Uuid;

use crate::data::document::WorldRole;

/// A resolved per-user authority within one world. Built once per WS
/// connection and per HTTP request; gates writes/reads and filters broadcasts.
#[derive(Debug, Clone, Copy)]
pub struct PermissionContext {
    /// The authenticated user this context authorizes.
    pub user_id: Uuid,
    /// Their role in the world being acted on.
    pub world_role: WorldRole,
}
