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
