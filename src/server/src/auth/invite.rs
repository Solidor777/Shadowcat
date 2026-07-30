//! World-invite bearer codes.
//!
//! A code is `<invite-id>.<secret>`: a public **selector** (the invite row's
//! UUID, used for the lookup and for the GM's revoke route) and a secret
//! **verifier** the server stores only as an Argon2id PHC hash, mirroring
//! `users.password_hash`. The database therefore never holds a value that can
//! be replayed as a credential, and the row lookup keys on a non-secret
//! column — no secret is ever compared by SQLite's index machinery.
//!
//! Entropy: the verifier is [`SECRET_BYTES`] = 16 CSPRNG bytes = 128 bits, so a
//! blind guess succeeds with probability 2^-128 per attempt. Even at an absurd
//! 10^9 online attempts per second against a server that did no rate limiting
//! at all, the expected time to a first hit is 2^127 / 10^9 s ≈ 5.4 x 10^21
//! years. Each attempt additionally costs a full Argon2id verify, so the real
//! attainable rate is lower by ~7 orders of magnitude.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use argon2::password_hash::rand_core::{OsRng, RngCore};
use std::sync::OnceLock;
use uuid::Uuid;

use crate::auth::password::hash_password;

/// CSPRNG bytes in the verifier half of a code. 16 bytes = 128 bits.
pub const SECRET_BYTES: usize = 16;
/// Hex-encoded verifier length. Parsing enforces it exactly, which also bounds
/// the plaintext handed to Argon2 (an unbounded input is a CPU amplifier).
const SECRET_HEX_LEN: usize = SECRET_BYTES * 2;

/// A freshly minted code: the plaintext handed to the GM once, and the PHC
/// hash that is all the server retains.
pub struct MintedCode {
    /// Selector half (also the invite row id).
    pub id: Uuid,
    /// The full bearer code (selector.verifier) — shown once, never stored.
    pub code: String,
    /// Argon2 PHC hash of the verifier half — all the server retains.
    pub secret_hash: String,
}

/// Generate an invite code. Fails only if Argon2 hashing fails.
pub fn mint() -> Result<MintedCode, argon2::password_hash::Error> {
    let id = Uuid::new_v4();
    let mut bytes = [0u8; SECRET_BYTES];
    OsRng.fill_bytes(&mut bytes);
    let secret = hex(&bytes);
    let secret_hash = hash_password(&secret)?;
    Ok(MintedCode {
        id,
        code: format!("{}.{}", id.simple(), secret),
        secret_hash,
    })
}

/// Split a code into its selector and verifier. `None` for any malformed
/// input; callers must treat that identically to an unknown or unusable code.
///
/// Both halves are matched case-INSENSITIVELY: `Uuid::parse_str` accepts either
/// hex case, and the verifier is folded to the lowercase form `mint` emits
/// before it reaches the hash. Hex case carries no information, so folding it
/// widens the accepted spelling of one code without widening the value space —
/// the 2^-128 guess bound is unchanged. A case-SENSITIVE verifier would instead
/// turn a re-typed or auto-capitalized code into an indistinguishable
/// "unusable code" that the holder cannot diagnose.
pub fn parse(code: &str) -> Option<(Uuid, String)> {
    let (id, secret) = code.split_once('.')?;
    if secret.len() != SECRET_HEX_LEN || !secret.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some((Uuid::parse_str(id).ok()?, secret.to_ascii_lowercase()))
}

/// A real Argon2id hash of a fixed throwaway secret, computed once. Redemption
/// of a code with no matching row verifies [`DUMMY_SECRET`] against it, so the
/// unknown-code path costs the same as the wrong-secret path and neither the
/// existence of an invite nor its state is observable through timing.
pub fn dummy_phc() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY
        .get_or_init(|| hash_password(DUMMY_SECRET).expect("hash dummy"))
        .as_str()
}

/// The plaintext verified against [`dummy_phc`] on the no-such-invite path.
pub const DUMMY_SECRET: &str = "invite-anti-enumeration-unused";

/// Lowercase hex encoding (verifier-half rendering).
///
/// # Examples
///
/// ```text
/// hex(&[0xab, 0x01]) == "ab01"
/// ```
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        // Writing to a String is infallible.
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::password::verify_password;
    use std::collections::HashSet;

    #[test]
    fn minted_code_round_trips_and_verifies_against_its_hash() {
        let m = mint().expect("mint");
        let (id, secret) = parse(&m.code).expect("parse");
        assert_eq!(id, m.id);
        assert!(verify_password(&secret, &m.secret_hash));
        assert!(!verify_password(
            "00000000000000000000000000000000",
            &m.secret_hash
        ));
    }

    #[test]
    fn the_plaintext_secret_is_never_recoverable_from_the_stored_hash() {
        let m = mint().expect("mint");
        let (_, secret) = parse(&m.code).expect("parse");
        assert!(m.secret_hash.starts_with("$argon2"));
        assert!(!m.secret_hash.contains(&secret));
    }

    #[test]
    fn minted_secrets_are_distinct_and_full_width() {
        let mut seen = HashSet::new();
        for _ in 0..32 {
            let m = mint().expect("mint");
            let (_, secret) = parse(&m.code).expect("parse");
            assert_eq!(secret.len(), SECRET_HEX_LEN);
            assert!(seen.insert(secret), "CSPRNG repeated a 128-bit secret");
        }
    }

    #[test]
    fn a_code_is_matched_case_insensitively_in_both_halves() {
        let m = mint().expect("mint");
        let (id, secret) = parse(&m.code.to_uppercase()).expect("parse uppercased");
        assert_eq!(id, m.id);
        // Folded back to the minted spelling, so it still verifies.
        assert!(verify_password(&secret, &m.secret_hash));
    }

    #[test]
    fn malformed_codes_do_not_parse() {
        for bad in [
            "",
            "no-separator",
            "not-a-uuid.0123456789abcdef0123456789abcdef",
            // Right shape, short/long/non-hex verifier.
            &format!("{}.{}", Uuid::new_v4().simple(), "abc"),
            &format!("{}.{}", Uuid::new_v4().simple(), "z".repeat(SECRET_HEX_LEN)),
            &format!(
                "{}.{}",
                Uuid::new_v4().simple(),
                "a".repeat(SECRET_HEX_LEN + 1)
            ),
            &format!("{}", Uuid::new_v4().simple()),
        ] {
            assert!(parse(bad).is_none(), "parsed a malformed code: {bad:?}");
        }
    }

    #[test]
    fn the_dummy_hash_is_stable_and_matches_only_the_dummy_secret() {
        assert_eq!(dummy_phc(), dummy_phc());
        assert!(verify_password(DUMMY_SECRET, dummy_phc()));
        let m = mint().expect("mint");
        let (_, secret) = parse(&m.code).expect("parse");
        assert!(!verify_password(&secret, dummy_phc()));
    }
}
