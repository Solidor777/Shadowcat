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
