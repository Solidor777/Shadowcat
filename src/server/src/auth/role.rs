#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};

/// Server-tier role. Orthogonal to `WorldRole` (per-world) and `DocRole`
/// (per-document): this gates server-level administration only.
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
    pub fn as_str(self) -> &'static str {
        match self {
            ServerRole::Admin => "admin",
            ServerRole::User => "user",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_role_serde_round_trips_snake_case() {
        assert_eq!(
            serde_json::to_value(ServerRole::Admin).unwrap(),
            serde_json::json!("admin")
        );
        let r: ServerRole = serde_json::from_value(serde_json::json!("user")).unwrap();
        assert_eq!(r, ServerRole::User);
        assert_eq!(ServerRole::Admin.as_str(), "admin");
    }
}
