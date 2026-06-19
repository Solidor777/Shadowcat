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

/// Async wrapper for the CPU-bound verify. A `spawn_blocking` join failure
/// (panic) is treated as a verification failure — the safe default on the auth
/// path. Owned `String`s for the `'static` closure.
pub async fn verify_password_async(plain: String, phc: String) -> bool {
    tokio::task::spawn_blocking(move || verify_password(&plain, &phc))
        .await
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_true_on_match_false_on_mismatch() {
        let hash = hash_password("correct horse").expect("hash");
        assert!(verify_password("correct horse", &hash));
        assert!(!verify_password("wrong horse", &hash));
        assert!(!verify_password("correct horse", "not-a-phc-string"));
    }

    #[test]
    fn distinct_salts_produce_distinct_hashes() {
        let a = hash_password("same").expect("hash a");
        let b = hash_password("same").expect("hash b");
        assert_ne!(a, b, "random salt must make hashes differ");
        assert!(verify_password("same", &a));
        assert!(verify_password("same", &b));
    }

    #[tokio::test]
    async fn async_hash_then_async_verify_roundtrips() {
        let hash = hash_password_async("correct horse".to_owned())
            .await
            .expect("hash");
        assert!(verify_password_async("correct horse".to_owned(), hash.clone()).await);
        assert!(!verify_password_async("wrong horse".to_owned(), hash).await);
    }

    #[tokio::test]
    async fn async_verify_false_on_unparseable_phc() {
        assert!(!verify_password_async("x".to_owned(), "not-a-phc-string".to_owned()).await);
    }
}
