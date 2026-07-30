//! Accounts and access: password hashing, sessions, server roles, invites,
//! and the first-run setup flow.

/// Invite-code mint/verify (selector + Argon2-hashed verifier halves).
pub mod invite;
/// Argon2id password hashing + verification.
pub mod password;
/// The server-tier role (admin/user).
pub mod role;
/// DB-backed sessions + the `AuthUser`/`AdminUser` extractors.
pub mod session;
/// Guarded first-admin creation (concurrent-safe insert; token policy lives
/// in the `/api/setup` route).
pub mod setup;
