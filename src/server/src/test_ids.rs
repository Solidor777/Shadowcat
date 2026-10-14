//! Deterministic test-fixture UUIDs.
//!
//! Ingress accepts only a v4 or v7 UUID as a document id (`data::validation::validate_document_id`),
//! so a fixture id built from a small integer must carry the version and variant bits of one or a
//! Create-path test using it fails at ingress.

/// Builds a UUID from `n`'s big-endian bytes with the version/variant nibbles forced to v4 /
/// RFC4122, so two distinct small `n` values remain distinct while every fixture id passes
/// `data::validation::validate_document_id`. Not a real random id: only for test fixtures.
pub(crate) const fn test_uuid(n: u128) -> uuid::Uuid {
    let mut bytes = n.to_be_bytes();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes)
}
