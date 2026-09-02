//! Tests for the merge engine: unit tests per source unit plus the
//! conformance runner that pins this implementation against the corpus
//! generated from the client engine.

mod bands;
mod conformance;
mod embedded;
mod plan;
mod tree;
mod visibility;

use std::collections::BTreeMap;

use serde_json::{json, Value};
use uuid::Uuid;

use crate::data::document::{Document, PermissionSet, Scope};

/// A deterministic id for test documents (stable across runs, unlike a
/// freshly minted uuid, so failures diff readably).
pub(crate) fn test_id(s: &str) -> Uuid {
    Uuid::from_u128(s.bytes().fold(0u128, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(u128::from(b))
    }))
}

/// A minimal world-scoped document envelope: deny-all permissions, no
/// provenance, empty bands. Tests mutate the fields they exercise.
pub(crate) fn doc(id: &str) -> Document {
    Document {
        id: test_id(id),
        scope: Scope::World {
            world_id: test_id("w1"),
        },
        doc_type: "actor".to_string(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: None,
        permissions: PermissionSet::default(),
        embedded: BTreeMap::new(),
        parent_id: None,
        engine: None,
        system: json!({}),
        created_at: 0,
        updated_at: 0,
    }
}

/// A provenance record pointing at template document `id` (version 1, no
/// compendium pack).
pub(crate) fn source_from(id: &str) -> crate::data::document::Source {
    crate::data::document::Source {
        id: test_id(id),
        pack: None,
        version: 1,
    }
}

/// Whether `v` is a full document envelope (as opposed to a payload object
/// or a base record) — the shared shape test the id-mapping and
/// normalization walks use.
pub(crate) fn is_envelope(v: &Value) -> bool {
    v.as_object().is_some_and(|o| {
        o.get("id").and_then(Value::as_str).is_some()
            && o.get("doc_type").and_then(Value::as_str).is_some()
    })
}
