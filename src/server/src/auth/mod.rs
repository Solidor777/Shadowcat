//! Accounts and access: password hashing, sessions, server roles, invites,
//! and the first-run setup flow.

/// Invite-code mint/verify (selector + Argon2-hashed verifier halves).
pub mod invite;
pub mod password;
/// The server-tier role (admin/user).
pub mod role;
/// DB-backed sessions + the `AuthUser`/`AdminUser` extractors.
pub mod session;
pub mod setup;
