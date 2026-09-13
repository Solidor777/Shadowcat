//! Sandboxed third-party server-side validators: opt-in, per-world, per-module WASM code run
//! inside `wasmi` over the `system` band only, after the engine's own validation. A validator
//! can refuse a write (a structured reason) and nothing else — see `runtime` for the host,
//! `registry` for the compiled-module cache. Every validator call is fuel/memory/instance-capped
//! and has no host imports beyond a rate-limited `env.log`; `VALIDATOR_FAULT_LIMIT` bounds how
//! many consecutive technical failures a module gets before a caller auto-disables it.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::data::document::{Document, SchemaDeclaration};
use crate::data::validation;
use crate::data::DataError;

pub mod registry;
pub mod runtime;

/// Technical failure of the sandbox itself — never a validator's own authored refusal
/// (`ValidatorVerdict::Refuse`). Every variant maps to a `ValidatorFault`, whose `consecutive`
/// count `Room::commit_ops_locked`'s error arm — the one funnel every guarded write path
/// shares — compares against `VALIDATOR_FAULT_LIMIT` before calling
/// `disable_faulting_validator_locked`.
///
/// # Examples
///
/// ```
/// use shadowcat::sandbox::FaultKind;
///
/// // An infinite loop exhausts the per-call fuel budget.
/// assert_eq!(FaultKind::OutOfFuel, FaultKind::OutOfFuel);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    /// `consume_fuel` exhausted the per-call budget before `validate` returned.
    OutOfFuel,
    /// The guest exceeded its `StoreLimits` memory ceiling (e.g. an `alloc`/`memory.grow` bomb).
    MemoryLimit,
    /// The module is missing a required export, or an export has the wrong signature.
    BadAbi,
    /// `alloc` returned a pointer/length pair outside the guest's own linear memory.
    BadPointer,
    /// The serialized `ValidatorInput` exceeded 1 MiB; the module was never instantiated.
    InputTooLarge,
    /// The call COMPLETED (an authored accept/refuse) but its `Instant`-measured duration
    /// exceeded the 50 ms per-call budget — a slow-but-under-fuel validator, reclassified as
    /// a fault. A call that instead traps keeps its precise kind (an infinite loop stays
    /// `OutOfFuel`); both count toward auto-disable identically.
    TooSlow,
    /// The call never returned at all within the 250 ms `tokio::time::timeout` hang guard
    /// around the `spawn_blocking` join (fuel bounds wasm instructions, not a stalled host
    /// import or allocator loop) — the blocking thread is abandoned, and this is logged at
    /// `warn`.
    Hung,
    /// Any other wasmi trap (unreachable, integer overflow, out-of-bounds table access, ...).
    Trap,
}

/// One sandboxed-validator technical failure: which module, what kind, and that module's
/// current consecutive-fault streak for the world this call belongs to (including this
/// fault). The SAME value both `ValidatorVerdict::Fault` and `DataError::Validator` carry —
/// one type for "what technically went wrong," never duplicated across the two.
///
/// # Examples
///
/// ```
/// use shadowcat::sandbox::{FaultKind, ValidatorFault};
///
/// let fault = ValidatorFault {
///     module: "example-module".into(),
///     kind: FaultKind::OutOfFuel,
///     consecutive: 1,
/// };
/// assert_eq!(fault.module, "example-module");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorFault {
    /// The faulting module's id.
    pub module: String,
    /// What went wrong.
    pub kind: FaultKind,
    /// This module's consecutive-fault streak for the world this call validated, INCLUDING
    /// this fault. Always `0` when the value has not yet been stamped by
    /// `validate_document` — `runtime::run_validator` itself has no world/registry context
    /// to compute the real streak.
    pub consecutive: u32,
}

/// Consecutive faults before a module's `validators_enabled` flag is auto-disabled for a
/// world (the sandbox must never become a denial-of-service lever against the table).
///
/// # Examples
///
/// ```
/// use shadowcat::sandbox::VALIDATOR_FAULT_LIMIT;
///
/// assert_eq!(VALIDATOR_FAULT_LIMIT, 5);
/// ```
pub const VALIDATOR_FAULT_LIMIT: u32 = 5;

/// What one validator call decided.
///
/// # Examples
///
/// ```
/// use shadowcat::sandbox::ValidatorVerdict;
///
/// let verdict = ValidatorVerdict::Refuse {
///     module: "example-module".into(),
///     reason: "system.hp must not be negative".into(),
/// };
/// assert!(matches!(verdict, ValidatorVerdict::Refuse { .. }));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidatorVerdict {
    /// The write may proceed as far as this validator is concerned.
    Accept,
    /// The write is refused with a player/GM-presentable reason (≤ 512 bytes, control
    /// characters stripped, lossy UTF-8 — see `runtime::run_validator`).
    Refuse {
        /// The refusing module's id.
        module: String,
        /// The reason text.
        reason: String,
    },
    /// The sandbox itself failed technically; the write is refused and the fault is counted.
    Fault(ValidatorFault),
}

/// The guest-visible input, UTF-8 JSON, passed by pointer/length through the guest's own
/// `alloc` export. `#[serde(rename_all = "camelCase")]` produces the wire keys third-party
/// guest code depends on; this struct's own field names stay idiomatic Rust — a field added
/// here changes the wire contract every installed validator's guest code parses against.
///
/// # Examples
///
/// ```
/// use shadowcat::sandbox::ValidatorInput;
///
/// let input = ValidatorInput {
///     doc_type: "actor".into(),
///     op: "create".into(),
///     system: serde_json::json!({ "hp": 3 }),
///     prior: None,
///     name: None,
///     world_id: uuid::Uuid::nil(),
///     module_id: "example-module".into(),
/// };
/// // The guest ABI's wire keys are camelCase.
/// let wire = serde_json::to_value(&input).unwrap();
/// assert_eq!(wire["docType"], "actor");
/// assert_eq!(wire["worldId"], uuid::Uuid::nil().to_string());
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorInput {
    /// The document's `doc_type`.
    pub doc_type: String,
    /// `"create"` or `"update"` (a `Move` op never touches `system`, so a validator never sees
    /// `"move"` in practice — the ABI reserves the value but nothing currently sends it).
    pub op: String,
    /// Post-image `system` band.
    pub system: serde_json::Value,
    /// Pre-image `system` band; `None` for a Create or a freshly-added embedded child.
    pub prior: Option<serde_json::Value>,
    /// The document's envelope `name`, if any.
    pub name: Option<String>,
    /// The world this write belongs to.
    pub world_id: Uuid,
    /// The validating module's own id (so a multi-`doc_type` validator can branch).
    pub module_id: String,
}

/// One document (or embedded descendant)'s validator input, gathered by `collect_validated_nodes`
/// before any WASM runs — pure and synchronous, mirroring `validation::validate_system_schema_tree`'s
/// recursion exactly (a child judged under its OWN `doc_type`; a child absent from `prior`
/// validates with `system_prior: None`, i.e. as a Create).
struct ValidatedNode<'a> {
    /// The node's `doc_type`.
    doc_type: &'a str,
    /// Post-image `system` band.
    system_post: &'a serde_json::Value,
    /// Pre-image `system` band, if this node existed before this write.
    system_prior: Option<&'a serde_json::Value>,
    /// The node's envelope `name`.
    name: Option<&'a str>,
}

/// Runs Phase 1's own pure structural validators against `doc` in place, in the EXACT order
/// `SqliteRepository::apply_intent`'s Create and Update arms each run before their own Phase 2
/// write: `validate_system_size`, `validate_property_overrides`, `validate_engine_tree`,
/// `validate_system_size` again (the engine/note-derivation re-check both arms perform), then
/// `validate_containment` and `validate_system_schema_tree` — every one of which already
/// recurses `doc.embedded` on its own, so this runs once at the tree root, never per node.
/// Returns Phase 1's own error on the first failure: a document that fails here would fail
/// Phase 1 identically once it reaches the write transaction, so `validate_document` returns
/// before consulting any validator — a malformed submission never reaches, and never faults, a
/// validator. Phase 1 inside the transaction re-runs this exact chain, unchanged, and remains
/// the sole authority over what actually commits.
fn validate_structural(doc: &mut Document, schemas: &[SchemaDeclaration]) -> Result<(), DataError> {
    validation::validate_system_size(doc)?;
    validation::validate_property_overrides(doc)?;
    validation::validate_engine_tree(doc)?;
    validation::validate_system_size(doc)?;
    validation::validate_containment(doc)?;
    validation::validate_system_schema_tree(doc, schemas)?;
    Ok(())
}

/// Recurses `doc`'s embedded children exactly like `validation::validate_system_schema_tree`,
/// pairing each with its PRIOR counterpart (same collection key, matched by id) when `prior`
/// is `Some`.
fn collect_validated_nodes<'a>(
    doc: &'a Document,
    prior: Option<&'a Document>,
    out: &mut Vec<ValidatedNode<'a>>,
) {
    out.push(ValidatedNode {
        doc_type: &doc.doc_type,
        system_post: &doc.system,
        system_prior: prior.map(|p| &p.system),
        name: doc.name.as_deref(),
    });
    for (key, children) in &doc.embedded {
        let prior_children = prior.and_then(|p| p.embedded.get(key));
        for child in children {
            let prior_child = prior_children.and_then(|pc| pc.iter().find(|c| c.id == child.id));
            collect_validated_nodes(child, prior_child, out);
        }
    }
}

/// First runs `validate_structural` (Phase 1's own pure structural chain) against `doc`,
/// returning its error untouched on failure without consulting any validator. Only a
/// structurally valid `doc` reaches the wasm pass below: every matching enabled+opted-in
/// validator, against `doc` and its embedded descendants, in ASCENDING module-id order —
/// `enabled_module_ids` is sorted by this function itself, never trusted from the caller's own
/// collection order, so `apply_intent` and `import_world` inherit identical, deterministic
/// ordering with no way for the two chokepoints to fork — short-circuiting on the FIRST
/// non-`Accept` verdict anywhere in the tree. `prior` is `doc`'s pre-image (`None` for a
/// Create). `prior_permitted` is the caller's statement that the WRITER holds whole-document
/// READ on the pre-image: when `false`, `prior` is withheld entirely (every node validates as
/// a Create) so a write-without-read configuration never hands the validator — or a crafted
/// refusal reason reflected back to the writer — stored content no egress path would give
/// them. Maintains `registry`'s per-(world, module) consecutive-fault counter itself: a
/// module whose call faults has its streak incremented and stamped onto the returned
/// `ValidatorVerdict::Fault`'s `consecutive` field; a module whose call returns `Accept` OR
/// `Refuse` — either is a working, non-technical decision — has its own streak reset to zero; a
/// module never consulted for this call (an earlier node/module short-circuited first, or the
/// structural pre-pass rejected `doc` before any validator ran) is untouched either way.
///
/// # Examples
///
/// ```
/// # #[tokio::main]
/// # async fn main() -> Result<(), shadowcat::data::DataError> {
/// use shadowcat::data::document::{Document, PermissionSet, Scope};
/// use shadowcat::sandbox::registry::ValidatorRegistry;
/// use shadowcat::sandbox::{validate_document, ValidatorVerdict};
///
/// // An empty registry (no installed module declares a validator) accepts everything.
/// let registry = ValidatorRegistry::default();
/// let mut doc = Document {
///     id: uuid::Uuid::new_v4(),
///     scope: Scope::World { world_id: uuid::Uuid::nil() },
///     doc_type: "item".into(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: std::collections::BTreeMap::new(),
///     parent_id: None,
///     engine: None,
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
/// let verdict =
///     validate_document(&registry, &[], &mut doc, None, true, uuid::Uuid::nil(), &[]).await?;
/// assert_eq!(verdict, ValidatorVerdict::Accept);
/// # Ok(())
/// # }
/// ```
pub async fn validate_document(
    registry: &registry::ValidatorRegistry,
    enabled_module_ids: &[String],
    doc: &mut Document,
    prior: Option<&Document>,
    prior_permitted: bool,
    world_id: Uuid,
    schemas: &[SchemaDeclaration],
) -> Result<ValidatorVerdict, DataError> {
    validate_structural(doc, schemas)?;
    let mut ids: Vec<String> = enabled_module_ids.to_vec();
    ids.sort();
    let prior = if prior_permitted { prior } else { None };
    let mut nodes = Vec::new();
    collect_validated_nodes(doc, prior, &mut nodes);
    for node in &nodes {
        for module_id in &ids {
            let Some(compiled) = registry.validator_for(module_id, node.doc_type) else {
                continue;
            };
            let input = ValidatorInput {
                doc_type: node.doc_type.to_string(),
                op: if node.system_prior.is_some() {
                    "update".to_string()
                } else {
                    "create".to_string()
                },
                system: node.system_post.clone(),
                prior: node.system_prior.cloned(),
                name: node.name.map(str::to_string),
                world_id,
                module_id: module_id.clone(),
            };
            match runtime::run_validator(compiled, &input).await {
                ValidatorVerdict::Accept => {
                    registry.reset_faults(world_id, module_id);
                    continue;
                }
                ValidatorVerdict::Refuse { module, reason } => {
                    registry.reset_faults(world_id, &module);
                    return Ok(ValidatorVerdict::Refuse { module, reason });
                }
                ValidatorVerdict::Fault(fault) => {
                    let consecutive = registry.record_fault(world_id, &fault.module);
                    return Ok(ValidatorVerdict::Fault(ValidatorFault {
                        consecutive,
                        ..fault
                    }));
                }
            }
        }
    }
    Ok(ValidatorVerdict::Accept)
}

#[cfg(test)]
mod tests;
