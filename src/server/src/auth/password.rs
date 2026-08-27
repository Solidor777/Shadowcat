#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;

/// Hash a plaintext password with Argon2id (default params), returning a PHC
/// string that embeds the random salt. Source: Argon2 RFC 9106 via the `argon2` crate.
pub fn hash_password(plain: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default().hash_password(plain.as_bytes(), &salt)?;
    Ok(hash.to_string())
}

/// Verify a plaintext password against a stored PHC string. Returns false on
/// any parse or mismatch error — callers must not distinguish the two.
pub fn verify_password(plain: &str, phc: &str) -> bool {
    match PasswordHash::new(phc) {
        Ok(parsed) => Argon2::default()
            .verify_password(plain.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// Async wrapper: runs the CPU-bound Argon2 hash on a blocking thread so the
/// async worker is not stalled for the ~tens of ms each hash costs. Owned
/// `String` because `spawn_blocking` requires a `'static` closure.
pub async fn hash_password_async(plain: String) -> Result<String, argon2::password_hash::Error> {
    tokio::task::spawn_blocking(move || hash_password(&plain))
        .await
        .map_err(|_| argon2::password_hash::Error::Crypto)?
}

// Counts calls to `verify_password_async`. Test-only: the anti-enumeration
// paths (`login`, `accept_invite`) owe a CONSTANT number of verifies across
// every outcome, and that is a property about work performed, not about the
// response. A response-shape assertion cannot see it, and wall-clock timing is
// not portable across the CI matrix — so the tests count instead.
//
// Thread-local, not a global atomic: `#[tokio::test]` gives each test its own
// current-thread runtime on its own thread, and the count is taken before the
// `spawn_blocking` hand-off, so a test observes only its own verifies. A
// process-global counter would be raced by every other test that logs in.
#[cfg(test)]
thread_local! {
    pub(crate) static VERIFY_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Verifies performed on this thread so far. Tests assert on the DELTA across a
/// request, never the absolute value.
#[cfg(test)]
pub(crate) fn verify_count() -> usize {
    VERIFY_COUNT.with(std::cell::Cell::get)
}

/// Async wrapper for the CPU-bound verify. A `spawn_blocking` join failure
/// (panic) is treated as a verification failure — the safe default on the auth
/// path. Owned `String`s for the `'static` closure.
pub async fn verify_password_async(plain: String, phc: String) -> bool {
    #[cfg(test)]
    VERIFY_COUNT.with(|c| c.set(c.get() + 1));
    tokio::task::spawn_blocking(move || verify_password(&plain, &phc))
        .await
        .unwrap_or(false)
}

#[cfg(test)]
mod tests;
