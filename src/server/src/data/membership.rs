//! Per-world membership: roles, the per-actor PermissionContext, and the
//! queries that resolve a user's role within a world.

use uuid::Uuid;

use crate::data::document::WorldRole;

/// A resolved per-actor authority within one world. Built once per WS
/// connection and per HTTP request; gates writes/reads and filters broadcasts.
#[derive(Debug, Clone, Copy)]
pub struct PermissionContext {
    pub user_id: Uuid,
    pub world_role: WorldRole,
}
