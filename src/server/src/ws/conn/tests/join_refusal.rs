//! These tests check the mapping from a join-path `DataError` to the refusal frame a client sees.

use super::*;
use crate::data::DataError;

/// A real non-member is the only error that reads as a membership refusal.
#[test]
fn forbidden_maps_to_a_forbidden_refusal() {
    let (code, message) = join_refusal(&DataError::Forbidden);
    assert_eq!(code, WsErrorCode::Forbidden);
    assert_eq!(message, "not a member of this world");
}

/// A database failure while resolving membership is transient: the client must not read it as an
/// ejection.
#[test]
fn a_database_error_maps_to_an_internal_refusal() {
    let (code, message) = join_refusal(&DataError::Sqlx(sqlx::Error::PoolTimedOut));
    assert_eq!(code, WsErrorCode::Internal);
    assert_eq!(message, "internal");
}

/// A serialization failure on the join path is a server fault, not a membership answer.
#[test]
fn a_serialization_error_maps_to_an_internal_refusal() {
    let e = DataError::from(serde_json::from_str::<u8>("x").unwrap_err());
    let (code, message) = join_refusal(&e);
    assert_eq!(code, WsErrorCode::Internal);
    assert_eq!(message, "internal");
}

/// `WorldNotFound` has one source, `get_or_create`'s `Ok(None)` arm; no `DataError` maps to it.
#[test]
fn no_data_error_maps_to_world_not_found() {
    let samples = [
        DataError::Forbidden,
        DataError::NotFound,
        DataError::Sqlx(sqlx::Error::PoolTimedOut),
        DataError::OpFailed("x".into()),
    ];
    for e in &samples {
        assert_ne!(join_refusal(e).0, WsErrorCode::WorldNotFound);
    }
}
