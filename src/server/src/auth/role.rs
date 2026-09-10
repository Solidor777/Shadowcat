#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};

/// Server-tier role. Orthogonal to `WorldRole` (per-world) and `DocRole`
/// (per-document): this gates server-level administration only.
///
/// # Examples
///
/// ```
/// use shadowcat::auth::role::ServerRole;
///
/// let role = ServerRole::Admin;
/// assert_eq!(role.as_str(), "admin");
/// assert_ne!(role, ServerRole::User);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerRole {
    /// Manages accounts and worlds server-wide; no world role confers this.
    Admin,
    /// Ordinary account; world authority comes only from per-world roles.
    User,
}

impl ServerRole {
    /// Stable storage token persisted in `users.server_role`.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::auth::role::ServerRole;
    ///
    /// assert_eq!(ServerRole::User.as_str(), "user");
    /// ```
    pub fn as_str(self) -> &'static str {
        match self {
            ServerRole::Admin => "admin",
            ServerRole::User => "user",
        }
    }
}

#[cfg(test)]
mod tests;
