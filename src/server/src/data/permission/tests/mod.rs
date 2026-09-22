use super::*;
use crate::data::document::{PermissionSet, Scope};
use crate::data::snapshot::{CommandSnapshot, OpSnapshot};

fn doc(perms: PermissionSet, system: serde_json::Value) -> Document {
    Document {
        id: crate::test_ids::test_uuid(1),
        scope: Scope::World {
            world_id: crate::test_ids::test_uuid(9),
        },
        doc_type: "actor".into(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: None,
        permissions: perms,
        embedded: Default::default(),
        parent_id: None,
        // "actor" is engine-defined; a minimal valid body so `Create`
        // clears the ingress gate. These tests exercise `/system`
        // redaction only, unrelated to `engine`'s content.
        engine: crate::data::document::tests::default_test_engine("actor"),
        system,
        created_at: 0,
        updated_at: 0,
    }
}

fn perms_with(overrides: &[(&str, Visibility)]) -> PermissionSet {
    let mut p = PermissionSet::default();
    for (ptr, v) in overrides {
        p.property_overrides.insert((*ptr).into(), *v);
    }
    p
}

/// Build a `PermissionSet` carrying one override at `pointer`, hidden from non-GMs.
fn perms_with_override(pointer: &str) -> PermissionSet {
    let mut p = PermissionSet {
        default: crate::data::document::DocRole::Observer,
        ..Default::default()
    };
    p.property_overrides.insert(
        pointer.to_string(),
        crate::data::document::Visibility::GmOnly,
    );
    p
}

fn non_gm() -> Access {
    Access {
        caps: Default::default(),
        all: false,
        see_gm_only: false,
        is_owner: false,
    }
}

/// A `CommandSnapshot` for `cmd` whose commit-time state exactly mirrors `current` (and, for
/// `Create`/`Delete`, the op's own carried `doc`) — for a pre-existing test that writes and
/// redacts at the SAME instant, so the commit-time and current-time halves agree by
/// construction and their union changes nothing observable. `gm_at_commit` lists every
/// recipient who holds GM standing (both at commit and now, in these single-instant tests).
fn immediate_snapshot<'a>(
    cmd: &Command,
    current: &HashMap<Uuid, CurrentDoc>,
    gm_at_commit: &[Uuid],
    actor_lookup: &impl Fn(&Uuid) -> Option<&'a Document>,
) -> CommandSnapshot {
    let mut per_op = Vec::with_capacity(cmd.ops.len());
    for op in &cmd.ops {
        let target_doc: Option<&Document> = match op {
            Operation::Update { doc_id, .. } => current.get(doc_id).map(|c| &c.doc),
            Operation::Create { doc } => Some(doc),
            Operation::Delete { doc } => Some(doc),
            Operation::Move { doc_id, .. } => current.get(doc_id).map(|c| &c.doc),
        };
        let Some(d) = target_doc else {
            per_op.push(None);
            continue;
        };
        // A test poisoning a document with an unclassifiable override (to exercise
        // `filter_properties`'s own fail-closed path) makes this best-effort: the partial
        // set collected before the error is used as-is, matching what a real snapshot
        // builder would have to do (this classifier never runs against genuinely
        // unclassifiable data outside a deliberately-poisoned test fixture).
        let mut overrides = Vec::new();
        let _ = collect_overrides(d, "", &mut overrides);
        let touches_perms = matches!(
            op,
            Operation::Update { changes, .. }
                if changes.iter().any(|c| touches_permissions(&c.path))
        );
        per_op.push(Some(OpSnapshot {
            owner_at_commit: effective_owner_via(d, actor_lookup),
            doc_type: d.doc_type.clone(),
            overrides_at_commit: overrides.clone(),
            retraction_hidden_at_commit: if touches_perms { Some(overrides) } else { None },
            created_seq_at_commit: None,
            permissions_at_commit: Some(PermissionSet {
                property_overrides: Default::default(),
                ..d.permissions.clone()
            }),
            // No before-image in this single-instant helper; `snapshot_with_before`
            // overrides this for tests exercising the READ-transition rule.
            permissions_before_commit: None,
            owner_before_commit: None,
        }));
    }
    CommandSnapshot {
        per_op,
        world_gm_at_commit: gm_at_commit.iter().map(|u| (*u, true)).collect(),
    }
}

/// A `CurrentDoc` wrapping `doc` at generation `created_seq`.
fn current_doc(doc: Document, created_seq: i64) -> CurrentDoc {
    CurrentDoc { doc, created_seq }
}

#[cfg(test)]
mod core_redaction;
#[cfg(test)]
mod ownership;
#[cfg(test)]
mod read_transition;
#[cfg(test)]
mod snapshot_redaction;
